//! Configuration panel modal state (issue #232).
//!
//! The scroll cursor for the ~90% fullscreen Configuration overlay plus
//! the owned, already-resolved [`crate::config::ConfigRow`] snapshot the
//! renderer paints. Unlike the Command Logs slice there is no `sync`:
//! the `App` resolves the rows once on open (cheap local TOML reads via
//! [`crate::config::resolved_rows`]) and assigns them here, so the panel
//! renders from owned state with no I/O on the render path.
//!
//! Scroll mirrors the help / Command Logs overlays: the cursor lives
//! here, but `max_scroll` / `max_x_scroll` are republished by the
//! renderer each frame against the live viewport, since only the renderer
//! knows both the content length and the inner modal height. The scroll
//! methods clamp against whatever bound the last frame published.

use crate::config::ConfigRow;

/// Owned state for the Configuration overlay: the resolved rows and the
/// (vertical + horizontal) scroll cursor with its renderer-published
/// bounds.
#[derive(Debug, Default)]
pub struct ConfigPanel {
  /// Resolved config rows (key, value, source), grouped section-first by
  /// the renderer. Assigned by [`crate::tui::App::enter_config_panel`].
  pub rows: Vec<ConfigRow>,
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

impl ConfigPanel {
  /// An empty overlay at the origin.
  pub fn new() -> Self {
    Self::default()
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

  /// Reset the scroll cursor to the origin, keeping the resolved rows.
  /// Called when the overlay opens.
  pub fn reset(&mut self) {
    self.scroll = 0;
    self.x_scroll = 0;
  }
}
