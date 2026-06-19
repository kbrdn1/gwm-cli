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

/// The picker label for the no-`--profile` choice — the directory set
/// `gwm clean` resolves with no profile (`[clean.profiles.default]` when
/// defined, else the built-in `target` / `node_modules` / `dist` / `build`).
pub const DEFAULT_CHOICE_LABEL: &str = "(default)";

/// Pure state for the clean overlay. `Default` is a closed overlay (no
/// choices, no snapshot, disarmed countdown).
#[derive(Debug, Default)]
pub struct CleanOverlay {
  /// Picker choices: `None` (the no-`--profile` path — `gwm clean` resolves
  /// the `default` profile when defined, else the built-in set) followed by
  /// each configured `[clean.profiles.*]` name. The leading `None` is always
  /// present once [`Self::open`] has run, so the overlay's default matches
  /// the CLI even when a repo defines only non-`default` profiles — and the
  /// built-in set stays reachable.
  choices: Vec<Option<String>>,
  /// Highlighted choice row.
  selected: usize,
  /// The most recent gated scan snapshot for the selected worktree, filled
  /// by `App::enter_clean_overlay` and re-filled on a choice change. `None`
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

  /// Populate the picker from the configured profile names, reset the
  /// highlight to the `(default)` choice, and clear any stale snapshot /
  /// countdown. The orchestrator follows this with [`Self::set_scan`] once
  /// the first scan completes.
  ///
  /// Opens on the no-`--profile` choice so the overlay's first preview always
  /// matches what `gwm clean` would resolve — the built-in set, or
  /// `[clean.profiles.default]` when defined — regardless of which optional
  /// profiles the repo adds.
  pub fn open(&mut self, profiles: Vec<String>) {
    self.choices = std::iter::once(None).chain(profiles.into_iter().map(Some)).collect();
    self.selected = 0;
    self.reclaim = None;
    self.skipped.clear();
    self.confirm.reset();
  }

  /// The picker labels, in display order. The no-`--profile` entry renders as
  /// [`DEFAULT_CHOICE_LABEL`]; named profiles render as themselves.
  pub fn choice_labels(&self) -> Vec<&str> {
    self
      .choices
      .iter()
      .map(|c| c.as_deref().unwrap_or(DEFAULT_CHOICE_LABEL))
      .collect()
  }

  /// `true` when the repo configures at least one named profile, so the
  /// picker is worth rendering (otherwise there is only the implicit
  /// `(default)` choice).
  pub fn has_profiles(&self) -> bool {
    self.choices.len() > 1
  }

  /// The highlighted choice row.
  pub fn selected_index(&self) -> usize {
    self.selected
  }

  /// The highlighted profile name to hand to `resolve_clean_dirs`: `None` for
  /// the `(default)` choice (no `--profile`), `Some(name)` for a configured
  /// profile.
  pub fn selected_profile(&self) -> Option<&str> {
    self.choices.get(self.selected).and_then(|c| c.as_deref())
  }

  /// Move the highlight down one row, wrapping. No-op with fewer than two
  /// choices (nothing to cycle). Disarms the countdown — changing the target
  /// must be re-confirmed.
  pub fn next(&mut self) {
    if self.choices.len() < 2 {
      return;
    }
    self.selected = (self.selected + 1) % self.choices.len();
    self.confirm.reset();
  }

  /// Move the highlight up one row, wrapping. No-op with fewer than two
  /// choices. Disarms the countdown.
  pub fn prev(&mut self) {
    if self.choices.len() < 2 {
      return;
    }
    self.selected = (self.selected + self.choices.len() - 1) % self.choices.len();
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
