//! The sidebar's session search field, this build's own (upstream has none): a field in the title bar over the
//! sidebar (atop the sidebar's column while the toolbar is hidden) that narrows the sidebar to the sessions whose name
//! holds its text. The model keeps the text and decides what it keeps (`ngterm_core::model` `sidebar_search`); the
//! field only edits it.
//!
//! The magnifier, the field (`sidebar-search`, placeholder "Search sessions") and a clear button (`sidebar-search-clear`)
//! while it holds text, in the window chrome's colors. Return opens the match the cursor is on (the first unless Up or
//! Down moved it) and gives its terminal the keyboard; Escape empties the field and gives the selected session's
//! terminal the keyboard back; the clear button empties it and keeps the keyboard in it. The field's keys reach it
//! ahead of the input's own bindings through the window's key hook ([`field_key`]).
//!
//! The field is the text's writer while the user types: the model's text is pushed into it only once every text the
//! field sent has landed, so a settle in between never rewinds what is being typed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use ngterm_core::Core;
use ngterm_core::model::AppStore;
use ngterm_core::sidebar_actions::SidebarAction;
use uuid::Uuid;

use crate::chrome::button::Button;
use crate::state::{AppState, Poster, post};
use crate::terminal::input::KeyStroke;

/// The field's placeholder.
pub const PLACEHOLDER: &str = "Search sessions";
/// The field's height: it fits the compact title bar.
pub const HEIGHT: f32 = 24.;
/// The field's text size.
const TEXT_SIZE: f32 = 12.;
/// The clear button's square.
const CLEAR_SIZE: f32 = 18.;

/// What a key in the field does beyond editing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKey {
    /// Return: open the cursor's match.
    Open,
    /// Escape: empty the field and give the terminal the keyboard back.
    Dismiss,
    /// Down (`true`) or Up: move the cursor among the matches.
    Step(bool),
}

/// The field's own meaning of `stroke`, if any: Return, Escape, Up and Down with no modifier but Shift.
#[must_use]
pub fn field_key(stroke: &KeyStroke) -> Option<FieldKey> {
    use ngterm_core::ghostty_config::binds::Mods;
    if !(stroke.mods.binding() - Mods::SHIFT).is_empty() {
        return None;
    }
    match stroke.key.name() {
        "enter" | "numpad_enter" => Some(FieldKey::Open),
        "escape" => Some(FieldKey::Dismiss),
        "arrow_down" => Some(FieldKey::Step(true)),
        "arrow_up" => Some(FieldKey::Step(false)),
        _ => None,
    }
}

/// One window's search field (module docs).
pub struct SearchField {
    window: Uuid,
    poster: Poster,
    input: Entity<InputState>,
    /// The texts the field sent, and how many of them landed.
    sent: u64,
    landed: Arc<AtomicU64>,
    _events: Subscription,
}

impl SearchField {
    /// The field of model window `window_id`, posting through `poster`.
    pub fn new(window_id: Uuid, poster: Poster, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(PLACEHOLDER));
        // `set_value` emits no change: only typing sends the text
        let events = cx.subscribe(&input, |field, input, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                field.send(SidebarAction::SetSearch(text));
                cx.notify();
            }
            // the rows mark the cursor's match only while the field holds the keyboard
            InputEvent::Focus | InputEvent::Blur => cx.notify(),
            InputEvent::PressEnter { .. } => {}
        });
        Self {
            window: window_id,
            poster,
            input,
            sent: 0,
            landed: Arc::new(AtomicU64::new(0)),
            _events: events,
        }
    }

    /// What takes the keyboard: the input.
    #[must_use]
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    /// Whether the field holds the window's keyboard.
    #[must_use]
    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle(cx).is_focused(window)
    }

    /// The field's text.
    #[must_use]
    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Posts `action` for the window; a text change counts toward the field's idle state.
    fn send(&mut self, action: SidebarAction) {
        let window = self.window;
        let landed = matches!(action, SidebarAction::SetSearch(_)).then(|| {
            self.sent += 1;
            (Arc::clone(&self.landed), self.sent)
        });
        post(&self.poster, move |state: &mut AppState| {
            if let Some((landed, number)) = landed {
                landed.store(number, Ordering::Release);
            }
            state.sidebar_action(window, action);
        });
    }

    /// Whether every text the field sent has landed in the model.
    fn idle(&self) -> bool {
        self.landed.load(Ordering::Acquire) == self.sent
    }

    /// Takes the model's text once the field is idle.
    pub fn sync(&mut self, core: &Core, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = core
            .library()
            .store(self.window)
            .map(|store: &AppStore| store.sidebar_search().query().to_owned())
        else {
            return;
        };
        if self.idle() && self.text(cx) != query {
            self.input.update(cx, |input, cx| input.set_value(query, window, cx));
            cx.notify();
        }
    }

    /// Gives the field the keyboard, its text selected (Search Sessions…).
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// Runs one of the field's keys.
    pub fn key(&mut self, key: FieldKey, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            FieldKey::Open => self.send(SidebarAction::OpenSearchMatch),
            FieldKey::Step(down) => self.send(SidebarAction::StepSearch(down)),
            FieldKey::Dismiss => self.clear(true, window, cx),
        }
    }

    /// Empties the field; with `leave` the selected session's terminal takes the keyboard back, else the field keeps it.
    pub fn clear(&mut self, leave: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            if !leave {
                input.focus(window, cx);
            }
        });
        // the empty text lands with the clear, so a settle in between pushes nothing stale back
        self.sent += 1;
        let (landed, number, window_id) = (Arc::clone(&self.landed), self.sent, self.window);
        post(&self.poster, move |state: &mut AppState| {
            landed.store(number, Ordering::Release);
            state.sidebar_action(window_id, SidebarAction::ClearSearch { focus_terminal: leave });
        });
        cx.notify();
    }
}

impl Render for SearchField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = crate::chrome::current(cx);
        let ink = crate::chrome::color(colors.foreground, 1.0);
        let soft = crate::chrome::color(colors.foreground, 0.55);
        let focused = self.is_focused(window, cx);
        let has_text = !self.input.read(cx).value().is_empty();
        let clear = Button::new("sidebar-search-clear")
            .flex_none()
            .size(px(CLEAR_SIZE))
            .rounded(px(4.))
            .cursor_pointer()
            .wash(ink)
            .hint("Clear search", &colors)
            .keeps_keyboard()
            .on_click(cx.listener(|field, _, window, cx| field.clear(false, window, cx)))
            .child(crate::picker::glyph(IconName::X, soft, px(13.)));
        div()
            .id("sidebar-search-box")
            .debug_selector(|| "sidebar-search".to_owned())
            // a press here edits the field: it never starts a window move or a title-bar double click
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .w_full()
            .min_w(px(0.))
            .h(px(HEIGHT))
            .pl(px(7.))
            .pr(px(3.))
            .rounded(px(6.))
            .bg(crate::chrome::color(
                colors.foreground,
                if focused { 0.08 } else { 0.05 },
            ))
            .border_1()
            .border_color(crate::chrome::color(
                colors.foreground,
                if focused { 0.38 } else { 0.12 },
            ))
            .text_size(px(TEXT_SIZE))
            .text_color(ink)
            .child(crate::picker::glyph(IconName::Search, soft, px(13.)))
            .child(
                div().flex_1().min_w(px(0.)).child(
                    Input::new(&self.input)
                        .appearance(false)
                        .accessibility_id("sidebar-search")
                        .aria_label(PLACEHOLDER)
                        .h(px(HEIGHT - 2.))
                        .text_color(ink),
                ),
            )
            // the button's room stays while the field is empty, so the text never reflows as it appears
            .map(|row| {
                if has_text {
                    row.child(clear)
                } else {
                    row.child(div().flex_none().size(px(CLEAR_SIZE)))
                }
            })
    }
}

#[cfg(test)]
mod tests;
