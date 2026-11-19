//! The Go to window ([`ngterm_core::goto`]), an extension beyond upstream, drawn while the core holds it open in this
//! window: a scrim over the window (`go-to-scrim`, a press on it closes) under a panel (`go-to`) centered in it, the
//! search line and the pane count (`go-to-search`, `go-to-count`) over the rows (`go-to-row-<id>`), and under them the
//! selected row's detail, its cwd or branch, and the key hints (`go-to-detail`, `go-to-hints`).
//!
//! A workspace row is its name in bold with its branch trailing. A session's row hangs under it on a `├─`/`└─`
//! connector, and a split session's pane rows and a session's child sessions under that one a level deeper, each level
//! a cell of the same width. A pane row shows `◆` marking the focused pane, then the session's status glyph as the
//! sidebar draws it on the pane that holds the status, the label, and the agent and status columns, which a narrow
//! panel drops ([`Columns`]).
//!
//! The panel holds the keyboard: the window's key hook hands it every press ([`goto_key`]), which it posts to the core,
//! whose rows every sync hands back, so a status changing under the open window shows at once. A click opens its row.
//!
//! herdr: `src/client/shell/overlays.rs` (`render_navigator_overlay`) at `HERDR_REV` (`build/pins.env`), licensed
//! under the Apache License 2.0, modified: drawn with the palettes' panel and rows instead of terminal cells.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Bounds, Context, FocusHandle, FontWeight, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Pixels, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};
use ngterm_core::chrome::ChromeColors;
use ngterm_core::ghostty_config::binds::Mods;
use ngterm_core::goto::{GotoKey, GotoRow, GotoTarget, GotoView, NO_MATCH, PaneColumns};
use ngterm_core::keymap::Modifiers;
use ngterm_core::persist::AppSettings;
use uuid::Uuid;

use crate::modal::{self, Metrics};
use crate::picker::{self, Placement};
use crate::state::{AppState, Poster, post};
use crate::status_glyph::{self, GlyphLook};
use crate::terminal::input::{KeyStroke, chord};

const PANEL_WIDTH: f64 = 760.0;
const BOTH_COLUMNS: f64 = 520.0;
const AGENT_COLUMN: f64 = 300.0;
const COLUMN_WIDTH: f64 = 90.0;
const TREE_CELL: f64 = 22.0;
const PANEL_CHROME: f64 = 170.0;
const MINIMUM_ROWS: f64 = 120.0;

/// What the window asks of the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Key(GotoKey),
    Open(GotoTarget),
    Close,
}

/// Where the window's requests go: the app (tests collect them).
pub type Sink = Rc<dyn Fn(Request)>;

#[must_use]
pub fn sink(window: Uuid, poster: &Poster) -> Sink {
    let poster = poster.clone();
    Rc::new(move |request| {
        post(&poster, move |state: &mut AppState| state.goto_request(window, request));
    })
}

/// The window's reading of a press: the navigation keys by name, a letter with Control alone held, else the text a key
/// typed with neither Control nor Super held beyond those its layout used up, with the key its chord names for the
/// list's letters.
#[must_use]
pub fn goto_key(stroke: &KeyStroke) -> Option<GotoKey> {
    let named = match stroke.key.name() {
        "arrow_up" => Some(GotoKey::Up),
        "arrow_down" => Some(GotoKey::Down),
        "arrow_left" => Some(GotoKey::Left),
        "arrow_right" => Some(GotoKey::Right),
        "home" => Some(GotoKey::Home),
        "end" => Some(GotoKey::End),
        "enter" | "numpad_enter" => Some(GotoKey::Enter),
        "escape" => Some(GotoKey::Escape),
        "backspace" => Some(GotoKey::Backspace),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if let Some(chord) = chord(stroke).filter(|chord| chord.mods == Modifiers::CONTROL) {
        let mut letters = chord.key.chars();
        return letters
            .next()
            .filter(|_| letters.next().is_none())
            .map(GotoKey::Control);
    }
    let typed = !stroke.composing
        && !stroke
            .mods
            .difference(stroke.consumed)
            .intersects(Mods::CTRL | Mods::SUPER)
        && !stroke.text.is_empty()
        && !stroke.text.chars().any(char::is_control);
    typed.then(|| GotoKey::Text {
        text: stroke.text.clone(),
        key: list_key(stroke).unwrap_or_else(|| stroke.text.clone()),
    })
}

fn list_key(stroke: &KeyStroke) -> Option<String> {
    let chord = chord(stroke)?;
    let mut chars = chord.key.chars();
    let key = chars.next().filter(|_| chars.next().is_none())?;
    if chord.mods.is_empty() {
        Some(key.to_string())
    } else if chord.mods == Modifiers::SHIFT && key.is_ascii_lowercase() {
        Some(key.to_ascii_uppercase().to_string())
    } else {
        None
    }
}

/// Which of a pane row's columns a panel shows: both, the agent alone, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Columns {
    None,
    Agent,
    Both,
}

impl Columns {
    #[must_use]
    pub fn of(width: f64, metrics: Metrics) -> Self {
        if width >= metrics.scaled(BOTH_COLUMNS) {
            Self::Both
        } else if width >= metrics.scaled(AGENT_COLUMN) {
            Self::Agent
        } else {
            Self::None
        }
    }
}

/// Each row's status glyph under `settings`: the session's on the pane that holds its status.
#[must_use]
pub fn glyphs(view: &GotoView, settings: &AppSettings) -> Vec<Option<GlyphLook>> {
    view.rows
        .iter()
        .map(|row| {
            row.pane
                .as_ref()
                .and_then(|pane| pane.indicator.as_ref())
                .and_then(|indicator| status_glyph::resolve(indicator, settings))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Reveal {
    Settled,
    Due,
    Against(Bounds<Pixels>),
}

/// One open Go to window (module docs).
pub struct GotoPanel {
    view: GotoView,
    glyphs: Vec<Option<GlyphLook>>,
    sink: Sink,
    focus: FocusHandle,
    metrics: Metrics,
    scroll: ScrollHandle,
    reveal: Reveal,
    floor: Rc<Cell<Pixels>>,
}

impl GotoPanel {
    /// A panel showing `view` with `glyphs` at the interface size `metrics`; its requests go to `sink`.
    pub fn new(
        (view, glyphs): (GotoView, Vec<Option<GlyphLook>>),
        metrics: Metrics,
        sink: Sink,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            view,
            glyphs,
            sink,
            focus: cx.focus_handle(),
            metrics,
            scroll: ScrollHandle::new(),
            reveal: Reveal::Due,
            floor: Rc::new(Cell::new(px(0.))),
        }
    }

    #[must_use]
    pub fn view(&self) -> &GotoView {
        &self.view
    }

    #[must_use]
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// The core's rows now and the interface size.
    pub fn refresh(
        &mut self,
        (view, glyphs): (GotoView, Vec<Option<GlyphLook>>),
        metrics: Metrics,
        cx: &mut Context<Self>,
    ) {
        if (&self.view, &self.glyphs, self.metrics) == (&view, &glyphs, metrics) {
            return;
        }
        if view.selected != self.view.selected || targets(&view).ne(targets(&self.view)) {
            self.reveal = Reveal::Due;
        }
        self.view = view;
        self.glyphs = glyphs;
        self.metrics = metrics;
        cx.notify();
    }

    pub fn key(&mut self, key: GotoKey, cx: &mut Context<Self>) {
        (self.sink)(Request::Key(key));
        self.reveal = Reveal::Due;
        cx.notify();
    }

    fn measured(&self) -> bool {
        self.scroll.bounds() != Bounds::default()
    }

    fn reveal_selected(&mut self, window: &mut Window) {
        let viewport = self.scroll.bounds();
        self.reveal = match self.reveal {
            Reveal::Settled => Reveal::Settled,
            Reveal::Against(measured) if measured == viewport => Reveal::Settled,
            Reveal::Due | Reveal::Against(_) => {
                if let Some(selected) = self.view.selected.filter(|_| self.measured()) {
                    self.scroll.scroll_to_item(selected);
                }
                window.request_animation_frame();
                Reveal::Against(viewport)
            }
        };
    }

    fn header(&self, colors: ChromeColors) -> AnyElement {
        let metrics = self.metrics;
        let ink = crate::chrome::color(colors.foreground, 1.0);
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        let mut search = self.view.search_line();
        if self.view.searching {
            search.push('▏');
        }
        let count = self.view.count_label();
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(modal::logical(metrics.scaled(4.0)))
            .px(modal::logical(metrics.scaled(12.0)))
            .py(modal::logical(metrics.scaled(8.0)))
            .text_size(modal::logical(metrics.base))
            .child(
                div()
                    .debug_selector(|| "go-to-title".to_owned())
                    .text_color(crate::chrome::color(modal::ACCENT, 1.0))
                    .child("Go to"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        labelled("go-to-search", search)
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(if self.view.shows_placeholder() { dim } else { ink }),
                    )
                    .child(labelled("go-to-count", count).flex_none().text_color(dim)),
            )
            .into_any_element()
    }

    fn footer(&self, colors: ChromeColors) -> AnyElement {
        let metrics = self.metrics;
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        let (detail, meta) = self
            .view
            .selected_row()
            .map(|row| (row.detail.clone(), row.meta.clone()))
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(2.))
            .px(modal::logical(metrics.scaled(12.0)))
            .py(modal::logical(metrics.scaled(8.0)))
            .text_size(modal::logical(metrics.base))
            .child(
                labelled("go-to-detail", detail)
                    .truncate()
                    .text_color(crate::chrome::color(colors.foreground, 0.8)),
            )
            .child(div().truncate().text_color(dim).child(SharedString::from(meta)))
            .child(
                labelled("go-to-hints", self.view.hints().to_owned())
                    .truncate()
                    .text_color(dim),
            )
            .into_any_element()
    }

    fn render_row(
        &self,
        index: usize,
        row: &GotoRow,
        (colors, columns, animate): (ChromeColors, Columns, bool),
    ) -> AnyElement {
        let id = SharedString::from(format!("go-to-row-{}", row.target.id()));
        let sink = Rc::clone(&self.sink);
        let target = row.target;
        let frame = picker::row(
            (id, &row.label),
            self.view.selected == Some(index),
            true,
            self.metrics,
            colors,
        )
        .text_size(modal::logical(self.metrics.base))
        .on_click(move |_, _, _| sink(Request::Open(target)));
        let label = div()
            .debug_selector(move || format!("go-to-label-{}", target.id()))
            .flex_1()
            .min_w(px(0.))
            .truncate()
            .child(SharedString::from(row.label.clone()));
        if row.is_workspace() {
            return frame
                .child(label.font_weight(FontWeight::BOLD))
                .child(
                    div()
                        .flex_none()
                        .text_color(crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA))
                        .child(SharedString::from(row.meta.clone())),
                )
                .into_any_element();
        }
        let glyph = self.glyphs.get(index).cloned().flatten();
        let current = row.pane.as_ref().is_some_and(|pane| pane.current);
        frame
            .children(self.tree(row, colors))
            .children(self.pane_lead(current, glyph, colors, animate))
            .child(label)
            .children(
                row.pane
                    .as_ref()
                    .map(|pane| self.pane_columns(pane, columns, colors))
                    .unwrap_or_default(),
            )
            .into_any_element()
    }

    fn tree(&self, row: &GotoRow, colors: ChromeColors) -> Vec<AnyElement> {
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        let target = row.target;
        row.tree
            .iter()
            .enumerate()
            .map(|(level, &lines)| {
                div()
                    .debug_selector(move || format!("go-to-tree-{}-{level}", target.id()))
                    .flex_none()
                    .w(modal::logical(self.metrics.scaled(TREE_CELL)))
                    .pl(modal::logical(self.metrics.scaled(4.0)))
                    .text_color(dim)
                    .child(lines)
                    .into_any_element()
            })
            .collect()
    }

    fn pane_lead(
        &self,
        current: bool,
        glyph: Option<GlyphLook>,
        colors: ChromeColors,
        animate: bool,
    ) -> [AnyElement; 2] {
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        [
            div()
                .flex_none()
                .w(modal::logical(self.metrics.scaled(10.0)))
                .text_color(dim)
                .when(current, |marker| marker.child("◆"))
                .into_any_element(),
            div()
                .flex_none()
                .size(px(crate::row_icon::SLOT))
                .children(glyph.map(|look| status_glyph::glyph(&look, animate).into_any_element()))
                .into_any_element(),
        ]
    }

    fn pane_columns(&self, pane: &PaneColumns, columns: Columns, colors: ChromeColors) -> Vec<AnyElement> {
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        let column = |text: &'static str, selector: &'static str| {
            div()
                .debug_selector(move || selector.to_owned())
                .flex_none()
                .w(modal::logical(self.metrics.scaled(COLUMN_WIDTH)))
                .truncate()
                .text_color(dim)
                .child(text)
                .into_any_element()
        };
        let agent = column(pane.agent, "go-to-agent");
        match columns {
            Columns::None => Vec::new(),
            Columns::Agent => vec![agent],
            Columns::Both => vec![agent, column(pane.status.name(), "go-to-status")],
        }
    }

    fn rows(&self, placement: Placement, colors: ChromeColors, animate: bool) -> AnyElement {
        let columns = Columns::of(placement.width, self.metrics);
        let cap = (placement.cap - self.metrics.scaled(PANEL_CHROME)).max(self.metrics.scaled(MINIMUM_ROWS));
        let rows: Vec<AnyElement> = if self.view.rows.is_empty() {
            vec![
                labelled("go-to-empty", NO_MATCH.to_owned())
                    .px(modal::logical(self.metrics.scaled(12.0)))
                    .py(modal::logical(self.metrics.scaled(6.0)))
                    .text_color(crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA))
                    .into_any_element(),
            ]
        } else {
            self.view
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| self.render_row(index, row, (colors, columns, animate)))
                .collect()
        };
        div()
            .id("go-to-rows")
            .debug_selector(|| "go-to-rows".to_owned())
            .flex()
            .flex_col()
            .flex_grow(1.)
            .min_h(px(0.))
            .max_h(modal::logical(cap))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .when(!self.measured(), gpui_kit::Styled::invisible)
            .children(rows)
            .into_any_element()
    }
}

fn targets(view: &GotoView) -> impl Iterator<Item = GotoTarget> + '_ {
    view.rows.iter().map(|row| row.target)
}

fn labelled(id: &'static str, text: String) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .accessibility_id(id)
        .role(gpui_kit::Role::Label)
        .aria_value(SharedString::from(text.clone()))
        .debug_selector(move || id.to_owned())
        .child(SharedString::from(text))
}

impl Render for GotoPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.reveal_selected(window);
        let colors = crate::chrome::current(cx);
        let animate = status_glyph::animations_enabled(cx);
        let placement = Placement::of(window.viewport_size(), 0.0, PANEL_WIDTH, self.metrics);
        let separator = crate::chrome::separator(&colors);
        let sink = Rc::clone(&self.sink);
        let floor = Rc::clone(&self.floor);
        div()
            .on_children_prepainted(move |panel, _, _| {
                if let Some(panel) = panel.first() {
                    floor.set(floor.get().max(panel.size.height));
                }
            })
            .id("go-to-scrim")
            .accessibility_id("go-to-scrim")
            .role(gpui_kit::Role::Button)
            .aria_label("Dismiss")
            .debug_selector(|| "go-to-scrim".to_owned())
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(modal::dim())
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                sink(Request::Close);
            })
            .child(
                modal::panel(colors)
                    .id("go-to")
                    .accessibility_id("go-to")
                    .role(gpui_kit::Role::Dialog)
                    .debug_selector(|| "go-to".to_owned())
                    .track_focus(&self.focus)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .w(modal::logical(placement.width))
                    .min_h(self.floor.get().min(modal::logical(placement.cap)))
                    .max_h(modal::logical(placement.cap))
                    .flex()
                    .flex_col()
                    .child(self.header(colors))
                    .child(div().h(px(1.)).flex_shrink_0().bg(separator))
                    .child(self.rows(placement, colors, animate))
                    .child(div().h(px(1.)).flex_shrink_0().bg(separator))
                    .child(self.footer(colors)),
            )
    }
}

impl AppState {
    pub fn goto_request(&mut self, window: Uuid, request: Request) {
        if self.is_quitting() {
            return;
        }
        match request {
            Request::Key(key) => self.core.goto_key(window, key),
            Request::Open(target) => self.core.goto_open(window, target),
            Request::Close if self.core.goto_holds(window) => self.core.close_goto(),
            Request::Close => {}
        }
        self.settle();
    }
}

#[cfg(test)]
mod tests;
