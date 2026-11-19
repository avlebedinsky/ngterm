use std::path::Path;

use ngterm_host::ProcessStat;
use ngterm_host::fakes::{FakeProcessTable, FakeSurface};
use ngterm_proto::command::{AgentCmd, SessionCmd, WindowCmd, WorkspaceCmd};
use ngterm_proto::{AgentReport, AppIdentity, Args, Command, Request};
use uuid::Uuid;

use super::branch::git_branch;
use super::state::{GotoOutcome, GotoState};
use super::*;
use crate::model::{AgentStatus, CompanionOptions, NewSession, SessionContent, WindowLibrary};
use crate::pane_env::uuid_string;
use crate::persist::AppSettings;
use crate::{Core, CoreConfig, CoreState};

struct Tree {
    workspaces: Vec<GotoWorkspace>,
    ai: Uuid,
    diablo: Uuid,
    api: Uuid,
    nix: Uuid,
    fansee: Uuid,
    empty: Uuid,
}

fn pane(session: Uuid, pane: StatusPane, name: &str) -> GotoPane {
    GotoPane {
        session,
        parent: None,
        pane,
        session_name: name.to_owned(),
        split: false,
        title: None,
        cwd: format!("/src/{name}"),
        agent: None,
        status: PaneStatus::Shell,
        indicator: None,
        current: false,
    }
}

fn with_agent(mut pane: GotoPane, agent: AgentKind, status: PaneStatus) -> GotoPane {
    pane.agent = Some(agent);
    pane.status = status;
    pane
}

fn workspace(id: Uuid, name: &str, branch: Option<&str>, panes: Vec<GotoPane>) -> GotoWorkspace {
    GotoWorkspace {
        id,
        name: name.to_owned(),
        branch: branch.map(str::to_owned),
        panes,
    }
}

fn tree() -> Tree {
    let [ai, diablo, api, nix, fansee, empty] = [(); 6].map(|()| Uuid::new_v4());
    let mut left = with_agent(
        pane(diablo, StatusPane::Left, "diablo 2"),
        AgentKind::Claude,
        PaneStatus::Working,
    );
    let mut right = pane(diablo, StatusPane::Right, "diablo 2");
    left.split = true;
    right.split = true;
    right.title = Some("vim notes".to_owned());
    let workspaces = vec![
        workspace(
            nix,
            "nix-os",
            Some("main"),
            vec![
                with_agent(pane(ai, StatusPane::Left, "ai"), AgentKind::Claude, PaneStatus::Idle),
                left,
                right,
            ],
        ),
        workspace(
            fansee,
            "fansee-wall",
            Some("develop"),
            vec![with_agent(
                pane(api, StatusPane::Left, "api-tests"),
                AgentKind::Codex,
                PaneStatus::Blocked,
            )],
        ),
        workspace(empty, "empty", None, Vec::new()),
    ];
    Tree {
        workspaces,
        ai,
        diablo,
        api,
        nix,
        fansee,
        empty,
    }
}

fn labels(rows: &[GotoRow]) -> Vec<&str> {
    rows.iter().map(|row| row.label.as_str()).collect()
}

fn left(session: Uuid) -> GotoTarget {
    GotoTarget::Pane {
        session,
        pane: StatusPane::Left,
    }
}

fn right(session: Uuid) -> GotoTarget {
    GotoTarget::Pane {
        session,
        pane: StatusPane::Right,
    }
}

#[test]
fn the_tree_lists_every_workspace_its_sessions_and_a_split_sessions_panes() {
    let tree = tree();
    let rows = rows(&tree.workspaces, "", None);
    assert_eq!(
        labels(&rows),
        [
            "nix-os",
            "ai",
            "diablo 2",
            "claude · diablo 2",
            "terminal · diablo 2",
            "fansee-wall",
            "api-tests",
            "empty"
        ]
    );
    assert_eq!(rows[0].target, GotoTarget::Workspace(tree.nix));
    assert_eq!((rows[0].detail.as_str(), rows[0].meta.as_str()), ("nix-os", "main"));
    assert!(rows[0].pane.is_none());
    let columns = |index: usize| rows[index].pane.clone().unwrap();
    assert_eq!((columns(1).agent, columns(1).status.name()), ("claude", "idle"));
    assert_eq!(rows[1].detail, "nix-os / ai");
    assert_eq!(rows[2].target, GotoTarget::Session(tree.diablo));
    assert!(rows[2].pane.is_none(), "a split session's row has no columns");
    assert_eq!(
        (rows[2].detail.as_str(), rows[2].meta.as_str()),
        ("nix-os / diablo 2", "/src/diablo 2")
    );
    assert_eq!((columns(3).agent, columns(3).status.name()), ("claude", "working"));
    assert_eq!((columns(4).agent, columns(4).status.name()), ("terminal", "shell"));
    assert_eq!(rows[4].target, right(tree.diablo));
    assert_eq!(
        (rows[3].detail.as_str(), rows[4].detail.as_str(), rows[4].meta.as_str()),
        (
            "nix-os / diablo 2 / claude",
            "nix-os / diablo 2 / terminal",
            "/src/diablo 2"
        )
    );
    assert_eq!(rows[7].target, GotoTarget::Workspace(tree.empty));
}

fn split_session(session: Uuid, name: &str) -> [GotoPane; 2] {
    [StatusPane::Left, StatusPane::Right].map(|side| GotoPane {
        split: true,
        ..pane(session, side, name)
    })
}

fn drawn(rows: &[GotoRow]) -> Vec<(Vec<&'static str>, &str)> {
    rows.iter().map(|row| (row.tree.clone(), row.label.as_str())).collect()
}

#[test]
fn a_split_sessions_panes_hang_under_its_row_on_a_rail_while_more_sessions_follow() {
    let [home, build, zsh, logs] = [(); 4].map(|()| Uuid::new_v4());
    let mut panes = Vec::from(split_session(build, "build"));
    panes.push(pane(zsh, StatusPane::Left, "zsh"));
    panes.extend(split_session(logs, "logs"));
    let rows = rows(&[workspace(home, "home", None, panes)], "", None);
    assert_eq!(
        drawn(&rows),
        [
            (vec![], "home"),
            (vec!["├─"], "build"),
            (vec!["│", "├─"], "terminal · build"),
            (vec!["│", "└─"], "terminal · build"),
            (vec!["├─"], "zsh"),
            (vec!["└─"], "logs"),
            (vec!["", "├─"], "terminal · logs"),
            (vec!["", "└─"], "terminal · logs"),
        ]
    );
    assert_eq!(rows[1].target, GotoTarget::Session(build));
    assert_eq!(
        [2, 3, 4, 7].map(|index| rows[index].detail.as_str()),
        [
            "home / build / terminal",
            "home / build / terminal",
            "home / zsh",
            "home / logs / terminal"
        ]
    );
}

#[test]
fn a_filter_keeping_one_pane_of_a_split_session_keeps_its_session_row_over_it() {
    let tree = tree();
    let working = rows(&tree.workspaces, "", Some(GotoFilter::Working));
    assert_eq!(
        drawn(&working),
        [
            (vec![], "nix-os"),
            (vec!["└─"], "diablo 2"),
            (vec!["", "└─"], "claude · diablo 2")
        ]
    );
    let searched = rows(&tree.workspaces, "/src/ai", None);
    assert_eq!(drawn(&searched), [(vec![], "nix-os"), (vec!["└─"], "ai")]);
}

fn child(pane: GotoPane, parent: Uuid) -> GotoPane {
    GotoPane {
        parent: Some(parent),
        ..pane
    }
}

#[test]
fn child_sessions_nest_under_their_parent_at_any_depth_in_their_workspaces_session_order() {
    let [home, lav, one, deep, deeper, two, zsh] = [(); 7].map(|()| Uuid::new_v4());
    let panes = vec![
        pane(lav, StatusPane::Left, "lav"),
        child(pane(one, StatusPane::Left, "111"), lav),
        child(pane(deep, StatusPane::Left, "deep"), one),
        child(pane(deeper, StatusPane::Left, "deeper"), deep),
        child(pane(two, StatusPane::Left, "222"), lav),
        pane(zsh, StatusPane::Left, "zsh"),
    ];
    let rows = rows(&[workspace(home, "home", None, panes)], "", None);
    assert_eq!(
        drawn(&rows),
        [
            (vec![], "home"),
            (vec!["├─"], "lav"),
            (vec!["│", "├─"], "111"),
            (vec!["│", "│", "└─"], "deep"),
            (vec!["│", "│", "", "└─"], "deeper"),
            (vec!["│", "└─"], "222"),
            (vec!["└─"], "zsh"),
        ]
    );
    assert_eq!(rows[1].target, left(lav), "an unsplit parent's row is its pane's");
    assert_eq!(
        (rows[4].target, rows[4].detail.as_str()),
        (left(deeper), "home / deeper")
    );
    assert!(
        rows[1..].iter().all(|row| row.pane.is_some()),
        "every session row is a terminal"
    );
}

#[test]
fn a_split_sessions_panes_come_before_its_child_sessions() {
    let [home, build, test, logs, zsh] = [(); 5].map(|()| Uuid::new_v4());
    let mut panes = Vec::from(split_session(build, "build"));
    panes.extend(split_session(test, "test").map(|pane| child(pane, build)));
    panes.push(child(pane(logs, StatusPane::Left, "logs"), build));
    panes.push(pane(zsh, StatusPane::Left, "zsh"));
    let rows = rows(&[workspace(home, "home", None, panes)], "", None);
    assert_eq!(
        drawn(&rows),
        [
            (vec![], "home"),
            (vec!["├─"], "build"),
            (vec!["│", "├─"], "terminal · build"),
            (vec!["│", "├─"], "terminal · build"),
            (vec!["│", "├─"], "test"),
            (vec!["│", "│", "├─"], "terminal · test"),
            (vec!["│", "│", "└─"], "terminal · test"),
            (vec!["│", "└─"], "logs"),
            (vec!["└─"], "zsh"),
        ]
    );
    assert_eq!(
        [1, 3, 4, 6].map(|index| rows[index].target),
        [
            GotoTarget::Session(build),
            right(build),
            GotoTarget::Session(test),
            right(test)
        ]
    );
    assert_eq!(rows[6].detail, "home / test / terminal");
}

#[test]
fn a_session_whose_parent_is_not_above_it_in_its_workspace_stands_at_the_top_as_in_the_sidebar() {
    let [home, work, lav, orphan, late, stray] = [(); 6].map(|()| Uuid::new_v4());
    let workspaces = [
        workspace(
            home,
            "home",
            None,
            vec![
                pane(lav, StatusPane::Left, "lav"),
                child(pane(orphan, StatusPane::Left, "orphan"), Uuid::new_v4()),
                child(pane(late, StatusPane::Left, "late"), lav),
            ],
        ),
        workspace(
            work,
            "work",
            None,
            vec![child(pane(stray, StatusPane::Left, "stray"), lav)],
        ),
    ];
    assert_eq!(
        drawn(&rows(&workspaces, "", None)),
        [
            (vec![], "home"),
            (vec!["├─"], "lav"),
            (vec!["├─"], "orphan"),
            (vec!["└─"], "late"),
            (vec![], "work"),
            (vec!["└─"], "stray"),
        ]
    );
}

struct Family {
    tree: Tree,
    lav: Uuid,
    one: Uuid,
    deep: Uuid,
    two: Uuid,
    build: Uuid,
    test: Uuid,
    zsh: Uuid,
}

fn family() -> Family {
    let [home, lav, one, deep, two, build, test, zsh] = [(); 8].map(|()| Uuid::new_v4());
    let mut panes = vec![
        pane(lav, StatusPane::Left, "lav"),
        child(pane(one, StatusPane::Left, "111"), lav),
        child(
            with_agent(
                pane(deep, StatusPane::Left, "deep"),
                AgentKind::Claude,
                PaneStatus::Blocked,
            ),
            one,
        ),
        child(pane(two, StatusPane::Left, "222"), lav),
    ];
    panes.extend(split_session(build, "build"));
    panes.push(child(
        with_agent(
            pane(test, StatusPane::Left, "test"),
            AgentKind::Codex,
            PaneStatus::Working,
        ),
        build,
    ));
    panes.push(pane(zsh, StatusPane::Left, "zsh"));
    Family {
        tree: Tree {
            workspaces: vec![
                workspace(home, "home", Some("main"), panes),
                workspace(Uuid::new_v4(), "empty", None, Vec::new()),
            ],
            ..tree()
        },
        lav,
        one,
        deep,
        two,
        build,
        test,
        zsh,
    }
}

#[test]
fn a_search_or_a_filter_keeps_the_sessions_over_a_row_it_keeps() {
    let family = family();
    let workspaces = &family.tree.workspaces;
    let searched = rows(workspaces, "deep", None);
    assert_eq!(
        drawn(&searched),
        [
            (vec![], "home"),
            (vec!["└─"], "lav"),
            (vec!["", "└─"], "111"),
            (vec!["", "", "└─"], "deep"),
        ]
    );
    assert_eq!(
        searched[1..]
            .iter()
            .map(|row| (row.target, row.pane.is_some()))
            .collect::<Vec<_>>(),
        [
            (GotoTarget::Session(family.lav), false),
            (GotoTarget::Session(family.one), false),
            (left(family.deep), true)
        ],
        "a session kept over its child is a session row"
    );
    let working = rows(workspaces, "", Some(GotoFilter::Working));
    assert_eq!(
        drawn(&working),
        [(vec![], "home"), (vec!["└─"], "build"), (vec!["", "└─"], "test")]
    );
    assert_eq!(working[1].target, GotoTarget::Session(family.build));

    let mut state = GotoState::opened(None);
    press(&mut state, &family.tree, &[text("b")]);
    let view = state.view(workspaces);
    assert_eq!(labels(&view.rows), ["home", "lav", "111", "deep"]);
    assert_eq!(view.count_label(), "1 terminal");
    let selected_row = view.selected_row().map(|row| (row.target, row.detail.as_str()));
    assert_eq!(
        selected_row,
        Some((left(family.deep), "home / deep")),
        "the first pane row"
    );
    assert_eq!(
        press(&mut state, &family.tree, &[GotoKey::Up, GotoKey::Enter]),
        GotoOutcome::Open(GotoTarget::Session(family.one))
    );
}

#[test]
fn the_selection_stays_on_a_nested_row_across_syncs() {
    let mut family = family();
    let mut state = GotoState::opened(Some(left(family.deep)));
    assert_eq!(state.view(&family.tree.workspaces).selected, Some(3));
    let panes = &mut family.tree.workspaces[0].panes;
    panes.insert(0, pane(Uuid::new_v4(), StatusPane::Left, "new"));
    panes.insert(3, child(pane(Uuid::new_v4(), StatusPane::Left, "sibling"), family.one));
    let view = state.view(&family.tree.workspaces);
    assert_eq!(
        labels(&view.rows[..7]),
        ["home", "new", "lav", "111", "sibling", "deep", "222"]
    );
    assert_eq!(view.selected, Some(5));
    press(&mut state, &family.tree, &[text("j")]);
    assert_eq!(selected(&state, &family.tree), Some(left(family.two)));
    press(&mut state, &family.tree, &[text("j")]);
    assert_eq!(selected(&state, &family.tree), Some(GotoTarget::Session(family.build)));
    let state = GotoState::opened(Some(GotoTarget::Session(family.lav)));
    assert_eq!(
        selected(&state, &family.tree),
        Some(left(family.lav)),
        "an unsplit session's row is its pane's"
    );
}

fn holding_the_status(mut family: Family) -> Family {
    let panes = &mut family.tree.workspaces[0].panes;
    panes[4].current = true;
    panes[5] = with_agent(panes[5].clone(), AgentKind::Claude, PaneStatus::Blocked);
    panes[5].indicator = Some(AgentIndicator {
        status: AgentStatus::Blocked,
        ..AgentIndicator::default()
    });
    family
}

#[test]
fn the_top_level_filter_keeps_every_workspace_with_its_top_level_sessions_alone() {
    let mut family = holding_the_status(family());
    let api = Uuid::new_v4();
    let work = workspace(Uuid::new_v4(), "work", None, Vec::from(split_session(api, "api")));
    family.tree.workspaces.insert(1, work);
    let mut state = GotoState::opened(None);
    press(&mut state, &family.tree, &[text("t")]);
    let view = state.view(&family.tree.workspaces);
    assert_eq!(
        drawn(&view.rows),
        [
            (vec![], "home"),
            (vec!["├─"], "lav"),
            (vec!["├─"], "build"),
            (vec!["└─"], "zsh"),
            (vec![], "work"),
            (vec!["└─"], "api"),
            (vec![], "empty"),
        ]
    );
    let build = &view.rows[2];
    assert_eq!(
        (build.target, build.detail.as_str()),
        (GotoTarget::Session(family.build), "home / build")
    );
    let columns = build.pane.clone().unwrap();
    assert_eq!(
        (columns.agent, columns.status, columns.current),
        ("claude", PaneStatus::Blocked, true),
        "a split session's row reads its pane that holds the status"
    );
    assert!(columns.indicator.is_some());
    assert_eq!(
        (view.search_line().as_str(), view.count_label().as_str()),
        ("/ top level", "6 terminals")
    );
    assert!(!view.shows_placeholder());
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(family.lav)));
    assert_eq!(
        press(&mut state, &family.tree, &[text("j"), GotoKey::Enter]),
        GotoOutcome::Open(GotoTarget::Session(family.build))
    );
    press(&mut state, &family.tree, &[GotoKey::Right]);
    assert_eq!(
        selected(&state, &family.tree),
        Some(GotoTarget::Session(api)),
        "a workspace of split sessions is a stop"
    );
    press(&mut state, &family.tree, &[GotoKey::Left]);
    assert_eq!(selected(&state, &family.tree), Some(left(family.lav)));
    press(&mut state, &family.tree, &[GotoKey::Left]);
    assert_eq!(
        selected(&state, &family.tree),
        Some(left(family.lav)),
        "nothing before it"
    );
    press(&mut state, &family.tree, &[GotoKey::Backspace]);
    assert_eq!(
        state.view(&family.tree.workspaces).rows.len(),
        15,
        "Backspace drops the filter"
    );
}

#[test]
fn the_top_level_filter_moves_a_selection_it_hides_to_its_top_level_session() {
    let mut family = family();
    for (hidden, shown) in [
        (left(family.deep), left(family.lav)),
        (right(family.build), GotoTarget::Session(family.build)),
        (left(family.test), GotoTarget::Session(family.build)),
        (left(family.zsh), left(family.zsh)),
    ] {
        let mut state = GotoState::opened(Some(hidden));
        press(&mut state, &family.tree, &[text("t")]);
        assert_eq!(selected(&state, &family.tree), Some(shown), "{hidden:?}");
    }
    let mut state = GotoState::opened(Some(left(family.deep)));
    press(&mut state, &family.tree, &[text("t"), text("j")]);
    assert_eq!(selected(&state, &family.tree), Some(GotoTarget::Session(family.build)));
    let mut state = GotoState::opened(Some(left(family.zsh)));
    press(&mut state, &family.tree, &[text("t")]);
    let panes = &mut family.tree.workspaces[0].panes;
    let zsh = panes.len() - 1;
    panes[zsh].split = true;
    panes.push(GotoPane {
        pane: StatusPane::Right,
        ..panes[zsh].clone()
    });
    assert_eq!(
        selected(&state, &family.tree),
        Some(GotoTarget::Session(family.zsh)),
        "a sync that splits the session keeps its row"
    );
    assert_eq!(
        press(&mut state, &family.tree, &[GotoKey::Enter]),
        GotoOutcome::Open(GotoTarget::Session(family.zsh))
    );
}

#[test]
fn the_top_level_filter_keeps_a_selected_workspace_row() {
    let family = family();
    let home = GotoTarget::Workspace(family.tree.workspaces[0].id);
    let mut state = GotoState::opened(Some(home));
    press(&mut state, &family.tree, &[text("t")]);
    let view = state.view(&family.tree.workspaces);
    assert_eq!(view.filter, Some(GotoFilter::TopLevel));
    assert_eq!(view.selected_row().map(|row| row.target), Some(home));
}

#[test]
fn the_top_level_filter_keeps_a_selection_whose_session_is_gone_on_the_first_row() {
    let mut family = family();
    let mut state = GotoState::opened(Some(left(family.zsh)));
    family.tree.workspaces[0].panes.pop();
    press(&mut state, &family.tree, &[text("t")]);
    let view = state.view(&family.tree.workspaces);
    assert_eq!(labels(&view.rows), ["home", "lav", "build", "empty"]);
    assert_eq!(view.selected, Some(0));
    press(&mut state, &family.tree, &[text("j")]);
    assert_eq!(selected(&state, &family.tree), Some(left(family.lav)));
}

#[test]
fn a_status_filter_after_the_top_level_filter_drops_the_selection_it_lifted() {
    let family = family();
    for (opened, key, filter, first_pane) in [
        (left(family.test), "b", GotoFilter::Blocked, left(family.deep)),
        (left(family.deep), "w", GotoFilter::Working, left(family.test)),
    ] {
        let mut state = GotoState::opened(Some(opened));
        press(&mut state, &family.tree, &[text("t"), text(key)]);
        let view = state.view(&family.tree.workspaces);
        assert_eq!(view.filter, Some(filter), "{key}");
        assert_eq!(view.selected_row().map(|row| row.target), Some(first_pane), "{key}");
    }
}

#[test]
fn leaving_the_top_level_filter_keeps_its_selection() {
    let family = family();
    for (opened, leave, kept) in [
        (left(family.test), GotoKey::Backspace, GotoTarget::Session(family.build)),
        (left(family.zsh), text("a"), left(family.zsh)),
    ] {
        let mut state = GotoState::opened(Some(opened));
        press(&mut state, &family.tree, &[text("t"), leave.clone()]);
        let view = state.view(&family.tree.workspaces);
        assert_eq!(view.filter, None, "{leave:?}");
        assert_eq!(view.selected_row().map(|row| row.target), Some(kept), "{leave:?}");
    }
}

#[test]
fn under_the_top_level_filter_a_row_is_current_while_a_pane_under_it_is_focused() {
    let mut family = family();
    let mut state = GotoState::opened(None);
    press(&mut state, &family.tree, &[text("t")]);
    for (focused, row) in [
        (family.deep, left(family.lav)),
        (family.test, GotoTarget::Session(family.build)),
        (family.zsh, left(family.zsh)),
    ] {
        for pane in &mut family.tree.workspaces[0].panes {
            pane.current = pane.session == focused;
        }
        let view = state.view(&family.tree.workspaces);
        let marked: Vec<GotoTarget> = view
            .rows
            .iter()
            .filter(|row| row.pane.as_ref().is_some_and(|pane| pane.current))
            .map(|row| row.target)
            .collect();
        assert_eq!(marked, [row]);
    }
}

#[test]
fn a_selected_pane_a_sync_filters_out_keeps_the_selection_on_its_session_kept_over_a_child() {
    let mut family = family();
    let lav = &mut family.tree.workspaces[0].panes[0];
    *lav = with_agent(lav.clone(), AgentKind::Claude, PaneStatus::Blocked);
    let mut state = GotoState::opened(None);
    press(&mut state, &family.tree, &[text("b"), text("j"), text("k")]);
    assert_eq!(selected(&state, &family.tree), Some(left(family.lav)));
    family.tree.workspaces[0].panes[0].status = PaneStatus::Idle;
    let view = state.view(&family.tree.workspaces);
    assert_eq!(labels(&view.rows), ["home", "lav", "111", "deep"]);
    assert_eq!(
        view.selected_row().map(|row| row.target),
        Some(GotoTarget::Session(family.lav))
    );
}

#[test]
fn every_word_of_the_query_must_be_in_one_field() {
    let tree = tree();
    let found = |query: &str| {
        rows(&tree.workspaces, query, None)
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(found("DIABLO claude"), ["nix-os", "diablo 2", "claude · diablo 2"]);
    assert_eq!(found("codex"), ["fansee-wall", "api-tests"], "the agent");
    assert_eq!(
        found("notes vim"),
        ["nix-os", "diablo 2", "terminal · diablo 2"],
        "the pane's title"
    );
    assert_eq!(found("/src/ai"), ["nix-os", "ai"], "the cwd");
    assert!(found("diablo codex").is_empty(), "two fields");
    assert_eq!(
        found("develop"),
        ["fansee-wall", "api-tests"],
        "a matching branch keeps its workspace's panes"
    );
    assert_eq!(
        found("empty"),
        ["empty"],
        "a matching workspace shows even with no pane"
    );
    assert!(found("nothing here").is_empty());
    assert_eq!(found("   ").len(), 8, "a blank query keeps everything");
}

#[test]
fn a_filter_keeps_the_panes_of_its_status_and_never_a_shell() {
    let tree = tree();
    let filtered = |filter| labels(&rows(&tree.workspaces, "", Some(filter))).join("|");
    assert_eq!(filtered(GotoFilter::Blocked), "fansee-wall|api-tests");
    assert_eq!(filtered(GotoFilter::Working), "nix-os|diablo 2|claude · diablo 2");
    assert_eq!(filtered(GotoFilter::Idle), "nix-os|ai");
    assert_eq!(filtered(GotoFilter::Done), "");
}

#[test]
fn the_selection_falls_back_to_the_first_pane_row() {
    let tree = tree();
    let rows = rows(&tree.workspaces, "", None);
    assert_eq!(selected_index(&rows, None), Some(1));
    assert_eq!(selected_index(&rows, Some(left(tree.api))), Some(6));
    assert_eq!(selected_index(&rows, Some(left(Uuid::new_v4()))), Some(0), "gone");
    assert_eq!(selected_index(&[], None), None);
}

#[test]
fn a_selected_session_that_is_no_longer_split_keeps_the_selection_on_its_pane() {
    let mut tree = tree();
    let state = GotoState::opened(Some(GotoTarget::Session(tree.diablo)));
    assert_eq!(selected(&state, &tree), Some(GotoTarget::Session(tree.diablo)));
    let panes = &mut tree.workspaces[0].panes;
    panes.truncate(2);
    panes[1].split = false;
    assert_eq!(selected(&state, &tree), Some(left(tree.diablo)));
}

fn press(state: &mut GotoState, tree: &Tree, keys: &[GotoKey]) -> GotoOutcome {
    keys.iter()
        .map(|key| state.key(key.clone(), &tree.workspaces))
        .last()
        .unwrap_or(GotoOutcome::Stay)
}

fn text(key: &str) -> GotoKey {
    GotoKey::Text {
        text: key.to_owned(),
        key: key.to_owned(),
    }
}

fn selected(state: &GotoState, tree: &Tree) -> Option<GotoTarget> {
    let view = state.view(&tree.workspaces);
    view.selected_row().map(|row| row.target)
}

#[test]
fn the_list_keys_walk_the_rows() {
    let tree = tree();
    let mut state = GotoState::opened(Some(right(tree.diablo)));
    assert_eq!(selected(&state, &tree), Some(right(tree.diablo)));
    press(&mut state, &tree, &[text("j")]);
    assert_eq!(selected(&state, &tree), Some(GotoTarget::Workspace(tree.fansee)));
    press(&mut state, &tree, &[text("k"), GotoKey::Up]);
    assert_eq!(selected(&state, &tree), Some(left(tree.diablo)));
    press(&mut state, &tree, &[GotoKey::Control('d')]);
    assert_eq!(
        selected(&state, &tree),
        Some(GotoTarget::Workspace(tree.empty)),
        "held at the end"
    );
    press(&mut state, &tree, &[GotoKey::Control('u')]);
    assert_eq!(
        selected(&state, &tree),
        Some(GotoTarget::Workspace(tree.nix)),
        "held at the top"
    );
    press(&mut state, &tree, &[GotoKey::Down, GotoKey::End]);
    assert_eq!(selected(&state, &tree), Some(GotoTarget::Workspace(tree.empty)));
    press(&mut state, &tree, &[GotoKey::Home]);
    assert_eq!(selected(&state, &tree), Some(left(tree.ai)), "the first pane row");
    press(&mut state, &tree, &[text("j")]);
    assert_eq!(
        selected(&state, &tree),
        Some(GotoTarget::Session(tree.diablo)),
        "a split session's row"
    );
    assert_eq!(
        press(&mut state, &tree, &[GotoKey::Enter]),
        GotoOutcome::Open(GotoTarget::Session(tree.diablo))
    );
    press(&mut state, &tree, &[text("G")]);
    assert_eq!(selected(&state, &tree), Some(GotoTarget::Workspace(tree.empty)));
    assert_eq!(
        press(&mut state, &tree, &[GotoKey::Home, GotoKey::Enter]),
        GotoOutcome::Open(left(tree.ai))
    );
    assert_eq!(
        press(&mut state, &tree, &[text("x"), GotoKey::Escape]),
        GotoOutcome::Close
    );
}

#[test]
fn left_and_right_jump_to_the_first_pane_of_a_workspace_with_panes() {
    let tree = tree();
    let mut state = GotoState::opened(Some(right(tree.diablo)));
    press(&mut state, &tree, &[GotoKey::Right]);
    assert_eq!(selected(&state, &tree), Some(left(tree.api)));
    press(&mut state, &tree, &[GotoKey::Right]);
    assert_eq!(
        selected(&state, &tree),
        Some(left(tree.api)),
        "the empty workspace is no stop"
    );
    press(&mut state, &tree, &[GotoKey::Left]);
    assert_eq!(selected(&state, &tree), Some(left(tree.ai)));
    press(&mut state, &tree, &[GotoKey::Left]);
    assert_eq!(selected(&state, &tree), Some(left(tree.ai)), "nothing before it");
}

#[test]
fn a_jump_lands_on_the_first_pane_under_a_split_session() {
    let [home, zsh, work, build] = [(); 4].map(|()| Uuid::new_v4());
    let tree = Tree {
        workspaces: vec![
            workspace(home, "home", None, vec![pane(zsh, StatusPane::Left, "zsh")]),
            workspace(work, "work", None, Vec::from(split_session(build, "build"))),
        ],
        ..tree()
    };
    let mut state = GotoState::opened(Some(left(zsh)));
    press(&mut state, &tree, &[GotoKey::Right]);
    assert_eq!(selected(&state, &tree), Some(left(build)));
    press(&mut state, &tree, &[GotoKey::Left]);
    assert_eq!(selected(&state, &tree), Some(left(zsh)));
}

#[test]
fn a_filter_key_sets_the_filter_and_backspace_or_a_drops_it() {
    let tree = tree();
    let mut state = GotoState::opened(None);
    press(&mut state, &tree, &[text("b")]);
    let view = state.view(&tree.workspaces);
    assert_eq!(labels(&view.rows), ["fansee-wall", "api-tests"]);
    assert_eq!(
        (view.search_line().as_str(), view.count_label().as_str()),
        ("/ blocked", "1 terminal")
    );
    assert!(!view.shows_placeholder());
    press(&mut state, &tree, &[GotoKey::Backspace]);
    assert_eq!(state.view(&tree.workspaces).rows.len(), 8, "Backspace drops the filter");
    press(&mut state, &tree, &[text("i"), text("a")]);
    let view = state.view(&tree.workspaces);
    assert_eq!((view.rows.len(), view.search_line().as_str()), (8, PLACEHOLDER));
    assert!(view.shows_placeholder());
    assert_eq!(view.count_label(), "4 terminals");
}

#[test]
fn the_search_types_moves_and_keeps_its_query_after_escape() {
    let tree = tree();
    let mut state = GotoState::opened(None);
    press(&mut state, &tree, &[text("w"), text("/")]);
    let view = state.view(&tree.workspaces);
    assert!(view.searching && view.filter.is_none(), "searching clears the filter");
    assert_eq!((view.search_line().as_str(), view.hints()), ("/ ", SEARCH_HINTS));
    press(&mut state, &tree, &[text("a"), text("p"), text("j")]);
    let view = state.view(&tree.workspaces);
    assert_eq!(view.query, "apj", "letters type into the query");
    assert!(view.rows.is_empty());
    press(&mut state, &tree, &[GotoKey::Backspace]);
    let view = state.view(&tree.workspaces);
    assert_eq!(labels(&view.rows), ["fansee-wall", "api-tests"]);
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(tree.api)));
    press(&mut state, &tree, &[GotoKey::Control('p')]);
    assert_eq!(selected(&state, &tree), Some(GotoTarget::Workspace(tree.fansee)));
    press(&mut state, &tree, &[GotoKey::Control('n')]);
    assert_eq!(selected(&state, &tree), Some(left(tree.api)));
    assert_eq!(press(&mut state, &tree, &[GotoKey::Escape]), GotoOutcome::Stay);
    let view = state.view(&tree.workspaces);
    assert_eq!(
        (view.searching, view.search_line().as_str(), view.hints()),
        (false, "/ ap", HINTS),
        "Escape leaves the query"
    );
    press(&mut state, &tree, &[text("d")]);
    assert_eq!(state.view(&tree.workspaces).query, "", "a filter clears the query");
    assert_eq!(
        press(&mut state, &tree, &[text("a"), text("/"), GotoKey::Enter]),
        GotoOutcome::Open(left(tree.ai))
    );
}

#[test]
fn the_list_reads_the_key_and_the_query_takes_the_typed_text() {
    let tree = tree();
    let russian = |text: &str, key: &str| GotoKey::Text {
        text: text.to_owned(),
        key: key.to_owned(),
    };
    let mut state = GotoState::opened(Some(left(tree.ai)));
    press(&mut state, &tree, &[russian("о", "j")]);
    assert_eq!(selected(&state, &tree), Some(GotoTarget::Session(tree.diablo)));
    press(&mut state, &tree, &[russian("и", "b")]);
    assert_eq!(state.view(&tree.workspaces).filter, Some(GotoFilter::Blocked));
    press(&mut state, &tree, &[russian(".", "/"), russian("о", "j")]);
    let view = state.view(&tree.workspaces);
    assert!(view.searching);
    assert_eq!(view.query, "о");
}

#[test]
fn enter_on_no_row_stays_and_a_search_with_no_rows_keeps_no_selection() {
    let tree = tree();
    let mut state = GotoState::opened(None);
    press(&mut state, &tree, &[text("d")]);
    assert_eq!(
        press(&mut state, &tree, &[GotoKey::Down, GotoKey::Enter]),
        GotoOutcome::Stay
    );
    assert_eq!(state.view(&tree.workspaces).selected, None);
    press(&mut state, &tree, &[GotoKey::Right]);
    assert_eq!(state.view(&tree.workspaces).selected, None);
}

#[test]
fn a_target_names_its_row() {
    let session = Uuid::new_v4();
    assert_eq!(right(session).id(), format!("{}:right", uuid_string(session)));
    assert_eq!(GotoTarget::Workspace(session).id(), uuid_string(session));
    assert_eq!(GotoTarget::Session(session).id(), uuid_string(session));
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_branch_is_read_from_head_up_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    write(&repo.join(".git/HEAD"), "ref: refs/heads/feature/goto\n");
    std::fs::create_dir_all(repo.join("src/deep")).unwrap();
    assert_eq!(git_branch(&repo).as_deref(), Some("feature/goto"));
    assert_eq!(git_branch(&repo.join("src/deep")).as_deref(), Some("feature/goto"));
    assert_eq!(
        git_branch(&repo.join("src/deep/missing.rs")).as_deref(),
        Some("feature/goto")
    );
    assert_eq!(git_branch(dir.path()), None, "outside the repository");
    write(&repo.join(".git/HEAD"), "4b825dc642cb6eb9a060e54bf8d69288fbee4904\n");
    assert_eq!(git_branch(&repo), None, "a detached HEAD");
    write(&repo.join(".git/HEAD"), "ref: refs/heads/\n");
    assert_eq!(git_branch(&repo), None);
}

#[test]
fn a_worktree_follows_its_gitdir_file() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    write(&main.join(".git/HEAD"), "ref: refs/heads/main\n");
    write(&main.join(".git/worktrees/fix/HEAD"), "ref: refs/heads/fix-goto\n");
    let absolute = dir.path().join("fix");
    write(
        &absolute.join(".git"),
        &format!("gitdir: {}\n", main.join(".git/worktrees/fix").display()),
    );
    assert_eq!(git_branch(&absolute).as_deref(), Some("fix-goto"));
    let relative = main.join("nested");
    write(&relative.join(".git"), "gitdir: ../.git/worktrees/fix\n");
    assert_eq!(git_branch(&relative).as_deref(), Some("fix-goto"));
    let broken = dir.path().join("broken");
    write(&broken.join(".git"), "not a link\n");
    assert_eq!(git_branch(&broken), None);
    write(&broken.join(".git"), &"x".repeat(70_000));
    assert_eq!(git_branch(&broken), None, "an oversized file is not read");
}

fn core(home: &str) -> (Core, Uuid) {
    let library = WindowLibrary::seeded(home);
    let window = library.active_window_id().unwrap();
    let core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: AppSettings::default(),
            persistence: None,
        },
    );
    (core, window)
}

fn send(core: &mut Core, cmd: Command, target: Option<Uuid>, args: Args) {
    let response = core.dispatch(&Request {
        cmd,
        target: target.map(uuid_string),
        args: Some(args),
    });
    assert!(response.ok, "{response:?}");
}

fn split(core: &mut Core, session: Uuid) {
    let args = Args {
        mode: Some("on".to_owned()),
        ..Args::default()
    };
    send(core, Command::Session(SessionCmd::Split), Some(session), args);
}

fn report_claude(core: &mut Core, session: Uuid, identity: Uuid, role: &str) {
    let args = Args {
        pane_id: Some(uuid_string(identity)),
        pane: Some(role.to_owned()),
        report: Some(Box::new(AgentReport {
            agent: "claude".to_owned(),
            event: "SessionStart".to_owned(),
            transcript_path: Some("/home/user/.claude/projects/-home-user-project/aaaa.jsonl".to_owned()),
            conversation_id: Some("aaaa".to_owned()),
            approval: None,
        })),
        ..Args::default()
    };
    send(core, Command::Agent(AgentCmd::Report), Some(session), args);
}

fn status(core: &mut Core, session: Uuid, status: &str, pane: &str) {
    let args = Args {
        status: Some(status.to_owned()),
        pane: Some(pane.to_owned()),
        ..Args::default()
    };
    send(core, Command::Session(SessionCmd::Status), Some(session), args);
}

fn selected_session(core: &Core, window: Uuid) -> Option<Uuid> {
    core.library().store(window)?.selected_session_id()
}

#[test]
fn the_window_reads_the_panes_agents_statuses_and_branch_and_opens_the_pane() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".git/HEAD"), "ref: refs/heads/main\n");
    let home = dir.path().to_string_lossy().into_owned();
    let (mut core, window) = core(&home);
    let first = selected_session(&core, window).unwrap();
    split(&mut core, first);
    let split_pane = core
        .library()
        .store(window)
        .and_then(|store| store.session(first))
        .and_then(|session| session.split_pane_identity)
        .unwrap();
    report_claude(&mut core, first, split_pane, "right");
    status(&mut core, first, "blocked", "right");
    let store = core.library_mut().store_mut(window).unwrap();
    let workspace = store.current_workspace_id().unwrap();
    let other = store
        .add_session(
            workspace,
            NewSession {
                name: Some("logs".to_owned()),
                select: false,
                ..NewSession::at_cwd("/var/log")
            },
        )
        .unwrap();
    store.set_split_focus(first, false);
    let _ = core.commit();

    core.toggle_goto(window);
    assert!(core.goto_holds(window));
    let view = core.goto_view(window).unwrap();
    let store = core.library().store(window).unwrap();
    let name = store.session(first).unwrap().display_name();
    assert_eq!(
        labels(&view.rows),
        [
            store.workspaces()[0].name.as_str(),
            name.as_str(),
            format!("terminal · {name}").as_str(),
            format!("claude · {name}").as_str(),
            "logs"
        ]
    );
    assert_eq!(view.rows[0].meta, "main", "the branch where the first session started");
    assert_eq!(
        view.rows[3].detail,
        format!("{} / {name} / claude", store.workspaces()[0].name)
    );
    let columns: Vec<(&str, &str, bool)> = view.rows[2..]
        .iter()
        .map(|row| {
            let pane = row.pane.as_ref().unwrap();
            (pane.agent, pane.status.name(), pane.current)
        })
        .collect();
    assert_eq!(
        columns,
        [
            ("terminal", "shell", true),
            ("claude", "blocked", false),
            ("terminal", "shell", false)
        ]
    );
    let indicator = view.rows[3].pane.as_ref().unwrap().indicator.as_ref();
    assert_eq!(indicator.map(|indicator| indicator.status), Some(AgentStatus::Blocked));
    assert_eq!(view.selected, Some(2), "on the focused pane");

    core.goto_key(window, GotoKey::Down);
    core.goto_key(window, GotoKey::Enter);
    assert!(!core.goto_holds(window));
    let session = core.library().store(window).unwrap().session(first).unwrap();
    assert!(session.split.focused, "the split pane took the focus");

    core.toggle_goto(window);
    core.goto_open(window, left(other));
    assert_eq!(selected_session(&core, window), Some(other));
    assert!(core.goto_view(window).is_none());

    core.toggle_goto(window);
    core.goto_open(window, GotoTarget::Session(first));
    assert_eq!(selected_session(&core, window), Some(first));
    let session = core.library().store(window).unwrap().session(first).unwrap();
    assert!(session.split.focused, "the session keeps its focused pane");
}

#[test]
fn a_status_on_the_other_pane_reads_unknown_and_a_remote_workspace_has_no_branch() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".git/HEAD"), "ref: refs/heads/main\n");
    let cwd = dir.path().to_string_lossy().into_owned();
    let (mut core, window) = core("/");
    let first = selected_session(&core, window).unwrap();
    let identity = core
        .library()
        .store(window)
        .and_then(|store| store.session(first))
        .unwrap()
        .pane_identity;
    report_claude(&mut core, first, identity, "left");
    split(&mut core, first);
    status(&mut core, first, "active", "right");
    let store = core.library_mut().store_mut(window).unwrap();
    let remote = store.add_workspace("remote");
    store.add_session(
        remote,
        NewSession {
            remote_host: Some("server".to_owned()),
            select: false,
            ..NewSession::at_cwd(cwd)
        },
    );
    core.toggle_goto(window);
    let view = core.goto_view(window).unwrap();
    let main_pane = view.rows[2].pane.clone().unwrap();
    assert_eq!((main_pane.agent, main_pane.status), ("claude", PaneStatus::Unknown));
    assert_eq!(
        view.rows[3].pane.as_ref().unwrap().status,
        PaneStatus::Working,
        "the pane holding the status"
    );
    assert_eq!(view.rows[4].label, "remote");
    assert_eq!(view.rows[4].meta, "", "no branch is read for a remote session");
    status(&mut core, first, "completed", "left");
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.rows[2].pane.as_ref().unwrap().status, PaneStatus::Done, "live");
    status(&mut core, first, "idle", "left");
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.rows[2].pane.as_ref().unwrap().status, PaneStatus::Idle);
}

#[test]
fn the_window_closes_on_its_chord_escape_or_a_cover_and_opens_only_where_it_can() {
    let (mut core, window) = core("/tmp");
    let session = selected_session(&core, window).unwrap();
    core.toggle_goto(window);
    assert!(core.goto_holds(window));
    core.toggle_goto(window);
    assert!(!core.goto_holds(window), "the chord again closes it");

    core.toggle_goto(window);
    core.goto_key(window, text("/"));
    core.goto_key(window, GotoKey::Escape);
    assert!(core.goto_holds(window), "the first Escape leaves the search");
    core.goto_key(window, GotoKey::Escape);
    assert!(!core.goto_holds(window));
    assert_eq!(selected_session(&core, window), Some(session));

    core.toggle_goto(window);
    core.open_palette(window, crate::palette::PaletteMode::Actions);
    let _ = core.commit();
    assert!(!core.goto_holds(window), "the palette takes over");
    core.toggle_goto(window);
    assert!(core.goto_holds(window));
    assert_eq!(core.palette_mode(), None, "and gives way to it");

    let grid = Args {
        targets: Some(vec![uuid_string(session)]),
        ..Args::default()
    };
    send(&mut core, Command::Dashboard, None, grid);
    let _ = core.commit();
    assert!(!core.goto_holds(window), "a dashboard covers the window");
    core.toggle_goto(window);
    assert!(!core.goto_holds(window), "nor opens under one");
    let close = Args {
        close: Some(true),
        ..Args::default()
    };
    send(&mut core, Command::Dashboard, None, close);

    let other = core.library_mut().new_window(None);
    let behind = if core.library().active_window_id() == Some(other) {
        window
    } else {
        other
    };
    core.toggle_goto(behind);
    assert!(!core.goto_holds(behind), "only in the frontmost window");
    core.goto_key(behind, GotoKey::Escape);
    core.goto_open(behind, left(session));
    assert!(core.goto_view(behind).is_none());
}

#[test]
fn the_chord_and_the_menu_row_toggle_the_window() {
    use crate::app_actions::{ActionEffect, ActionOrigin};
    use crate::keymap::BuiltinAction;

    let (mut core, window) = core("/tmp");
    let origin = ActionOrigin { window, quick: false };
    assert_eq!(core.perform_builtin(origin, BuiltinAction::GoTo), ActionEffect::Done);
    assert!(core.goto_holds(window));
    core.perform_menu_command(window, crate::palette_catalog::PaletteCommand::GoTo);
    assert!(!core.goto_holds(window));
}

fn background(name: &str, cwd: &str) -> NewSession {
    NewSession {
        name: Some(name.to_owned()),
        select: false,
        ..NewSession::at_cwd(cwd)
    }
}

#[test]
fn a_page_companion_gets_no_row_and_no_count() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let store = core.library_mut().store_mut(window).unwrap();
    let workspace = store.current_workspace_id().unwrap();
    let page = store
        .add_session(
            workspace,
            NewSession {
                parent: Some(first),
                content: SessionContent::Page,
                ..background("docs", "/tmp")
            },
        )
        .unwrap();
    let _ = core.commit();

    core.toggle_goto(window);
    let view = core.goto_view(window).unwrap();
    let store = core.library().store(window).unwrap();
    assert_eq!(
        labels(&view.rows),
        [
            store.workspaces()[0].name.as_str(),
            store.session(first).unwrap().display_name().as_str()
        ]
    );
    assert!(view.rows.iter().all(|row| row.target != left(page)));
    assert_eq!(view.count_label(), "1 terminal");
}

#[test]
fn enter_on_a_workspace_row_selects_its_first_session_and_focuses_its_pane() {
    let (mut core, window) = core("/tmp");
    let store = core.library_mut().store_mut(window).unwrap();
    let other = store.add_workspace("other");
    let [logs, build] = ["logs", "build"].map(|name| store.add_session(other, background(name, "/var/log")).unwrap());
    let _ = core.commit();
    core.clear_pane_focus_requests();

    core.toggle_goto(window);
    core.goto_key(window, GotoKey::Home);
    core.goto_key(window, GotoKey::Down);
    let view = core.goto_view(window).unwrap();
    assert_eq!(
        view.selected_row().map(|row| row.target),
        Some(GotoTarget::Workspace(other))
    );
    core.goto_key(window, GotoKey::Enter);
    assert!(!core.goto_holds(window));
    let store = core.library().store(window).unwrap();
    assert_eq!(store.current_workspace_id(), Some(other));
    assert_eq!(selected_session(&core, window), Some(logs));
    assert!(core.pane_focus_requested(logs), "its terminal takes the keyboard");
    assert!(!core.pane_focus_requested(build));
}

#[test]
fn a_deleted_workspace_drops_its_rows_and_a_closed_window_closes_the_window() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let store = core.library_mut().store_mut(window).unwrap();
    let home = store.current_workspace_id().unwrap();
    let other = store.add_workspace("other");
    let logs = store.add_session(other, background("logs", "/var/log")).unwrap();
    let _ = core.commit();

    core.toggle_goto(window);
    core.goto_key(window, GotoKey::End);
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(logs)));
    send(
        &mut core,
        Command::Workspace(WorkspaceCmd::Delete),
        Some(other),
        Args::default(),
    );
    let _ = core.commit();
    assert!(core.goto_holds(window), "the window stays open");
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.rows.len(), 2, "{:?}", labels(&view.rows));
    assert!(view.rows.iter().all(|row| row.target != GotoTarget::Workspace(other)));
    assert_eq!(
        view.selected_row().map(|row| row.target),
        Some(GotoTarget::Workspace(home)),
        "the selection falls back to the first row"
    );
    core.goto_key(window, GotoKey::Enter);
    assert!(!core.goto_holds(window));
    assert_eq!(selected_session(&core, window), Some(first));

    core.toggle_goto(window);
    assert!(core.goto_holds(window));
    send(
        &mut core,
        Command::Window(WindowCmd::Close),
        Some(window),
        Args::default(),
    );
    let _ = core.commit();
    assert!(!core.goto_holds(window), "the closed window's Go to closes");
    assert!(core.goto_view(window).is_none());
    core.goto_key(window, GotoKey::Enter);
    core.goto_open(window, left(first));
    core.toggle_goto(window);
    assert!(!core.goto_holds(window), "nor opens on a closed window");
}

fn pane_columns(core: &Core, window: Uuid, row: usize) -> (&'static str, PaneStatus) {
    let view = core.goto_view(window).unwrap();
    let pane = view.rows[row].pane.clone().unwrap();
    (pane.agent, pane.status)
}

fn targets(core: &Core, window: Uuid) -> Vec<GotoTarget> {
    let view = core.goto_view(window).unwrap();
    view.rows.iter().map(|row| row.target).collect()
}

fn marked(core: &Core, window: Uuid) -> Vec<GotoTarget> {
    let view = core.goto_view(window).unwrap();
    view.rows
        .iter()
        .filter(|row| row.pane.as_ref().is_some_and(|pane| pane.current))
        .map(|row| row.target)
        .collect()
}

#[test]
fn a_status_with_no_agent_record_shows_in_its_column_and_its_filter() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let workspace = core.library().store(window).unwrap().current_workspace_id().unwrap();
    for (wire, shown, filter) in [
        ("blocked", PaneStatus::Blocked, "b"),
        ("active", PaneStatus::Working, "w"),
        ("completed", PaneStatus::Done, "d"),
    ] {
        status(&mut core, first, wire, "left");
        core.toggle_goto(window);
        assert_eq!(pane_columns(&core, window, 1), ("terminal", shown), "{wire}");
        let view = core.goto_view(window).unwrap();
        let glyph = view.rows[1].pane.as_ref().unwrap().indicator.as_ref();
        assert_eq!(glyph.map(|indicator| PaneStatus::of(indicator.status)), Some(shown));
        core.goto_key(window, text(filter));
        assert_eq!(
            targets(&core, window),
            [GotoTarget::Workspace(workspace), left(first)],
            "{filter} keeps the pane"
        );
        core.toggle_goto(window);
    }
    status(&mut core, first, "idle", "left");
    core.toggle_goto(window);
    assert_eq!(pane_columns(&core, window, 1), ("terminal", PaneStatus::Shell));
    core.goto_key(window, text("i"));
    assert!(
        targets(&core, window).is_empty(),
        "an idle pane with no agent is a shell"
    );
    core.toggle_goto(window);

    split(&mut core, first);
    status(&mut core, first, "blocked", "right");
    core.toggle_goto(window);
    assert_eq!(pane_columns(&core, window, 2), ("terminal", PaneStatus::Shell));
    assert_eq!(pane_columns(&core, window, 3), ("terminal", PaneStatus::Blocked));
    core.goto_key(window, text("b"));
    assert_eq!(
        targets(&core, window),
        [
            GotoTarget::Workspace(workspace),
            GotoTarget::Session(first),
            right(first)
        ]
    );
}

#[test]
fn a_status_from_the_scratch_shows_on_the_main_pane_row() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let workspace = core.library().store(window).unwrap().current_workspace_id().unwrap();
    split(&mut core, first);
    status(&mut core, first, "blocked", "scratch");

    core.toggle_goto(window);
    assert_eq!(pane_columns(&core, window, 2), ("terminal", PaneStatus::Blocked));
    assert_eq!(pane_columns(&core, window, 3), ("terminal", PaneStatus::Shell));
    let view = core.goto_view(window).unwrap();
    let glyph = view.rows[2].pane.as_ref().unwrap().indicator.as_ref();
    assert_eq!(glyph.map(|indicator| indicator.status), Some(AgentStatus::Blocked));
    core.goto_key(window, text("b"));
    assert_eq!(
        targets(&core, window),
        [
            GotoTarget::Workspace(workspace),
            GotoTarget::Session(first),
            left(first)
        ]
    );
}

#[test]
fn opening_a_pane_hides_the_scratch_that_held_the_keyboard() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    split(&mut core, first);
    let store = core.library_mut().store_mut(window).unwrap();
    store.set_split_focus(first, false);
    store.toggle_scratch(first, true);
    let _ = core.commit();
    core.clear_pane_focus_requests();

    core.toggle_goto(window);
    assert!(marked(&core, window).is_empty(), "the scratch holds the keyboard");
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(first)));
    core.goto_key(window, GotoKey::Down);
    core.goto_key(window, GotoKey::Enter);
    let session = core.library().store(window).unwrap().session(first).unwrap();
    assert!(!session.scratch_active, "the scratch steps aside");
    assert!(session.split.focused, "the split pane took the focus");
    assert!(core.pane_focus_requested(first));

    core.toggle_goto(window);
    assert_eq!(marked(&core, window), [right(first)]);
}

#[test]
fn a_selected_page_companion_opens_the_window_on_its_parents_main_pane() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let workspace = core.library().store(window).unwrap().current_workspace_id().unwrap();
    let logs = core
        .library_mut()
        .store_mut(window)
        .unwrap()
        .add_session(workspace, background("logs", "/var/log"))
        .unwrap();
    split(&mut core, logs);
    let page = |name: &str, parent: Option<Uuid>| NewSession {
        parent,
        content: SessionContent::Page,
        ..background(name, "/tmp")
    };
    let store = core.library_mut().store_mut(window).unwrap();
    store.set_split_focus(logs, true);
    let docs = store.add_session(workspace, page("docs", Some(logs))).unwrap();
    let notes = store.add_session(workspace, page("notes", None)).unwrap();
    store.select_session(Some(docs));
    let _ = core.commit();

    core.toggle_goto(window);
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(logs)));
    assert!(marked(&core, window).is_empty(), "the page holds the keyboard");
    core.goto_key(window, GotoKey::Enter);
    assert_eq!(selected_session(&core, window), Some(logs));
    let session = core.library().store(window).unwrap().session(logs).unwrap();
    assert!(!session.split.focused, "the main pane took the focus");

    core.library_mut()
        .store_mut(window)
        .unwrap()
        .select_session(Some(notes));
    core.toggle_goto(window);
    let view = core.goto_view(window).unwrap();
    assert_eq!(
        view.selected_row().map(|row| row.target),
        Some(left(first)),
        "a page with no parent opens on the first pane row"
    );
}

#[test]
fn an_agent_that_exited_to_its_shell_reads_as_a_terminal() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let table = FakeProcessTable::default();
    for (pid, parent, group, argv) in [(100, 1, 100, "sh"), (101, 100, 101, "claude")] {
        let stat = ProcessStat {
            pid,
            ppid: parent,
            pgrp: group,
            session: 100,
        };
        table.insert(stat, Some(&[argv]));
    }
    core.set_process_table(Box::new(table), Some("sh".to_owned()));
    let store = core.library_mut().store_mut(window).unwrap();
    let session = store.session_mut(first).unwrap();
    let identity = session.pane_identity;
    let surface = FakeSurface::new(uuid_string(identity));
    let probe = surface.probe();
    probe.set_realized(true);
    probe.set_foreground_pid(Some(101));
    let _ = session.surface.replace(Box::new(surface));
    report_claude(&mut core, first, identity, "left");

    core.toggle_goto(window);
    assert_eq!(pane_columns(&core, window, 1), ("claude", PaneStatus::Idle));
    core.toggle_goto(window);
    probe.set_foreground_pid(Some(100));
    core.toggle_goto(window);
    assert_eq!(pane_columns(&core, window, 1), ("terminal", PaneStatus::Shell));
}

#[test]
fn the_window_nests_a_companion_under_its_parent_and_the_top_level_filter_lifts_its_selection() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let store = core.library_mut().store_mut(window).unwrap();
    let home = store.current_workspace_id().unwrap();
    let logs = store.add_session(home, background("logs", "/var/log")).unwrap();
    let options = CompanionOptions {
        name: Some("notes".to_owned()),
        select: false,
        ..CompanionOptions::default()
    };
    let notes = store.add_companion(first, options, "/tmp").unwrap();
    let other = store.add_workspace("other");
    let stray = NewSession {
        parent: Some(first),
        ..background("stray", "/tmp")
    };
    let stray = store.add_session(other, stray).unwrap();
    store.select_session(Some(notes));
    let _ = core.commit();

    core.toggle_goto(window);
    let view = core.goto_view(window).unwrap();
    let store = core.library().store(window).unwrap();
    let (workspace, name) = (
        &store.workspaces()[0].name,
        store.session(first).unwrap().display_name(),
    );
    assert_eq!(
        drawn(&view.rows),
        [
            (vec![], workspace.as_str()),
            (vec!["├─"], name.as_str()),
            (vec!["│", "└─"], "notes"),
            (vec!["└─"], "logs"),
            (vec![], "other"),
            (vec!["└─"], "stray"),
        ]
    );
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(notes)));
    core.goto_key(window, text("t"));
    assert_eq!(
        targets(&core, window),
        [
            GotoTarget::Workspace(home),
            left(first),
            left(logs),
            GotoTarget::Workspace(other),
            left(stray)
        ]
    );
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(first)));
    assert_eq!(
        marked(&core, window),
        [left(first)],
        "the parent's row marks its focused companion"
    );
    core.goto_key(window, GotoKey::Enter);
    assert_eq!(selected_session(&core, window), Some(first));
}

#[test]
fn the_top_level_filter_keeps_the_selection_it_lifted_when_the_companion_closes() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    let store = core.library_mut().store_mut(window).unwrap();
    let notes = store.add_companion(first, CompanionOptions::default(), "/tmp").unwrap();
    store.select_session(Some(notes));
    let _ = core.commit();

    core.toggle_goto(window);
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(notes)));
    core.goto_key(window, text("t"));
    core.library_mut().store_mut(window).unwrap().close_session(notes);
    let _ = core.commit();
    assert!(core.goto_holds(window));
    assert_eq!(targets(&core, window).len(), 2);
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(first)));
}

#[test]
fn a_selected_split_pane_whose_split_closes_keeps_the_selection_on_its_session() {
    let (mut core, window) = core("/tmp");
    let first = selected_session(&core, window).unwrap();
    split(&mut core, first);

    core.toggle_goto(window);
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(right(first)));
    core.library_mut().store_mut(window).unwrap().close_split(first);
    let _ = core.commit();
    assert!(core.goto_holds(window));
    let home = core.library().store(window).unwrap().workspaces()[0].id();
    assert_eq!(targets(&core, window), [GotoTarget::Workspace(home), left(first)]);
    let view = core.goto_view(window).unwrap();
    assert_eq!(view.selected_row().map(|row| row.target), Some(left(first)));
}
