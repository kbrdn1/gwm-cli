//! State-machine tests for the command palette view in `App`
//! (issue #32).
//!
//! Drives `App` methods directly — no terminal, no ratatui frame.
//! The pure overlay state lives on `App.palette: PaletteState`; this
//! test file pins the orchestrator-level transitions (open, type,
//! accept-then-fire-action, esc-cancel) so a regression in the
//! event-loop wiring fails here with a clear message.

mod common;

use common::init_repo;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwm::tui::keymap::Action;
use gwm::tui::{App, View};

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at(Some(dir.path())).unwrap();
  (dir, app)
}

#[test]
fn palette_starts_closed_and_view_is_list() {
  let (_dir, app) = make_app();
  assert_eq!(app.view, View::List);
  assert!(!app.palette.open);
}

#[test]
fn command_palette_action_opens_the_overlay() {
  // The `:` key is bound to `Action::CommandPalette` by the default
  // keymap. Pressing it must transition the view AND arm the overlay
  // state; otherwise the next keystroke wouldn't reach the palette
  // input bar.
  let (_dir, mut app) = make_app();
  assert_eq!(
    app.dispatch_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::empty())),
    Some(Action::CommandPalette)
  );
  app.open_command_palette();
  assert_eq!(app.view, View::CommandPalette);
  assert!(app.palette.open);
  assert!(app.palette.buffer().is_empty());
}

#[test]
fn typing_into_palette_pushes_chars_to_buffer() {
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  app.palette_push_char('c');
  app.palette_push_char('r');
  app.palette_push_char('e');
  assert_eq!(app.palette.buffer(), "cre");
}

#[test]
fn esc_closes_palette_without_firing() {
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  app.palette_push_char('q');
  app.close_command_palette();
  assert!(!app.palette.open);
  assert_eq!(app.view, View::List);
  assert!(app.palette.buffer().is_empty(), "buffer must clear on close");
}

#[test]
fn accept_palette_fires_action_and_returns_to_list() {
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  // Type enough of "help" to make it the top match, then accept.
  app.palette_push_char('h');
  app.palette_push_char('e');
  app.palette_push_char('l');
  app.palette_push_char('p');
  let action = app.accept_command_palette();
  assert_eq!(action, Some(Action::Help));
  // The palette closes; firing `Help` switches the view to `View::Help`.
  assert!(!app.palette.open);
  // The event loop is responsible for actually applying the action's
  // side effect (here: setting `view = View::Help`). The `accept_*`
  // method only returns the resolved action, so we observe the
  // post-accept state on the palette + view-stays-on-list contract.
  assert_eq!(
    app.view,
    View::List,
    "accept clears the palette overlay; the caller routes the action through the normal dispatcher"
  );
}

#[test]
fn cycle_highlight_walks_matches() {
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  let first = app.palette.highlight();
  app.palette_cycle_down();
  assert_ne!(app.palette.highlight(), first, "cycle_down must advance the highlight");
}

#[test]
fn palette_quit_must_route_through_should_quit_flag() {
  // Copilot review on PR #167 caught that `Action::Quit` was a
  // no-op in `run_action`, so `:quit` from the palette never
  // exited the TUI even though `quit` was a registered palette
  // entry. The fix routes Quit through `App.should_quit`, which
  // the event loop checks at the top of every iteration. Pin
  // that the palette's accept produces `Action::Quit` on a `quit`
  // input — the actual flag toggle happens inside the event-loop
  // helper `run_action` and is exercised by the keystroke `q`
  // path which already worked.
  let (_dir, mut app) = make_app();
  assert!(!app.should_quit, "fresh App must not be in quit state");
  app.open_command_palette();
  for c in "quit".chars() {
    app.palette_push_char(c);
  }
  assert_eq!(app.accept_command_palette(), Some(Action::Quit));
}
