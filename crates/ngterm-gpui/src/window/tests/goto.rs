use ngterm_core::keymap::BuiltinAction;

use super::*;

impl Shell<'_> {
    fn press_go_to(&mut self) {
        let chord = self
            .state
            .core
            .keymap()
            .equivalent(BuiltinAction::GoTo)
            .cloned()
            .expect("Go to has a chord");
        self.cx.simulate_keystrokes(&keystroke(&chord));
        crate::keymap_dispatch::release_simulated(&keystroke(&chord));
        self.settle();
    }

    fn goto_view(&mut self) -> Option<ngterm_core::goto::GotoView> {
        self.view
            .read_with(self.cx, |view, cx| view.goto().map(|goto| goto.read(cx).view().clone()))
    }

    fn goto_labels(&mut self) -> Vec<String> {
        self.goto_view()
            .map(|view| view.rows.into_iter().map(|row| row.label).collect())
            .unwrap_or_default()
    }

    fn goto_holds_keyboard(&mut self) -> bool {
        self.holds(|view, cx| view.goto().map(|goto| goto.read(cx).focus_handle()))
    }
}

#[gpui_kit::test]
fn the_go_to_chord_opens_the_window_whose_keys_walk_and_open_a_pane(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    s.with_keys();
    s.press_go_to();
    assert!(s.view.read_with(s.cx, |view, _| view.has_layer(Layer::GoTo)));
    assert!(s.goto_holds_keyboard());
    assert!(pages_covered(&mut s), "the pages step aside under it");
    assert_eq!(s.goto_labels(), ["workspace 1", "u", "var"]);
    assert_eq!(s.goto_view().unwrap().selected, Some(2), "on the focused pane");
    s.cx.simulate_keystrokes("k");
    s.settle();
    assert_eq!(s.goto_view().unwrap().selected, Some(1));
    s.cx.simulate_keystrokes("enter");
    s.settle();
    assert_eq!(s.active(), s.session("u"), "the row opened");
    assert!(s.goto_view().is_none());
    assert!(!s.view.read_with(s.cx, |view, _| view.has_layer(Layer::GoTo)));
    assert!(s.terminal_focused(), "the terminal has the keyboard back");
}

#[gpui_kit::test]
fn the_go_to_window_keeps_every_key_and_closes_on_escape_or_its_chord(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    s.with_keys();
    s.press_go_to();
    s.cx.simulate_keystrokes("/ v");
    s.settle();
    let view = s.goto_view().unwrap();
    assert_eq!((view.searching, view.query.as_str()), (true, "v"));
    assert_eq!(s.goto_labels(), ["workspace 1", "var"]);
    s.cx.simulate_keystrokes("ctrl-shift-t");
    s.settle();
    assert_eq!(
        s.state.core.library().store(s.window).unwrap().sessions().count(),
        2,
        "a chord other than its own does nothing"
    );
    s.cx.simulate_keystrokes("escape");
    s.settle();
    assert!(
        s.goto_view().is_some_and(|view| !view.searching),
        "the first Escape leaves the search"
    );
    s.cx.simulate_keystrokes("escape");
    s.settle();
    assert!(s.goto_view().is_none());
    assert_eq!(s.active(), s.session("var"), "nothing changed");
    assert!(s.terminal_focused());

    s.press_go_to();
    assert!(s.goto_holds_keyboard());
    s.press_go_to();
    assert!(s.goto_view().is_none(), "its chord closes it");
    assert!(s.terminal_focused());
}

#[gpui_kit::test]
fn the_open_go_to_window_follows_a_status_change(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    s.state.menu_activated(
        s.window,
        crate::app_menu::Item::Row(ngterm_core::palette_catalog::PaletteCommand::GoTo),
    );
    s.settle();
    assert!(s.goto_holds_keyboard(), "the menu row opens it");
    let u = s.session("u").unwrap();
    let response = s.state.core.dispatch(&ngterm_proto::Request {
        cmd: ngterm_proto::Command::Session(ngterm_proto::command::SessionCmd::Status),
        target: Some(ngterm_core::pane_env::uuid_string(u)),
        args: Some(ngterm_proto::Args {
            status: Some("blocked".to_owned()),
            ..ngterm_proto::Args::default()
        }),
    });
    assert!(response.ok, "{response:?}");
    s.state.settle();
    s.settle();
    let view = s.goto_view().unwrap();
    let indicator = view.rows[1].pane.as_ref().and_then(|pane| pane.indicator.as_ref());
    assert_eq!(
        indicator.map(|indicator| indicator.status),
        Some(ngterm_core::model::AgentStatus::Blocked)
    );
    assert_eq!(
        view.rows[1].pane.as_ref().map(|pane| (pane.agent, pane.status)),
        Some(("terminal", ngterm_core::goto::PaneStatus::Blocked)),
        "the status column agrees with the glyph"
    );
    assert!(s.cx.debug_bounds("agent-status").is_some(), "its glyph is drawn");
    let panel = s.cx.debug_bounds("go-to").unwrap();
    let below = gpui_kit::point(px(4.), panel.bottom() + px(4.));
    s.cx.simulate_click(below, gpui_kit::Modifiers::default());
    s.settle();
    assert!(s.goto_view().is_none(), "a press on the scrim closes it");
}
