use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{OnceLock, mpsc};
use std::task::Poll;

use gpui_kit::{App, Task};
use tempfile::NamedTempFile;

use super::{Applied, materialize};
use crate::state::{AppState, Poster, post};

thread_local! {
    static IMAGES: RefCell<HashMap<PathBuf, Weak<TextImage>>> = RefCell::new(HashMap::new());
    #[cfg(test)]
    static TEST_WORKER: RefCell<Option<mpsc::Sender<Render>>> = const { RefCell::new(None) };
}

enum ImageState {
    Pending,
    Rendered(Option<NamedTempFile>),
    Published(Option<String>),
}

/// A text render that panes share until they change or close.
pub(crate) struct TextImage {
    applied: Applied,
    path: PathBuf,
    state: RefCell<ImageState>,
    task: RefCell<Option<Task<()>>>,
    receiver: async_channel::Receiver<Option<NamedTempFile>>,
}

impl Drop for TextImage {
    fn drop(&mut self) {
        self.receiver.close();
    }
}

impl TextImage {
    pub(crate) fn matches(&self, applied: &Applied) -> bool {
        self.applied == *applied
    }

    pub(crate) fn request(applied: &Applied, poster: Poster, cx: &App) -> Option<Rc<Self>> {
        let path = applied.text_file.clone()?;
        Some(IMAGES.with_borrow_mut(|images| {
            images.retain(|_, image| image.strong_count() > 0);
            if let Some(image) = images
                .get(&path)
                .and_then(Weak::upgrade)
                .filter(|image| image.matches(applied))
            {
                return image;
            }
            let (sender, receiver) = async_channel::bounded(1);
            let image = Rc::new(Self {
                applied: applied.clone(),
                path: path.clone(),
                state: RefCell::new(ImageState::Pending),
                task: RefCell::new(None),
                receiver: receiver.clone(),
            });
            let render = Render {
                applied: applied.clone(),
                sender,
            };
            if submit(render) {
                image.listen(receiver, poster, cx);
            } else {
                *image.state.borrow_mut() = ImageState::Rendered(None);
            }
            images.insert(path, Rc::downgrade(&image));
            image
        }))
    }

    fn listen(self: &Rc<Self>, receiver: async_channel::Receiver<Option<NamedTempFile>>, poster: Poster, cx: &App) {
        let weak = Rc::downgrade(self);
        *self.task.borrow_mut() = Some(cx.foreground_executor().spawn(async move {
            let result = receiver.recv().await.ok().flatten();
            let Some(image) = weak.upgrade() else { return };
            *image.state.borrow_mut() = ImageState::Rendered(result);
            post(&poster, AppState::settle);
        }));
    }

    /// Publishes a completed render only when a live pane still wants it.
    pub(crate) fn resolve(&self) -> Poll<Option<String>> {
        let mut state = self.state.borrow_mut();
        match &*state {
            ImageState::Pending => return Poll::Pending,
            ImageState::Published(path) => return Poll::Ready(path.clone()),
            ImageState::Rendered(_) => {}
        }
        let ImageState::Rendered(file) = std::mem::replace(&mut *state, ImageState::Pending) else {
            unreachable!()
        };
        let path = &self.path;
        let resolved = file.and_then(|file| match file.persist(path) {
            Ok(_) => Some(path.to_string_lossy().into_owned()),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "watermark text PNG install failed");
                None
            }
        });
        *state = ImageState::Published(resolved.clone());
        Poll::Ready(resolved)
    }
}

struct Render {
    applied: Applied,
    sender: async_channel::Sender<Option<NamedTempFile>>,
}

impl Render {
    fn run(self) {
        if self.sender.is_closed() {
            return;
        }
        let result = self.draw();
        let _ = self.sender.send_blocking(result);
    }

    fn draw(&self) -> Option<NamedTempFile> {
        let path = self.applied.text_file.as_ref()?;
        let directory = path.parent()?;
        std::fs::create_dir_all(directory).ok()?;
        let file = NamedTempFile::new_in(directory).ok()?;
        let mut applied = self.applied.clone();
        applied.text_file = Some(file.path().to_owned());
        let state_dir = directory.parent()?;
        materialize(&applied, state_dir).map(|_| file)
    }
}

fn submit(render: Render) -> bool {
    static WORKER: OnceLock<Option<mpsc::Sender<Render>>> = OnceLock::new();
    #[cfg(test)]
    if let Some(sender) = TEST_WORKER.with_borrow(Clone::clone) {
        return sender.send(render).is_ok();
    }
    WORKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Render>();
            match std::thread::Builder::new()
                .name("watermark-text".into())
                .spawn(move || {
                    for render in receiver {
                        render.run();
                    }
                }) {
                Ok(_) => Some(sender),
                Err(error) => {
                    tracing::warn!(%error, "watermark text worker could not start");
                    None
                }
            }
        })
        .as_ref()
        .is_some_and(|worker| worker.send(render).is_ok())
}

#[cfg(test)]
pub(super) mod tests;
