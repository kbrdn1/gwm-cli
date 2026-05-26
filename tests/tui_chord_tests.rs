//! Tests for the generic chord buffer in `App` (issue #87).
//!
//! Drives `App::dispatch_key` end-to-end: feed it crossterm
//! `KeyEvent`s, observe the returned `Option<Action>` and the
//! internal pending-keys buffer state. The dispatcher is the bridge
//! between raw terminal events and the rebindable keymap — it owns
//! the "is this the first half of a chord?" decision the old hard-
//! coded `gg` handler used to make in `App::handle_g`.
//!
//! These tests deliberately stay below the event-loop layer: they do
//! not touch ratatui or crossterm's terminal handle. The harness
//! mirrors the one in `tests/tui_app_tests.rs` (an `App` built on a
//! tempdir-backed repo).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwm::tui::keymap::Action;
use gwm::tui::App;

mod common;
use common::init_repo;

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at(Some(dir.path())).unwrap();
  (dir, app)
}

fn press(c: char) -> KeyEvent {
  KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn press_named(code: KeyCode) -> KeyEvent {
  KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn single_key_dispatches_action_immediately() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('j')), Some(Action::Down));
  assert!(app.pending_chord_is_empty(), "buffer must clear after a matched action");
}

#[test]
fn first_g_arms_pending_buffer() {
  // `g` alone is the strict prefix of the default `g g` (Top). The
  // dispatcher must keep the buffer armed and return None so the
  // event loop can render a status hint.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert!(
    !app.pending_chord_is_empty(),
    "first g must leave the chord buffer armed"
  );
}

#[test]
fn second_g_completes_chord() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert_eq!(app.dispatch_key(press('g')), Some(Action::Top));
  assert!(
    app.pending_chord_is_empty(),
    "buffer must clear after the chord matches"
  );
}

#[test]
fn mismatched_second_key_falls_back_to_single_key_dispatch() {
  // Vim's classic recovery: `g j` is not a bound chord, but `j` alone
  // is bound to Down. The dispatcher must clear the buffer AND retry
  // the new stroke alone so the user's `j` still navigates down.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert_eq!(app.dispatch_key(press('j')), Some(Action::Down));
  assert!(app.pending_chord_is_empty());
}

#[test]
fn mismatched_second_key_with_no_fallback_clears_buffer() {
  // `g` then `z` — neither a chord match nor a single-key binding.
  // Returns None and clears the buffer; the event loop ignores it.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert_eq!(app.dispatch_key(press('z')), None);
  assert!(app.pending_chord_is_empty());
}

#[test]
fn unbound_key_returns_none_without_arming_buffer() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('z')), None);
  assert!(app.pending_chord_is_empty());
}

#[test]
fn named_key_dispatch() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press_named(KeyCode::Down)), Some(Action::Down));
  assert_eq!(app.dispatch_key(press_named(KeyCode::Tab)), Some(Action::FocusSwap));
}

#[test]
fn shifted_uppercase_g_dispatches_bottom() {
  // The default `Bottom` binding is `G` (uppercase char, no modifier),
  // not `Shift+g`. Most terminals deliver Shift+G as `KeyCode::Char('G')`
  // sans modifier, which is what `KeyStroke::from_event` consumes.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('G')), Some(Action::Bottom));
}

#[test]
fn s_dispatches_toggle_sidebar_mode() {
  // Issue #34: pressing `s` in the list view must cycle the sidebar
  // preview mode (Commits ↔ Stashes). The binding is wired through
  // the rebindable keymap (`Action::ToggleSidebarMode`) so users can
  // remap it via `[tui.keys]` if `s` clashes with their muscle
  // memory.
  let (_dir, mut app) = make_app();
  assert_eq!(
    app.dispatch_key(press('s')),
    Some(gwm::tui::keymap::Action::ToggleSidebarMode)
  );
}

#[test]
fn help_overlay_lists_toggle_sidebar_mode() {
  // The keymap-driven help overlay surfaces the new action with its
  // default `s` binding so users discover the stashes mode through
  // `?` rather than the changelog.
  use gwm::tui::help_lines;
  use gwm::tui::keymap::Keymap;

  let km = Keymap::defaults();
  let lines = help_lines(&km, false);
  let row = lines
    .iter()
    .find(|l| l.contains("sidebar mode") || l.contains("stash") || l.contains("toggle_sidebar_mode"))
    .unwrap_or_else(|| panic!("expected a sidebar-mode row in:\n{}", lines.join("\n")));
  assert!(
    row.contains('s'),
    "expected the default `s` binding to appear, got: {row}"
  );
}

#[test]
fn help_overlay_reflects_user_keymap_override() {
  // The help overlay must read from the resolved keymap rather than
  // hard-coded strings, so a user who rebinds `down = ["Ctrl+n"]`
  // sees `Ctrl+n` next to "next" in `?`. Otherwise the discoverable
  // documentation drifts from the actual bindings — the worst kind
  // of doc bug for a TUI.
  use gwm::tui::help_lines;
  use gwm::tui::keymap::Keymap;

  let mut km = Keymap::defaults();
  km.apply_override(
    gwm::tui::keymap::Action::Down,
    vec![gwm::tui::keymap::KeyStroke::parse_chord("Ctrl+n").unwrap()],
  )
  .unwrap();

  let lines = help_lines(&km, false);
  let next_line = lines
    .iter()
    .find(|l| l.contains("next"))
    .unwrap_or_else(|| panic!("expected a `next` line in:\n{}", lines.join("\n")));
  assert!(
    next_line.contains("Ctrl+n"),
    "expected the override to appear next to `next`, got: {next_line}"
  );
  assert!(
    !next_line.contains("j"),
    "the default `j` binding must NOT appear after the override, got: {next_line}"
  );
}

#[test]
fn user_keymap_override_dispatches_through_app() {
  // End-to-end: writing `[tui.keys] down = ["Ctrl+n"]` in `.gwm.toml`
  // must change which keystroke fires `Action::Down` in the TUI. This
  // is the contract the whole #87 stack exists to deliver — surface
  // it as one concrete test so a regression in any layer
  // (Config::load → Keymap → App::keymap → dispatch_key) lights up
  // here with an unambiguous message.
  let (dir, _) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[tui.keys]
down = ["Ctrl+n"]
"#,
  )
  .unwrap();

  let mut app = App::new_at(Some(dir.path())).unwrap();

  // The default `j` no longer fires Down — the override replaces.
  assert_eq!(app.dispatch_key(press('j')), None);

  // `Ctrl+n` does.
  let ctrl_n = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
  assert_eq!(app.dispatch_key(ctrl_n), Some(Action::Down));
}
