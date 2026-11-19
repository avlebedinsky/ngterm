use std::collections::HashMap;
use std::path::Path;

use uuid::Uuid;

use super::state::{GotoOutcome, GotoState};
use super::{GotoKey, GotoPane, GotoTarget, GotoView, GotoWorkspace, PaneStatus};
use crate::Core;
use crate::model::{AgentStatus, AppStore, Session, SessionContent, StatusPane};

#[derive(Debug, Default)]
pub(crate) struct GotoRuntime {
    open: Option<OpenGoto>,
}

#[derive(Debug)]
struct OpenGoto {
    window: Uuid,
    state: GotoState,
    branches: HashMap<Uuid, String>,
}

impl Core {
    /// Whether the Go to window is open in window `window`.
    #[must_use]
    pub fn goto_holds(&self, window: Uuid) -> bool {
        self.goto.open.as_ref().is_some_and(|open| open.window == window)
    }

    /// The Go to chord: closes the window open in `window`, else opens one there, the focused pane selected, while
    /// `window` is the frontmost and takes UI actions. Commit afterwards.
    pub fn toggle_goto(&mut self, window: Uuid) {
        if self.goto_holds(window) {
            self.close_goto();
            return;
        }
        if self.library().active_window_id() != Some(window) || !self.ui_actions_enabled(window) {
            return;
        }
        self.close_palette();
        self.end_goto();
        self.forget_exited_agents(window);
        let branches = self.workspace_branches(window);
        let state = GotoState::opened(self.current_goto_target(window));
        self.suppress_auto_follow(window);
        self.goto.open = Some(OpenGoto {
            window,
            state,
            branches,
        });
    }

    /// Closes the Go to window, the selected session's terminal taking the keyboard back. Commit afterwards.
    pub fn close_goto(&mut self) {
        let Some(window) = self.end_goto() else {
            return;
        };
        if let Some(session) = self.library().store(window).and_then(AppStore::selected_session_id) {
            self.request_pane_focus(session);
        }
    }

    /// A key on the Go to window open in `window`. Commit afterwards.
    pub fn goto_key(&mut self, window: Uuid, key: GotoKey) {
        if !self.goto_holds(window) {
            return;
        }
        let workspaces = self.goto_workspaces(window);
        let Some(open) = self.goto.open.as_mut() else {
            return;
        };
        match open.state.key(key, &workspaces) {
            GotoOutcome::Stay => {}
            GotoOutcome::Close => self.close_goto(),
            GotoOutcome::Open(target) => self.goto_open(window, target),
        }
    }

    /// Opens `target` from the Go to window open in `window`, which closes: a pane's session is selected with that
    /// pane focused, a session selected as it is, a workspace's first session selected. Commit afterwards.
    pub fn goto_open(&mut self, window: Uuid, target: GotoTarget) {
        if !self.goto_holds(window) {
            return;
        }
        self.end_goto();
        let Some(store) = self.library_mut().store_mut(window) else {
            return;
        };
        match target {
            GotoTarget::Workspace(workspace) => {
                store.select_workspace(workspace);
            }
            GotoTarget::Pane { session, pane } if store.session(session).is_some() => {
                store.select_session(Some(session));
                if store.session(session).is_some_and(|session| session.scratch_active) {
                    store.toggle_scratch(session, true);
                }
                store.set_split_focus(session, pane == StatusPane::Right);
            }
            GotoTarget::Session(session) if store.session(session).is_some() => {
                store.select_session(Some(session));
            }
            GotoTarget::Pane { .. } | GotoTarget::Session(_) => {}
        }
        self.note_window_activity(window);
        if let Some(session) = self.library().store(window).and_then(AppStore::selected_session_id) {
            self.request_pane_focus(session);
        }
    }

    /// What the Go to window open in `window` draws now; `None` while it is not open there.
    #[must_use]
    pub fn goto_view(&self, window: Uuid) -> Option<GotoView> {
        let open = self.goto.open.as_ref().filter(|open| open.window == window)?;
        Some(open.state.view(&self.goto_workspaces(window)))
    }

    pub(crate) fn settle_goto(&mut self) {
        let Some(window) = self.goto.open.as_ref().map(|open| open.window) else {
            return;
        };
        let holds = self.library().active_window_id() == Some(window)
            && self.ui_actions_enabled(window)
            && self.palette_mode().is_none();
        if !holds {
            self.close_goto();
        }
    }

    fn end_goto(&mut self) -> Option<Uuid> {
        let window = self.goto.open.take()?.window;
        self.resume_auto_follow(window);
        Some(window)
    }

    fn forget_exited_agents(&mut self, window: Uuid) {
        let identities: Vec<Uuid> = self
            .library()
            .store(window)
            .into_iter()
            .flat_map(AppStore::sessions)
            .flat_map(|session| std::iter::once(session.pane_identity).chain(session.split_pane_identity))
            .collect();
        for identity in identities {
            self.presented_agent(identity);
        }
    }

    fn workspace_branches(&self, window: Uuid) -> HashMap<Uuid, String> {
        let Some(store) = self.library().store(window) else {
            return HashMap::new();
        };
        store
            .workspaces()
            .iter()
            .filter_map(|workspace| {
                let first = workspace.sessions.first()?;
                if first.remote_host().is_some() {
                    return None;
                }
                let branch = super::branch::git_branch(Path::new(first.effective_cwd()))?;
                Some((workspace.id(), branch))
            })
            .collect()
    }

    fn current_goto_target(&self, window: Uuid) -> Option<GotoTarget> {
        let store = self.library().store(window)?;
        let session = store.active_session()?;
        if session.content() == SessionContent::Page {
            let parent = store.session(session.parent_id()?)?;
            return Some(GotoTarget::Pane {
                session: parent.id(),
                pane: StatusPane::Left,
            });
        }
        let pane = if session.split.exists && session.focuses_split() {
            StatusPane::Right
        } else {
            StatusPane::Left
        };
        Some(GotoTarget::Pane {
            session: session.id(),
            pane,
        })
    }

    fn focused_goto_pane(&self, window: Uuid) -> Option<GotoTarget> {
        let session = self.library().store(window)?.active_session()?;
        if session.scratch_active || session.content() == SessionContent::Page {
            return None;
        }
        self.current_goto_target(window)
    }

    fn goto_workspaces(&self, window: Uuid) -> Vec<GotoWorkspace> {
        let Some(store) = self.library().store(window) else {
            return Vec::new();
        };
        let branches = self.goto.open.as_ref().map(|open| &open.branches);
        let current = self.focused_goto_pane(window);
        store
            .workspaces()
            .iter()
            .map(|workspace| GotoWorkspace {
                id: workspace.id(),
                name: workspace.name.clone(),
                branch: branches.and_then(|branches| branches.get(&workspace.id()).cloned()),
                panes: workspace
                    .sessions
                    .iter()
                    .filter(|session| session.content() != SessionContent::Page)
                    .flat_map(|session| self.goto_panes(session, current))
                    .collect(),
            })
            .collect()
    }

    fn goto_panes(&self, session: &Session, current: Option<GotoTarget>) -> Vec<GotoPane> {
        let split = session.split.exists;
        let holder = match session.agent_indicator.normalized_pane(split) {
            Some(StatusPane::Right) => StatusPane::Right,
            _ => StatusPane::Left,
        };
        let panes: &[StatusPane] = if split {
            &[StatusPane::Left, StatusPane::Right]
        } else {
            &[StatusPane::Left]
        };
        panes
            .iter()
            .map(|&pane| {
                let identity = match pane {
                    StatusPane::Right => session.split_pane_identity,
                    _ => Some(session.pane_identity),
                };
                let agent = identity.and_then(|identity| self.pane_agent(identity));
                let holds = holder == pane;
                let status = session.agent_indicator.status;
                let title = match pane {
                    StatusPane::Right => session.split_title.clone(),
                    _ => session.osc_title.clone(),
                };
                GotoPane {
                    session: session.id(),
                    parent: session.parent_id(),
                    pane,
                    session_name: session.display_name(),
                    split,
                    title,
                    cwd: session.cwd_for(pane).to_owned(),
                    agent,
                    status: match (agent, holds) {
                        (_, true) if agent.is_some() || status != AgentStatus::Idle => PaneStatus::of(status),
                        (Some(_), _) => PaneStatus::Unknown,
                        (None, _) => PaneStatus::Shell,
                    },
                    indicator: holds.then(|| session.agent_indicator.clone()),
                    current: current
                        == Some(GotoTarget::Pane {
                            session: session.id(),
                            pane,
                        }),
                }
            })
            .collect()
    }
}
