//! Socket round-trip tests for the daemon (issue #38, phase 2). Unix +
//! `daemon`-feature only: the server is `cfg`-compiled out elsewhere, so
//! the whole file is gated and compiles to nothing on Windows / minimal
//! builds. The pure RPC core is covered cross-platform in
//! `daemon_tests.rs`.
#![cfg(all(unix, feature = "daemon"))]

mod common;

use common::init_repo;
use gwm::daemon::{serve, ServeOptions};
use gwm::worktree;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tempfile::TempDir;

/// A running daemon under test: holds the shutdown flag and the serve
/// thread so the test can tear it down deterministically.
struct TestDaemon {
  socket: PathBuf,
  shutdown: Arc<AtomicBool>,
  handle: Option<JoinHandle<()>>,
}

impl TestDaemon {
  /// Start a daemon serving `repo_workdir`. The socket is bound inside
  /// `sock_dir` under a 1-char name to stay well under the ~104-byte
  /// `sun_path` limit on macOS.
  fn start(repo_workdir: &Path, sock_dir: &Path, poll: Duration) -> Self {
    let socket = sock_dir.join("s");
    let shutdown = Arc::new(AtomicBool::new(false));
    let opts = ServeOptions {
      socket: socket.clone(),
      repo_workdir: repo_workdir.to_path_buf(),
      poll_interval: poll,
    };
    let flag = Arc::clone(&shutdown);
    let handle = thread::spawn(move || {
      serve(&opts, flag).expect("serve must bind and run");
    });
    TestDaemon {
      socket,
      shutdown,
      handle: Some(handle),
    }
  }

  /// Connect a fresh client, retrying until the serve thread has bound.
  fn connect(&self) -> UnixStream {
    for _ in 0..200 {
      if let Ok(s) = UnixStream::connect(&self.socket) {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        return s;
      }
      thread::sleep(Duration::from_millis(10));
    }
    panic!("could not connect to daemon socket at {}", self.socket.display());
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

/// A persistent client connection: one cloned write half + one long-lived
/// `BufReader` over the read half. Re-creating a `BufReader` per request
/// would drop a cloned fd (and any read-ahead) between calls, so a
/// connection is opened once and reused — matching how a real client
/// behaves.
struct Client {
  writer: UnixStream,
  reader: BufReader<UnixStream>,
}

impl Client {
  fn new(stream: UnixStream) -> Self {
    let writer = stream.try_clone().unwrap();
    Client {
      writer,
      reader: BufReader::new(stream),
    }
  }

  fn request(&mut self, line: &str) -> serde_json::Value {
    writeln!(self.writer, "{line}").unwrap();
    self.writer.flush().unwrap();
    self.read_value()
  }

  fn read_value(&mut self) -> serde_json::Value {
    let mut resp = String::new();
    self.reader.read_line(&mut resp).expect("must read a response line");
    serde_json::from_str(&resp).expect("response must be valid JSON")
  }
}

#[test]
fn list_round_trips_over_the_socket() {
  let (dir, _repo) = init_repo();
  let sock_dir = TempDir::new().unwrap();
  let daemon = TestDaemon::start(dir.path(), sock_dir.path(), Duration::from_millis(50));

  let mut client = Client::new(daemon.connect());
  let v = client.request(r#"{"jsonrpc":"2.0","method":"list","id":1}"#);

  assert_eq!(v["id"], serde_json::json!(1));
  let arr = v["result"].as_array().expect("result must be an array");
  assert_eq!(arr.len(), 1, "fresh repo has only the main worktree");
  assert_eq!(arr[0]["is_main"], serde_json::json!(true));
}

#[test]
fn unknown_method_over_socket_keeps_connection_alive() {
  let (dir, _repo) = init_repo();
  let sock_dir = TempDir::new().unwrap();
  let daemon = TestDaemon::start(dir.path(), sock_dir.path(), Duration::from_millis(50));

  let mut client = Client::new(daemon.connect());
  let err = client.request(r#"{"method":"nope","id":1}"#);
  assert!(err.get("error").is_some());

  // A second request on the same connection still works — one bad call
  // doesn't kill the session.
  let ok = client.request(r#"{"method":"list","id":2}"#);
  assert_eq!(ok["id"], serde_json::json!(2));
  assert!(ok["result"].is_array());
}

#[test]
fn subscribe_streams_snapshot_then_pushes_on_worktree_change() {
  let (dir, _repo) = init_repo();
  let sock_dir = TempDir::new().unwrap();
  // Fast poll so the change is detected promptly without a long test.
  let daemon = TestDaemon::start(dir.path(), sock_dir.path(), Duration::from_millis(30));

  let stream = daemon.connect();
  let mut writer = stream.try_clone().unwrap();
  let mut reader = BufReader::new(stream);

  writeln!(writer, r#"{{"method":"subscribe","id":1}}"#).unwrap();
  writer.flush().unwrap();

  // First line: the initial snapshot notification.
  let mut snapshot = String::new();
  reader
    .read_line(&mut snapshot)
    .expect("must receive the initial snapshot");
  let snap: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
  assert_eq!(snap["method"], serde_json::json!("worktrees.changed"));
  let n0 = snap["params"]["worktrees"].as_array().unwrap().len();
  assert_eq!(n0, 1, "snapshot starts with just the main worktree");

  // Mutate the worktree set out-of-band; the daemon's poll loop should
  // notice and push a fresh notification.
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-38-pushed");
  worktree::add(&repo, "feat-38-pushed", &target, "feat/#38-pushed", false).unwrap();

  // Next line: the change notification (read timeout guards against a hang).
  let mut changed = String::new();
  reader.read_line(&mut changed).expect("must receive a change push");
  let chg: serde_json::Value = serde_json::from_str(&changed).unwrap();
  assert_eq!(chg["method"], serde_json::json!("worktrees.changed"));
  let n1 = chg["params"]["worktrees"].as_array().unwrap().len();
  assert_eq!(n1, 2, "push reflects the newly created worktree");
  assert!(
    chg["params"]["worktrees"]
      .as_array()
      .unwrap()
      .iter()
      .any(|w| w["name"] == serde_json::json!("feat-38-pushed")),
    "the pushed list names the new worktree"
  );
}
