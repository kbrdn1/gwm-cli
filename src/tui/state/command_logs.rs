//! Command Logs modal state (issue #226).
//!
//! The scroll cursor for the lazygit-style Command Logs overlay plus an
//! owned snapshot of the [`crate::command_log`] global. The `App` calls
//! [`CommandLogs::sync`] when the overlay opens (and on each tick while it
//! is up) so the renderer paints from this owned state rather than locking
//! the global mid-frame — the same boundary the modal-render tests inject
//! through.
//!
//! Scroll mirrors the help overlay (`App::help_scroll` / `help_max_scroll`):
//! the cursor lives here, but `max_scroll` / `max_x_scroll` are republished
//! by the renderer each frame against the real viewport, since only the
//! renderer knows both the content length and the inner modal height. The
//! scroll methods clamp against whatever bound the last frame published.

use crate::command_log::{self, CommandLogEntry};

/// Owned state for the Command Logs overlay: the snapshotted entries and
/// the (vertical + horizontal) scroll cursor with its renderer-published
/// bounds.
#[derive(Debug, Default)]
pub struct CommandLogs {
  /// Snapshot of the global command log, oldest-first (the renderer paints
  /// it newest-first). Refreshed by [`Self::sync`].
  pub entries: Vec<CommandLogEntry>,
  /// Vertical scroll offset, in rows. Clamped to `max_scroll`.
  pub scroll: u16,
  /// Maximum vertical scroll offset, republished by the renderer each
  /// frame as `content_rows.saturating_sub(viewport_rows)`.
  pub max_scroll: u16,
  /// Horizontal scroll offset, in columns. Clamped to `max_x_scroll`.
  pub x_scroll: u16,
  /// Maximum horizontal scroll offset, republished by the renderer.
  pub max_x_scroll: u16,
}

impl CommandLogs {
  /// An empty overlay at the origin.
  pub fn new() -> Self {
    Self::default()
  }

  /// Copy the global command log into [`Self::entries`]. Cheap clone of a
  /// bounded ring (≤ `command_log::MAX_ENTRIES`); called on open and per
  /// tick while the overlay is up so it tracks commands that finish (e.g.
  /// an off-thread GitHub fetch) while the user is reading.
  pub fn sync(&mut self) {
    self.entries = command_log::snapshot();
  }

  /// Scroll down one row, never past the last line.
  pub fn scroll_down(&mut self) {
    self.scroll = (self.scroll + 1).min(self.max_scroll);
  }

  /// Scroll up one row, never above the top.
  pub fn scroll_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(1);
  }

  /// Scroll right one column, never past the widest line.
  pub fn scroll_right(&mut self) {
    self.x_scroll = (self.x_scroll + 1).min(self.max_x_scroll);
  }

  /// Scroll left one column, never before the first.
  pub fn scroll_left(&mut self) {
    self.x_scroll = self.x_scroll.saturating_sub(1);
  }

  /// Jump to the first row (`g`).
  pub fn scroll_to_top(&mut self) {
    self.scroll = 0;
  }

  /// Jump to the last row (`G`).
  pub fn scroll_to_bottom(&mut self) {
    self.scroll = self.max_scroll;
  }

  /// Reset the scroll cursor to the origin, keeping the snapshotted
  /// entries. Called when the overlay opens.
  pub fn reset(&mut self) {
    self.scroll = 0;
    self.x_scroll = 0;
  }
}
