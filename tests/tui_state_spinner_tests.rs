//! Unit tests for the pure `Spinner` loader state (issue #187).
//!
//! Exercises the frame cursor in isolation — no `App`, no ratatui
//! backend. The event loop advances the spinner on its 200 ms tick and
//! the renderer reads `glyph`; both contracts are pinned here.

use gwm::tui::state::spinner::{Spinner, DOT_FRAMES};

#[test]
fn new_spinner_shows_the_first_frame() {
  let s = Spinner::new();
  assert_eq!(s.glyph(DOT_FRAMES), DOT_FRAMES[0]);
}

#[test]
fn tick_advances_to_the_next_frame() {
  let mut s = Spinner::new();
  s.tick();
  assert_eq!(s.glyph(DOT_FRAMES), DOT_FRAMES[1]);
  s.tick();
  assert_eq!(s.glyph(DOT_FRAMES), DOT_FRAMES[2]);
}

#[test]
fn glyph_wraps_after_a_full_cycle() {
  let mut s = Spinner::new();
  for _ in 0..DOT_FRAMES.len() {
    s.tick();
  }
  // Back to the first frame after exactly `len` ticks.
  assert_eq!(s.glyph(DOT_FRAMES), DOT_FRAMES[0]);
}

#[test]
fn reset_returns_to_the_first_frame() {
  let mut s = Spinner::new();
  s.tick();
  s.tick();
  s.reset();
  assert_eq!(s.glyph(DOT_FRAMES), DOT_FRAMES[0]);
}

#[test]
fn glyph_is_blank_for_an_empty_frame_set() {
  // Defensive contract: a mis-configured (empty) frame list degrades to
  // a blank glyph rather than panicking the render.
  let mut s = Spinner::new();
  s.tick();
  assert_eq!(s.glyph(&[]), "");
}
