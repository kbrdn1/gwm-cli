//! Daemon mode: a long-running JSON-RPC 2.0 server exposing gwm's
//! machine-readable surface over a unix domain socket (issue #38,
//! phase 2). Editors, statusbars, and tooling connect once and call
//! `list` / `doctor` / `path`, or `subscribe` for pushed updates when
//! the worktree set changes — instead of spawning `gwm` per query and
//! parsing human output.
//!
//! Layering, deliberately split so the testable core needs no socket and
//! compiles on every platform:
//!
//! - **Pure RPC core** (`parse` → [`dispatch`] → serialize, tied together
//!   by [`handle_line`]): always compiled, unit-tested in
//!   `tests/daemon_tests.rs`. Does git I/O against a repo workdir but
//!   never touches a socket.
//! - **Socket server** ([`serve`] / [`socket_path`]): `cfg(all(unix,
//!   feature = "daemon"))`. A thin accept loop that shuttles
//!   newline-delimited JSON between the socket and [`handle_line`].
//!
//! Wire format: newline-delimited JSON (NDJSON). One request object per
//! line, one response object per line. `subscribe` is the exception — it
//! turns the connection into a one-way stream of `worktrees.changed`
//! notifications (the first is the current snapshot).
//!
//! The reference scheme in issue #38 calls for a filesystem watch on
//! `.git/worktrees/`; this MVP uses interval polling instead (no `notify`
//! dependency, deterministic to test, MSRV-safe). The trade-off — update
//! latency bounded by the poll interval — is documented on the `daemon`
//! subcommand's `--poll-ms` flag.

use crate::error::Result;
use crate::json_api::{self, JsonDoctorReport, JsonPath, JsonWorktree};
use crate::{config::Config, doctor, worktree};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

/// JSON-RPC 2.0 standard error codes (subset we emit).
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// A parsed JSON-RPC 2.0 request. `params` and `id` default so a minimal
/// `{"method":"list"}` line still parses. When the `id` member is absent
/// the request is a **notification** (no response is sent — see
/// [`handle_line`]); an explicit `"id": null` is a request and is echoed
/// back as `null`. The two are distinguished at parse time in
/// [`handle_line`], not here.
#[derive(Debug, Clone, Deserialize)]
pub struct RpcRequest {
  #[serde(default)]
  pub jsonrpc: String,
  pub method: String,
  #[serde(default)]
  pub params: Value,
  #[serde(default)]
  pub id: Value,
}

/// Build a JSON-RPC success envelope echoing the request `id`.
pub fn success(id: &Value, result: Value) -> Value {
  json!({ "jsonrpc": "2.0", "result": result, "id": id })
}

/// Build a JSON-RPC error envelope echoing the request `id`.
pub fn error(id: &Value, code: i64, message: &str) -> Value {
  json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": id })
}

/// Open the repo that owns `workdir`. A daemon is pinned to one repo
/// (the one it was launched in); `discover_repo` walks back to the main
/// workdir if `workdir` is itself a linked worktree.
fn open_repo(workdir: &Path) -> Result<git2::Repository> {
  worktree::discover_repo(Some(workdir))
}

fn run_list(workdir: &Path) -> Result<Vec<JsonWorktree>> {
  let repo = open_repo(workdir)?;
  json_api::worktrees(&repo)
}

fn run_path(workdir: &Path, pattern: &str) -> Result<JsonPath> {
  let repo = open_repo(workdir)?;
  let found = worktree::find_fuzzy(&repo, pattern)?;
  Ok(JsonPath::from(&found))
}

fn run_doctor(workdir: &Path) -> Result<JsonDoctorReport> {
  // Mirror `cli::repo_context_lenient` + `cmd_doctor`: lenient config
  // load and the real global layer, so the daemon's doctor matches
  // `gwm doctor --format json` byte-for-byte.
  let repo = open_repo(workdir)?;
  let repo_workdir = repo
    .workdir()
    .ok_or(crate::error::GwmError::NotInGitRepo)?
    .to_path_buf();
  let config = Config::load_for_repo(&repo_workdir).unwrap_or_default();
  let global = crate::config::global_config_path();
  let ctx = doctor::DoctorCtx {
    repo_workdir: &repo_workdir,
    repo: &repo,
    config: &config,
    global_config_path: global.as_deref(),
  };
  Ok(JsonDoctorReport::from(&doctor::run(&ctx)?))
}

/// Route one parsed request to its handler and build the response
/// envelope. Pure of any socket concern; git I/O only. The single place
/// that knows the method set, shared verbatim by the CLI-equivalent
/// surface so `list`/`doctor`/`path` over RPC match the `--format=json`
/// flags.
pub fn dispatch(workdir: &Path, req: &RpcRequest) -> Value {
  let id = &req.id;
  match req.method.as_str() {
    "list" => match run_list(workdir) {
      Ok(list) => match serde_json::to_value(list) {
        Ok(v) => success(id, v),
        Err(e) => error(id, INTERNAL_ERROR, &e.to_string()),
      },
      Err(e) => error(id, INTERNAL_ERROR, &e.to_string()),
    },
    "doctor" => match run_doctor(workdir) {
      Ok(report) => match serde_json::to_value(report) {
        Ok(v) => success(id, v),
        Err(e) => error(id, INTERNAL_ERROR, &e.to_string()),
      },
      Err(e) => error(id, INTERNAL_ERROR, &e.to_string()),
    },
    "path" => match req.params.get("pattern").and_then(|v| v.as_str()) {
      None => error(id, INVALID_PARAMS, "method 'path' requires a string 'pattern' param"),
      Some(pattern) => match run_path(workdir, pattern) {
        Ok(p) => match serde_json::to_value(p) {
          Ok(v) => success(id, v),
          Err(e) => error(id, INTERNAL_ERROR, &e.to_string()),
        },
        Err(e) => error(id, INTERNAL_ERROR, &e.to_string()),
      },
    },
    // `subscribe` is handled by the connection loop (it streams
    // notifications), not here; reaching dispatch with it means a caller
    // used a request/response transport that can't stream.
    "subscribe" => error(
      id,
      INVALID_PARAMS,
      "method 'subscribe' is only valid over a streaming socket connection",
    ),
    other => error(id, METHOD_NOT_FOUND, &format!("unknown method '{other}'")),
  }
}

/// Parse one NDJSON request line and return the serialized response line,
/// or `None` when no response must be sent.
///
/// JSON-RPC 2.0 notification handling: a request object with **no `id`
/// member** is a notification — it is processed but MUST NOT be answered
/// (returns `None`). An explicit `"id": null` is a normal request and is
/// answered with `"id": null`. The absent-vs-null distinction is made on
/// the raw value here (serde would collapse both to `Value::Null`).
///
/// A malformed line yields a JSON-RPC parse error (`null` id) rather than
/// crashing the connection; a well-formed object that isn't a valid
/// request yields an invalid-request error.
pub fn handle_line(workdir: &Path, line: &str) -> Option<String> {
  let value: Value = match serde_json::from_str(line) {
    Ok(v) => v,
    Err(e) => return Some(error(&Value::Null, PARSE_ERROR, &format!("parse error: {e}")).to_string()),
  };
  let req: RpcRequest = match serde_json::from_value(value.clone()) {
    Ok(r) => r,
    Err(e) => return Some(error(&Value::Null, INVALID_REQUEST, &format!("invalid request: {e}")).to_string()),
  };
  // Absent `id` ⇒ notification: process for side effects (none for our
  // read-only methods) but send nothing back. The `?` short-circuits to
  // `None` (no response) when the `id` member is missing.
  value.get("id")?;
  Some(dispatch(workdir, &req).to_string())
}

/// Build the `worktrees.changed` notification payload (no `id` — it's a
/// JSON-RPC notification, not a response). Used for the initial
/// `subscribe` snapshot and every subsequent change.
///
/// `params.schema_version` carries [`crate::contract::SCHEMA_VERSION`] so a
/// long-lived `subscribe` client can detect a contract drift it was not
/// built for (issue #317). It is an additive, ignorable field — older
/// clients that only read `params.worktrees` are unaffected.
pub fn worktrees_changed_notification(worktrees: &[JsonWorktree]) -> Value {
  json!({
    "jsonrpc": "2.0",
    "method": "worktrees.changed",
    "params": {
      "schema_version": crate::contract::SCHEMA_VERSION,
      "worktrees": worktrees,
    },
  })
}

/// True when two worktree snapshots differ in a way a `subscribe` client
/// should be notified about.
///
/// Deliberately **excludes `age_seconds`**: it is recomputed from the
/// current time on every poll, so for any non-trunk branch it ticks up
/// each second. Comparing it (a naive `old != new`) would fire a spurious
/// `worktrees.changed` on every poll, breaking the documented "one per
/// detected change" contract. Every other field is compared.
pub fn worktrees_differ(old: &[JsonWorktree], new: &[JsonWorktree]) -> bool {
  if old.len() != new.len() {
    return true;
  }
  old.iter().zip(new).any(|(a, b)| {
    a.name != b.name
      || a.id != b.id
      || a.path != b.path
      || a.branch != b.branch
      || a.head != b.head
      || a.is_main != b.is_main
      || a.is_locked != b.is_locked
      || a.is_prunable != b.is_prunable
      || a.status != b.status
      || a.issue != b.issue
      || a.pr != b.pr
  })
}

/// Decide what a `subscribe` stream should push next, given the previous
/// snapshot (`None` until the first successful one) and the latest poll
/// result. Returns `Some(snapshot)` to push, or `None` to stay quiet.
///
/// Issue #341: a **transient** `Err` from `run_list` (a flaky git scan, an
/// index lock contended by a concurrent write) is swallowed — we keep the
/// last good snapshot and push nothing. The pre-fix code did
/// `run_list(..).unwrap_or_default()`, turning that `Err` into an **empty**
/// list, which `worktrees_differ` then read as "everything vanished" and
/// pushed a phantom `worktrees.changed` (subscribers flicker empty, then
/// self-heal next poll). A genuine `Ok(empty)` — the last worktree really
/// removed — is still a real change and IS pushed; only the error path is
/// skipped. Pure so it can be unit-tested without a live socket.
pub fn next_subscription_push(
  last: &Option<Vec<JsonWorktree>>,
  latest: Result<Vec<JsonWorktree>>,
) -> Option<Vec<JsonWorktree>> {
  let now = match latest {
    Ok(now) => now,
    Err(_) => return None,
  };
  match last {
    None => Some(now),                                       // first snapshot
    Some(prev) if worktrees_differ(prev, &now) => Some(now), // genuine change
    Some(_) => None,                                         // unchanged
  }
}

// ---------------------------------------------------------------------------
// Client side — request lines + response/notification parsers (issue #309).
// Cross-platform and pure: the statusline consumer and any other client
// reuse these to talk to a running daemon. The socket transport that wraps
// them lives in the `client` submodule below (unix + `daemon` feature).
// ---------------------------------------------------------------------------

/// Canonical `list` request line a client writes to the socket.
pub const LIST_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"list","id":1}"#;

/// Canonical `subscribe` request line a client writes to upgrade the
/// connection into a one-way `worktrees.changed` stream.
pub const SUBSCRIBE_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"subscribe","id":1}"#;

/// Parse a `list` JSON-RPC **response** line into its worktree vec — the
/// client counterpart to [`dispatch`]'s `list` arm. A server-sent `error`
/// envelope is surfaced as a [`GwmError`] rather than silently yielding an
/// empty list.
pub fn parse_list_result(line: &str) -> Result<Vec<JsonWorktree>> {
  let v: Value = serde_json::from_str(line)
    .map_err(|e| crate::error::GwmError::Other(format!("daemon: malformed list response: {e}")))?;
  if let Some(err) = v.get("error") {
    let msg = err.get("message").and_then(Value::as_str).unwrap_or("unknown error");
    return Err(crate::error::GwmError::Other(format!("daemon list error: {msg}")));
  }
  let result = v
    .get("result")
    .ok_or_else(|| crate::error::GwmError::Other("daemon list response missing 'result'".into()))?;
  serde_json::from_value(result.clone())
    .map_err(|e| crate::error::GwmError::Other(format!("daemon: cannot decode worktree list: {e}")))
}

/// Parse a `worktrees.changed` **notification** line into its worktree vec
/// (the `params.worktrees` array). The client counterpart to
/// [`worktrees_changed_notification`]; consumed by a `subscribe` stream.
pub fn parse_worktrees_changed(line: &str) -> Result<Vec<JsonWorktree>> {
  let v: Value = serde_json::from_str(line)
    .map_err(|e| crate::error::GwmError::Other(format!("daemon: malformed notification: {e}")))?;
  let arr = v
    .get("params")
    .and_then(|p| p.get("worktrees"))
    .ok_or_else(|| crate::error::GwmError::Other("daemon notification missing 'params.worktrees'".into()))?;
  serde_json::from_value(arr.clone())
    .map_err(|e| crate::error::GwmError::Other(format!("daemon: cannot decode notification worktrees: {e}")))
}

/// Daemon **client** transport — connect / one-shot `list` / `subscribe`
/// stream. Unix + `daemon` feature, mirroring the server gate: the pure
/// parsers above stay cross-platform, only the socket I/O is gated.
#[cfg(all(unix, feature = "daemon"))]
pub mod client {
  use super::*;
  use crate::error::GwmError;
  use std::io::{BufRead, BufReader, Write};
  use std::os::unix::net::UnixStream;
  use std::time::Duration;

  /// Bounded wait for the daemon's first response. A wedged or foreign process
  /// can accept the connection and then stay silent; without a deadline the
  /// blocking read would hang the caller — e.g. a shell prompt that shells out
  /// to `gwm statusline` would freeze instead of degrading. On timeout the read
  /// errors, which the CLI treats as the documented blank-line degradation.
  /// Generous enough not to false-trip a slow `run_list` git scan on a large
  /// repo. `subscribe` drops it once the first snapshot arrives so a long-lived
  /// `--watch` stream can wait indefinitely between change pushes.
  const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

  fn connect(socket: &Path, timeout: Option<Duration>) -> Result<UnixStream> {
    let stream = UnixStream::connect(socket)
      .map_err(|e| GwmError::Other(format!("daemon: cannot connect to {}: {e}", socket.display())))?;
    stream
      .set_read_timeout(timeout)
      .map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    stream
      .set_write_timeout(timeout)
      .map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    Ok(stream)
  }

  /// One-shot `list`: connect, send the request, read and parse the single
  /// response line. Powers a non-`--watch` statusline render.
  pub fn list_once(socket: &Path) -> Result<Vec<JsonWorktree>> {
    list_once_with_timeout(socket, Some(CLIENT_TIMEOUT))
  }

  /// [`list_once`] with an explicit read/write deadline. Public so tests can
  /// drive the timeout path quickly; production callers use [`list_once`],
  /// which applies [`CLIENT_TIMEOUT`].
  #[doc(hidden)]
  pub fn list_once_with_timeout(socket: &Path, timeout: Option<Duration>) -> Result<Vec<JsonWorktree>> {
    let stream = connect(socket, timeout)?;
    let mut writer = stream
      .try_clone()
      .map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    writeln!(writer, "{LIST_REQUEST}").map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    writer.flush().map_err(|e| GwmError::Other(format!("daemon: {e}")))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
      .read_line(&mut line)
      .map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    parse_list_result(line.trim())
  }

  /// Subscribe to `worktrees.changed`: connect, send `subscribe`, then
  /// invoke `on_snapshot` once per notification — the initial snapshot plus
  /// every detected change. The loop ends when `on_snapshot` returns
  /// `false` or the stream closes. Generic over the callback so a `--watch`
  /// CLI loops forever while a test stops after a fixed number of updates.
  pub fn subscribe(socket: &Path, mut on_snapshot: impl FnMut(&[JsonWorktree]) -> bool) -> Result<()> {
    let stream = connect(socket, Some(CLIENT_TIMEOUT))?;
    let mut writer = stream
      .try_clone()
      .map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    writeln!(writer, "{SUBSCRIBE_REQUEST}").map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    writer.flush().map_err(|e| GwmError::Other(format!("daemon: {e}")))?;

    let mut reader = BufReader::new(stream);
    let mut delivered_any = false;
    let mut line = String::new();
    loop {
      line.clear();
      match reader.read_line(&mut line) {
        Ok(0) => break, // EOF — peer closed
        Ok(_) => {}
        Err(_) => break, // timeout / dead link — end the stream
      }
      let trimmed = line.trim();
      if trimmed.is_empty() {
        continue;
      }
      let worktrees = parse_worktrees_changed(trimmed)?;
      if !delivered_any {
        // First snapshot arrived: drop the handshake deadline so the
        // long-lived stream can wait indefinitely between change pushes.
        let _ = reader.get_ref().set_read_timeout(None);
      }
      delivered_any = true;
      if !on_snapshot(&worktrees) {
        break;
      }
    }
    // The stream ended without ever yielding a snapshot: the daemon accepted
    // then closed before its first push (crash right after `accept`, or a
    // foreign process on the path). Surface this as an error so the caller's
    // graceful-degradation branch fires — e.g. `statusline --watch` still
    // emits its promised empty line instead of nothing (issue #312).
    if !delivered_any {
      return Err(GwmError::Other(
        "daemon: stream closed before the first snapshot".to_string(),
      ));
    }
    Ok(())
  }
}

// ---------------------------------------------------------------------------
// Socket server — unix only, behind the `daemon` feature.
// ---------------------------------------------------------------------------

#[cfg(all(unix, feature = "daemon"))]
mod server {
  use super::*;
  use crate::error::GwmError;
  use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
  use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
  use std::os::unix::net::{UnixListener, UnixStream};
  use std::path::PathBuf;
  use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
  use std::sync::Arc;
  use std::time::Duration;

  /// RAII counter of live client connections. [`ActiveGuard::try_acquire`]
  /// increments (refusing past the configured cap); `Drop` decrements — so
  /// even a panicking connection thread frees its slot (issue #341).
  struct ActiveGuard(Arc<AtomicUsize>);

  impl ActiveGuard {
    fn try_acquire(active: &Arc<AtomicUsize>, max: usize) -> Option<Self> {
      if active.fetch_add(1, Ordering::SeqCst) + 1 > max {
        active.fetch_sub(1, Ordering::SeqCst);
        return None;
      }
      Some(ActiveGuard(Arc::clone(active)))
    }
  }

  impl Drop for ActiveGuard {
    fn drop(&mut self) {
      self.0.fetch_sub(1, Ordering::SeqCst);
    }
  }

  /// How long the accept loop blocks before re-checking the shutdown
  /// flag. Independent of the worktree poll interval; small enough that a
  /// test's `serve` thread tears down promptly.
  const ACCEPT_TICK: Duration = Duration::from_millis(50);

  /// Configuration for [`serve`]. Construct with [`ServeOptions::new`] for
  /// the production DoS defaults, then override individual guard fields in
  /// tests (tiny caps / timeouts make the limits assertable without flaky
  /// timing — issue #341).
  pub struct ServeOptions {
    /// Path to bind the unix domain socket at.
    pub socket: PathBuf,
    /// The repo this daemon answers for (its main workdir).
    pub repo_workdir: PathBuf,
    /// Interval between worktree-state polls for `subscribe` streams.
    pub poll_interval: Duration,
    /// Max bytes accepted for a single request line before the connection is
    /// dropped. Caps memory a client can force the daemon to buffer by never
    /// sending a newline (DoS guard).
    pub max_line_len: usize,
    /// Idle read timeout on the request/response path: a client that opens a
    /// connection and then stalls (sends nothing, or a partial line) is
    /// dropped after this, freeing its detached thread (slow-loris guard).
    /// `None` disables the timeout.
    pub read_timeout: Option<Duration>,
    /// Max concurrent client connections. Excess connections are accepted
    /// then immediately closed, so a connection flood can't exhaust threads
    /// / file descriptors (DoS guard).
    pub max_connections: usize,
  }

  impl ServeOptions {
    /// 64 KiB is far above any real JSON-RPC request line the daemon serves
    /// (`list` / `path` / `doctor` / `subscribe`), but bounds a malicious
    /// unterminated line.
    pub const DEFAULT_MAX_LINE_LEN: usize = 64 * 1024;
    /// Request/response clients do one short round-trip; 30 s is generous for
    /// a real client yet promptly reaps a stalled one.
    pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
    /// One daemon serves one repo's handful of consumers (TUI, statusline,
    /// the odd `nc`); 128 concurrent connections is comfortably above that.
    pub const DEFAULT_MAX_CONNECTIONS: usize = 128;

    /// Build options with production DoS defaults. Tests override the guard
    /// fields directly afterwards.
    pub fn new(socket: PathBuf, repo_workdir: PathBuf, poll_interval: Duration) -> Self {
      Self {
        socket,
        repo_workdir,
        poll_interval,
        max_line_len: Self::DEFAULT_MAX_LINE_LEN,
        read_timeout: Some(Self::DEFAULT_READ_TIMEOUT),
        max_connections: Self::DEFAULT_MAX_CONNECTIONS,
      }
    }
  }

  /// The current real user id. `getuid(2)` is infallible and has no
  /// preconditions, so the `unsafe` call is sound.
  fn current_uid() -> u32 {
    unsafe { libc::getuid() }
  }

  /// Name of the per-user private dir the `/tmp` fallback nests the socket
  /// in (`gwm-<uid>`). The uid namespaces it so two users on the same host
  /// don't collide on one shared `/tmp` dir.
  fn private_subdir_name() -> String {
    format!("gwm-{}", current_uid())
  }

  /// Resolve the default socket path: `$XDG_RUNTIME_DIR/gwm.sock`, falling
  /// back to `$TMPDIR/gwm.sock`, then `/tmp/gwm-<uid>/gwm.sock`.
  ///
  /// `$XDG_RUNTIME_DIR` (Linux) and `$TMPDIR` (always set, per-user and
  /// `0700`, on macOS) are already private dirs, so the socket sits directly
  /// as `<base>/gwm.sock` — the path the consumer docs advertise. Only the
  /// world-writable `/tmp` last resort needs isolating: there the socket is
  /// nested in a per-user owner-only `gwm-<uid>/` dir (created + verified in
  /// [`serve`]), so it stays un-connectable cross-user even on platforms
  /// that don't enforce socket-file perms for `connect(2)` (issue #341).
  pub fn socket_path() -> PathBuf {
    if let Some(base) = std::env::var_os("XDG_RUNTIME_DIR").filter(|s| !s.is_empty()) {
      return PathBuf::from(base).join("gwm.sock");
    }
    if let Some(base) = std::env::var_os("TMPDIR").filter(|s| !s.is_empty()) {
      return PathBuf::from(base).join("gwm.sock");
    }
    PathBuf::from("/tmp").join(private_subdir_name()).join("gwm.sock")
  }

  /// Ensure `dir` exists as a directory we own with `0700` perms — creating
  /// it if absent, tightening it if we own it but it's too permissive, and
  /// refusing if it's a symlink / not a directory / owned by another user (a
  /// squat on a shared `/tmp`). Only ever called on the gwm-managed
  /// `gwm-<uid>` dir, never on a system base dir or a user's `--socket`
  /// parent (issue #341).
  fn ensure_private_dir(dir: &Path) -> Result<()> {
    let meta = match std::fs::symlink_metadata(dir) {
      Ok(m) => m,
      Err(_) => {
        return std::fs::DirBuilder::new().mode(0o700).create(dir).map_err(|e| {
          GwmError::Other(format!("daemon: failed to create private dir {}: {e}", dir.display()))
        });
      }
    };
    if !meta.file_type().is_dir() {
      return Err(GwmError::Other(format!(
        "daemon: refusing to use {}: exists and is not a directory",
        dir.display()
      )));
    }
    if meta.uid() != current_uid() {
      return Err(GwmError::Other(format!(
        "daemon: refusing to use {}: not owned by the current user",
        dir.display()
      )));
    }
    // We own it — tighten loose perms rather than refuse (idempotent on a
    // dir we created `0700` ourselves on a previous run).
    if meta.mode() & 0o077 != 0 {
      std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| {
        GwmError::Other(format!("daemon: failed to restrict perms on {}: {e}", dir.display()))
      })?;
    }
    Ok(())
  }

  /// If a socket file already exists at `path`, decide whether it's stale.
  /// A successful connect means a live daemon owns it → refuse. A failed
  /// connect means the previous daemon crashed and left the file →
  /// unlink it so `bind` can succeed (a stale socket otherwise fails
  /// `bind` with `EADDRINUSE`).
  fn clear_stale_socket(path: &Path) -> Result<()> {
    // `symlink_metadata` (not `metadata`) so a symlink is seen as a
    // symlink, not followed to its target.
    let meta = match std::fs::symlink_metadata(path) {
      Ok(m) => m,
      Err(_) => return Ok(()), // nothing there — bind will create it
    };
    // Refuse to touch anything that isn't a unix socket. A regular file or
    // symlink at `--socket <path>` would otherwise be deleted as if it were
    // a stale socket — a data-loss footgun (issue #38 review).
    if !meta.file_type().is_socket() {
      return Err(GwmError::Other(format!(
        "daemon: refusing to use {}: exists and is not a unix socket",
        path.display()
      )));
    }
    if UnixStream::connect(path).is_ok() {
      return Err(GwmError::Other(format!(
        "daemon: socket {} is already in use by a live daemon",
        path.display()
      )));
    }
    // A socket that no one is listening on — left by a crashed daemon.
    // Unlink it so `bind` can succeed (it otherwise fails `EADDRINUSE`).
    let _ = std::fs::remove_file(path);
    Ok(())
  }

  /// Bind the socket and serve connections until `shutdown` flips. Each
  /// connection is handled on its own detached thread. In production the
  /// flag never flips (the process runs until killed); tests pass a flag
  /// they flip on teardown.
  pub fn serve(opts: &ServeOptions, shutdown: Arc<AtomicBool>) -> Result<()> {
    // If the socket lives in a gwm-managed private dir (the `/tmp` fallback's
    // `gwm-<uid>/`), create + verify it `0700` before binding. `chmod 0600`
    // on the socket alone doesn't block cross-user connect on platforms that
    // don't enforce socket-file perms (macOS/BSD); an owner-only parent dir
    // does, since directory-traversal perms are enforced everywhere. We never
    // touch a system base dir or a user-supplied `--socket` parent — only the
    // dir whose name we own (issue #341).
    let private_name = private_subdir_name();
    if let Some(parent) = opts.socket.parent() {
      if parent.file_name().and_then(|n| n.to_str()) == Some(private_name.as_str()) {
        ensure_private_dir(parent)?;
      }
    }
    clear_stale_socket(&opts.socket)?;
    let listener = UnixListener::bind(&opts.socket)
      .map_err(|e| GwmError::Other(format!("daemon: failed to bind {}: {e}", opts.socket.display())))?;
    // Restrict the socket to the owner (`0600`). A unix socket is created
    // `0777 & ~umask`; the usual `022` umask leaves it group/other-
    // connectable — and on Linux socket perms ARE enforced for connect, so
    // on a shared host's `/tmp` fallback another local user could read the
    // worktree list. `chmod` (not a `umask` twiddle) because `umask` is
    // process-global and not thread-safe — a daemon under test runs many
    // `serve`s in parallel. Fail closed: refuse to serve an over-permissive
    // socket rather than expose it. The brief bind→chmod window is a
    // negligible exposure for a read-only socket (issue #341).
    std::fs::set_permissions(&opts.socket, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
      let _ = std::fs::remove_file(&opts.socket);
      GwmError::Other(format!(
        "daemon: failed to restrict permissions on {}: {e}",
        opts.socket.display()
      ))
    })?;
    listener
      .set_nonblocking(true)
      .map_err(|e| GwmError::Other(format!("daemon: set_nonblocking failed: {e}")))?;

    // Live-connection counter shared with each connection's `ActiveGuard`,
    // so a connection flood can't exhaust threads / file descriptors. Kept
    // per-`serve` (not a global static) so parallel tests don't interfere.
    let active = Arc::new(AtomicUsize::new(0));

    // Announce readiness ONLY now that the socket is bound, so the line
    // can't precede a bind failure and mislead a wrapper that treats it as
    // a readiness signal (issue #38 review). stderr keeps stdout clean.
    eprintln!("gwm daemon listening on {}", opts.socket.display());

    loop {
      if shutdown.load(Ordering::Relaxed) {
        break;
      }
      match listener.accept() {
        Ok((stream, _addr)) => {
          // The listener is non-blocking so the accept loop can poll the
          // shutdown flag. On macOS the accepted stream INHERITS that
          // non-blocking flag (unlike Linux, where accept() clears it),
          // which would make the per-connection blocking read loop spin
          // out on the first `WouldBlock`. Force the connection back to
          // blocking so reads wait for the next request.
          if let Err(e) = stream.set_nonblocking(false) {
            eprintln!("daemon: failed to set connection blocking: {e}");
            continue;
          }
          // Refuse past the concurrency cap: accept then immediately drop
          // the stream (closing it) so a flood can't pile up threads.
          let Some(guard) = ActiveGuard::try_acquire(&active, opts.max_connections) else {
            continue;
          };
          let workdir = opts.repo_workdir.clone();
          let poll = opts.poll_interval;
          let max_line_len = opts.max_line_len;
          let read_timeout = opts.read_timeout;
          let shutdown = Arc::clone(&shutdown);
          // Detached: a long-running daemon must not accumulate JoinHandles
          // for every short-lived client (`nc`, reconnecting integrations).
          // Each connection thread observes the shared `shutdown` flag and
          // exits on its own (issue #38 review). `guard` rides along and
          // frees the connection slot when the thread ends.
          std::thread::spawn(move || {
            let _guard = guard;
            handle_connection(stream, &workdir, poll, max_line_len, read_timeout, &shutdown);
          });
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
          std::thread::sleep(ACCEPT_TICK);
        }
        Err(e) => {
          // Transient accept error — log and keep serving rather than
          // tearing the daemon down.
          eprintln!("daemon: accept error: {e}");
          std::thread::sleep(ACCEPT_TICK);
        }
      }
    }

    // Best-effort cleanup so the next launch sees no stale socket.
    let _ = std::fs::remove_file(&opts.socket);
    Ok(())
  }

  /// Serve one connection: a loop of request→response lines, until the
  /// client disconnects — or, on a `subscribe`, a switch into a one-way
  /// notification stream.
  ///
  /// Two DoS guards apply on the request/response path (issue #341):
  /// `read_timeout` reaps a client that opens the connection then stalls;
  /// `max_line_len` caps how much an unterminated line can buffer.
  fn handle_connection(
    stream: UnixStream,
    workdir: &Path,
    poll: Duration,
    max_line_len: usize,
    read_timeout: Option<Duration>,
    shutdown: &AtomicBool,
  ) {
    // A stalled or slow-loris client must not pin this detached thread.
    let _ = stream.set_read_timeout(read_timeout);
    let read_half = match stream.try_clone() {
      Ok(s) => s,
      Err(_) => return,
    };
    let mut writer = stream;
    let mut reader = BufReader::new(read_half);

    loop {
      // Bound each line: read at most `max_line_len + 1` bytes looking for a
      // newline. `take` is recreated per line so the cap is per-line, not
      // per-connection. An unterminated line that hits the cap (no trailing
      // `\n`) is a misbehaving / hostile client — drop the connection.
      let mut buf = Vec::new();
      match (&mut reader).take(max_line_len as u64 + 1).read_until(b'\n', &mut buf) {
        Ok(0) => break, // EOF: client closed
        Ok(_) => {}
        Err(_) => break, // idle read timeout, connection reset, or dead link
      }
      if buf.len() > max_line_len && buf.last() != Some(&b'\n') {
        break; // oversized line — refuse to keep buffering
      }
      let line = match std::str::from_utf8(&buf) {
        Ok(s) => s.trim(),
        Err(_) => break, // not a UTF-8 JSON-RPC client
      };
      if line.is_empty() {
        continue;
      }

      // Peek the method: `subscribe` upgrades the connection to a stream
      // and never returns to request/response mode.
      let is_subscribe = serde_json::from_str::<RpcRequest>(line)
        .map(|r| r.method == "subscribe")
        .unwrap_or(false);
      if is_subscribe {
        stream_subscription(&mut writer, workdir, poll, shutdown);
        return;
      }

      // A notification (no `id`) returns None — process, send nothing.
      if let Some(response) = handle_line(workdir, line) {
        if writeln!(writer, "{response}").is_err() || writer.flush().is_err() {
          break;
        }
      }
    }
  }

  /// Push `worktrees.changed` notifications: an immediate snapshot, then
  /// one per detected change. Change detection uses [`worktrees_differ`],
  /// which ignores the always-ticking `age_seconds` so a non-trunk branch
  /// doesn't spam a notification every poll.
  ///
  /// The read timeout doubles as the poll cadence AND the disconnect
  /// detector: a closed peer makes `read` return `Ok(0)` promptly. Without
  /// it, the loop only ever *writes* (on change), so a subscriber that
  /// disconnects during a no-change period would never be observed and the
  /// detached thread would keep scanning git forever (issue #38 review).
  fn stream_subscription(stream: &mut UnixStream, workdir: &Path, poll: Duration, shutdown: &AtomicBool) {
    if stream.set_read_timeout(Some(poll)).is_err() {
      return;
    }
    // `None` until the first SUCCESSFUL snapshot — so a transient git error
    // on the very first poll defers the immediate snapshot to the next tick
    // instead of pushing a phantom-empty one (issue #341).
    let mut last: Option<Vec<JsonWorktree>> = None;
    if let Some(snapshot) = next_subscription_push(&last, run_list(workdir)) {
      if send_notification(stream, &snapshot).is_err() {
        return;
      }
      last = Some(snapshot);
    }
    let mut buf = [0u8; 64];
    loop {
      if shutdown.load(Ordering::Relaxed) {
        return;
      }
      // Blocks up to `poll` waiting for client input — this read IS the
      // poll wait. A timeout (`WouldBlock`/`TimedOut`) is the normal idle
      // tick; `Ok(0)` is the peer closing; other errors are a dead link.
      match stream.read(&mut buf) {
        Ok(0) => return,
        Ok(_) => {} // unexpected client chatter on a push stream — ignore
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
        Err(_) => return,
      }
      if let Some(snapshot) = next_subscription_push(&last, run_list(workdir)) {
        if send_notification(stream, &snapshot).is_err() {
          return;
        }
        last = Some(snapshot);
      }
    }
  }

  fn send_notification(writer: &mut UnixStream, worktrees: &[JsonWorktree]) -> std::io::Result<()> {
    let note = worktrees_changed_notification(worktrees);
    writeln!(writer, "{note}")?;
    writer.flush()
  }
}

#[cfg(all(unix, feature = "daemon"))]
pub use server::{serve, socket_path, ServeOptions};
