//! Animated spinner frames for the TUI loader (issue #187).
//!
//! Pure state — no I/O, no `App` dependency. The event loop advances it
//! from the same 200 ms poll cadence that drives the confirm-countdown
//! tick (`src/tui/state/confirm.rs`), and the renderer reads
//! [`Spinner::glyph`] for the current frame. Patterned on the `Spinner`
//! component from [`ratatui-cheese`](https://github.com/shashanktomar/ratatui-cheese)
//! (the braille "dot" cycle).
//!
//! Honest scope: the animation only advances while the event loop keeps
//! ticking. That is the case during the armed confirm countdown (the
//! 200 ms poll re-enters every tick), so the loader animates there. A
//! blocking `gh` shell-out does not yield to the loop, so it cannot
//! animate mid-call without moving the work off-thread — out of scope
//! for this polish pass.

/// Braille "dot" spinner frames — the default loader animation.
pub const DOT_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A frame cursor over a spinner animation. `Default` starts at frame 0.
#[derive(Debug, Default, Clone)]
pub struct Spinner {
  frame: usize,
}

impl Spinner {
  pub fn new() -> Self {
    Self::default()
  }

  /// Advance to the next frame. Uses `wrapping_add` so a long-lived
  /// spinner never overflows `usize`; the read-time modulo in
  /// [`Spinner::glyph`] keeps the visible frame in range.
  pub fn tick(&mut self) {
    self.frame = self.frame.wrapping_add(1);
  }

  /// Reset to the first frame. Called when the owning overlay opens so
  /// each loader starts from a deterministic frame.
  pub fn reset(&mut self) {
    self.frame = 0;
  }

  /// Current glyph from `frames`, wrapping the cursor over its length.
  /// Returns `""` for an empty frame set rather than panicking — a
  /// defensive contract so a mis-configured frame list degrades to a
  /// blank rather than crashing the render.
  pub fn glyph<'a>(&self, frames: &[&'a str]) -> &'a str {
    if frames.is_empty() {
      return "";
    }
    frames[self.frame % frames.len()]
  }
}
