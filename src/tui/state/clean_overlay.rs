//! Clean overlay state (issue #325).
//!
//! Holds the reclaim scan snapshot for the selected worktree, the
//! `[clean.profiles.*]` picker, and a dedicated safety countdown
//! ([`ConfirmModal`]) for the destructive delete.
//!
//! Pure state: the `App` orchestrator owns the I/O — the
//! `clean::scan_worktree_safe` walk that fills the snapshot (already gated to
//! the git-ignored, untracked artifacts the CLI would delete) and the
//! `clean::delete_reclaim` that consumes it. The countdown reuses the exact
//! same timer the delete-confirm modal uses (`confirm.rs`), but as a separate
//! instance driven by `[tui] confirm_countdown_secs` directly — clean has no
//! `delete_branch_on_remove` gate.

use super::confirm::ConfirmModal;
use crate::clean::WorktreeReclaim;

/// Pure state for the clean overlay. `Default` is a closed overlay (no
/// profiles, no snapshot, disarmed countdown).
#[derive(Debug, Default)]
pub struct CleanOverlay {
  /// Configured `[clean.profiles.*]` names (sorted). Empty ⇒ the overlay
  /// scans the built-in default set and the picker has nothing to cycle.
  profiles: Vec<String>,
  /// Highlighted profile row (meaningful only while `profiles` is
  /// non-empty).
  selected: usize,
  /// The most recent gated scan snapshot for the selected worktree, filled
  /// by `App::enter_clean_overlay` and re-filled on a profile change. `None`
  /// before the first scan.
  reclaim: Option<WorktreeReclaim>,
  /// Directory names found but preserved by the safety gate (not
  /// git-ignored, or holding tracked files) — surfaced so the user
  /// understands why a visible `target/` was not counted.
  skipped: Vec<String>,
  /// Safety countdown for the delete. Armed by the confirm key; the run
  /// loop fires `clean::delete_reclaim` when it elapses.
  pub confirm: ConfirmModal,
}

impl CleanOverlay {
  pub fn new() -> Self {
    Self::default()
  }

  /// Populate the picker profile names, reset the highlight, and clear any
  /// stale snapshot / countdown. The orchestrator follows this with
  /// [`Self::set_scan`] once the first scan completes.
  pub fn open(&mut self, profiles: Vec<String>) {
    self.profiles = profiles;
    self.selected = 0;
    self.reclaim = None;
    self.skipped.clear();
    self.confirm.reset();
  }

  /// The configured profile names, in display order.
  pub fn profiles(&self) -> &[String] {
    &self.profiles
  }

  /// `true` when at least one `[clean.profiles]` entry exists (the picker
  /// can cycle).
  pub fn has_profiles(&self) -> bool {
    !self.profiles.is_empty()
  }

  /// The highlighted profile row.
  pub fn selected_index(&self) -> usize {
    self.selected
  }

  /// The highlighted profile name, or `None` when no profiles are
  /// configured (the overlay scans the built-in default set).
  pub fn selected_profile(&self) -> Option<&str> {
    self.profiles.get(self.selected).map(String::as_str)
  }

  /// Move the highlight down one row, wrapping. No-op with fewer than two
  /// profiles (nothing to cycle). Disarms the countdown — changing the
  /// target must be re-confirmed.
  pub fn next(&mut self) {
    if self.profiles.len() < 2 {
      return;
    }
    self.selected = (self.selected + 1) % self.profiles.len();
    self.confirm.reset();
  }

  /// Move the highlight up one row, wrapping. No-op with fewer than two
  /// profiles. Disarms the countdown.
  pub fn prev(&mut self) {
    if self.profiles.len() < 2 {
      return;
    }
    self.selected = (self.selected + self.profiles.len() - 1) % self.profiles.len();
    self.confirm.reset();
  }

  /// Store a fresh scan snapshot + the gate-preserved names, disarming any
  /// running countdown (the figures it was about to act on just changed).
  pub fn set_scan(&mut self, reclaim: WorktreeReclaim, skipped: Vec<String>) {
    self.reclaim = Some(reclaim);
    self.skipped = skipped;
    self.confirm.reset();
  }

  /// The current gated scan snapshot, if any.
  pub fn reclaim(&self) -> Option<&WorktreeReclaim> {
    self.reclaim.as_ref()
  }

  /// Directory names the safety gate preserved in the current scan.
  pub fn skipped(&self) -> &[String] {
    &self.skipped
  }

  /// Total reclaimable bytes in the current snapshot (`0` before a scan or
  /// when nothing is safe to delete).
  pub fn total_bytes(&self) -> u64 {
    self.reclaim.as_ref().map(|r| r.total_bytes).unwrap_or(0)
  }

  /// `true` when the current scan has nothing safe to reclaim.
  pub fn is_empty_scan(&self) -> bool {
    self.total_bytes() == 0
  }
}
