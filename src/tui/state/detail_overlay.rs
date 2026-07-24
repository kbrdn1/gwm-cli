//! Generic detail overlay state (issue #408).
//!
//! A ratatui-free row-list overlay: a title plus `(label, value, role, meta)`
//! rows with a selection cursor. Deliberately content-agnostic — the
//! agent-session view is its first consumer, the planned rich PR/Issue view
//! is the second — so the session-specific knowledge lives in
//! [`agent_detail_rows`], not in the state machine. `meta` is an opaque
//! per-row payload for consumer actions (the session id for attach/detach).
//! Pinned by `tests/tui_app_tests.rs::agent_detail_overlay`.

use crate::agent_sessions::{AgentSession, Freshness, WorktreeAgents};
use std::time::SystemTime;

/// What the overlay is currently doing: browsing the worktree's sessions,
/// or typing a query to attach one by id (palette-style — user feedback
/// 2026-07-22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailMode {
  #[default]
  List,
  Input,
}

/// Which consumer the open overlay belongs to (issue #436). The shell is
/// content-agnostic; the dispatch routes the keys (and the Enter action)
/// by this discriminant — agents attach/detach, CI checks open-in-browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailKind {
  #[default]
  Agents,
  CiChecks,
}

/// Semantic style role of a detail row — mapped to theme colours at render
/// time so the state stays ratatui-free and theme-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailRole {
  Normal,
  /// Highlighted (an active agent session).
  Active,
  /// De-emphasised (idle sessions, empty-state text).
  Muted,
  /// A green outcome (a passing CI check) — theme `clean` (issue #436).
  Success,
  /// A red outcome (a failing CI check) — theme `prunable`.
  Failure,
  /// An in-flight outcome (a running CI check) — theme `dirty`.
  Running,
}

/// One overlay row: a left-aligned label, its value text, and an opaque
/// `meta` payload consumer actions can key off (`None` for inert rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRow {
  pub label: String,
  pub value: String,
  pub role: DetailRole,
  pub meta: Option<String>,
}

/// The overlay's whole state. "Closed" is simply `View::List` — the `App`
/// flips views; this struct only carries what the open overlay shows.
#[derive(Debug, Clone, Default)]
pub struct DetailOverlay {
  /// Which consumer opened the overlay — routes the contextual keymap and
  /// the Enter action (issue #436).
  pub kind: DetailKind,
  pub title: String,
  pub rows: Vec<DetailRow>,
  /// Selection cursor (user feedback 2026-07-22: rows are selectable, the
  /// render highlights this row and keeps it inside the visible window).
  pub selected: usize,
  /// Browsing vs attach-by-id input (palette-style).
  pub mode: DetailMode,
  /// The attach-by-id query buffer while [`DetailMode::Input`] is active.
  pub input: String,
  /// Highlight inside the filtered candidate list of the input mode.
  pub input_selected: usize,
}

impl DetailOverlay {
  /// Load fresh content and reset the selection cursor.
  pub fn open(&mut self, kind: DetailKind, title: String, rows: Vec<DetailRow>) {
    self.kind = kind;
    self.title = title;
    self.rows = rows;
    self.selected = 0;
    self.mode = DetailMode::List;
    self.input.clear();
    self.input_selected = 0;
  }

  /// Replace the rows in place (post-action rebuild), clamping the cursor.
  pub fn set_rows(&mut self, rows: Vec<DetailRow>) {
    self.rows = rows;
    self.selected = self.selected.min(self.rows.len().saturating_sub(1));
  }

  pub fn select_next(&mut self) {
    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
  }

  pub fn select_prev(&mut self) {
    self.selected = self.selected.saturating_sub(1);
  }

  /// The selected row's `meta` payload, if any.
  pub fn selected_meta(&self) -> Option<&str> {
    self.rows.get(self.selected).and_then(|r| r.meta.as_deref())
  }
}

/// Map a worktree's agent sessions to overlay rows, most recent first.
/// `pinned` is the worktree's manual pin (session id), marked on its row.
/// `None` / empty yields a single explicit "no agent session found" row —
/// the overlay never opens blank (spec US2 scenario 3).
///
/// Display favours the session *name* when the artefacts carry one (user
/// feedback 2026-07-22); the full id otherwise — never truncated, it is
/// what `gwm agents attach` takes.
pub fn agent_detail_rows(agents: Option<&WorktreeAgents>, pinned: &[String], now: SystemTime) -> Vec<DetailRow> {
  let sessions = agents.map(|a| a.sessions.as_slice()).unwrap_or_default();
  if sessions.is_empty() {
    return vec![DetailRow {
      label: "agents".into(),
      value: "no agent session found".into(),
      role: DetailRole::Muted,
      meta: None,
    }];
  }
  sessions
    .iter()
    .map(|s| {
      let freshness = Freshness::classify(s.last_activity, s.ended, now);
      let (word, role) = match freshness {
        Freshness::Active => ("active", DetailRole::Active),
        Freshness::Idle => ("idle", DetailRole::Muted),
      };
      let ago = now
        .duration_since(s.last_activity)
        .map(crate::worktree::format_relative_duration)
        .unwrap_or_else(|_| "now".into());
      let identity = s.name.as_deref().unwrap_or(&s.id);
      let pin_mark = if pinned.iter().any(|p| p == &s.id) {
        " · pinned"
      } else {
        ""
      };
      DetailRow {
        label: s.kind.display().to_string(),
        value: format!("{word} · {ago} ago · {identity}{pin_mark}"),
        role,
        meta: Some(s.id.clone()),
      }
    })
    .collect()
}

/// Fuzzy-ish filter for the attach-by-id prompt: case-insensitive substring
/// match on the session id, its name, and the agent kind. An empty query
/// lists the whole pool. Pure — pinned by
/// `tests/tui_app_tests.rs::agent_overlay_input`.
/// One overlay row per classified rollup check (issue #436): the state
/// icon + outcome label on the left, the check name as the value, the
/// details URL as the opaque meta so Enter can open it in the browser.
/// Same icons as the sidebar's `ci_indicator`; role picks the matching
/// theme colour at render time. Pure — pinned by
/// `tests/tui_app_tests.rs::enter_ci_checks_opens_the_overlay_with_one_row_per_check`.
pub fn ci_check_rows(checks: &[crate::github::PrCheck]) -> Vec<DetailRow> {
  use crate::github::CheckOutcome;
  use crate::tui::ui::{CI_FAILING_ICON, CI_PASSING_ICON, CI_RUNNING_ICON};
  checks
    .iter()
    .map(|c| {
      let (icon, word, role) = match c.outcome {
        CheckOutcome::Passing => (CI_PASSING_ICON, "passing", DetailRole::Success),
        CheckOutcome::Failing => (CI_FAILING_ICON, "failing", DetailRole::Failure),
        CheckOutcome::Running => (CI_RUNNING_ICON, "running", DetailRole::Running),
      };
      DetailRow {
        label: format!("{icon} {word}"),
        value: c.name.clone(),
        role,
        meta: c.url.clone(),
      }
    })
    .collect()
}

/// The `f` filter of the CI checks overlay (issue #436): case-insensitive
/// substring over the check name, same convention as [`filter_sessions`]
/// right below (the attach prompt of the same shell). Returns indices into
/// `rows` so the caller can both render and resolve the selection.
pub fn filter_rows(rows: &[DetailRow], query: &str) -> Vec<usize> {
  let q = query.to_lowercase();
  rows
    .iter()
    .enumerate()
    .filter(|(_, r)| q.is_empty() || r.value.to_lowercase().contains(&q))
    .map(|(i, _)| i)
    .collect()
}

pub fn filter_sessions<'a>(all: &'a [AgentSession], query: &str) -> Vec<&'a AgentSession> {
  let q = query.to_lowercase();
  all
    .iter()
    .filter(|s| {
      q.is_empty()
        || s.id.to_lowercase().contains(&q)
        || s.name.as_deref().is_some_and(|n| n.to_lowercase().contains(&q))
        || s.kind.display().contains(&q)
    })
    .collect()
}
