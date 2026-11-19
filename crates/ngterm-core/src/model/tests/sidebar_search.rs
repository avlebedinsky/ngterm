//! The sidebar's session search (this build's own): what it keeps, with companions and the other filters, and its
//! cursor.

use std::collections::HashMap;

use uuid::Uuid;

use super::support::add;
use crate::model::{AppStore, CompanionOptions, SearchHit, SidebarMode};

/// `api` holds `api-server`, its companion `server logs`, and `docs`; `web` holds `frontend` and `Server Admin`.
struct Fixture {
    store: AppStore,
    web: Uuid,
    server: Uuid,
    logs: Uuid,
    docs: Uuid,
    frontend: Uuid,
    admin: Uuid,
}

fn named(store: &mut AppStore, workspace: Uuid, name: &str) -> Uuid {
    let id = add(store, workspace, &format!("/{name}"));
    store.rename_session(id, name);
    id
}

impl Fixture {
    fn new() -> Self {
        let mut store = AppStore::new();
        let api = store.add_workspace("api");
        let web = store.add_workspace("web");
        let server = named(&mut store, api, "api-server");
        let docs = named(&mut store, api, "docs");
        let logs = store
            .add_companion(server, CompanionOptions::default(), "/home/u")
            .expect("the companion is created");
        store.rename_session(logs, "server logs");
        let frontend = named(&mut store, web, "frontend");
        let admin = named(&mut store, web, "Server Admin");
        Self {
            store,
            web,
            server,
            logs,
            docs,
            frontend,
            admin,
        }
    }

    fn search(&mut self, query: &str) -> Option<HashMap<Uuid, SearchHit>> {
        self.store.set_sidebar_search(query);
        self.store.sidebar_search_hits()
    }
}

#[test]
fn an_empty_or_blank_query_keeps_every_row() {
    let mut f = Fixture::new();
    assert_eq!(f.search(""), None);
    assert_eq!(f.search("   "), None);
    assert!(f.store.sidebar_search_matches().is_empty());
    assert_eq!(f.store.sidebar_search_target(), None);
}

#[test]
fn matches_keep_their_workspace_order_and_ignore_case() {
    let mut f = Fixture::new();
    f.store.set_sidebar_search(" SERVER ");
    assert_eq!(f.store.sidebar_search_matches(), [f.server, f.logs, f.admin]);
    assert_eq!(
        f.store.sidebar_search().query(),
        " SERVER ",
        "the field's text is kept as typed"
    );
}

#[test]
fn a_query_matching_nothing_keeps_no_row() {
    let mut f = Fixture::new();
    assert_eq!(f.search("zzz"), Some(HashMap::new()));
    assert_eq!(f.store.sidebar_search_target(), None);
}

#[test]
fn a_workspace_name_alone_does_not_match() {
    let mut f = Fixture::new();
    assert_eq!(f.search("web"), Some(HashMap::new()));
    let hits = f.search("api").expect("active");
    assert_eq!(hits.keys().copied().collect::<Vec<_>>(), [f.server]);
}

#[test]
fn a_matching_companion_brings_its_parent_as_context() {
    let mut f = Fixture::new();
    let hits = f.search("logs").expect("active");
    assert_eq!(
        hits,
        HashMap::from([(f.logs, SearchHit::Match), (f.server, SearchHit::Context)])
    );
    assert_eq!(f.store.sidebar_search_matches(), [f.logs], "a context row is no match");
}

#[test]
fn a_matching_parent_does_not_bring_its_companions() {
    let mut f = Fixture::new();
    let hits = f.search("api-").expect("active");
    assert_eq!(hits, HashMap::from([(f.server, SearchHit::Match)]));
}

#[test]
fn the_focus_filter_narrows_the_search() {
    let mut f = Fixture::new();
    f.store.set_focused_workspace(f.web);
    let hits = f.search("server").expect("active");
    assert_eq!(hits, HashMap::from([(f.admin, SearchHit::Match)]));
}

#[test]
fn the_flagged_view_narrows_the_search_and_adds_no_context() {
    let mut f = Fixture::new();
    f.store.set_flags(true, &[f.logs, f.frontend]);
    f.store.set_sidebar_mode(SidebarMode::Flagged);
    let hits = f.search("s").expect("active");
    assert_eq!(hits, HashMap::from([(f.logs, SearchHit::Match)]));
    assert!(
        f.search("admin").expect("active").is_empty(),
        "an unflagged match stays hidden"
    );
}

#[test]
fn the_search_leaves_navigation_and_the_selection_alone() {
    let mut f = Fixture::new();
    f.store.select_session(Some(f.docs));
    let before = f.store.navigable_session_ids();
    f.store.set_sidebar_search("server");
    assert_eq!(f.store.selected_session_id(), Some(f.docs));
    assert_eq!(f.store.navigable_session_ids(), before);
    assert_eq!(
        before,
        [f.server, f.logs, f.docs, f.frontend, f.admin],
        "every session of both workspaces"
    );
}

#[test]
fn the_cursor_steps_between_matches_and_stops_at_the_ends() {
    let mut f = Fixture::new();
    f.store.set_sidebar_search("server");
    assert_eq!(f.store.sidebar_search_target(), Some(f.server));
    f.store.move_sidebar_search_cursor(false);
    assert_eq!(f.store.sidebar_search_target(), Some(f.server), "the top stops it");
    f.store.move_sidebar_search_cursor(true);
    f.store.move_sidebar_search_cursor(true);
    assert_eq!(f.store.sidebar_search_target(), Some(f.admin));
    f.store.move_sidebar_search_cursor(true);
    assert_eq!(f.store.sidebar_search_target(), Some(f.admin), "the bottom stops it");
    f.store.move_sidebar_search_cursor(false);
    assert_eq!(f.store.sidebar_search_target(), Some(f.logs));
}

#[test]
fn a_new_query_or_a_lost_match_puts_the_cursor_back_on_the_first() {
    let mut f = Fixture::new();
    f.store.set_sidebar_search("server");
    f.store.move_sidebar_search_cursor(true);
    assert_eq!(f.store.sidebar_search_target(), Some(f.logs));
    f.store.set_sidebar_search("server");
    assert_eq!(
        f.store.sidebar_search_target(),
        Some(f.logs),
        "the same text keeps the cursor"
    );
    f.store.rename_session(f.logs, "tail");
    assert_eq!(f.store.sidebar_search_target(), Some(f.server));
    f.store.move_sidebar_search_cursor(true);
    f.store.set_sidebar_search("serve");
    assert_eq!(f.store.sidebar_search_target(), Some(f.server));
}

#[test]
fn the_search_outlives_a_hidden_sidebar() {
    let mut f = Fixture::new();
    f.store.set_sidebar_search("docs");
    f.store.toggle_sidebar_visible();
    f.store.toggle_sidebar_visible();
    assert_eq!(f.store.sidebar_search().query(), "docs");
    assert_eq!(f.store.sidebar_search_matches(), [f.docs]);
}
