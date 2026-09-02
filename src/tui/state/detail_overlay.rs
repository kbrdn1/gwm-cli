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
/// by this discriminant — agents attach/detach, CI checks open-in-browser,
/// the rich view (#420) opens whatever URL its selected row carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailKind {
  #[default]
  Agents,
  CiChecks,
  /// Rich issue view (issue #420).
  RichIssue,
  /// Rich pull / merge request view (issue #420).
  RichPr,
}

impl DetailKind {
  /// `true` for the consumers whose rows come from a **forge link**, and
  /// which therefore must not outlive it: a workspace selection gone stale
  /// or a re-link means the rows describe a PR/issue that is no longer the
  /// selected one, and every verb — `Enter` opening a URL above all —
  /// would act on the wrong thing.
  ///
  /// Written as an exhaustive `match` with no `_` arm on purpose (issue
  /// #420): a fourth consumer added later does not compile until someone
  /// answers the question for it, which is the only way this guard stays
  /// complete. The equality tests it replaces had to be remembered at each
  /// new site instead.
  pub fn is_forge_linked(self) -> bool {
    match self {
      Self::CiChecks | Self::RichIssue | Self::RichPr => true,
      Self::Agents => false,
    }
  }
}

/// Semantic style role of a detail row — mapped to theme colours at render
/// time so the state stays ratatui-free and theme-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailRole {
  #[default]
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetailRow {
  pub label: String,
  pub value: String,
  pub role: DetailRole,
  pub meta: Option<String>,
  /// Optional right-aligned detail column, rendered muted (issue #436:
  /// workflow name + run duration on a CI check row). `None` keeps the
  /// pre-#436 two-column layout.
  pub extra: Option<String>,
  /// The row's text split into styled runs (issue #551), for the consumer
  /// that renders Markdown. Empty means "paint `value` in `role`", which is
  /// what every row did before this field existed and what the agents and
  /// CI consumers still do.
  ///
  /// When it is non-empty the invariant is that the segments concatenate to
  /// `value`: the plain string stays the one thing that measuring, filtering
  /// and the tests work against, so the styling is additive rather than a
  /// second source of truth about what the row says.
  pub segments: Vec<crate::tui::state::markdown::Segment>,
  /// True for a row that must not be reflowed and may therefore be wider
  /// than the modal: a fenced code line, a diff hunk (issue #551).
  ///
  /// Every other row is wrapped to fit before it gets here, so the renderer
  /// can paint it as-is. One of these is clipped against the view's
  /// horizontal offset instead, which is the only way to reach its tail.
  pub preformatted: bool,
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
    // The input-mode cursor indexes the FILTERED list (Codex review #455):
    // a landing with fewer matches than the cursor position left it out of
    // bounds — no highlight, Enter returning None, a stuck filter.
    self.input_selected = self
      .input_selected
      .min(filter_rows(&self.rows, &self.input).len().saturating_sub(1));
  }

  /// Point the selection straight at `index` (issue #624).
  pub fn select_index(&mut self, index: usize) {
    if index < self.rows.len() {
      self.selected = index;
    }
  }

  pub fn select_next(&mut self) {
    self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
  }

  /// Jump `n` rows down, stopping at the last one (`D`, issue #551).
  ///
  /// Clamped rather than wrapped: `Ctrl+D` in a pager stops at the bottom,
  /// and a jump that wrapped to the top would lose the reader's place in a
  /// body that can now run to hundreds of rows.
  pub fn select_page_down(&mut self, n: usize) {
    if self.rows.is_empty() {
      return;
    }
    self.selected = (self.selected + n).min(self.rows.len() - 1);
  }

  /// Jump `n` rows up, stopping at the first.
  pub fn select_page_up(&mut self, n: usize) {
    self.selected = self.selected.saturating_sub(n);
  }

  pub fn select_first(&mut self) {
    self.selected = 0;
  }

  pub fn select_last(&mut self) {
    self.selected = self.rows.len().saturating_sub(1);
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
      extra: None,
      ..Default::default()
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
        extra: None,
        ..Default::default()
      }
    })
    .collect()
}

/// One overlay row per classified rollup check (issue #436): the state
/// icon + outcome label on the left, the check name as the value, the
/// details URL as the opaque meta so Enter can open it in the browser,
/// and the workflow + run duration as the right-aligned `extra` column
/// (#436 validation feedback). Same icons as the sidebar's
/// `ci_indicator`; role picks the matching theme colour at render time.
/// Pure — `now` anchors the elapsed time of in-flight runs.
pub fn ci_check_rows(checks: &[crate::github::PrCheck], now: SystemTime) -> Vec<DetailRow> {
  use crate::github::CheckOutcome;
  use crate::tui::ui::{CI_FAILING_ICON, CI_PASSING_ICON, CI_RUNNING_ICON, CI_UNKNOWN_ICON};
  checks
    .iter()
    .map(|c| {
      let (icon, word, role) = match c.outcome {
        CheckOutcome::Passing => (CI_PASSING_ICON, "passing", DetailRole::Success),
        CheckOutcome::Failing => (CI_FAILING_ICON, "failing", DetailRole::Failure),
        CheckOutcome::Running => (CI_RUNNING_ICON, "running", DetailRole::Running),
        // Named honestly rather than folded into "running" (issue #419):
        // the row is telling the user gwm could not classify the state,
        // which is different from knowing it is in flight.
        CheckOutcome::Unknown => (CI_UNKNOWN_ICON, "unknown", DetailRole::Muted),
      };
      DetailRow {
        label: format!("{icon} {word}"),
        value: c.name.clone(),
        role,
        meta: c.url.clone(),
        extra: check_extra(c, now),
        ..Default::default()
      }
    })
    .collect()
}

/// The detail column of one check row: `workflow · duration` for a
/// completed run, `workflow · elapsed…` for an in-flight one, whichever
/// part exists otherwise, `None` when the rollup entry carries no run
/// metadata at all (legacy StatusContext shape).
fn check_extra(c: &crate::github::PrCheck, now: SystemTime) -> Option<String> {
  let parse = |s: &Option<String>| {
    s.as_deref()
      .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
      .map(|d| d.with_timezone(&chrono::Utc))
  };
  let duration = match (parse(&c.started_at), parse(&c.completed_at)) {
    (Some(start), Some(end)) => u64::try_from((end - start).num_seconds()).ok().map(format_run_duration),
    // The elapsed form requires the RUNNING outcome (Codex review #455),
    // not just a missing end: a terminal StatusContext carrying a start
    // but no completion would otherwise read as still active ("2m…") and
    // freeze there — the duration tick only rebuilds while an outcome is
    // Running. A terminal check with an unknown end shows no duration.
    (Some(start), None) if c.outcome == crate::github::CheckOutcome::Running => {
      let now = chrono::DateTime::<chrono::Utc>::from(now);
      u64::try_from((now - start).num_seconds())
        .ok()
        .map(|secs| format!("{}…", format_run_duration(secs)))
    }
    _ => None,
  };
  match (c.workflow_name.as_deref(), duration) {
    (Some(w), Some(d)) => Some(format!("{w} · {d}")),
    (Some(w), None) => Some(w.to_string()),
    (None, Some(d)) => Some(d),
    (None, None) => None,
  }
}

/// Compact human duration for a check run: `47s`, `1m18s`, `2m`, `1h02m`.
fn format_run_duration(secs: u64) -> String {
  let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
  if h > 0 {
    format!("{h}h{m:02}m")
  } else if m > 0 && s > 0 {
    format!("{m}m{s:02}s")
  } else if m > 0 {
    format!("{m}m")
  } else {
    format!("{s}s")
  }
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

/// Fuzzy-ish filter for the attach-by-id prompt: case-insensitive substring
/// match on the session id, its name, and the agent kind. An empty query
/// lists the whole pool. Pure — pinned by
/// `tests/tui_app_tests.rs::agent_overlay_input`.
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
