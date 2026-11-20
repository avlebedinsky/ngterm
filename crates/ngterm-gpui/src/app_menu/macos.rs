//! `AppKit`'s menu bar shares the window menu's model and dispatch path. Edit uses the responder chain so `WebKit` owns editing.
use super::{AppAction, AppMenu, Entry, MenuAction, MenuItem};
use crate::{
    keymap_dispatch::{KeyOrigin, window_of},
    state::{AppState, Poster, post},
};
use gpui_kit::App;
use ngterm_core::Core;
use ngterm_core::keymap::{Chord, FUNCTION_KEYS, Modifiers};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    ffi::{CStr, CString},
    sync::OnceLock,
};
use uuid::Uuid;

#[derive(Default)]
struct Registry {
    model: Option<(Uuid, AppMenu)>,
    actions: Vec<(Uuid, MenuAction)>,
}
thread_local! { static REGISTRY: RefCell<Registry> = RefCell::default(); }
static EVENTS: OnceLock<async_channel::Sender<(Uuid, MenuAction)>> = OnceLock::new();
unsafe extern "C" {
    fn ngterm_menus_set(json: *const std::ffi::c_char, clicked: extern "C" fn(usize));
}
extern "C" fn clicked(tag: usize) {
    let action = REGISTRY.with(|registry| registry.borrow().actions.get(tag).copied());
    if let (Some(events), Some(action)) = (EVENTS.get(), action) {
        let _ = events.try_send(action);
    }
}
/// Queue native menu actions outside `AppKit`'s synchronous key dispatch.
pub fn init(poster: &Poster, cx: &mut App) {
    let (sender, receiver) = async_channel::unbounded();
    if EVENTS.set(sender).is_err() {
        return;
    }
    let poster = poster.clone();
    cx.spawn(async move |cx| {
        while let Ok((id, action)) = receiver.recv().await {
            cx.update(|cx| match action {
                MenuAction::Command(item) => post(&poster, move |state: &mut AppState| state.menu_activated(id, item)),
                MenuAction::List(list) => {
                    post(&poster, move |state: &mut AppState| state.menu_list_activated(id, list));
                }
                MenuAction::App(AppAction::Quit) => post(&poster, AppState::quit_from_menu),
                MenuAction::App(AppAction::ResetLiveSessions) => post(&poster, AppState::reset_live_sessions),
                MenuAction::App(_) => {
                    if let Some(handle) = window_of(KeyOrigin::Window(id)) {
                        let run = super::perform(&poster, id);
                        let _ = handle.update(cx, |_, window, cx| run(action, window, cx));
                    }
                }
            });
        }
    })
    .detach();
}
fn item(item: &MenuItem, id: Uuid, actions: &mut Vec<(Uuid, MenuAction)>) -> Value {
    let tag = actions.len();
    actions.push((id, item.action));
    let (key, mods) = item
        .chord
        .as_ref()
        .map_or((String::new(), 0), |chord| (key_equivalent(chord), chord.mods.bits()));
    json!({"title":item.title,"enabled":item.enabled,"checked":item.checked,"tag":tag,"key":key,"mods":mods})
}
/// The `NSMenuItem` key equivalent of a chord (upstream's `toShortcut`): `AppKit`'s character for a named key
/// (`NSCarriageReturnCharacter`, `NSTabCharacter`, `NSBackspaceCharacter`, the `NSUpArrowFunctionKey` and
/// `NSF1FunctionKey` families), else the key itself, uppercase when the chord holds Shift. Delete is
/// `NSBackspaceCharacter` (0x08, `SwiftUI`'s `.delete`), which `AppKit` shows as ⌫: it shows `NSDeleteCharacter`
/// (0x7F) as Forward Delete ⌦.
fn key_equivalent(chord: &Chord) -> String {
    let named = match chord.key.as_str() {
        "return" => Some('\r'),
        "tab" => Some('\t'),
        "space" => Some(' '),
        "delete" => Some('\u{8}'),
        "up" => Some('\u{f700}'),
        "down" => Some('\u{f701}'),
        "left" => Some('\u{f702}'),
        "right" => Some('\u{f703}'),
        key => FUNCTION_KEYS
            .iter()
            .position(|name| *name == key)
            .and_then(|index| char::from_u32(0xf704 + u32::try_from(index).ok()?)),
    };
    let key = named.map_or_else(|| chord.key.clone(), String::from);
    if chord.mods.contains(Modifiers::SHIFT) {
        key.to_uppercase()
    } else {
        key
    }
}
fn entries(items: &[Entry], id: Uuid, actions: &mut Vec<(Uuid, MenuAction)>) -> Vec<Value> {
    items
        .iter()
        .map(|entry| match entry {
            Entry::Item(value) => item(value, id, actions),
            Entry::Separator => json!({"separator":true}),
            Entry::Label(title) => json!({"title":title,"enabled":false}),
            Entry::Submenu {
                title,
                entries: children,
            } => json!({"title":title,"children":entries(children,id,actions)}),
        })
        .collect()
}
/// Refresh only when model titles, checks or availability change.
pub fn sync(core: &Core) {
    sync_with(core, |json| {
        // SAFETY: main thread, native code copies JSON and retains only a static callback.
        unsafe { ngterm_menus_set(json.as_ptr(), clicked) };
    });
}
/// Hands the menu's JSON to `set` when the model changed. `set` is a parameter because `AppKit`'s menu bar needs the
/// main thread, which a unit test does not run on.
fn sync_with(core: &Core, set: impl FnOnce(&CStr)) {
    let id = core.library().active_window_id().unwrap_or_default();
    let model = AppMenu::of(core, id);
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if registry.model.as_ref() == Some(&(id, model.clone())) {
            return;
        }
        registry.actions.clear();
        let app: Vec<_> = model
            .app
            .iter()
            .map(|value| item(value, id, &mut registry.actions))
            .collect();
        let menus: Vec<_> = model
            .menus
            .iter()
            .map(|(title, items)| json!({"title":title,"children":entries(items,id,&mut registry.actions)}))
            .collect();
        let Ok(value) = CString::new(json!({"app":app,"menus":menus}).to_string()) else {
            return;
        };
        set(&value);
        registry.model = Some((id, model));
    });
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
