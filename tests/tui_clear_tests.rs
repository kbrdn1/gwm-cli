//! Issue #548 — returning from a fullscreen surface must not depend on the
//! terminal answering a cursor-position query.
//!
//! `Terminal::clear` snapshots the cursor via `ESC [ 6 n` and waits for the
//! DSR report. On the return path from the PTY overlay / an `exec` run / a
//! review launch that answer can be late, crossterm gives up with
//! `The cursor position could not be read within a normal duration`, and the
//! `?` in `run_app` used to end the session over it.
//!
//! The race itself is timing-dependent and not reproducible on demand, so what
//! is pinned here is the error path: a backend whose `get_cursor_position`
//! always fails the way crossterm fails on a timeout. Two properties are
//! asserted, and both are needed:
//!
//!   1. the replacement survives a backend that cannot report the cursor —
//!      guarded by first asserting that `Terminal::clear` *does* fail on the
//!      same terminal, otherwise the fixture could be passing vacuously;
//!   2. it still forces a full repaint. A fix that only wiped the screen
//!      (`clear_region(All)`) without resetting the back buffer would leave the
//!      next `draw` diffing the frame against itself, writing nothing, and the
//!      screen would stay blank — a plausible-looking fix that trades a crash
//!      for an empty TUI.

use gwm::tui::clear_without_cursor_query;
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Size};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use std::io;

/// A `TestBackend` that never answers the DSR query, the way a terminal that
/// is still mid-stream on the return path from a fullscreen child does not.
/// Every other operation is delegated untouched.
struct DsrTimeout {
  inner: TestBackend,
}

impl DsrTimeout {
  fn new(width: u16, height: u16) -> Self {
    Self {
      inner: TestBackend::new(width, height),
    }
  }

  fn buffer(&self) -> &Buffer {
    self.inner.buffer()
  }
}

/// `TestBackend::Error` is `Infallible`; widen it to the `io::Error` a real
/// crossterm backend reports so the fixture can fail the one method it must.
fn widen<T>(result: std::result::Result<T, core::convert::Infallible>) -> io::Result<T> {
  match result {
    Ok(value) => Ok(value),
    Err(never) => match never {},
  }
}

impl Backend for DsrTimeout {
  type Error = io::Error;

  fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
  where
    I: Iterator<Item = (u16, u16, &'a Cell)>,
  {
    widen(self.inner.draw(content))
  }

  fn hide_cursor(&mut self) -> io::Result<()> {
    widen(self.inner.hide_cursor())
  }

  fn show_cursor(&mut self) -> io::Result<()> {
    widen(self.inner.show_cursor())
  }

  fn get_cursor_position(&mut self) -> io::Result<Position> {
    // Verbatim the message crossterm produces when the DSR report is late.
    Err(io::Error::other(
      "The cursor position could not be read within a normal duration",
    ))
  }

  fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
    widen(self.inner.set_cursor_position(position))
  }

  fn clear(&mut self) -> io::Result<()> {
    widen(self.inner.clear())
  }

  fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
    widen(self.inner.clear_region(clear_type))
  }

  fn size(&self) -> io::Result<Size> {
    widen(self.inner.size())
  }

  fn window_size(&mut self) -> io::Result<WindowSize> {
    widen(self.inner.window_size())
  }

  fn flush(&mut self) -> io::Result<()> {
    widen(self.inner.flush())
  }
}

/// Flatten the backend buffer into one string so a row's text can be found
/// without caring which column it starts at.
fn buffer_text(buffer: &Buffer) -> String {
  buffer
    .content()
    .iter()
    .map(|cell| cell.symbol())
    .collect::<Vec<_>>()
    .join("")
}

#[test]
fn clear_without_cursor_query_survives_a_dsr_timeout() {
  let mut terminal = Terminal::new(DsrTimeout::new(20, 5)).expect("terminal");

  // The fixture must actually bite: if `Terminal::clear` succeeded here, the
  // assertion below would prove nothing about the DSR path.
  let via_clear = terminal.clear();
  assert!(
    via_clear.is_err(),
    "fixture is vacuous: Terminal::clear did not fail on a backend that cannot report the cursor"
  );
  assert!(
    via_clear
      .unwrap_err()
      .to_string()
      .contains("cursor position could not be read"),
    "fixture failed for the wrong reason"
  );

  clear_without_cursor_query(&mut terminal).expect("clearing must not depend on a DSR report");
}

#[test]
fn clear_without_cursor_query_forces_a_full_repaint() {
  let mut terminal = Terminal::new(DsrTimeout::new(20, 5)).expect("terminal");

  terminal
    .draw(|frame| frame.render_widget(Paragraph::new("worktrees"), frame.area()))
    .expect("first draw");
  assert!(
    buffer_text(terminal.backend().buffer()).contains("worktrees"),
    "sanity: the first frame should have painted"
  );

  clear_without_cursor_query(&mut terminal).expect("clear");
  assert!(
    !buffer_text(terminal.backend().buffer()).contains("worktrees"),
    "the screen should have been wiped"
  );

  // Same frame as before. Without the back-buffer reset this diffs against
  // itself and paints nothing, leaving the wiped screen blank.
  terminal
    .draw(|frame| frame.render_widget(Paragraph::new("worktrees"), frame.area()))
    .expect("repaint");
  assert!(
    buffer_text(terminal.backend().buffer()).contains("worktrees"),
    "the frame after the clear must be a full repaint, not a diff against stale content"
  );
}
