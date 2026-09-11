//! Keybindings (help) overlay state (issues #217, #222, #635).
//!
//! Four scroll numbers and the verbs that move them. They sat flat on `App`
//! as `help_scroll` / `help_x_scroll` / `help_max_scroll` /
//! `help_max_x_scroll` until #635 — satellites of an overlay that had no
//! state module at all, which is why they were never extracted with the
//! rest of #102.
//!
//! Scroll follows the contract the command logs and the two full-size
//! listings share: the cursor lives here, but both bounds are republished
//! by the renderer each frame against the real viewport, since only the
//! renderer knows the content size and the inner modal height.

/// Owned state for the help overlay: where the reader is, and how far the
/// renderer says they may go.
#[derive(Debug, Default)]
pub struct HelpOverlay {
  /// Vertical scroll offset, in rows. Reset to 0 every time the overlay
  /// opens; clamped to [`Self::max_scroll`] (#217).
  pub scroll: u16,
  /// Horizontal scroll offset, in columns (#222).
  pub x_scroll: u16,
  /// Maximum vertical offset, republished by `ui::draw_help` each frame as
  /// `content_rows.saturating_sub(viewport_rows)` so the offset can never
  /// scroll past the last line into the void.
  pub max_scroll: u16,
  /// Maximum horizontal offset, republished by the renderer.
  pub max_x_scroll: u16,
}

impl HelpOverlay {
  pub fn new() -> Self {
    Self::default()
  }

  /// Rewind to the top-left. Called on every open, so a previously-scrolled
  /// visit starts fresh.
  ///
  /// The two `max_*` bounds are deliberately left alone: they belong to the
  /// renderer, which republishes them on the next frame, and zeroing them
  /// here would make the first keypress after an open clamp to 0 and read
  /// as a dead key.
  pub fn rewind(&mut self) {
    self.scroll = 0;
    self.x_scroll = 0;
  }

  /// Scroll down one row, clamped to the renderer-published bound so it
  /// never scrolls past the last line.
  pub fn scroll_down(&mut self) {
    self.scroll = (self.scroll + 1).min(self.max_scroll);
  }

  /// Scroll up one row, clamped at the top.
  pub fn scroll_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(1);
  }

  /// Pan right one column, clamped to the renderer-published bound.
  pub fn scroll_right(&mut self) {
    self.x_scroll = (self.x_scroll + 1).min(self.max_x_scroll);
  }

  /// Pan left one column, clamped at the left edge.
  pub fn scroll_left(&mut self) {
    self.x_scroll = self.x_scroll.saturating_sub(1);
  }
}
