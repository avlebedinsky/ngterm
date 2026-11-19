//! The Go to window, an extension beyond upstream (which has none): one window's workspaces as a tree of their
//! sessions' panes, searched by text and filtered by agent status, which opens the pane it lands on.
//!
//! A workspace row names the workspace and the branch checked out where its first session's main pane started
//! ([`branch::git_branch`], read when the window opens; none for a session attached from another machine). Under it
//! comes one row per terminal session (a page companion has none) on a `├─`/`└─` connector, nested as the sidebar nests
//! them: a session's child sessions (its companions) hang a level under its row, to any depth, a `│` running down a
//! level while more of its rows follow. An unsplit session's row is its pane's, labelled with the session's name; a
//! split session's row names the session and holds one row per pane, labelled `{agent} · {session}`, ahead of its child
//! sessions. A pane's agent is the one the hooks last reported in it (`terminal` without one); its status is
//! the session's while the pane holds the session's status, as the sidebar's glyph shows it (`shell` while that is idle
//! with no agent), and on the other pane `unknown` with an agent, `shell` without one.
//!
//! The query's words must all be in one field of a pane (its workspace and branch, the session, the label, the cwd,
//! the agent, the pane's title), and a workspace that matches keeps every pane. A status filter keeps the panes of one
//! status. With either on, a workspace left with no pane hides, and a session kept only over a child's rows shows as a
//! session row. The top-level filter keeps every workspace with its top-level sessions alone, a split one as one
//! session row, and moves a selection it hides to its top-level session's row.
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

use std::iter::Peekable;

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
pub const HINTS: &str = "↑↓/j/k rows · ←→ workspace · / search · a/b/w/i/d/t filter · enter open · esc close";
/// The key hints while the query takes the keys.
pub const SEARCH_HINTS: &str = "search type · move ↑↓/ctrl+n/p · open enter · back esc";

/// What a row opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoTarget {
    /// A workspace: its first session.
    Workspace(Uuid),
    /// A session row's session (a split one's, or one kept over its children's rows), its focus as it is.
    Session(Uuid),
    /// A session's main pane ([`StatusPane::Left`]) or split pane ([`StatusPane::Right`]).
    Pane { session: Uuid, pane: StatusPane },
}

impl GotoTarget {
    /// The row's element id: the workspace's or the session's id, or `<session>:left` and `<session>:right`.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::Workspace(id) | Self::Session(id) => uuid_string(id),
            Self::Pane { session, pane } => format!("{}:{}", uuid_string(session), pane.as_str()),
        }
    }
}

/// The filter: a status's (`b`, `w`, `i` and `d`), or the top-level sessions' (`t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoFilter {
    Blocked,
    Working,
    Idle,
    Done,
    TopLevel,
}

impl GotoFilter {
    #[must_use]
    pub fn name(self) -> &'static str {
        self.status().map_or("top level", PaneStatus::name)
    }

    fn status(self) -> Option<PaneStatus> {
        match self {
            Self::Blocked => Some(PaneStatus::Blocked),
            Self::Working => Some(PaneStatus::Working),
            Self::Idle => Some(PaneStatus::Idle),
            Self::Done => Some(PaneStatus::Done),
            Self::TopLevel => None,
        }
    }

    fn of_key(key: &str) -> Option<Self> {
        Some(match key {
            "b" => Self::Blocked,
            "w" => Self::Working,
            "i" => Self::Idle,
            "d" => Self::Done,
            "t" => Self::TopLevel,
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
        filter.and_then(GotoFilter::status).is_none_or(|status| status == self)
    }
}

/// One pane as the rows read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoPane {
    pub session: Uuid,
    pub parent: Option<Uuid>,
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
    /// The tree lines before the label, one per level under the workspace: `├─`, or `└─` on the last row of its
    /// level, after a `│` for each session it hangs under (blank under a last row); none on a workspace row.
    pub tree: Vec<&'static str>,
    pub label: String,
    /// The first line under the rows: `{workspace} / {session}`, `{workspace} / {session} / {agent}` on a split
    /// session's pane, or a workspace's name.
    pub detail: String,
    /// The second: a session's or a pane's cwd, or a workspace's branch.
    pub meta: String,
    /// A pane row's columns, and under the top-level filter a split session's (its pane that holds the status), current
    /// there with any pane of the session or of its child sessions; `None` on a workspace or another session row.
    pub pane: Option<PaneColumns>,
    pub terminals: usize,
}

impl GotoRow {
    #[must_use]
    pub fn is_workspace(&self) -> bool {
        matches!(self.target, GotoTarget::Workspace(_))
    }
}

/// What a pane row shows beside its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneColumns {
    pub agent: &'static str,
    pub status: PaneStatus,
    pub indicator: Option<AgentIndicator>,
    pub current: bool,
}

const BRANCH: &str = "├─";
const LAST_BRANCH: &str = "└─";
const RAIL: &str = "│";

#[must_use]
pub fn rows(workspaces: &[GotoWorkspace], query: &str, filter: Option<GotoFilter>) -> Vec<GotoRow> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let matches = |value: &str| {
        let value = value.to_lowercase();
        words.iter().all(|word| value.contains(word))
    };
    let status = filter.and_then(GotoFilter::status);
    let narrowing = status.is_some() || !words.is_empty();
    let mut rows = Vec::new();
    for workspace in workspaces {
        let workspace_matches = matches(&workspace.name) || workspace.branch.as_deref().is_some_and(matches);
        let keeps = |pane: &GotoPane| pane.status.passes(filter) && (workspace_matches || pane_matches(pane, &matches));
        let sessions: Vec<Node> = nodes(workspace).iter().filter_map(|node| node.kept(&keeps)).collect();
        if !narrowing || !sessions.is_empty() || (status.is_none() && workspace_matches) {
            rows.push(workspace_row(workspace));
            workspace_sessions(&mut rows, workspace, &sessions, filter == Some(GotoFilter::TopLevel));
        }
    }
    rows
}

struct Node<'a> {
    main: &'a GotoPane,
    panes: Vec<&'a GotoPane>,
    children: Vec<Node<'a>>,
}

impl<'a> Node<'a> {
    fn kept(&self, keeps: &impl Fn(&GotoPane) -> bool) -> Option<Self> {
        let panes: Vec<&'a GotoPane> = self.panes.iter().copied().filter(|pane| keeps(pane)).collect();
        let children: Vec<Self> = self.children.iter().filter_map(|child| child.kept(keeps)).collect();
        (!panes.is_empty() || !children.is_empty()).then_some(Self {
            main: self.main,
            panes,
            children,
        })
    }

    fn holds(&self, session: Uuid) -> bool {
        self.main.session == session || self.children.iter().any(|child| child.holds(session))
    }

    fn current(&self) -> bool {
        self.panes.iter().any(|pane| pane.current) || self.children.iter().any(Self::current)
    }
}

fn nodes(workspace: &GotoWorkspace) -> Vec<Node<'_>> {
    let mut sessions = workspace
        .panes
        .chunk_by(|one, other| one.session == other.session)
        .filter_map(|panes| Some((panes.first()?, panes)))
        .peekable();
    nested(&mut sessions, None)
}

fn nested<'a>(
    sessions: &mut Peekable<impl Iterator<Item = (&'a GotoPane, &'a [GotoPane])>>,
    parent: Option<Uuid>,
) -> Vec<Node<'a>> {
    let mut nodes = Vec::new();
    while let Some((main, panes)) =
        sessions.next_if(|(main, _)| parent.is_none_or(|parent| main.parent == Some(parent)))
    {
        nodes.push(Node {
            main,
            panes: panes.iter().collect(),
            children: nested(sessions, Some(main.session)),
        });
    }
    nodes
}

fn workspace_sessions(rows: &mut Vec<GotoRow>, workspace: &GotoWorkspace, sessions: &[Node<'_>], top_level: bool) {
    let last = sessions.len().saturating_sub(1);
    for (index, session) in sessions.iter().enumerate() {
        if top_level {
            rows.push(top_level_row(workspace, session, tree(&[], index == last)));
        } else {
            session_rows(rows, workspace, session, (&[], index == last));
        }
    }
}

fn tree(rails: &[&'static str], last: bool) -> Vec<&'static str> {
    let connector = if last { LAST_BRANCH } else { BRANCH };
    rails.iter().copied().chain([connector]).collect()
}

fn session_rows(
    rows: &mut Vec<GotoRow>,
    workspace: &GotoWorkspace,
    session: &Node<'_>,
    (rails, last): (&[&'static str], bool),
) {
    let split_panes: &[&GotoPane] = if session.main.split { &session.panes } else { &[] };
    rows.push(match session.panes.first() {
        Some(pane) if !session.main.split => pane_row(workspace, pane, tree(rails, last)),
        _ => session_row(workspace, session.main, tree(rails, last)),
    });
    let rails: Vec<&'static str> = rails.iter().copied().chain([if last { "" } else { RAIL }]).collect();
    let under = split_panes.len() + session.children.len();
    for (index, pane) in split_panes.iter().enumerate() {
        rows.push(pane_row(workspace, pane, tree(&rails, index + 1 == under)));
    }
    for (index, child) in session.children.iter().enumerate() {
        session_rows(rows, workspace, child, (&rails, split_panes.len() + index + 1 == under));
    }
}

fn top_level_row(workspace: &GotoWorkspace, session: &Node<'_>, tree: Vec<&'static str>) -> GotoRow {
    let (row, holder) = if session.main.split {
        let holder = session
            .panes
            .iter()
            .copied()
            .find(|pane| pane.indicator.is_some())
            .unwrap_or(session.main);
        let row = GotoRow {
            terminals: session.panes.len(),
            ..session_row(workspace, session.main, tree)
        };
        (row, holder)
    } else {
        (pane_row(workspace, session.main, tree), session.main)
    };
    GotoRow {
        pane: Some(PaneColumns {
            current: session.current(),
            ..columns(holder)
        }),
        ..row
    }
}

fn top_level_target(workspaces: &[GotoWorkspace], target: GotoTarget) -> GotoTarget {
    let (GotoTarget::Session(session) | GotoTarget::Pane { session, .. }) = target else {
        return target;
    };
    workspaces
        .iter()
        .flat_map(nodes)
        .find(|node| node.holds(session))
        .map_or(target, |node| GotoTarget::Session(node.main.session))
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
        tree: Vec::new(),
        label: workspace.name.clone(),
        detail: workspace.name.clone(),
        meta: workspace.branch.clone().unwrap_or_default(),
        pane: None,
        terminals: 0,
    }
}

fn session_row(workspace: &GotoWorkspace, main: &GotoPane, tree: Vec<&'static str>) -> GotoRow {
    GotoRow {
        target: GotoTarget::Session(main.session),
        tree,
        label: main.session_name.clone(),
        detail: format!("{} / {}", workspace.name, main.session_name),
        meta: main.cwd.clone(),
        pane: None,
        terminals: 0,
    }
}

fn pane_row(workspace: &GotoWorkspace, pane: &GotoPane, tree: Vec<&'static str>) -> GotoRow {
    let session = format!("{} / {}", workspace.name, pane.session_name);
    GotoRow {
        target: GotoTarget::Pane {
            session: pane.session,
            pane: pane.pane,
        },
        tree,
        label: pane.label(),
        detail: if pane.split {
            format!("{session} / {}", pane.kind())
        } else {
            session
        },
        meta: pane.cwd.clone(),
        pane: Some(columns(pane)),
        terminals: 1,
    }
}

fn columns(pane: &GotoPane) -> PaneColumns {
    PaneColumns {
        agent: pane.kind(),
        status: pane.status,
        indicator: pane.indicator.clone(),
        current: pane.current,
    }
}

/// The selected row of `rows`: the row of `selected`, else its session's row, else its session's main pane, the first
/// row when all are gone, the first pane row with nothing selected; `None` with no rows.
#[must_use]
pub fn selected_index(rows: &[GotoRow], selected: Option<GotoTarget>) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let position = |target| rows.iter().position(|row| row.target == target);
    Some(match selected {
        Some(target @ (GotoTarget::Session(session) | GotoTarget::Pane { session, .. })) => position(target)
            .or_else(|| position(GotoTarget::Session(session)))
            .or_else(|| {
                position(GotoTarget::Pane {
                    session,
                    pane: StatusPane::Left,
                })
            })
            .unwrap_or(0),
        Some(target) => position(target).unwrap_or(0),
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

    /// `N terminal(s)`: the terminals the rows shown count for.
    #[must_use]
    pub fn count_label(&self) -> String {
        let count: usize = self.rows.iter().map(|row| row.terminals).sum();
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
