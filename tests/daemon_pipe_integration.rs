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

#[test]
fn a_stopped_subscription_frees_its_server_connection_slot() {
  // Codex review #439: when the callback stops the subscription, both
  // stream halves must actually drop (closing the pipe) — with a daemon
  // capped at ONE connection, a leaked subscribe connection would make
  // every later request fail, so a passing list_once IS the proof the
  // slot was released.
  let (dir, _repo) = init_repo();
  let workdir = dir.path().to_path_buf();
  let pipe = pipe_name("slot");
  let shutdown = Arc::new(AtomicBool::new(false));
  let mut opts = ServeOptions::new(pipe.clone(), workdir, Duration::from_millis(50));
  opts.max_connections = 1;
  let flag = Arc::clone(&shutdown);
  let handle = thread::spawn(move || {
    serve(&opts, flag).expect("serve must bind and run");
  });
  let daemon = TestDaemon {
    pipe,
    shutdown,
    handle: Some(handle),
  };
  for _ in 0..200 {
    if client::list_once(&daemon.pipe).is_ok() {
      break;
    }
    thread::sleep(Duration::from_millis(10));
  }

  client::subscribe(&daemon.pipe, |_| false).expect("one snapshot then stop");
  // The reader thread exits within a tick; give it a moment, then the
  // single slot must be usable again.
  let mut freed = false;
  for _ in 0..100 {
    if client::list_once(&daemon.pipe).is_ok() {
      freed = true;
      break;
    }
    thread::sleep(Duration::from_millis(20));
  }
  assert!(freed, "the subscription's connection slot must be released after stop");
}

#[test]
fn an_idle_connection_survives_between_two_requests() {
  // Codex review #439 (P1): an empty PIPE_NOWAIT read surfaces raw
  // ERROR_NO_DATA, which std maps to BrokenPipe — kind-only matching
  // treated the idle gap between two requests as a dead link and closed
  // the connection. A raw client sends two `list` requests 200 ms apart
  // on ONE connection; both must get a response line.
  use gwm::daemon::LIST_REQUEST;
  use interprocess::local_socket::{prelude::*, GenericNamespaced, Stream};
  use std::io::{BufRead, BufReader, Write};

  let (dir, _repo) = init_repo();
  let workdir = dir.path().to_path_buf();
  let daemon = TestDaemon::start(&workdir, "idle", Duration::from_millis(50));

  let name = daemon
    .pipe
    .to_string_lossy()
    .into_owned()
    .to_ns_name::<GenericNamespaced>()
    .expect("pipe name");
  let stream = Stream::connect(name).expect("connect");
  let (recv, mut send) = stream.split();
  let mut reader = BufReader::new(recv);

  for round in 0..2u8 {
    writeln!(send, "{LIST_REQUEST}").expect("request write");
    send.flush().expect("request flush");
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("response read");
    assert!(n > 0, "round {round}: the connection must still be open");
    assert!(
      line.contains("\"result\""),
      "round {round}: a JSON-RPC response is expected: {line:?}"
    );
    // Idle gap spanning many NB_TICK polls before the second request.
    thread::sleep(Duration::from_millis(200));
  }
}
