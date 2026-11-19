//! The Go to window, an extension beyond upstream (which has none): one window's workspaces as a tree of their
//! sessions' panes, searched by text and filtered by agent status, which opens the pane it lands on.
//!
//! A workspace row names the workspace and the branch checked out where its first session's main pane started
//! ([`branch::git_branch`], read when the window opens; none for a session attached from another machine). Under it
//! comes one row per pane: each terminal session's main pane and, while it has one, its split pane (a page companion
//! has none), labelled with the session's name, or `{agent} · {session}` for each pane of a split session. A pane's
//! agent is the one the hooks last reported in it (`terminal` without one); its status is the session's while the pane
//! holds the session's status, as the sidebar's glyph shows it (`shell` while that is idle with no agent), and on the
//! other pane `unknown` with an agent, `shell` without one.
//!
//! The query's words must all be in one field of a pane (its workspace and branch, the session, the label, the cwd,
//! the agent, the pane's title), and a workspace that matches keeps every pane. A filter keeps the panes of one
//! status. With either on, a workspace left with no pane hides.
//!
//! herdr: `src/client/shell/aggregate_navigation.rs` (`navigator_rows`, `navigator_selected_index`),
//! `src/client/shell/overlay_input.rs` (the navigator's keys, `move_navigator_selection`, `move_navigator_workspace`),
//! `src/client/shell/overlays.rs` (`render_navigator_overlay`'s lines) and `src/client/shell/state.rs`
//! (`ClientNavigatorOverlay`, `ClientNavigatorFilter`) at `HERDR_REV` (`build/pins.env`), licensed under the Apache
//! License 2.0, modified: a window's workspaces and sessions in place of herdr's machines, tabs and panes, a query
//! edited at its end only, and the rows a pure function of the model the frontend draws.

pub mod branch;
mod runtime;
mod state;

use uuid::Uuid;

use crate::agent_chat::AgentKind;
use crate::model::{AgentIndicator, AgentStatus, StatusPane};
use crate::pane_env::uuid_string;

pub(crate) use runtime::GotoRuntime;
pub use state::GotoKey;

/// The search line while no query or filter is on.
pub const PLACEHOLDER: &str = "/ search agents and terminals";
/// What the rows say when nothing matches.
pub const NO_MATCH: &str = "No matching agents or terminals";
pub const HINTS: &str = "↑↓/j/k rows · ←→ workspace · / search · a/b/w/i/d filter · enter open · esc close";
/// The key hints while the query takes the keys.
pub const SEARCH_HINTS: &str = "search type · move ↑↓/ctrl+n/p · open enter · back esc";

/// What a row opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoTarget {
    /// A workspace: its first session.
    Workspace(Uuid),
    /// A session's main pane ([`StatusPane::Left`]) or split pane ([`StatusPane::Right`]).
    Pane { session: Uuid, pane: StatusPane },
}

impl GotoTarget {
    /// The row's element id: the workspace's id, or `<session>:left` and `<session>:right`.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::Workspace(workspace) => uuid_string(workspace),
            Self::Pane { session, pane } => format!("{}:{}", uuid_string(session), pane.as_str()),
        }
    }
}

/// The status filter: `b`, `w`, `i` and `d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoFilter {
    Blocked,
    Working,
    Idle,
    Done,
}

impl GotoFilter {
    #[must_use]
    pub fn name(self) -> &'static str {
        self.status().name()
    }

    fn status(self) -> PaneStatus {
        match self {
            Self::Blocked => PaneStatus::Blocked,
            Self::Working => PaneStatus::Working,
            Self::Idle => PaneStatus::Idle,
            Self::Done => PaneStatus::Done,
        }
    }

    fn of_key(key: &str) -> Option<Self> {
        Some(match key {
            "b" => Self::Blocked,
            "w" => Self::Working,
            "i" => Self::Idle,
            "d" => Self::Done,
            _ => return None,
        })
    }
}

/// A pane row's status column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneStatus {
    Shell,
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl PaneStatus {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }

    fn of(status: AgentStatus) -> Self {
        match status {
            AgentStatus::Idle => Self::Idle,
            AgentStatus::Active => Self::Working,
            AgentStatus::Blocked => Self::Blocked,
            AgentStatus::Completed => Self::Done,
        }
    }

    fn passes(self, filter: Option<GotoFilter>) -> bool {
        filter.is_none_or(|filter| filter.status() == self)
    }
}

/// One pane as the rows read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoPane {
    pub session: Uuid,
    pub pane: StatusPane,
    pub session_name: String,
    /// The session has a split pane.
    pub split: bool,
    pub title: Option<String>,
    pub cwd: String,
    pub agent: Option<AgentKind>,
    pub status: PaneStatus,
    /// The session's indicator, on the pane that holds the session's status.
    pub indicator: Option<AgentIndicator>,
    /// The pane the window shows focused.
    pub current: bool,
}

impl GotoPane {
    fn kind(&self) -> &'static str {
        self.agent.map_or("terminal", AgentKind::name)
    }

    fn label(&self) -> String {
        if self.split {
            format!("{} · {}", self.kind(), self.session_name)
        } else {
            self.session_name.clone()
        }
    }
}

/// One workspace as the rows read it, its panes in sidebar order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoWorkspace {
    pub id: Uuid,
    pub name: String,
    pub branch: Option<String>,
    pub panes: Vec<GotoPane>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoRow {
    pub target: GotoTarget,
    pub label: String,
    /// The first line under the rows: `{workspace} / {session}`, or a workspace's name.
    pub detail: String,
    /// The second: a pane's cwd, or a workspace's branch.
    pub meta: String,
    /// A pane row's columns; `None` on a workspace row.
    pub pane: Option<PaneColumns>,
}

/// What a pane row shows beside its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneColumns {
    pub agent: &'static str,
    pub status: PaneStatus,
    pub indicator: Option<AgentIndicator>,
    pub current: bool,
    /// The last pane row under its workspace row.
    pub last: bool,
}

#[must_use]
pub fn rows(workspaces: &[GotoWorkspace], query: &str, filter: Option<GotoFilter>) -> Vec<GotoRow> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let matches = |value: &str| {
        let value = value.to_lowercase();
        words.iter().all(|word| value.contains(word))
    };
    let filtering = filter.is_some() || !words.is_empty();
    let mut rows = Vec::new();
    for workspace in workspaces {
        let workspace_matches = matches(&workspace.name) || workspace.branch.as_deref().is_some_and(matches);
        let mut panes: Vec<GotoRow> = workspace
            .panes
            .iter()
            .filter(|pane| pane.status.passes(filter) && (workspace_matches || pane_matches(pane, &matches)))
            .map(|pane| pane_row(workspace, pane))
            .collect();
        if let Some(last) = panes.last_mut().and_then(|row| row.pane.as_mut()) {
            last.last = true;
        }
        if !filtering || !panes.is_empty() || (filter.is_none() && workspace_matches) {
            rows.push(workspace_row(workspace));
            rows.extend(panes);
        }
    }
    rows
}

fn pane_matches(pane: &GotoPane, matches: &impl Fn(&str) -> bool) -> bool {
    matches(&pane.session_name)
        || matches(&pane.label())
        || matches(&pane.cwd)
        || pane.agent.is_some_and(|agent| matches(agent.name()))
        || pane.title.as_deref().is_some_and(matches)
}

fn workspace_row(workspace: &GotoWorkspace) -> GotoRow {
    GotoRow {
        target: GotoTarget::Workspace(workspace.id),
        label: workspace.name.clone(),
        detail: workspace.name.clone(),
        meta: workspace.branch.clone().unwrap_or_default(),
        pane: None,
    }
}

fn pane_row(workspace: &GotoWorkspace, pane: &GotoPane) -> GotoRow {
    GotoRow {
        target: GotoTarget::Pane {
            session: pane.session,
            pane: pane.pane,
        },
        label: pane.label(),
        detail: format!("{} / {}", workspace.name, pane.session_name),
        meta: pane.cwd.clone(),
        pane: Some(PaneColumns {
            agent: pane.kind(),
            status: pane.status,
            indicator: pane.indicator.clone(),
            current: pane.current,
            last: false,
        }),
    }
}

/// The selected row of `rows`: the row of `selected`, the first row when it is gone, the first pane row with nothing
/// selected; `None` with no rows.
#[must_use]
pub fn selected_index(rows: &[GotoRow], selected: Option<GotoTarget>) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    Some(match selected {
        Some(target) => rows.iter().position(|row| row.target == target).unwrap_or(0),
        None => rows.iter().position(|row| row.pane.is_some()).unwrap_or(0),
    })
}

/// What the window draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoView {
    pub rows: Vec<GotoRow>,
    pub selected: Option<usize>,
    pub query: String,
    /// The query takes the keys.
    pub searching: bool,
    pub filter: Option<GotoFilter>,
}

impl GotoView {
    /// The search line: `/ ` and the query or the filter, else [`PLACEHOLDER`].
    #[must_use]
    pub fn search_line(&self) -> String {
        match self.filter {
            Some(filter) if !self.searching => format!("/ {}", filter.name()),
            _ if self.searching || !self.query.is_empty() => format!("/ {}", self.query),
            _ => PLACEHOLDER.to_owned(),
        }
    }

    #[must_use]
    pub fn shows_placeholder(&self) -> bool {
        !self.searching && self.filter.is_none() && self.query.is_empty()
    }

    /// `N terminal(s)`: the pane rows shown.
    #[must_use]
    pub fn count_label(&self) -> String {
        let count = self.rows.iter().filter(|row| row.pane.is_some()).count();
        let noun = if count == 1 { "terminal" } else { "terminals" };
        format!("{count} {noun}")
    }

    #[must_use]
    pub fn hints(&self) -> &'static str {
        if self.searching { SEARCH_HINTS } else { HINTS }
    }

    #[must_use]
    pub fn selected_row(&self) -> Option<&GotoRow> {
        self.selected.and_then(|index| self.rows.get(index))
    }
}

#[cfg(test)]
mod tests;
