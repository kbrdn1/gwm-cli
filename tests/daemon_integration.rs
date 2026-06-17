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
use std::os::unix::net::{UnixListener, UnixStream};
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
fn serve_refuses_to_unlink_a_non_socket_path() {
  // A regular file at --socket must NOT be deleted as if it were a stale
  // socket (data-loss footgun). serve returns Err before binding and
  // leaves the file untouched.
  let (dir, _repo) = init_repo();
  let sock_dir = TempDir::new().unwrap();
  let path = sock_dir.path().join("s");
  std::fs::write(&path, b"precious user data").unwrap();

  let opts = ServeOptions {
    socket: path.clone(),
    repo_workdir: dir.path().to_path_buf(),
    poll_interval: Duration::from_millis(50),
  };
  let err = serve(&opts, Arc::new(AtomicBool::new(false))).unwrap_err();
  assert!(
    err.to_string().contains("not a unix socket"),
    "must refuse a non-socket path, got: {err}"
  );
  assert!(path.exists(), "the regular file must be left intact");
  assert_eq!(std::fs::read(&path).unwrap(), b"precious user data");
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

#[test]
fn subscribe_reaps_an_idle_client_that_disconnects() {
  // A subscriber that closes the connection during a no-change period must
  // be detected and reaped — otherwise the detached thread polls git
  // forever (issue #38 review). We half-close the client write side
  // (signalling disconnect WITHOUT any worktree change); the server reads
  // EOF, returns, and drops its socket, so our subsequent read sees EOF
  // too. With the bug the server would never notice (no change ever
  // happens) and this read would block until the connect() timeout.
  let (dir, _repo) = init_repo();
  let sock_dir = TempDir::new().unwrap();
  let daemon = TestDaemon::start(dir.path(), sock_dir.path(), Duration::from_millis(30));

  let stream = daemon.connect();
  let mut writer = stream.try_clone().unwrap();
  let mut reader = BufReader::new(stream);

  writeln!(writer, r#"{{"method":"subscribe","id":1}}"#).unwrap();
  writer.flush().unwrap();

  let mut snapshot = String::new();
  reader
    .read_line(&mut snapshot)
    .expect("must receive the initial snapshot");
  assert!(snapshot.contains("worktrees.changed"));

  // Disconnect without triggering any worktree change.
  writer.shutdown(std::net::Shutdown::Write).unwrap();

  // The server must reap the subscription and close its end promptly. EOF
  // (0 bytes) proves it; a timeout error means the leak regressed.
  let mut tail = String::new();
  let n = reader
    .read_line(&mut tail)
    .expect("server must close the idle subscription (not time out)");
  assert_eq!(n, 0, "idle subscriber must be reaped on disconnect");
}

// --- Statusline consumer client (issue #309) -------------------------------
// The first real consumer: `gwm::daemon::client` connects, runs `list`
// once, or rides the `subscribe` stream. These drive it against a live
// daemon thread — the end-to-end proof that the wire protocol is usable.

#[test]
fn client_list_once_returns_the_current_worktrees() {
  let (dir, _repo) = init_repo();
  let sock_dir = TempDir::new().unwrap();
  let daemon = TestDaemon::start(dir.path(), sock_dir.path(), Duration::from_millis(50));
  let _probe = daemon.connect(); // blocks until the serve thread has bound

  let wts = gwm::daemon::client::list_once(&daemon.socket).expect("list_once must succeed");
  assert_eq!(wts.len(), 1, "fresh repo has exactly the main worktree");
  assert!(wts[0].is_main, "the sole worktree is the main one");
}

#[test]
fn client_list_once_errors_when_no_daemon_is_listening() {
  // No daemon bound here: the connect must fail so the CLI can fall back to
  // its graceful empty line (asserted at the CLI level).
  let sock_dir = TempDir::new().unwrap();
  let missing = sock_dir.path().join("nope");
  assert!(
    gwm::daemon::client::list_once(&missing).is_err(),
    "a missing socket must surface a connect error"
  );
}

#[test]
fn client_subscribe_streams_snapshot_then_pushes_on_change() {
  let (dir, _repo) = init_repo();
  let sock_dir = TempDir::new().unwrap();
  // Fast poll so the created worktree is noticed promptly.
  let daemon = TestDaemon::start(dir.path(), sock_dir.path(), Duration::from_millis(30));
  let _probe = daemon.connect(); // ensure the socket is bound before we subscribe

  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-309-pushed");
  let repo_path = dir.path().to_path_buf();

  let mut counts: Vec<usize> = Vec::new();
  let mut created = false;
  gwm::daemon::client::subscribe(&daemon.socket, |worktrees| {
    counts.push(worktrees.len());
    if !created {
      // First callback is the initial snapshot. Mutate the worktree set
      // out-of-band; the daemon's poll loop must push a fresh notification.
      created = true;
      let repo = worktree::discover_repo(Some(&repo_path)).unwrap();
      worktree::add(&repo, "feat-309-pushed", &target, "feat/#309-pushed", false).unwrap();
      true // keep listening for the change push
    } else {
      // Second callback is the change push: it must name the new worktree.
      assert!(
        worktrees.iter().any(|w| w.name == "feat-309-pushed"),
        "the change push must include the newly created worktree"
      );
      false // stop the stream
    }
  })
  .expect("subscribe must run cleanly");

  assert_eq!(counts, vec![1, 2], "initial snapshot (1) then the change push (2)");
}

#[test]
fn client_subscribe_errors_on_eof_before_first_snapshot() {
  // Contract (issue #312): if the daemon accepts the subscribe connection but
  // closes before pushing the first snapshot (it crashes right after accept,
  // or a foreign process owns the path), `subscribe` must NOT report a clean
  // `Ok(())`. cmd_statusline --watch emits its graceful empty line from the
  // error branch; a false Ok would skip it and break the documented contract.
  // The stream delivered nothing, so it must surface an error.
  let sock_dir = TempDir::new().unwrap();
  let socket = sock_dir.path().join("s");
  let listener = UnixListener::bind(&socket).unwrap();

  // Server: accept one client, consume its subscribe request line (so the
  // client's write succeeds and we exercise the read-side EOF, not a broken
  // pipe), then drop the connection without ever pushing a snapshot.
  let server = thread::spawn(move || {
    if let Ok((stream, _)) = listener.accept() {
      let mut reader = BufReader::new(stream);
      let mut line = String::new();
      let _ = reader.read_line(&mut line);
      // reader dropped here -> connection closed -> client sees EOF
    }
  });

  let mut calls = 0usize;
  let result = gwm::daemon::client::subscribe(&socket, |_worktrees| {
    calls += 1;
    true
  });

  server.join().unwrap();
  assert_eq!(calls, 0, "no snapshot was sent, so the callback must never fire");
  assert!(
    result.is_err(),
    "EOF before the first snapshot must surface as an error, not Ok(())"
  );
}
