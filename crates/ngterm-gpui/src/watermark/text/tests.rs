use gpui_kit::TestAppContext;
use ngterm_core::chrome::Rgb;
use ngterm_core::model::{BackgroundWatermark, Session, SessionSeed, StatusPane, WatermarkKind};

use super::*;
use crate::state::detached_poster;
use crate::watermark::desired;

pub(crate) struct PausedWorker(mpsc::Receiver<Render>);

impl PausedWorker {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        TEST_WORKER.with_borrow_mut(|worker| {
            assert!(worker.is_none());
            *worker = Some(sender);
        });
        Self(receiver)
    }

    pub(crate) fn complete_next(&self) {
        self.0.try_recv().expect("queued text render").run();
    }
}

impl Drop for PausedWorker {
    fn drop(&mut self) {
        TEST_WORKER.with_borrow_mut(|worker| *worker = None);
    }
}

fn text(state: &std::path::Path) -> Applied {
    let mut session = Session::new(SessionSeed::default());
    session.background_watermark = Some(BackgroundWatermark {
        kind: WatermarkKind::Text,
        text: Some("PROD".into()),
        image_path: None,
        color_hex: None,
        opacity: None,
        fit: None,
        position: None,
        repeats: None,
    });
    desired(&session, StatusPane::Left, Rgb::WHITE, state, 1.0)
}

#[gpui_kit::test]
fn panes_share_a_render_and_keep_the_old_file_until_the_new_image_is_ready(cx: &mut TestAppContext) {
    let worker = PausedWorker::new();
    let state = tempfile::tempdir().unwrap();
    let applied = text(state.path());
    let path = applied.text_file.as_ref().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"old image").unwrap();
    let image = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    let second = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    assert!(Rc::ptr_eq(&image, &second));
    assert!(image.resolve().is_pending());
    worker.complete_next();
    assert!(worker.0.try_recv().is_err(), "the panes share one render");
    assert_eq!(std::fs::read(path).unwrap(), b"old image");
    cx.run_until_parked();
    assert_eq!(image.resolve(), Poll::Ready(Some(path.to_string_lossy().into_owned())));
    assert_eq!(second.resolve(), image.resolve());
    assert!(std::fs::read(path).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[gpui_kit::test]
fn a_closed_pane_cancels_queued_work_and_drops_an_unpublished_image(cx: &mut TestAppContext) {
    let worker = PausedWorker::new();
    let state = tempfile::tempdir().unwrap();
    let applied = text(state.path());
    let path = applied.text_file.as_ref().unwrap();
    let image = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    drop(image);
    worker.complete_next();
    assert!(!path.parent().unwrap().exists());

    let image = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    worker.complete_next();
    cx.run_until_parked();
    assert!(!path.exists(), "a completed render does not publish itself");
    drop(image);
    assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
}

#[gpui_kit::test]
fn a_replacement_cannot_publish_the_previous_text(cx: &mut TestAppContext) {
    let worker = PausedWorker::new();
    let state = tempfile::tempdir().unwrap();
    let applied = text(state.path());
    let old = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    let mut next = applied.clone();
    next.watermark.as_mut().unwrap().text = Some("NEXT".into());
    let image = cx
        .update(|cx| TextImage::request(&next, detached_poster(), cx))
        .unwrap();
    assert!(!Rc::ptr_eq(&old, &image));
    worker.complete_next();
    cx.run_until_parked();
    drop(old);
    assert!(!applied.text_file.as_ref().unwrap().exists());
    assert!(image.resolve().is_pending());
    worker.complete_next();
    cx.run_until_parked();
    assert!(matches!(image.resolve(), Poll::Ready(Some(_))));
}

#[gpui_kit::test]
fn a_failed_render_or_install_reports_no_image(cx: &mut TestAppContext) {
    let worker = PausedWorker::new();
    let state = tempfile::tempdir().unwrap();
    let mut applied = text(state.path());
    applied.watermark.as_mut().unwrap().text = Some(String::new());
    let image = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    worker.complete_next();
    cx.run_until_parked();
    assert_eq!(image.resolve(), Poll::Ready(None));
    assert_eq!(image.resolve(), Poll::Ready(None));

    applied.watermark.as_mut().unwrap().text = Some("VALID".into());
    let path = applied.text_file.as_ref().unwrap();
    std::fs::create_dir_all(path).unwrap();
    let image = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    worker.complete_next();
    cx.run_until_parked();
    assert_eq!(image.resolve(), Poll::Ready(None));
    assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[gpui_kit::test]
fn a_stopped_worker_reports_no_image(cx: &mut TestAppContext) {
    let _worker = PausedWorker::new();
    let state = tempfile::tempdir().unwrap();
    let applied = text(state.path());
    let (sender, receiver) = mpsc::channel();
    drop(receiver);
    TEST_WORKER.with_borrow_mut(|worker| *worker = Some(sender));
    let image = cx
        .update(|cx| TextImage::request(&applied, detached_poster(), cx))
        .unwrap();
    assert_eq!(image.resolve(), Poll::Ready(None));
}
