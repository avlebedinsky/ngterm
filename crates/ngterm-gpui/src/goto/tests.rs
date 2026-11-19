use std::cell::RefCell;

use gpui_kit::{Entity, Keystroke, TestAppContext, VisualTestContext};
use ngterm_core::agent_chat::AgentKind;
use ngterm_core::goto::{GotoPane, GotoWorkspace, PaneStatus, rows};
use ngterm_core::model::{AgentIndicator, AgentStatus, StatusPane};

use super::*;
use crate::terminal::input::Source;

fn key(source: &str) -> Option<GotoKey> {
    let keystroke = Keystroke::parse(source).unwrap();
    goto_key(&KeyStroke::new(&keystroke, &Source::Keystroke, false))
}

#[test]
fn the_keys_read_as_named_keys_control_letters_and_typed_text() {
    let text = |text: &str| {
        Some(GotoKey::Text {
            text: text.to_owned(),
            key: text.to_owned(),
        })
    };
    for (pressed, read) in [
        ("up", Some(GotoKey::Up)),
        ("down", Some(GotoKey::Down)),
        ("left", Some(GotoKey::Left)),
        ("right", Some(GotoKey::Right)),
        ("home", Some(GotoKey::Home)),
        ("end", Some(GotoKey::End)),
        ("enter", Some(GotoKey::Enter)),
        ("escape", Some(GotoKey::Escape)),
        ("backspace", Some(GotoKey::Backspace)),
        ("ctrl-d", Some(GotoKey::Control('d'))),
        ("ctrl-n", Some(GotoKey::Control('n'))),
        ("j", text("j")),
        ("shift-g", text("G")),
        ("/", text("/")),
        ("ctrl-shift-k", None),
        ("super-a", None),
        ("tab", None),
    ] {
        assert_eq!(key(pressed), read, "{pressed}");
    }
}

const KEYCODE_J: u32 = 44;
const KEYCODE_G: u32 = 42;
const KEYCODE_B: u32 = 56;
const KEYCODE_SLASH: u32 = 61;
const KEYCODE_7: u32 = 16;

#[test]
fn the_list_letters_read_the_key_by_position_on_a_non_latin_layout() {
    use crate::terminal::input::NativeKey;
    let native = |keycode: u32, shift: bool, (text, unshifted): (&str, char), ascii_layout: bool| {
        let native = NativeKey {
            keycode,
            modifiers: gpui_kit::Modifiers {
                shift,
                ..gpui_kit::Modifiers::default()
            },
            unshifted: Some(unshifted),
            ascii_layout,
            text: text.into(),
            ..NativeKey::default()
        };
        let name = if shift { "shift-x" } else { "x" };
        goto_key(&KeyStroke::new(
            &Keystroke::parse(name).unwrap(),
            &Source::Native(native),
            false,
        ))
    };
    let read = |text: &str, key: &str| {
        Some(GotoKey::Text {
            text: text.to_owned(),
            key: key.to_owned(),
        })
    };
    assert_eq!(native(KEYCODE_J, false, ("о", 'о'), false), read("о", "j"));
    assert_eq!(native(KEYCODE_B, false, ("и", 'и'), false), read("и", "b"));
    assert_eq!(native(KEYCODE_G, true, ("П", 'п'), false), read("П", "G"));
    assert_eq!(native(KEYCODE_SLASH, false, (".", '.'), false), read(".", "/"));
    assert_eq!(
        native(KEYCODE_7, true, ("/", '7'), true),
        read("/", "/"),
        "a key its chord cannot name reads as it typed"
    );
}

const KEYCODE_Q: u32 = 24;

#[test]
fn altgr_on_windows_types_its_character_and_a_control_chord_does_not() {
    use crate::terminal::input::NativeKey;
    let ctrl_alt = gpui_kit::Modifiers {
        control: true,
        alt: true,
        ..gpui_kit::Modifiers::default()
    };
    let native = |consumed: gpui_kit::Modifiers, text: &str| {
        let native = NativeKey {
            keycode: KEYCODE_Q,
            modifiers: ctrl_alt,
            consumed,
            unshifted: Some('q'),
            ascii_layout: true,
            text: text.into(),
            ..NativeKey::default()
        };
        goto_key(&KeyStroke::new(
            &Keystroke::parse("ctrl-alt-q").unwrap(),
            &Source::Native(native),
            false,
        ))
    };
    assert_eq!(
        native(ctrl_alt, "@"),
        Some(GotoKey::Text {
            text: "@".to_owned(),
            key: "@".to_owned(),
        }),
        "AltGr is Ctrl+Alt held and used up"
    );
    assert_eq!(
        native(gpui_kit::Modifiers::default(), "q"),
        None,
        "a Ctrl+Alt chord types nothing"
    );
}

#[test]
fn a_narrow_panel_drops_the_status_column_then_the_agent_column() {
    let metrics = Metrics::default();
    assert_eq!(Columns::of(760.0, metrics), Columns::Both);
    assert_eq!(Columns::of(400.0, metrics), Columns::Agent);
    assert_eq!(Columns::of(200.0, metrics), Columns::None);
    let larger = Metrics::new(metrics.base * 2.0);
    assert_eq!(Columns::of(760.0, larger), Columns::Agent, "the thresholds scale");
}

struct Opened {
    workspace: Uuid,
    shell: Uuid,
    agent: Uuid,
}

fn pane(session: Uuid, name: &str, agent: Option<AgentKind>, status: PaneStatus) -> GotoPane {
    GotoPane {
        session,
        pane: StatusPane::Left,
        session_name: name.to_owned(),
        split: false,
        title: None,
        cwd: format!("/src/{name}"),
        agent,
        status,
        indicator: agent.map(|_| AgentIndicator {
            status: AgentStatus::Blocked,
            ..AgentIndicator::default()
        }),
        current: agent.is_none(),
    }
}

fn opened() -> (GotoView, Opened) {
    let ids = Opened {
        workspace: Uuid::new_v4(),
        shell: Uuid::new_v4(),
        agent: Uuid::new_v4(),
    };
    let workspaces = [GotoWorkspace {
        id: ids.workspace,
        name: "nix-os".to_owned(),
        branch: Some("main".to_owned()),
        panes: vec![
            pane(ids.shell, "zsh", None, PaneStatus::Shell),
            pane(ids.agent, "ai", Some(AgentKind::Claude), PaneStatus::Blocked),
        ],
    }];
    let view = GotoView {
        rows: rows(&workspaces, "", None),
        selected: Some(1),
        query: String::new(),
        searching: false,
        filter: None,
    };
    (view, ids)
}

fn open(
    cx: &mut TestAppContext,
    view: GotoView,
) -> (Entity<GotoPanel>, &mut VisualTestContext, Rc<RefCell<Vec<Request>>>) {
    cx.update(gpui_kit::init);
    let requests: Rc<RefCell<Vec<Request>>> = Rc::default();
    let seen = Rc::clone(&requests);
    let sink: Sink = Rc::new(move |request| seen.borrow_mut().push(request));
    let glyphs = glyphs(&view, &AppSettings::default());
    let (panel, cx) =
        cx.add_window_view(move |_, cx| GotoPanel::new((view, glyphs), (0.0, Metrics::default()), sink, cx));
    cx.simulate_resize(gpui_kit::size(px(900.), px(600.)));
    cx.run_until_parked();
    (panel, cx, requests)
}

fn selector(target: GotoTarget) -> &'static str {
    Box::leak(format!("go-to-row-{}", target.id()).into_boxed_str())
}

#[gpui_kit::test]
fn the_panel_draws_its_rows_and_a_click_opens_one(cx: &mut TestAppContext) {
    let (view, ids) = opened();
    let (panel, cx, requests) = open(cx, view);
    for drawn in [
        "go-to",
        "go-to-scrim",
        "go-to-search",
        "go-to-count",
        "go-to-detail",
        "go-to-hints",
    ] {
        assert!(cx.debug_bounds(drawn).is_some(), "{drawn}");
    }
    let agent = GotoTarget::Pane {
        session: ids.agent,
        pane: StatusPane::Left,
    };
    assert!(
        cx.debug_bounds(selector(GotoTarget::Workspace(ids.workspace)))
            .is_some()
    );
    let glyph = cx
        .debug_bounds("agent-status")
        .expect("the blocked pane draws its glyph");
    let row = cx.debug_bounds(selector(agent)).expect("the agent's row");
    assert!(row.contains(&glyph.center()), "the glyph sits in its pane's row");
    cx.simulate_click(row.center(), gpui_kit::Modifiers::default());
    panel.read_with(cx, |panel, _| panel.key(GotoKey::Down));
    let panel_bounds = cx.debug_bounds("go-to").unwrap();
    let outside = gpui_kit::point(px(4.), panel_bounds.bottom() + px(4.));
    cx.simulate_click(outside, gpui_kit::Modifiers::default());
    assert_eq!(
        *requests.borrow(),
        [Request::Open(agent), Request::Key(GotoKey::Down), Request::Close]
    );
}

#[gpui_kit::test]
fn a_narrow_window_drops_the_columns(cx: &mut TestAppContext) {
    let (view, _) = opened();
    let (_, cx, _) = open(cx, view);
    let drawn = |cx: &mut VisualTestContext| {
        (
            cx.debug_bounds("go-to-agent").is_some(),
            cx.debug_bounds("go-to-status").is_some(),
        )
    };
    assert_eq!(drawn(cx), (true, true));
    cx.simulate_resize(gpui_kit::size(px(420.), px(600.)));
    cx.run_until_parked();
    assert_eq!(drawn(cx), (true, false), "the status column goes first");
    cx.simulate_resize(gpui_kit::size(px(260.), px(600.)));
    cx.run_until_parked();
    assert_eq!(drawn(cx), (false, false));
}

#[gpui_kit::test]
fn the_panel_follows_the_cores_rows(cx: &mut TestAppContext) {
    let (view, ids) = opened();
    let (panel, cx, _) = open(cx, view.clone());
    assert!(cx.debug_bounds("go-to-empty").is_none());
    let searched = GotoView {
        rows: Vec::new(),
        selected: None,
        query: "nothing".to_owned(),
        searching: true,
        ..view
    };
    panel.update(cx, |panel, cx| {
        panel.refresh((searched.clone(), Vec::new()), (0.0, Metrics::default()), cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("go-to-empty").is_some(), "nothing matches");
    assert!(
        cx.debug_bounds(selector(GotoTarget::Workspace(ids.workspace)))
            .is_none()
    );
    assert_eq!(panel.read_with(cx, |panel, _| panel.view().clone()), searched);
}

#[gpui_kit::test]
fn the_title_count_and_footer_read_at_the_size_of_the_rows(cx: &mut TestAppContext) {
    let (view, ids) = opened();
    let (_, cx, _) = open(cx, view);
    let height = |cx: &mut VisualTestContext, line: &'static str| cx.debug_bounds(line).expect(line).size.height;
    let label = Box::leak(format!("go-to-label-{}", main_pane(ids.agent).id()).into_boxed_str());
    let row = height(cx, label);
    for line in [
        "go-to-title",
        "go-to-search",
        "go-to-count",
        "go-to-detail",
        "go-to-hints",
    ] {
        assert_eq!(height(cx, line), row, "{line}");
    }
}

fn split_view() -> (GotoView, Uuid, Uuid) {
    let [workspace, build, zsh] = [(); 3].map(|()| Uuid::new_v4());
    let half = |side| GotoPane {
        pane: side,
        split: true,
        ..pane(build, "build", None, PaneStatus::Shell)
    };
    let workspaces = [GotoWorkspace {
        id: workspace,
        name: "home".to_owned(),
        branch: None,
        panes: vec![
            half(StatusPane::Left),
            half(StatusPane::Right),
            pane(zsh, "zsh", None, PaneStatus::Shell),
        ],
    }];
    let view = GotoView {
        rows: rows(&workspaces, "", None),
        selected: Some(2),
        query: String::new(),
        searching: false,
        filter: None,
    };
    (view, build, zsh)
}

fn tree_cell(target: GotoTarget, level: usize) -> &'static str {
    Box::leak(format!("go-to-tree-{}-{level}", target.id()).into_boxed_str())
}

fn main_pane(session: Uuid) -> GotoTarget {
    GotoTarget::Pane {
        session,
        pane: StatusPane::Left,
    }
}

#[gpui_kit::test]
fn a_split_sessions_pane_draws_a_level_deeper_on_a_rail_and_the_footer_names_it(cx: &mut TestAppContext) {
    let (view, build, zsh) = split_view();
    let (_, cx, _) = open(cx, view);
    let mut cell = |target: GotoTarget, level: usize| cx.debug_bounds(tree_cell(target, level));
    let session = cell(GotoTarget::Session(build), 0).expect("the session's connector");
    let rail = cell(main_pane(build), 0).expect("the pane's rail");
    let connector = cell(main_pane(build), 1).expect("the pane's connector");
    let sibling = cell(main_pane(zsh), 0).expect("the next session's connector");
    assert_eq!(
        rail.left(),
        session.left(),
        "the rail runs under the session's connector"
    );
    assert_eq!(sibling.left(), session.left(), "sessions share a level");
    assert!(connector.left() > session.right(), "the pane hangs a level deeper");
    assert!(cell(main_pane(zsh), 1).is_none(), "an unsplit session is one level");
    cx.activate_a11y();
    let tree = cx.a11y_tree_update().expect("the panel's accessibility tree");
    let detail = tree
        .nodes
        .iter()
        .find(|(_, node)| node.author_id() == Some("go-to-detail"))
        .and_then(|(_, node)| node.value());
    assert_eq!(detail, Some("home / build / terminal"));
}
