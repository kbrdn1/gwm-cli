//! Generic detail overlay state (issue #408).
//!
//! A ratatui-free row-list overlay: a title plus `(label, value, role)`
//! triples. Deliberately content-agnostic — the agent-session view is its
//! first consumer, the planned rich PR/Issue view is the second — so the
//! session-specific knowledge lives in [`agent_detail_rows`], not in the
//! state machine. Pinned by `tests/tui_app_tests.rs::agent_detail_overlay`.

use crate::agent_sessions::{Freshness, WorktreeAgents};
use std::time::SystemTime;

/// Semantic style role of a detail row — mapped to theme colours at render
/// time so the state stays ratatui-free and theme-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailRole {
  Normal,
  /// Highlighted (an active agent session).
  Active,
  /// De-emphasised (idle sessions, empty-state text).
  Muted,
}

/// One overlay row: a left-aligned label and its value text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRow {
  pub label: String,
  pub value: String,
  pub role: DetailRole,
}

/// The overlay's whole state. "Closed" is simply `View::List` — the `App`
/// flips views; this struct only carries what the open overlay shows.
#[derive(Debug, Clone, Default)]
pub struct DetailOverlay {
  pub title: String,
  pub rows: Vec<DetailRow>,
  pub scroll: u16,
}

impl DetailOverlay {
  /// Load fresh content and reset the scroll cursor.
  pub fn open(&mut self, title: String, rows: Vec<DetailRow>) {
    self.title = title;
    self.rows = rows;
    self.scroll = 0;
  }

  pub fn scroll_down(&mut self, max: u16) {
    self.scroll = self.scroll.saturating_add(1).min(max);
  }

  pub fn scroll_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(1);
  }
}

/// Map a worktree's agent sessions to overlay rows, most recent first.
/// `None` / empty yields a single explicit "no agent session found" row —
/// the overlay never opens blank (spec US2 scenario 3).
pub fn agent_detail_rows(agents: Option<&WorktreeAgents>, now: SystemTime) -> Vec<DetailRow> {
  let sessions = agents.map(|a| a.sessions.as_slice()).unwrap_or_default();
  if sessions.is_empty() {
    return vec![DetailRow {
      label: "agents".into(),
      value: "no agent session found".into(),
      role: DetailRole::Muted,
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
      // Session ids are uuids; eight chars disambiguate without flooding
      // the value column.
      let short_id: String = s.id.chars().take(8).collect();
      DetailRow {
        label: s.kind.display().to_string(),
        value: format!("{word} · {ago} ago · {short_id}"),
        role,
      }
    })
    .collect()
}
