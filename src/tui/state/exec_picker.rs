//! Exec profile picker state (issue #325).
//!
//! Pure state — no `App` / `Config` / PTY dependency beyond the sorted
//! profile-name list handed in at open time. The `App` orchestrator owns
//! resolving the highlighted profile to an argv (via
//! [`crate::exec::resolve_exec_command`]); the run loop owns spawning the
//! PTY overlay with that argv in the selected worktree's directory.
//!
//! The picker is deliberately tiny: a sorted list of `[exec.profiles.*]`
//! names plus a wrapping highlight. `Enter` accepts the highlight, `Esc`
//! cancels — the `j`/`k`/arrow navigation lives in
//! [`crate::tui::modal_keymap`] under `KeyContext::ExecPicker`.

/// Pure state for the exec profile picker. `Default` is an empty, closed
/// picker (no profiles, highlight at 0).
#[derive(Debug, Default)]
pub struct ExecPicker {
  /// Exec profile names from `[exec.profiles.*]`, in the sorted order the
  /// orchestrator hands them over (a `BTreeMap` key walk).
  profiles: Vec<String>,
  /// Highlighted row index. Always a valid index into `profiles` while
  /// non-empty; the `next`/`prev` wrappers keep it in range.
  selected: usize,
}

impl ExecPicker {
  pub fn new() -> Self {
    Self::default()
  }

  /// (Re)populate the picker from a list of profile names, resetting the
  /// highlight to the top. Called by the orchestrator's
  /// `enter_exec_picker` each time the overlay opens.
  pub fn open(&mut self, profiles: Vec<String>) {
    self.profiles = profiles;
    self.selected = 0;
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
