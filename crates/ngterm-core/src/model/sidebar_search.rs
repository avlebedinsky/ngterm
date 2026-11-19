//! The sidebar's session search, this build's own (upstream has none): the query a window's search field holds, which
//! sessions it keeps, and the match the field's Return opens.
//!
//! A session matches when its shown name contains the query, case-insensitively, with the query's outer spaces
//! ignored; a blank query keeps every session. The search narrows what the other filters keep (the focus filter, the
//! flagged view), so a row shows only when every filter keeps it. In the ordinary tree a matching companion brings its
//! parent along as context. Workspace names never match: a short query would keep whole workspaces of sessions that
//! do not match.
//!
//! The search is view state: never persisted, and kept while the sidebar hides. Navigation does not follow it (the
//! session steps, `session.go`, Ctrl-Tab): a query typed in the GUI never moves the selection or changes what a script
//! reaches. The field's own Up and Down move its cursor among the matches instead.

use std::collections::HashMap;
use std::ops::Range;

use uuid::Uuid;

use super::session::Session;
use super::store::AppStore;
use super::workspace::SidebarMode;

/// A window's search: the field's text and the match its Up and Down moved to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidebarSearch {
    query: String,
    /// The match Return opens; `None` (or a session no longer matching) means the first match.
    cursor: Option<Uuid>,
}

impl SidebarSearch {
    /// The field's text, as typed.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// What a name must contain: the query without its outer spaces; `None` while that is empty.
    #[must_use]
    pub fn needle(&self) -> Option<&str> {
        Some(self.query.trim()).filter(|needle| !needle.is_empty())
    }

    /// Whether the search narrows the sidebar.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.needle().is_some()
    }
}

/// Where `needle` first occurs in `haystack`, case-insensitively, as a byte range of `haystack`; `None` when it does not,
/// or when `needle` is empty.
#[must_use]
pub fn match_range(haystack: &str, needle: &str) -> Option<Range<usize>> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return None;
    }
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    haystack.char_indices().find_map(|(start, _)| {
        let mut chars = haystack[start..].char_indices();
        for wanted in &needle {
            let (_, found) = chars.next()?;
            if !same(found, *wanted) {
                return None;
            }
        }
        let end = chars.next().map_or(haystack.len(), |(offset, _)| start + offset);
        Some(start..end)
    })
}

/// Why the search keeps a session row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchHit {
    /// Its name matches.
    Match,
    /// It does not match, but a companion of it does: it shows as that companion's context (the ordinary tree only).
    Context,
}

impl AppStore {
    /// The window's sidebar search.
    #[must_use]
    pub fn sidebar_search(&self) -> &SidebarSearch {
        &self.sidebar.search
    }

    /// Sets the search field's text; a change puts the cursor back on the first match.
    pub fn set_sidebar_search(&mut self, query: &str) {
        let search = &mut self.sidebar.search;
        if search.query != query {
            query.clone_into(&mut search.query);
            search.cursor = None;
        }
    }

    /// The sessions the search keeps among the ones the other filters keep, in the order the sidebar lists them; empty
    /// while the search is blank.
    #[must_use]
    pub fn sidebar_search_matches(&self) -> Vec<Uuid> {
        let Some(needle) = self.sidebar.search.needle() else {
            return Vec::new();
        };
        self.navigable_sessions()
            .into_iter()
            .filter(|session| match_range(&session.display_name(), needle).is_some())
            .map(Session::id)
            .collect()
    }

    /// The session rows the search keeps and why, `None` while it is blank (every row shows): the matches, and in the
    /// ordinary tree the parent of each matching companion.
    #[must_use]
    pub fn sidebar_search_hits(&self) -> Option<HashMap<Uuid, SearchHit>> {
        self.sidebar.search.is_active().then_some(())?;
        let matches = self.sidebar_search_matches();
        let mut hits: HashMap<Uuid, SearchHit> = matches.iter().map(|id| (*id, SearchHit::Match)).collect();
        if self.sidebar.mode == SidebarMode::Tree {
            for id in matches {
                if let Some(parent) = self.session(id).and_then(|session| self.parent_of(session)) {
                    hits.entry(parent.id()).or_insert(SearchHit::Context);
                }
            }
        }
        Some(hits)
    }

    /// The match the field's Return opens: the cursor's while it still matches, else the first.
    #[must_use]
    pub fn sidebar_search_target(&self) -> Option<Uuid> {
        let matches = self.sidebar_search_matches();
        self.sidebar
            .search
            .cursor
            .filter(|cursor| matches.contains(cursor))
            .or_else(|| matches.first().copied())
    }

    /// Moves the cursor to the next match (`down`) or the previous one, stopping at either end.
    pub fn move_sidebar_search_cursor(&mut self, down: bool) {
        let matches = self.sidebar_search_matches();
        let Some(target) = self.sidebar_search_target() else {
            return;
        };
        let index = matches.iter().position(|id| *id == target).unwrap_or(0);
        let next = if down {
            (index + 1).min(matches.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        self.sidebar.search.cursor = Some(matches[next]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_match_ignores_case_and_reports_the_original_bytes() {
        assert_eq!(match_range("Build Server", "server"), Some(6..12));
        assert_eq!(match_range("build", "BUI"), Some(0..3));
        assert_eq!(match_range("Straße", "SSE"), None);
        assert_eq!(match_range("ÄRGER", "är"), Some(0..3));
        assert_eq!(match_range("abc", "abcd"), None);
        assert_eq!(match_range("abc", ""), None);
        assert_eq!(match_range("aab", "ab"), Some(1..3));
    }

    #[test]
    fn a_blank_query_is_inactive() {
        let mut search = SidebarSearch::default();
        assert!(!search.is_active());
        search.query = "   ".to_owned();
        assert_eq!(search.needle(), None);
        search.query = "  api ".to_owned();
        assert_eq!(search.needle(), Some("api"));
        assert!(search.is_active());
    }
}
