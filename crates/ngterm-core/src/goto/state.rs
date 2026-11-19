//! The open window's query, filter and selection, and what its keys do to them.
//!
//! herdr: `src/client/shell/overlay_input.rs` (the navigator's keys, `move_navigator_selection`,
//! `move_navigator_workspace`) at `HERDR_REV` (`build/pins.env`), licensed under the Apache License 2.0, modified.

use super::{GotoFilter, GotoRow, GotoTarget, GotoView, GotoWorkspace, rows, selected_index};

const PAGE: isize = 8;

/// A key press as the window reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GotoKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
    Backspace,
    /// A letter with Control alone held.
    Control(char),
    /// A key with neither Control nor Super held: the `text` it typed, for the query, and the `key` its chord names,
    /// for the list's letters (by position on a layout typing no Latin letters).
    Text {
        text: String,
        key: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GotoOutcome {
    Stay,
    Close,
    Open(GotoTarget),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GotoState {
    query: String,
    searching: bool,
    filter: Option<GotoFilter>,
    selected: Option<GotoTarget>,
}

impl GotoState {
    pub(crate) fn opened(selected: Option<GotoTarget>) -> Self {
        Self {
            selected,
            ..Self::default()
        }
    }

    pub(crate) fn view(&self, workspaces: &[GotoWorkspace]) -> GotoView {
        let rows = self.rows(workspaces);
        GotoView {
            selected: selected_index(&rows, self.selected),
            rows,
            query: self.query.clone(),
            searching: self.searching,
            filter: self.filter,
        }
    }

    pub(crate) fn key(&mut self, key: GotoKey, workspaces: &[GotoWorkspace]) -> GotoOutcome {
        match key {
            GotoKey::Escape if self.searching => self.searching = false,
            GotoKey::Escape => return GotoOutcome::Close,
            GotoKey::Enter => return self.open(workspaces),
            key if self.searching => self.search_key(key, workspaces),
            key => self.list_key(key, workspaces),
        }
        GotoOutcome::Stay
    }

    fn rows(&self, workspaces: &[GotoWorkspace]) -> Vec<GotoRow> {
        rows(workspaces, &self.query, self.filter)
    }

    fn open(&self, workspaces: &[GotoWorkspace]) -> GotoOutcome {
        let rows = self.rows(workspaces);
        selected_index(&rows, self.selected)
            .and_then(|index| rows.get(index))
            .map_or(GotoOutcome::Stay, |row| GotoOutcome::Open(row.target))
    }

    fn search_key(&mut self, key: GotoKey, workspaces: &[GotoWorkspace]) {
        match key {
            GotoKey::Text { text, .. } => {
                self.query.push_str(&text);
                self.query_changed();
            }
            GotoKey::Backspace => {
                if self.query.pop().is_some() {
                    self.query_changed();
                }
            }
            GotoKey::Up | GotoKey::Control('p') => self.step(-1, workspaces),
            GotoKey::Down | GotoKey::Control('n') => self.step(1, workspaces),
            _ => {}
        }
    }

    fn list_key(&mut self, key: GotoKey, workspaces: &[GotoWorkspace]) {
        match key {
            GotoKey::Up => self.step(-1, workspaces),
            GotoKey::Down => self.step(1, workspaces),
            GotoKey::Control('u') => self.step(-PAGE, workspaces),
            GotoKey::Control('d') => self.step(PAGE, workspaces),
            GotoKey::Left => self.jump_workspace(false, workspaces),
            GotoKey::Right => self.jump_workspace(true, workspaces),
            GotoKey::Home => self.selected = None,
            GotoKey::End => self.select_last(workspaces),
            GotoKey::Backspace => {
                if self.filter.take().is_some() {
                    self.selected = None;
                }
            }
            GotoKey::Text { key, .. } => self.letter(&key, workspaces),
            _ => {}
        }
    }

    fn letter(&mut self, key: &str, workspaces: &[GotoWorkspace]) {
        match key {
            "j" => self.step(1, workspaces),
            "k" => self.step(-1, workspaces),
            "G" => self.select_last(workspaces),
            "/" => {
                self.searching = true;
                self.filter = None;
            }
            "a" => {
                self.query.clear();
                self.filter = None;
                self.selected = None;
            }
            _ => {
                if let Some(filter) = GotoFilter::of_key(key) {
                    self.query.clear();
                    self.filter = Some(filter);
                    self.selected = None;
                }
            }
        }
    }

    fn query_changed(&mut self) {
        self.filter = None;
        self.selected = None;
    }

    fn step(&mut self, delta: isize, workspaces: &[GotoWorkspace]) {
        let rows = self.rows(workspaces);
        let Some(index) = selected_index(&rows, self.selected) else {
            self.selected = None;
            return;
        };
        let next = index.saturating_add_signed(delta).min(rows.len() - 1);
        self.selected = rows.get(next).map(|row| row.target);
    }

    fn select_last(&mut self, workspaces: &[GotoWorkspace]) {
        self.selected = self.rows(workspaces).last().map(|row| row.target);
    }

    fn jump_workspace(&mut self, forward: bool, workspaces: &[GotoWorkspace]) {
        let rows = self.rows(workspaces);
        let Some(selected) = selected_index(&rows, self.selected) else {
            return;
        };
        let section = rows[..=selected]
            .iter()
            .rposition(|row| row.pane.is_none())
            .unwrap_or(selected);
        let mut destinations = rows.windows(2).enumerate().filter(|(index, pair)| {
            pair[0].pane.is_none()
                && pair[1].pane.is_some()
                && if forward { *index > section } else { *index < section }
        });
        let destination = if forward {
            destinations.next()
        } else {
            destinations.next_back()
        };
        if let Some((_, pair)) = destination {
            self.selected = Some(pair[1].target);
        }
    }
}
