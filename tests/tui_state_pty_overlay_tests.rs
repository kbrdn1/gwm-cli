//! State-machine and pure-function tests for the PTY overlay (issue #35).
//! Key-to-bytes conversion is pure and runs on all platforms; PTY spawn /
//! lifecycle tests are guarded with `#[cfg(unix)]` because the spawned
//! programs (`sh -c "sleep 60"`, `echo`) are not available on Windows CI.

mod common;

use common::init_repo;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwm::tui::{key_to_bytes, App, PtyKind, PtyOverlay, View};

// ── helpers ────────────────────────────────────────────────────────────────

fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
  KeyEvent::new(code, mods)
}

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();
  (dir, app)
}

// ── pure: key → bytes ──────────────────────────────────────────────────────

#[test]
fn key_to_bytes_char_ascii() {
  assert_eq!(key_to_bytes(ev(KeyCode::Char('a'), KeyModifiers::NONE)), b"a");
  assert_eq!(key_to_bytes(ev(KeyCode::Char('Z'), KeyModifiers::NONE)), b"Z");
  assert_eq!(key_to_bytes(ev(KeyCode::Char('1'), KeyModifiers::NONE)), b"1");
  assert_eq!(key_to_bytes(ev(KeyCode::Char(' '), KeyModifiers::NONE)), b" ");
}

#[test]
fn key_to_bytes_enter_is_carriage_return() {
  assert_eq!(key_to_bytes(ev(KeyCode::Enter, KeyModifiers::NONE)), b"\r");
}

#[test]
fn key_to_bytes_backspace_is_del() {
  assert_eq!(key_to_bytes(ev(KeyCode::Backspace, KeyModifiers::NONE)), &[127u8]);
}

#[test]
fn key_to_bytes_tab_is_ht() {
  assert_eq!(key_to_bytes(ev(KeyCode::Tab, KeyModifiers::NONE)), b"\t");
}

#[test]
fn key_to_bytes_esc_is_escape_byte() {
  assert_eq!(key_to_bytes(ev(KeyCode::Esc, KeyModifiers::NONE)), &[27u8]);
}

#[test]
fn key_to_bytes_arrow_sequences() {
  assert_eq!(key_to_bytes(ev(KeyCode::Up, KeyModifiers::NONE)),    &[27, b'[', b'A']);
  assert_eq!(key_to_bytes(ev(KeyCode::Down, KeyModifiers::NONE)),  &[27, b'[', b'B']);
  assert_eq!(key_to_bytes(ev(KeyCode::Right, KeyModifiers::NONE)), &[27, b'[', b'C']);
  assert_eq!(key_to_bytes(ev(KeyCode::Left, KeyModifiers::NONE)),  &[27, b'[', b'D']);
}

#[test]
fn key_to_bytes_ctrl_alpha_produces_control_codes() {
  // Ctrl+A = 0x01, Ctrl+C = 0x03, Ctrl+L = 0x0c, Ctrl+Z = 0x1a
  assert_eq!(key_to_bytes(ev(KeyCode::Char('a'), KeyModifiers::CONTROL)), &[1u8]);
  assert_eq!(key_to_bytes(ev(KeyCode::Char('c'), KeyModifiers::CONTROL)), &[3u8]);
  assert_eq!(key_to_bytes(ev(KeyCode::Char('l'), KeyModifiers::CONTROL)), &[12u8]);
  assert_eq!(key_to_bytes(ev(KeyCode::Char('z'), KeyModifiers::CONTROL)), &[26u8]);
}

#[test]
fn key_to_bytes_page_navigation() {
  assert_eq!(key_to_bytes(ev(KeyCode::PageUp,   KeyModifiers::NONE)), &[27, b'[', b'5', b'~']);
  assert_eq!(key_to_bytes(ev(KeyCode::PageDown, KeyModifiers::NONE)), &[27, b'[', b'6', b'~']);
  assert_eq!(key_to_bytes(ev(KeyCode::Home,     KeyModifiers::NONE)), &[27, b'[', b'H']);
  assert_eq!(key_to_bytes(ev(KeyCode::End,      KeyModifiers::NONE)), &[27, b'[', b'F']);
  assert_eq!(key_to_bytes(ev(KeyCode::Delete,   KeyModifiers::NONE)), &[27, b'[', b'3', b'~']);
}

// ── App state-machine (no PTY needed) ─────────────────────────────────────

#[test]
fn close_pty_overlay_is_noop_when_no_overlay_is_open() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.view, View::List);
  app.close_pty_overlay(); // must not panic
  assert_eq!(app.view, View::List);
  assert!(app.pty_overlay.is_none());
}

// ── Unix PTY lifecycle ─────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn pty_overlay_spawn_process_is_alive() {
  let (_dir, app) = make_app();
  let mut pty =
    PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
      .expect("PTY spawn must succeed on Unix");
  assert!(pty.is_alive(), "process must be alive right after spawn");
  pty.kill();
}

#[cfg(unix)]
#[test]
fn pty_overlay_is_dead_after_kill() {
  let (_dir, app) = make_app();
  let mut pty =
    PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
      .expect("PTY spawn must succeed on Unix");
  pty.kill();
  std::thread::sleep(std::time::Duration::from_millis(150));
  assert!(!pty.is_alive(), "process must be dead after kill");
}

#[cfg(unix)]
#[test]
fn open_pty_overlay_switches_view_to_pty() {
  let (_dir, mut app) = make_app();
  let pty =
    PtyOverlay::spawn(PtyKind::LazyGit, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
      .expect("PTY spawn must succeed on Unix");
  app.open_pty_overlay(pty);
  assert_eq!(app.view, View::Pty, "open_pty_overlay must switch to View::Pty");
  assert!(app.pty_overlay.is_some(), "pty_overlay must be Some after opening");
  app.close_pty_overlay();
}

#[cfg(unix)]
#[test]
fn close_pty_overlay_returns_view_to_list() {
  let (_dir, mut app) = make_app();
  let pty =
    PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
      .expect("PTY spawn must succeed on Unix");
  app.open_pty_overlay(pty);
  app.close_pty_overlay();
  assert_eq!(app.view, View::List, "close_pty_overlay must return to View::List");
  assert!(app.pty_overlay.is_none(), "pty_overlay must be None after close");
}

#[cfg(unix)]
#[test]
fn pty_overlay_poll_bytes_does_not_panic_on_output() {
  let (_dir, app) = make_app();
  let mut pty =
    PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "echo hello"], &app.workdir, 80, 24)
      .expect("PTY spawn must succeed on Unix");
  std::thread::sleep(std::time::Duration::from_millis(150));
  // poll_bytes drains the reader channel; must not panic even if the process
  // already exited and the channel is closed or has leftover output.
  pty.poll_bytes();
}
