//! State-machine and pure-function tests for the PTY overlay (issue #35).
//! Key-to-bytes conversion is pure and runs on all platforms; PTY spawn /
//! lifecycle tests are guarded with `#[cfg(unix)]` because the spawned
//! programs (`sh -c "sleep 60"`, `echo`) are not available on Windows CI.

mod common;

use common::init_repo;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwm::tui::{key_to_bytes, App, HintContext, View};
#[cfg(unix)]
use gwm::tui::{PtyKind, PtyOverlay};

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
  assert_eq!(key_to_bytes(ev(KeyCode::Up, KeyModifiers::NONE)), &[27, b'[', b'A']);
  assert_eq!(key_to_bytes(ev(KeyCode::Down, KeyModifiers::NONE)), &[27, b'[', b'B']);
  assert_eq!(key_to_bytes(ev(KeyCode::Right, KeyModifiers::NONE)), &[27, b'[', b'C']);
  assert_eq!(key_to_bytes(ev(KeyCode::Left, KeyModifiers::NONE)), &[27, b'[', b'D']);
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
fn key_to_bytes_alt_char_sends_meta_escape_prefix() {
  // Alt+<char> must send ESC followed by the UTF-8 bytes of the char so that
  // readline/bash word-navigation (Alt+f, Alt+b) and lazygit shortcuts work.
  assert_eq!(key_to_bytes(ev(KeyCode::Char('f'), KeyModifiers::ALT)), &[27, b'f']);
  assert_eq!(key_to_bytes(ev(KeyCode::Char('b'), KeyModifiers::ALT)), &[27, b'b']);
  assert_eq!(key_to_bytes(ev(KeyCode::Char('.'), KeyModifiers::ALT)), &[27, b'.']);
}

#[test]
fn key_to_bytes_page_navigation() {
  assert_eq!(
    key_to_bytes(ev(KeyCode::PageUp, KeyModifiers::NONE)),
    &[27, b'[', b'5', b'~']
  );
  assert_eq!(
    key_to_bytes(ev(KeyCode::PageDown, KeyModifiers::NONE)),
    &[27, b'[', b'6', b'~']
  );
  assert_eq!(key_to_bytes(ev(KeyCode::Home, KeyModifiers::NONE)), &[27, b'[', b'H']);
  assert_eq!(key_to_bytes(ev(KeyCode::End, KeyModifiers::NONE)), &[27, b'[', b'F']);
  assert_eq!(
    key_to_bytes(ev(KeyCode::Delete, KeyModifiers::NONE)),
    &[27, b'[', b'3', b'~']
  );
}

// ── App state-machine (no PTY needed) ─────────────────────────────────────

#[test]
fn hint_context_in_pty_view_is_pty_not_pane() {
  // While the PTY overlay is active, the footer must not show the underlying
  // list-view hints (new/delete/open/git) — the user can only interact via
  // the terminal, and Esc is the only gwm action available. The context must
  // be HintContext::Pty (not Worktrees/Status).
  let (_dir, mut app) = make_app();
  app.view = View::Pty;
  assert_eq!(
    app.hint_context(),
    HintContext::Pty,
    "hint_context must return Pty when View::Pty is active"
  );
}

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
  let mut pty = PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  assert!(pty.is_alive(), "process must be alive right after spawn");
  pty.kill();
}

#[cfg(unix)]
#[test]
fn pty_overlay_is_dead_after_kill() {
  let (_dir, app) = make_app();
  let mut pty = PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  pty.kill();
  std::thread::sleep(std::time::Duration::from_millis(150));
  assert!(!pty.is_alive(), "process must be dead after kill");
}

#[cfg(unix)]
#[test]
fn open_pty_overlay_switches_view_to_pty() {
  let (_dir, mut app) = make_app();
  let pty = PtyOverlay::spawn(PtyKind::LazyGit, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
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
  let pty = PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
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
  let mut pty = PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "echo hello"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  std::thread::sleep(std::time::Duration::from_millis(150));
  // poll_bytes drains the reader channel; must not panic even if the process
  // already exited and the channel is closed or has leftover output.
  pty.poll_bytes();
}

// ── kill() reaps child (no zombie) ────────────────────────────────────────

#[cfg(unix)]
#[test]
fn kill_reaps_child_no_zombie() {
  let (_dir, app) = make_app();
  let mut pty = PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  pty.kill();
  // After kill(), the process must be reaped — try_wait() returns Some.
  // We use try_wait_after_kill() which polls after the blocking wait in kill().
  let status = pty.try_wait_after_kill();
  assert!(
    status.is_some(),
    "try_wait_after_kill must return Some(exit_status) after kill() — process must be reaped"
  );
}

#[cfg(unix)]
#[test]
fn poll_bytes_caps_chunks_per_frame() {
  let (_dir, app) = make_app();
  // Spawn a process that writes a lot of output at once.
  let mut pty = PtyOverlay::spawn(
    PtyKind::Terminal,
    &["sh", "-c", "yes | head -10000"],
    &app.workdir,
    80,
    24,
  )
  .expect("PTY spawn must succeed on Unix");
  std::thread::sleep(std::time::Duration::from_millis(200));
  // poll_bytes must return without hanging even when there is a large backlog.
  // If it drains unboundedly it would hang/allocate without limit on continuous output.
  let t0 = std::time::Instant::now();
  pty.poll_bytes();
  assert!(
    t0.elapsed().as_millis() < 500,
    "poll_bytes must return quickly even with large backlog (cap per frame)"
  );
  pty.kill();
}

// ── diff_file lifetime ────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn diff_file_survives_while_overlay_is_alive() {
  let (_dir, app) = make_app();
  let tmp = tempfile::NamedTempFile::new().expect("tempfile must be created");
  let path = tmp.path().to_path_buf();
  assert!(path.exists(), "tempfile must exist before attaching to overlay");

  let mut pty = PtyOverlay::spawn(PtyKind::Review, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  pty.diff_file = Some(tmp);

  // The file must still exist while the overlay is alive.
  assert!(path.exists(), "diff tempfile must survive while PTY overlay is alive");

  pty.kill();
  drop(pty);
  // After the overlay is dropped, the tempfile must be deleted.
  assert!(
    !path.exists(),
    "diff tempfile must be deleted when PTY overlay is dropped"
  );
}

// ── PtyKind::Review discriminator ─────────────────────────────────────────

#[cfg(unix)]
#[test]
fn pty_kind_review_discriminates_from_lazygit_and_terminal() {
  assert_ne!(PtyKind::Review, PtyKind::LazyGit);
  assert_ne!(PtyKind::Review, PtyKind::Terminal);
}

#[cfg(unix)]
#[test]
fn exec_overlay_lingers_after_a_one_shot_command_exits() {
  // #325: an exec profile is typically a one-shot command (`cargo test`)
  // that exits the moment it finishes. The overlay must persist its output —
  // the run loop sets `finished` and keeps it open — rather than vanish like
  // an interactive lazygit session. Pin the detectable lifecycle the loop
  // keys off: a freshly spawned Exec overlay is not yet `finished`, and its
  // one-shot child reads `!is_alive()` once it exits on its own.
  let (_dir, app) = make_app();
  let mut pty = PtyOverlay::spawn(PtyKind::Exec, &["sh", "-c", "exit 0"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  assert_eq!(pty.kind, PtyKind::Exec);
  assert!(!pty.finished, "a freshly spawned overlay is not yet lingering");
  let mut dead = false;
  for _ in 0..50 {
    if !pty.is_alive() {
      dead = true;
      break;
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
  }
  assert!(dead, "the one-shot exec command must exit on its own");
  assert!(pty.is_reaped(), "observing the exit records the reap");
  assert!(
    !pty.group_signalled(),
    "the group is not signalled until the loop marks it finished"
  );
  // #333: the run loop marks a dead exec finished — which lingers the output
  // AND promptly SIGKILLs the process group (descendant cleanup) in the safe
  // window, before the dismissal keystroke could let the PGID be recycled.
  pty.mark_finished();
  assert!(pty.finished, "the overlay lingers showing its final output");
  assert!(pty.group_signalled(), "the process group is reaped promptly on finish");
  // Dismissal calls kill() — idempotent (no second signal), returns fast.
  let t0 = std::time::Instant::now();
  pty.kill();
  assert!(t0.elapsed().as_millis() < 100, "kill after finish is a fast no-op");
}

#[cfg(unix)]
#[test]
fn kill_after_reap_still_signals_the_process_group_once() {
  // #333: my earlier no-op-after-reap guard leaked backgrounded descendants
  // (`sh -c "sleep 60 &"`) because it skipped the process-group SIGKILL. kill
  // must STILL signal the group after the leader was reaped — once — so the
  // descendants are cleaned, without re-signalling a possibly-recycled PGID.
  let (_dir, app) = make_app();
  let mut pty = PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "exit 0"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  for _ in 0..50 {
    if !pty.is_alive() {
      break;
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
  }
  assert!(pty.is_reaped(), "the leader exited and was reaped");
  assert!(!pty.group_signalled(), "no group signal yet");
  let t0 = std::time::Instant::now();
  pty.kill();
  assert!(
    pty.group_signalled(),
    "kill still cleans the process group after a reap"
  );
  assert!(
    t0.elapsed().as_millis() < 100,
    "and returns fast (no drain loop — leader already reaped)"
  );
  // A second kill is idempotent — the group is never signalled twice.
  pty.kill();
  assert!(pty.group_signalled());
}

// ── teardown on close (issue #421) ─────────────────────────────────────────

#[cfg(unix)]
#[test]
fn kill_runs_the_attached_teardown() {
  // A containerised exec profile is spawned through `docker run`, and killing
  // that client leaves the container running: the daemon owns it, and `--rm`
  // only fires when it exits. The overlay therefore runs a teardown on close.
  // Stood in for by `touch`, which needs no daemon.
  let (dir, app) = make_app();
  let marker = dir.path().join("torn-down");
  let mut pty = PtyOverlay::spawn(PtyKind::Exec, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix")
    .with_teardown(
      vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("touch {}", marker.display()),
      ],
      app.workdir.clone(),
    );
  assert!(pty.teardown_argv().is_some(), "the teardown is attached");
  assert!(!marker.exists(), "and has not run before the kill");

  pty.kill();
  assert!(marker.exists(), "kill runs the teardown: {}", marker.display());
  assert!(pty.teardown_argv().is_none(), "and runs it once, not on every kill");
}

#[cfg(unix)]
#[test]
fn kill_runs_the_teardown_even_when_the_client_already_exited() {
  // The container outlives its client, so "the client is already reaped" says
  // nothing about the container — that early return must not skip the
  // teardown.
  let (dir, app) = make_app();
  let marker = dir.path().join("torn-down-late");
  let mut pty = PtyOverlay::spawn(PtyKind::Exec, &["sh", "-c", "exit 0"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix")
    .with_teardown(
      vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("touch {}", marker.display()),
      ],
      app.workdir.clone(),
    );
  // Let the leader exit and be reaped through the normal liveness path.
  for _ in 0..100 {
    if !pty.is_alive() {
      break;
    }
    std::thread::sleep(std::time::Duration::from_millis(10));
  }
  assert!(pty.is_reaped(), "the client exited on its own");

  pty.kill();
  assert!(
    marker.exists(),
    "the container is still torn down: {}",
    marker.display()
  );
}

#[cfg(unix)]
#[test]
fn an_overlay_without_a_teardown_runs_nothing_extra() {
  let (_dir, app) = make_app();
  let mut pty = PtyOverlay::spawn(PtyKind::Terminal, &["sh", "-c", "sleep 60"], &app.workdir, 80, 24)
    .expect("PTY spawn must succeed on Unix");
  assert!(pty.teardown_argv().is_none(), "host commands carry none");
  pty.kill(); // must not panic
}
