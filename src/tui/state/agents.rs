//! Agent-session detection state (issues #408, #635).
//!
//! The last completed detection snapshot, the candidate pool behind the
//! overlay's attach-by-id prompt, the per-worktree pins the sidebar reads,
//! and the two flags that chain a re-scan behind a run already in flight.
//!
//! Six fields flat on `App` until #635. They are one struct rather than a
//! shared prefix because they move together: every detection cycle replaces
//! the snapshot, stamps the clock, refills the pool and refreshes the pins,
//! and the two `*_wanted` flags exist only to defer exactly that cycle.
//!
//! Pure state. `App` owns the detection itself — the walk is off-render
//! (#343), and the pins are refreshed off-render too, since the render path
//! must not read git config.

use crate::agent_sessions::{AgentSession, WorktreeAgents};
use std::collections::BTreeMap;
use std::time::Instant;

/// Owned state for agent-session detection.
#[derive(Debug, Default)]
pub struct AgentState {
  /// Last completed agent-session snapshot, keyed by worktree path string
  /// (issue #408). `None` until the first detection lands — the table then
  /// renders without agent cells, no placeholder noise. Replaced atomically
  /// by `App::apply_agent_snapshot`; the render path only reads it.
  pub snapshot: Option<BTreeMap<String, WorktreeAgents>>,

  /// When the current snapshot was taken — drives the periodic re-detection
  /// in `App::maybe_refresh_agent_sessions` so freshness colours do not
  /// fossilise at their startup value.
  pub snapshot_at: Option<Instant>,

  /// Every session the last detection saw, matched or not — the candidate
  /// pool of the overlay's attach-by-id prompt (user feedback 2026-07-22).
  pub all_sessions: Vec<AgentSession>,

  /// Pinned session ids per worktree path — the sidebar Agents pane shows
  /// ONLY these (user feedback 2026-07-22), and the render path must not
  /// read git config, so the map is refreshed off-render (each detection
  /// cycle + immediately after attach/detach). Empty in workspace mode
  /// (same single-repo ceiling as the pins themselves).
  pub pins: BTreeMap<String, Vec<String>>,

  /// A full pool scan was requested while a detection run was in flight —
  /// it chains after that run lands instead of walking the store
  /// concurrently (Codex review round R).
  pub pool_wanted: bool,

  /// A pin changed while a detection run was in flight — the re-scan (and
  /// the pins refresh) chains after that run lands instead of racing a
  /// second walk against it (Codex review round U).
  pub redetect_wanted: bool,
}
