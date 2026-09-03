//! Exec profile picker state (issue #325).
//!
//! Pure state — no `App` / `Config` / PTY dependency beyond the sorted
//! profile-name list handed in at open time. The `App` orchestrator owns
//! resolving the highlighted profile to an argv (via
//! [`crate::exec::resolve_exec_command`]); the run loop owns spawning the
//! PTY overlay with that argv in the selected worktree's directory.
//!
//! The picker is deliberately tiny: a sorted list of `[exec.profiles.*]`
//! names, a wrapping highlight, and the worktree path it was opened for.
//! `Enter` accepts the highlight, `Esc` cancels — the `j`/`k`/arrow
//! navigation lives in [`crate::tui::modal_keymap`] under
//! `KeyContext::ExecPicker`.

use std::path::{Path, PathBuf};

/// Pure state for the exec profile picker. `Default` is an empty, closed
/// picker (no profiles, no target, highlight at 0).
#[derive(Debug, Default)]
pub struct ExecPicker {
  /// Exec profile names from `[exec.profiles.*]`, in the sorted order the
  /// orchestrator hands them over (a `BTreeMap` key walk).
  profiles: Vec<String>,
  /// Highlighted row index. Always a valid index into `profiles` while
  /// non-empty; the `next`/`prev` wrappers keep it in range.
  selected: usize,
  /// The worktree directory the picker was opened for. Captured at open so
  /// `Enter` runs the profile in *that* worktree even if an auto-refresh
  /// drifts the live selection while the overlay sits open (Codex #333).
  cwd: Option<PathBuf>,
}

impl ExecPicker {
  pub fn new() -> Self {
    Self::default()
  }

  /// (Re)populate the picker from a list of profile names + the worktree it
  /// targets, resetting the highlight to the top. Called by the
  /// orchestrator's `enter_exec_picker` each time the overlay opens.
  pub fn open(&mut self, profiles: Vec<String>, cwd: PathBuf) {
    self.profiles = profiles;
    self.selected = 0;
    self.cwd = Some(cwd);
  }

  /// The worktree directory the picker was opened for, if any.
  pub fn cwd(&self) -> Option<&Path> {
    self.cwd.as_deref()
  }

  /// The profile names, in display order.
  pub fn profiles(&self) -> &[String] {
    &self.profiles
  }

  /// `true` when no profiles are loaded (the orchestrator refuses to open
  /// the overlay in this state, but the picker stays well-defined).
  pub fn is_empty(&self) -> bool {
    self.profiles.is_empty()
  }

  /// The highlighted row index (clamped to `profiles`).
  pub fn selected_index(&self) -> usize {
    self.selected
  }

  /// The highlighted profile name, or `None` when the picker is empty.
  pub fn selected_profile(&self) -> Option<&str> {
    self.profiles.get(self.selected).map(String::as_str)
  }

  /// Move the highlight down one row, wrapping to the top. No-op when
  /// empty.
  /// Point the highlight straight at `index` (issue #624 — a click lands on
  /// an arbitrary row, not one step away). Out-of-range is ignored rather
  /// than clamped: the caller is reporting where the user clicked, and a
  /// click past the last profile picked nothing.
  pub fn select_index(&mut self, index: usize) {
    if index < self.profiles.len() {
      self.selected = index;
    }
  }

  pub fn next(&mut self) {
    if self.profiles.is_empty() {
      return;
    }
    self.selected = (self.selected + 1) % self.profiles.len();
  }

  /// Move the highlight up one row, wrapping to the bottom. No-op when
  /// empty.
  pub fn prev(&mut self) {
    if self.profiles.is_empty() {
      return;
    }
    self.selected = (self.selected + self.profiles.len() - 1) % self.profiles.len();
  }
}
