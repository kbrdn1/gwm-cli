//! Named-pipe round-trip tests for the daemon on Windows (issue #439) —
//! the pipe counterpart of `daemon_integration.rs`. Windows + `daemon`-
//! feature only: the pipe transport is `cfg`-compiled out elsewhere, so
//! this file compiles to nothing on unix. These tests CANNOT run on the
//! development machines (macOS/Linux) — the CI `windows-latest` job is
//! their execution environment, deliberately.
#![cfg(all(windows, feature = "daemon"))]

mod common;

use common::init_repo;
use gwm::daemon::{client, serve, ServeOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A unique pipe name per test: pipe names are machine-global (no tempdir
/// isolation like unix socket paths), so each test namespaces its own with
/// the process id plus a per-test tag.
fn pipe_name(tag: &str) -> PathBuf {
  PathBuf::from(format!("gwm-test-{}-{tag}.sock", std::process::id()))
}

/// A running daemon under test — same teardown contract as the unix
/// harness: flip the flag, join the serve thread.
struct TestDaemon {
  pipe: PathBuf,
  shutdown: Arc<AtomicBool>,
  handle: Option<JoinHandle<()>>,
}

impl TestDaemon {
  fn start(repo_workdir: &Path, tag: &str, poll: Duration) -> Self {
    let pipe = pipe_name(tag);
    let shutdown = Arc::new(AtomicBool::new(false));
    let opts = ServeOptions::new(pipe.clone(), repo_workdir.to_path_buf(), poll);
    let flag = Arc::clone(&shutdown);
    let handle = thread::spawn(move || {
      serve(&opts, flag).expect("serve must bind and run");
    });
    // Wait for the bind: a client list_once succeeds only once the pipe
    // exists. Retry briefly rather than sleeping a fixed amount.
    let daemon = TestDaemon {
      pipe,
      shutdown,
      handle: Some(handle),
    };
    for _ in 0..200 {
      if client::list_once(&daemon.pipe).is_ok() {
        return daemon;
      }
      thread::sleep(Duration::from_millis(10));
    }
    panic!("daemon never became reachable on {}", daemon.pipe.display());
  }
}

impl Drop for TestDaemon {
  fn drop(&mut self) {
    self.shutdown.store(true, Ordering::Relaxed);
    if let Some(h) = self.handle.take() {
      let _ = h.join();
    }
  }
}

#[test]
fn list_once_round_trips_over_the_named_pipe() {
  let (dir, _repo) = init_repo();
  let workdir = dir.path().to_path_buf();
  let daemon = TestDaemon::start(&workdir, "list", Duration::from_millis(50));

  let worktrees = client::list_once(&daemon.pipe).expect("list must round-trip");
  assert!(
    worktrees.iter().any(|w| w.is_main),
    "the main worktree must be in the snapshot: {worktrees:?}"
  );
}

#[test]
fn subscribe_delivers_the_initial_snapshot() {
  let (dir, _repo) = init_repo();
  let workdir = dir.path().to_path_buf();
  let daemon = TestDaemon::start(&workdir, "subscribe", Duration::from_millis(50));

  let mut snapshots = 0u32;
  client::subscribe(&daemon.pipe, |worktrees| {
    assert!(!worktrees.is_empty(), "the initial snapshot carries the worktree set");
    snapshots += 1;
    false // one snapshot is enough — end the stream
  })
  .expect("subscribe must deliver the initial snapshot");
  assert_eq!(snapshots, 1);
}

#[test]
fn list_once_errors_when_no_daemon_is_listening() {
  // The statusline's graceful degradation depends on a prompt error here,
  // not a hang: nothing listens on this name.
  let err = client::list_once(&pipe_name("nobody"));
  assert!(err.is_err(), "connecting to an unbound pipe must error");
}

#[test]
fn a_second_daemon_on_the_same_pipe_is_refused() {
  let (dir, _repo) = init_repo();
  let workdir = dir.path().to_path_buf();
  let daemon = TestDaemon::start(&workdir, "dup", Duration::from_millis(50));

  let opts = ServeOptions::new(daemon.pipe.clone(), workdir, Duration::from_millis(50));
  let err = serve(&opts, Arc::new(AtomicBool::new(true)));
  assert!(err.is_err(), "a live daemon must not be silently displaced");
  let msg = format!("{}", err.unwrap_err());
  assert!(msg.contains("already in use"), "the error names the conflict: {msg}");
}

#[test]
fn statusline_render_shape_survives_the_pipe() {
  // End-to-end through the public binary surface: `gwm statusline` against
  // the pipe daemon must print a non-empty segment line for the repo cwd.
  use std::process::Command;
  let (dir, _repo) = init_repo();
  let workdir = dir.path().to_path_buf();
  let daemon = TestDaemon::start(&workdir, "statusline", Duration::from_millis(50));

  let out = Command::new(env!("CARGO_BIN_EXE_gwm"))
    .args(["statusline", "--socket"])
    .arg(&daemon.pipe)
    .current_dir(&workdir)
    .output()
    .expect("gwm statusline must run");
  assert!(out.status.success(), "statusline exits 0: {out:?}");
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    !stdout.trim().is_empty(),
    "with a live pipe daemon the statusline is not the blank degradation: {stdout:?}"
  );
}
