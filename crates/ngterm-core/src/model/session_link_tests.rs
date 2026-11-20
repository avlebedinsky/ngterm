//! Session link namespaces from upstream's `WindowLibrarySessionLinkTests.swift`.

use super::*;
use crate::link_policy::ClickOrigin;
use crate::model::remote_presentation::{RemoteBinding, RemoteOrigin};
use ngterm_proto::ControlLiveEndpoint;

fn row(library: &mut WindowLibrary, window: Uuid, host: Option<&str>) -> Uuid {
    let store = library.store_mut(window).unwrap();
    let workspace = store.current_workspace_id().unwrap();
    store
        .add_session(
            workspace,
            NewSession {
                remote_host: host.map(str::to_owned),
                ..NewSession::at_cwd("/tmp")
            },
        )
        .unwrap()
}

fn endpoint(path: &str) -> ControlLiveEndpoint {
    ControlLiveEndpoint {
        executable: path.into(),
        socket_directory: "/run/live".into(),
        peer: None,
        attach_bridge: None,
    }
}

fn bind(library: &mut WindowLibrary, row: Uuid, id: Uuid, endpoint: Option<ControlLiveEndpoint>) -> Uuid {
    let store = library.store_for_session_mut(row).unwrap();
    let pane = store.session(row).unwrap().pane_identity;
    let binding = RemoteBinding::new(
        id.to_string(),
        HashMap::from([(pane, crate::live::daemon_name(Uuid::new_v4()))]),
        Some(1),
        endpoint.map(|endpoint| RemoteOrigin {
            host: "box".into(),
            endpoint,
            session_name: id.to_string(),
        }),
    );
    store.bind_remote(binding, row);
    pane
}

#[test]
fn session_links_cross_local_windows_and_refuse_closed_or_non_shell_origins() {
    let mut library = WindowLibrary::seeded("/tmp");
    let first = library.active_window_id().unwrap();
    let clicked = row(&mut library, first, None);
    let second = library.new_window(None);
    let target = row(&mut library, second, None);
    for origin in [
        ClickOrigin::Pane(clicked),
        ClickOrigin::RemotePane(clicked),
        ClickOrigin::Scratch(clicked),
    ] {
        assert_eq!(
            library.session_link_target(target, origin, None),
            Some((second, target))
        );
        assert_eq!(library.session_link_target(Uuid::new_v4(), origin, None), None);
    }
    for origin in [
        ClickOrigin::Hud,
        ClickOrigin::ProgramOverlay,
        ClickOrigin::Quick,
        ClickOrigin::Pane(Uuid::new_v4()),
        ClickOrigin::RemotePane(Uuid::new_v4()),
        ClickOrigin::Scratch(Uuid::new_v4()),
    ] {
        assert_eq!(library.session_link_target(target, origin, None), None);
    }
    library.store_mut(second).unwrap().soft_close_session(target);
    assert_eq!(
        library.session_link_target(target, ClickOrigin::Pane(clicked), None),
        None
    );
    let target = row(&mut library, second, None);
    library.close_window(second);
    assert_eq!(
        library.session_link_target(target, ClickOrigin::Pane(clicked), None),
        None
    );
}

#[test]
fn session_links_refuse_duplicate_local_ids_from_restored_windows() {
    let mut library = WindowLibrary::seeded("/tmp");
    let first = library.active_window_id().unwrap();
    let clicked = row(&mut library, first, None);
    let second = library.new_window(None);
    let target = row(&mut library, second, None);
    let snapshot = library.store(second).unwrap().snapshot();
    let third = library.new_window(None);
    library.store_mut(third).unwrap().restore(&snapshot, false);

    for origin in [
        ClickOrigin::Pane(clicked),
        ClickOrigin::RemotePane(clicked),
        ClickOrigin::Scratch(clicked),
    ] {
        assert_eq!(library.session_link_target(target, origin, None), None);
    }

    library.store_mut(third).unwrap().soft_close_session(target);
    assert_eq!(
        library.session_link_target(target, ClickOrigin::Pane(clicked), None),
        Some((second, target))
    );
    library.store_mut(third).unwrap().restore(&snapshot, false);
    assert_eq!(
        library.session_link_target(target, ClickOrigin::Pane(clicked), None),
        None
    );
    library.close_window(second);
    assert_eq!(
        library.session_link_target(target, ClickOrigin::Pane(clicked), None),
        Some((third, target))
    );
    library.close_window(third);
    assert_eq!(
        library.session_link_target(target, ClickOrigin::Pane(clicked), None),
        None
    );
}

#[test]
fn session_links_use_remote_ids_for_mapped_panes_and_local_ids_for_scratch_and_local_splits() {
    let mut library = WindowLibrary::seeded("/tmp");
    let first = library.active_window_id().unwrap();
    let clicked = row(&mut library, first, Some("box"));
    let pane = bind(&mut library, clicked, Uuid::new_v4(), Some(endpoint("/srv/live")));
    let second = library.new_window(None);
    let target = row(&mut library, second, Some("box"));
    let remote_id = Uuid::new_v4();
    bind(&mut library, target, remote_id, Some(endpoint("/srv/live")));
    let local = row(&mut library, second, None);
    for origin in [ClickOrigin::Pane(clicked), ClickOrigin::RemotePane(clicked)] {
        assert_eq!(
            library.session_link_target(remote_id, origin, Some(pane)),
            Some((second, target))
        );
        assert_eq!(library.session_link_target(target, origin, Some(pane)), None);
        assert_eq!(library.session_link_target(local, origin, Some(pane)), None);
        assert_eq!(
            library.session_link_target(local, origin, Some(Uuid::new_v4())),
            Some((second, local))
        );
        assert_eq!(
            library.session_link_target(remote_id, origin, Some(Uuid::new_v4())),
            None
        );
        assert_eq!(library.session_link_target(local, origin, None), None);
    }
    assert_eq!(
        library.session_link_target(local, ClickOrigin::Scratch(clicked), None),
        Some((second, local))
    );
    assert_eq!(
        library.session_link_target(remote_id, ClickOrigin::Scratch(clicked), None),
        None
    );
    let unbound = row(&mut library, first, Some("box"));
    assert_eq!(
        library.session_link_target(local, ClickOrigin::Pane(unbound), Some(pane)),
        None
    );
    bind(&mut library, clicked, Uuid::new_v4(), None);
    assert_eq!(
        library.session_link_target(remote_id, ClickOrigin::Pane(clicked), Some(pane)),
        None
    );
}

#[test]
fn session_links_match_one_open_remote_row_with_the_same_host_and_endpoint() {
    let mut library = WindowLibrary::seeded("/tmp");
    let first = library.active_window_id().unwrap();
    let clicked = row(&mut library, first, Some("box"));
    let pane = bind(&mut library, clicked, Uuid::new_v4(), Some(endpoint("/srv/live")));
    let origin = ClickOrigin::RemotePane(clicked);
    let id = Uuid::new_v4();
    let second = library.new_window(None);
    for (host, endpoint) in [
        ("other", Some(endpoint("/srv/live"))),
        ("box", Some(endpoint("/other/live"))),
        ("box", None),
    ] {
        let row = row(&mut library, second, Some(host));
        bind(&mut library, row, id, endpoint);
    }
    row(&mut library, second, Some("box"));
    assert_eq!(library.session_link_target(id, origin, Some(pane)), None);
    let target = row(&mut library, second, Some("box"));
    bind(&mut library, target, id, Some(endpoint("/srv/live")));
    assert_eq!(
        library.session_link_target(id, origin, Some(pane)),
        Some((second, target))
    );
    let duplicate = row(&mut library, first, Some("box"));
    bind(&mut library, duplicate, id, Some(endpoint("/srv/live")));
    assert_eq!(library.session_link_target(id, origin, Some(pane)), None);
    library.store_mut(first).unwrap().soft_close_session(duplicate);
    assert_eq!(
        library.session_link_target(id, origin, Some(pane)),
        Some((second, target))
    );
    library.close_window(second);
    assert_eq!(library.session_link_target(id, origin, Some(pane)), None);
}

fn catalog_row(library: &mut WindowLibrary, window: Uuid, id: Uuid, has_endpoint: bool) -> Uuid {
    let row = row(library, window, Some("box"));
    let mut binding = RemoteBinding::new(
        id.to_string(),
        HashMap::new(),
        Some(1),
        has_endpoint.then(|| RemoteOrigin {
            host: "box".into(),
            endpoint: endpoint("/srv/live"),
            session_name: id.to_string(),
        }),
    );
    binding.catalog_owned = true;
    library.store_for_session_mut(row).unwrap().bind_remote(binding, row);
    row
}

#[test]
fn session_links_use_origin_ids_for_catalog_scratches_and_match_originless_rows() {
    let mut library = WindowLibrary::seeded("/tmp");
    let window = library.active_window_id().unwrap();
    let clicked = catalog_row(&mut library, window, Uuid::new_v4(), true);
    let id = Uuid::new_v4();
    let target = catalog_row(&mut library, window, id, false);
    let local = row(&mut library, window, None);
    let origin = ClickOrigin::Scratch(clicked);
    assert_eq!(library.session_link_target(id, origin, None), Some((window, target)));
    assert_eq!(library.session_link_target(local, origin, None), None);
    assert_eq!(
        library.session_link_target(id, ClickOrigin::Scratch(target), None),
        None
    );
    let duplicate = catalog_row(&mut library, window, id, false);
    assert_eq!(library.session_link_target(id, origin, None), None);
    library.store_mut(window).unwrap().soft_close_session(duplicate);
    assert_eq!(library.session_link_target(id, origin, None), Some((window, target)));
    bind(&mut library, target, id, Some(endpoint("/srv/live")));
    library
        .store_for_session_mut(target)
        .unwrap()
        .session_mut(target)
        .unwrap()
        .remote_presentation
        .as_mut()
        .unwrap()
        .binding
        .catalog_owned = true;
    assert_eq!(
        library.session_link_target(id, ClickOrigin::Scratch(target), None),
        Some((window, target))
    );
}
