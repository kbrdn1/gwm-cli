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
      // Agents compared whole, `last_activity` included (Codex review round
      // D): unlike `age_seconds` — recomputed from the clock every poll —
      // `last_activity` only moves when the agent actually wrote an
      // artefact, so it IS a real change a subscriber wants (a statusline's
      // "Ns ago" would otherwise go stale while the agent works). Push
      // frequency is bounded by the 30 s detection cache, not the poll rate.
      || a.agents != b.agents
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
  use std::time::{Duration, Instant};

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
    /// Whether [`serve`] owns the socket's parent directory and must create
    /// it and secure it to `0700`. Set ONLY for the default resolution's
    /// private `gwm-<uid>/` fallback nest (see [`default_socket`]); never for
    /// a user-supplied `--socket`, whose parent is left untouched even when
    /// its name happens to match `gwm-<uid>` (issue #341 review).
    pub manage_socket_dir: bool,
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
        // Conservative: only the default `/tmp`-fallback resolution opts in.
        manage_socket_dir: false,
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

  /// True when `dir` is a real directory we own with no group/other access
  /// (`0700`-style). Used to decide whether a base dir is safe to drop the
  /// socket into directly, or whether it needs a private `gwm-<uid>/` nest.
  /// `symlink_metadata` so a symlinked base isn't trusted on its target.
  fn is_private_dir(dir: &Path) -> bool {
    match std::fs::symlink_metadata(dir) {
      Ok(m) => m.file_type().is_dir() && m.uid() == current_uid() && m.mode() & 0o077 == 0,
      Err(_) => false,
    }
  }

  /// Place the socket directly in `base` when `base` is genuinely owner-only
  /// (`$XDG_RUNTIME_DIR` per the XDG spec, macOS's per-user `$TMPDIR`) — the
  /// `<base>/gwm.sock` path the consumer docs advertise. Otherwise (a base
  /// that resolves to a shared dir like `/tmp`) nest the socket in a per-user
  /// owner-only `gwm-<uid>/` sub-dir so it stays un-connectable cross-user.
  pub fn socket_in(base: &Path) -> PathBuf {
    if is_private_dir(base) {
      base.join("gwm.sock")
    } else {
      base.join(private_subdir_name()).join("gwm.sock")
    }
  }

  /// Resolve the default socket path: `$XDG_RUNTIME_DIR` → `$TMPDIR` → `/tmp`
  /// for the base dir, then [`socket_in`] to decide direct vs. private-nested
  /// placement based on the base's actual ownership/perms (issue #341). The
  /// nested `gwm-<uid>/` dir is created + verified in [`serve`]. Pure modulo
  /// reading the env and stat-ing the base — server and client agree on the
  /// result since the base's perms are stable across their runs.
  pub fn socket_path() -> PathBuf {
    if let Some(base) = std::env::var_os("XDG_RUNTIME_DIR").filter(|s| !s.is_empty()) {
      return socket_in(&PathBuf::from(base));
    }
    if let Some(base) = std::env::var_os("TMPDIR").filter(|s| !s.is_empty()) {
      return socket_in(&PathBuf::from(base));
    }
    socket_in(Path::new("/tmp"))
  }

  /// The default [`socket_path`] plus whether [`serve`] should create +
  /// secure its parent dir. The flag is `true` only when resolution nested
  /// the socket in a private `gwm-<uid>/` fallback dir (a shared base);
  /// `false` for the direct `$XDG_RUNTIME_DIR` / `$TMPDIR` paths. The CLI
  /// passes a user `--socket` with the flag `false`, so a user-supplied
  /// parent is never modified — even one coincidentally named `gwm-<uid>`
  /// (issue #341 review).
  pub fn default_socket() -> (PathBuf, bool) {
    let path = socket_path();
    let managed =
      path.parent().and_then(|d| d.file_name()).and_then(|n| n.to_str()) == Some(private_subdir_name().as_str());
    (path, managed)
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
        return std::fs::DirBuilder::new()
          .mode(0o700)
          .create(dir)
          .map_err(|e| GwmError::Other(format!("daemon: failed to create private dir {}: {e}", dir.display())));
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
      std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| GwmError::Other(format!("daemon: failed to restrict perms on {}: {e}", dir.display())))?;
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
    // When we own the socket's parent dir (the `/tmp` fallback's private
    // `gwm-<uid>/`, flagged by `manage_socket_dir`), create + verify it
    // `0700` before binding. `chmod 0600` on the socket alone doesn't block
    // cross-user connect on platforms that don't enforce socket-file perms
    // (macOS/BSD); an owner-only parent dir does, since directory-traversal
    // perms are enforced everywhere. We never touch a system base dir or a
    // user-supplied `--socket` parent — the flag, not a name match, gates
    // this so a `--socket` path that happens to sit in a `gwm-<uid>` dir is
    // left alone (issue #341).
    if opts.manage_socket_dir {
      if let Some(parent) = opts.socket.parent() {
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

  /// Read one newline-terminated request line, bounded two ways (issue
  /// #341): `max_len` caps the bytes buffered (a never-terminated line is
  /// dropped), and `deadline` caps the WALL time spent on the whole line.
  /// The deadline is the real slow-loris guard — `SO_RCVTIMEO` alone resets
  /// on every successful read, so a client dribbling one byte just under the
  /// timeout could hold its connection slot until the length cap; shrinking
  /// the socket timeout toward a fixed per-line deadline closes that.
  ///
  /// Returns the line bytes (without the trailing `\n`), or `None` on EOF /
  /// timeout / oversize / error — the caller then drops the connection.
  fn read_request_line(
    stream: &UnixStream,
    reader: &mut BufReader<UnixStream>,
    max_len: usize,
    deadline: Option<Instant>,
  ) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
      if let Some(dl) = deadline {
        match dl.checked_duration_since(Instant::now()) {
          // Cap the next read at the line's remaining time budget.
          Some(rem) if !rem.is_zero() => {
            let _ = stream.set_read_timeout(Some(rem));
          }
          _ => return None, // per-line deadline exceeded — slow-loris
        }
      }
      let chunk = match reader.fill_buf() {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
        Err(_) => return None, // timeout / reset / dead link
      };
      if chunk.is_empty() {
        return None; // EOF (a partial line here is an incomplete request)
      }
      if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
        if buf.len() + pos > max_len {
          return None; // oversize before the newline
        }
        buf.extend_from_slice(&chunk[..pos]);
        reader.consume(pos + 1);
        return Some(buf);
      }
      if buf.len() + chunk.len() > max_len {
        return None; // unterminated line past the cap
      }
      let n = chunk.len();
      buf.extend_from_slice(chunk);
      reader.consume(n);
    }
  }

  /// Serve one connection: a loop of request→response lines, until the
  /// client disconnects — or, on a `subscribe`, a switch into a one-way
  /// notification stream.
  ///
  /// Three DoS guards apply on the request/response path (issue #341):
  /// `read_timeout` reaps a client that opens the connection then stalls and
  /// bounds the wall time per request line (slow-loris); `max_line_len` caps
  /// how much an unterminated line can buffer.
  fn handle_connection(
    stream: UnixStream,
    workdir: &Path,
    poll: Duration,
    max_line_len: usize,
    read_timeout: Option<Duration>,
    shutdown: &AtomicBool,
  ) {
    // Baseline blocking/timeout; `read_request_line` shrinks it per read when
    // a deadline is set. With `read_timeout = None` the read simply blocks.
    let _ = stream.set_read_timeout(read_timeout);
    let read_half = match stream.try_clone() {
      Ok(s) => s,
      Err(_) => return,
    };
    let mut writer = stream;
    let mut reader = BufReader::new(read_half);

    loop {
      // Fresh per-line deadline so each request gets the full budget, but no
      // single line (and no dribbling client) can outlast it.
      let deadline = read_timeout.map(|t| Instant::now() + t);
      let Some(bytes) = read_request_line(&writer, &mut reader, max_line_len, deadline) else {
        break; // EOF, timeout, oversize, or dead link — drop the connection
      };
      let line = match std::str::from_utf8(&bytes) {
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
pub use server::{default_socket, serve, socket_in, socket_path, ServeOptions};

// ---------------------------------------------------------------------------
// Named-pipe server & client — Windows only, behind the `daemon` feature
// (issue #439). Exposes the same public interface as the unix module, so
// `cmd_daemon` / `cmd_statusline` compile identically on both platforms.
//
// This is a sibling of `server`, not a shared generic core, on purpose:
// the unix module's #341 hardening is battle-tested and stays byte-
// identical, and the two transports differ exactly where a generic
// abstraction would be the most contorted —
// - `interprocess`'s sync streams have no read/write timeouts, so every
//   guard unix gets from `set_read_timeout` (slow-loris line deadline,
//   subscription poll tick, dead-peer detection) is rebuilt here on
//   NONBLOCKING streams plus bounded sleep-retry loops;
// - the cross-user barrier is the pipe's owner-only security descriptor,
//   the named-pipe analogue of `chmod 0600` + the private socket dir
//   (`\\.\pipe\` has no directories to restrict).
// The small shared bits (`ActiveGuard`, `ACCEPT_TICK`) are deliberately
// duplicated rather than hoisted, to keep the unix module untouched.
// ---------------------------------------------------------------------------

#[cfg(all(windows, feature = "daemon"))]
mod server_win {
  use super::*;
  use crate::error::GwmError;
  use interprocess::local_socket::{
    prelude::*, GenericNamespaced, ListenerNonblockingMode, ListenerOptions, RecvHalf, SendHalf, Stream,
  };
  use interprocess::os::windows::local_socket::ListenerOptionsExt;
  use interprocess::os::windows::security_descriptor::SecurityDescriptor;
  use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
  use std::path::PathBuf;
  use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
  use std::sync::Arc;
  use std::time::{Duration, Instant};

  /// RAII counter of live client connections — duplicated from the unix
  /// module (see the section comment above).
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

  /// How long the accept loop sleeps on `WouldBlock` before re-checking the
  /// shutdown flag — same value and role as the unix module's.
  const ACCEPT_TICK: Duration = Duration::from_millis(50);

  /// Sleep quantum of every nonblocking retry loop (reads, writes, the
  /// subscription tick). Small enough that deadlines land promptly, large
  /// enough that an idle connection costs a negligible wakeup rate.
  const NB_TICK: Duration = Duration::from_millis(15);

  /// Configuration for [`serve`] — same shape as the unix module's so the
  /// CLI builds it identically on both platforms. `socket` holds the PIPE
  /// NAME (`gwm-<user>.sock` → `\\.\pipe\gwm-<user>.sock`), not a
  /// filesystem path, and `manage_socket_dir` is accepted but meaningless
  /// (pipe names have no parent directory to secure).
  pub struct ServeOptions {
    /// Name of the pipe to create under `\\.\pipe\`.
    pub socket: PathBuf,
    /// The repo this daemon answers for (its main workdir).
    pub repo_workdir: PathBuf,
    /// Interval between worktree-state polls for `subscribe` streams.
    pub poll_interval: Duration,
    /// Max bytes accepted for a single request line before the connection
    /// is dropped (memory-bounding DoS guard, as on unix).
    pub max_line_len: usize,
    /// Per-request-line wall-time budget, and the write budget for pushes.
    /// Rebuilt on nonblocking I/O since the transport has no socket-level
    /// timeout. `None` disables the deadline.
    pub read_timeout: Option<Duration>,
    /// Max concurrent client connections (thread-bounding DoS guard).
    pub max_connections: usize,
    /// Interface parity with unix; no-op here (see the struct docs).
    pub manage_socket_dir: bool,
  }

  impl ServeOptions {
    /// Same defaults, same rationale as the unix module.
    pub const DEFAULT_MAX_LINE_LEN: usize = 64 * 1024;
    pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
    pub const DEFAULT_MAX_CONNECTIONS: usize = 128;

    pub fn new(socket: PathBuf, repo_workdir: PathBuf, poll_interval: Duration) -> Self {
      Self {
        socket,
        repo_workdir,
        poll_interval,
        max_line_len: Self::DEFAULT_MAX_LINE_LEN,
        read_timeout: Some(Self::DEFAULT_READ_TIMEOUT),
        max_connections: Self::DEFAULT_MAX_CONNECTIONS,
        manage_socket_dir: false,
      }
    }
  }

  /// Default pipe name. `\\.\pipe\` is machine-global, so the username
  /// namespaces the default and two users' daemons don't fight over one
  /// name — but the [`owner_only_descriptor`] is the actual access
  /// barrier; the name only prevents accidental clashes. Characters
  /// outside `[A-Za-z0-9]` are folded to `-` (a username is not a valid
  /// pipe-name fragment by construction).
  pub fn socket_path() -> PathBuf {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_string());
    let safe: String = user
      .chars()
      .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
      .collect();
    PathBuf::from(format!("gwm-{safe}.sock"))
  }

  /// The default [`socket_path`] plus the manage-dir flag, which is always
  /// `false` here: pipe names are not filesystem paths, there is no parent
  /// directory to create or secure.
  pub fn default_socket() -> (PathBuf, bool) {
    (socket_path(), false)
  }

  /// Owner-only DACL for the pipe: `D:P` (protected, no inheritance) with a
  /// single ACE granting `GENERIC_ALL` to OWNER RIGHTS (`S-1-3-4`) — the
  /// user the daemon runs as. A non-empty protected DACL implicitly denies
  /// every other SID, so a cross-user connect fails outright: the
  /// named-pipe analogue of the unix module's `chmod 0600`. Fail closed:
  /// a descriptor error refuses to serve rather than exposing the pipe
  /// with the default (Everyone-readable) DACL.
  fn owner_only_descriptor() -> Result<SecurityDescriptor> {
    let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)")
      .map_err(|e| GwmError::Other(format!("daemon: cannot encode the pipe SDDL: {e}")))?;
    SecurityDescriptor::deserialize(&sddl)
      .map_err(|e| GwmError::Other(format!("daemon: cannot build the pipe security descriptor: {e}")))
  }

  /// Resolve `opts.socket` into a namespaced local-socket name.
  fn ns_name(socket: &Path) -> Result<interprocess::local_socket::Name<'static>> {
    let name_str = socket.to_string_lossy().into_owned();
    name_str
      .clone()
      .to_ns_name::<GenericNamespaced>()
      .map_err(|e| GwmError::Other(format!("daemon: invalid pipe name {name_str}: {e}")))
  }

  /// Bind the pipe and serve connections until `shutdown` flips — the
  /// Windows counterpart of the unix `serve`, same loop shape.
  pub fn serve(opts: &ServeOptions, shutdown: Arc<AtomicBool>) -> Result<()> {
    // Refuse a name a live daemon already owns, mirroring the unix stale-
    // socket check (pipes need no stale cleanup: they vanish with their
    // process). The probe-then-bind window is the same benign TOCTOU.
    if Stream::connect(ns_name(&opts.socket)?).is_ok() {
      return Err(GwmError::Other(format!(
        "daemon: pipe {} is already in use by a live daemon",
        opts.socket.display()
      )));
    }
    let listener = ListenerOptions::new()
      .name(ns_name(&opts.socket)?)
      .security_descriptor(owner_only_descriptor()?)
      .create_sync()
      .map_err(|e| GwmError::Other(format!("daemon: failed to bind pipe {}: {e}", opts.socket.display())))?;
    // Nonblocking on BOTH sides: `accept` so this loop can poll the
    // shutdown flag (as on unix), and the accepted streams because every
    // per-connection deadline below is a sleep-retry loop over
    // `WouldBlock` — the transport has no `set_read_timeout` to lean on.
    listener
      .set_nonblocking(ListenerNonblockingMode::Both)
      .map_err(|e| GwmError::Other(format!("daemon: set_nonblocking failed: {e}")))?;

    let active = Arc::new(AtomicUsize::new(0));
    eprintln!("gwm daemon listening on \\\\.\\pipe\\{}", opts.socket.display());

    loop {
      if shutdown.load(Ordering::Relaxed) {
        break;
      }
      match listener.accept() {
        Ok(stream) => {
          let Some(guard) = ActiveGuard::try_acquire(&active, opts.max_connections) else {
            continue;
          };
          let workdir = opts.repo_workdir.clone();
          let poll = opts.poll_interval;
          let max_line_len = opts.max_line_len;
          let read_timeout = opts.read_timeout;
          let shutdown = Arc::clone(&shutdown);
          std::thread::spawn(move || {
            let _guard = guard;
            handle_connection(stream, &workdir, poll, max_line_len, read_timeout, &shutdown);
          });
        }
        Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
          std::thread::sleep(ACCEPT_TICK);
        }
        Err(e) => {
          eprintln!("daemon: accept error: {e}");
          std::thread::sleep(ACCEPT_TICK);
        }
      }
    }
    Ok(())
  }

  /// Read one newline-terminated request line from the nonblocking stream:
  /// the unix `read_request_line` with the socket timeout replaced by a
  /// sleep-retry loop bounded by `deadline` (slow-loris guard) and the
  /// shutdown flag. Same return contract: `None` on EOF / deadline /
  /// oversize / error, and the caller drops the connection.
  fn read_request_line(
    reader: &mut BufReader<RecvHalf>,
    max_len: usize,
    deadline: Option<Instant>,
    shutdown: &AtomicBool,
  ) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
      if shutdown.load(Ordering::Relaxed) {
        return None;
      }
      if let Some(dl) = deadline {
        if Instant::now() >= dl {
          return None; // per-line deadline exceeded — slow-loris
        }
      }
      let chunk = match reader.fill_buf() {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
          std::thread::sleep(NB_TICK);
          continue;
        }
        Err(_) => return None, // reset / dead link
      };
      if chunk.is_empty() {
        return None; // EOF (a partial line here is an incomplete request)
      }
      if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
        if buf.len() + pos > max_len {
          return None; // oversize before the newline
        }
        buf.extend_from_slice(&chunk[..pos]);
        reader.consume(pos + 1);
        return Some(buf);
      }
      if buf.len() + chunk.len() > max_len {
        return None; // unterminated line past the cap
      }
      let n = chunk.len();
      buf.extend_from_slice(chunk);
      reader.consume(n);
    }
  }

  /// `write_all` + `flush` over the nonblocking half, bounded by `budget`.
  /// The unix server relies on blocking writes here; on a nonblocking pipe
  /// a full kernel buffer surfaces as `WouldBlock`, so the loop retries —
  /// and the budget reaps a subscriber that never drains its end.
  fn write_all_nb(send: &mut SendHalf, bytes: &[u8], budget: Option<Duration>) -> std::io::Result<()> {
    let deadline = budget.map(|t| Instant::now() + t);
    let expired = |deadline: Option<Instant>| deadline.is_some_and(|dl| Instant::now() >= dl);
    let mut written = 0usize;
    while written < bytes.len() {
      if expired(deadline) {
        return Err(std::io::Error::from(ErrorKind::TimedOut));
      }
      match send.write(&bytes[written..]) {
        Ok(0) => return Err(std::io::Error::from(ErrorKind::WriteZero)),
        Ok(n) => written += n,
        Err(e) if e.kind() == ErrorKind::Interrupted => {}
        Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(NB_TICK),
        Err(e) => return Err(e),
      }
    }
    loop {
      if expired(deadline) {
        return Err(std::io::Error::from(ErrorKind::TimedOut));
      }
      match send.flush() {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == ErrorKind::Interrupted => {}
        Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(NB_TICK),
        Err(e) => return Err(e),
      }
    }
  }

  /// Serve one connection — the unix `handle_connection` on split
  /// nonblocking halves. Same guards, same `subscribe` upgrade.
  fn handle_connection(
    stream: Stream,
    workdir: &Path,
    poll: Duration,
    max_line_len: usize,
    read_timeout: Option<Duration>,
    shutdown: &AtomicBool,
  ) {
    let (recv, mut send) = stream.split();
    let mut reader = BufReader::new(recv);
    loop {
      let deadline = read_timeout.map(|t| Instant::now() + t);
      let Some(bytes) = read_request_line(&mut reader, max_line_len, deadline, shutdown) else {
        return; // EOF, deadline, oversize, or dead link — drop the connection
      };
      let line = match std::str::from_utf8(&bytes) {
        Ok(s) => s.trim(),
        Err(_) => return, // not a UTF-8 JSON-RPC client
      };
      if line.is_empty() {
        continue;
      }
      let is_subscribe = serde_json::from_str::<RpcRequest>(line)
        .map(|r| r.method == "subscribe")
        .unwrap_or(false);
      if is_subscribe {
        stream_subscription(&mut send, &mut reader, workdir, poll, read_timeout, shutdown);
        return;
      }
      if let Some(response) = handle_line(workdir, line) {
        let mut frame = response.into_bytes();
        frame.push(b'\n');
        if write_all_nb(&mut send, &frame, read_timeout).is_err() {
          return;
        }
      }
    }
  }

  /// Push `worktrees.changed` notifications — the unix `stream_subscription`
  /// with the timeout-as-poll-tick replaced by an explicit sliced sleep:
  /// each tick sleeps in `NB_TICK` steps while probing the (nonblocking)
  /// read half, so a closed peer and the shutdown flag are noticed promptly.
  fn stream_subscription(
    send: &mut SendHalf,
    reader: &mut BufReader<RecvHalf>,
    workdir: &Path,
    poll: Duration,
    write_budget: Option<Duration>,
    shutdown: &AtomicBool,
  ) {
    let mut last: Option<Vec<JsonWorktree>> = None;
    if let Some(snapshot) = next_subscription_push(&last, run_list(workdir)) {
      if send_notification(send, &snapshot, write_budget).is_err() {
        return;
      }
      last = Some(snapshot);
    }
    let mut buf = [0u8; 64];
    loop {
      let tick_end = Instant::now() + poll;
      loop {
        if shutdown.load(Ordering::Relaxed) {
          return;
        }
        match reader.read(&mut buf) {
          Ok(0) => return, // peer closed
          Ok(_) => {}      // unexpected client chatter on a push stream — ignore
          Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
          Err(e) if e.kind() == ErrorKind::Interrupted => {}
          Err(_) => return, // dead link
        }
        let now = Instant::now();
        if now >= tick_end {
          break;
        }
        std::thread::sleep(NB_TICK.min(tick_end - now));
      }
      if let Some(snapshot) = next_subscription_push(&last, run_list(workdir)) {
        if send_notification(send, &snapshot, write_budget).is_err() {
          return;
        }
        last = Some(snapshot);
      }
    }
  }

  fn send_notification(
    send: &mut SendHalf,
    worktrees: &[JsonWorktree],
    budget: Option<Duration>,
  ) -> std::io::Result<()> {
    let mut frame = worktrees_changed_notification(worktrees).to_string().into_bytes();
    frame.push(b'\n');
    write_all_nb(send, &frame, budget)
  }
}

#[cfg(all(windows, feature = "daemon"))]
pub use server_win::{default_socket, serve, socket_path, ServeOptions};

/// Daemon **client** transport for Windows — same public surface as the
/// unix `client` module. The sync pipe streams have no read timeout, so
/// every bounded wait runs the blocking read on a helper thread and takes
/// the deadline on the channel instead: a wedged daemon must degrade the
/// statusline to its documented blank line, never freeze the shell prompt.
#[cfg(all(windows, feature = "daemon"))]
pub mod client {
  use super::*;
  use crate::error::GwmError;
  use interprocess::local_socket::{prelude::*, GenericNamespaced, Stream};
  use std::io::{BufRead, BufReader, Write};
  use std::sync::mpsc;
  use std::time::Duration;

  /// Same value and rationale as the unix client's handshake deadline.
  const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

  fn connect(socket: &Path) -> Result<Stream> {
    let name_str = socket.to_string_lossy().into_owned();
    let name = name_str
      .clone()
      .to_ns_name::<GenericNamespaced>()
      .map_err(|e| GwmError::Other(format!("daemon: invalid pipe name {name_str}: {e}")))?;
    Stream::connect(name)
      .map_err(|e| GwmError::Other(format!("daemon: cannot connect to \\\\.\\pipe\\{name_str}: {e}")))
  }

  /// One-shot `list` with the default handshake deadline.
  pub fn list_once(socket: &Path) -> Result<Vec<JsonWorktree>> {
    list_once_with_timeout(socket, Some(CLIENT_TIMEOUT))
  }

  /// [`list_once`] with an explicit deadline (test seam, as on unix). The
  /// round-trip runs on a helper thread; on timeout that thread leaks
  /// until the short-lived CLI process exits — the accepted cost of the
  /// transport's missing read timeout.
  #[doc(hidden)]
  pub fn list_once_with_timeout(socket: &Path, timeout: Option<Duration>) -> Result<Vec<JsonWorktree>> {
    let socket = socket.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
      let _ = tx.send(round_trip(&socket));
    });
    match timeout {
      Some(t) => rx
        .recv_timeout(t)
        .map_err(|_| GwmError::Other("daemon: timed out waiting for the response".to_string()))?,
      None => rx
        .recv()
        .map_err(|_| GwmError::Other("daemon: client thread died".to_string()))?,
    }
  }

  fn round_trip(socket: &Path) -> Result<Vec<JsonWorktree>> {
    let stream = connect(socket)?;
    let (recv, mut send) = stream.split();
    writeln!(send, "{LIST_REQUEST}").map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    send.flush().map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    let mut reader = BufReader::new(recv);
    let mut line = String::new();
    reader
      .read_line(&mut line)
      .map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    parse_list_result(line.trim())
  }

  /// Subscribe to `worktrees.changed` — same contract as the unix client:
  /// the first snapshot is bounded by [`CLIENT_TIMEOUT`], later pushes wait
  /// indefinitely, and a stream that closes before any snapshot errors so
  /// the caller's degradation branch fires (issue #312). A reader thread
  /// pumps lines into a channel; it ends when the stream closes, and the
  /// send half is kept alive for the loop's whole lifetime so the pipe
  /// stays fully open.
  pub fn subscribe(socket: &Path, mut on_snapshot: impl FnMut(&[JsonWorktree]) -> bool) -> Result<()> {
    let stream = connect(socket)?;
    let (recv, mut send) = stream.split();
    writeln!(send, "{SUBSCRIBE_REQUEST}").map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    send.flush().map_err(|e| GwmError::Other(format!("daemon: {e}")))?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
      let mut reader = BufReader::new(recv);
      loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
          Ok(0) | Err(_) => break, // EOF or dead link — dropping tx ends the stream
          Ok(_) => {
            if tx.send(line).is_err() {
              break; // consumer stopped listening
            }
          }
        }
      }
    });
    let mut delivered_any = false;
    loop {
      let line = if delivered_any {
        rx.recv().ok()
      } else {
        rx.recv_timeout(CLIENT_TIMEOUT).ok()
      };
      let Some(line) = line else { break };
      let trimmed = line.trim();
      if trimmed.is_empty() {
        continue;
      }
      let worktrees = parse_worktrees_changed(trimmed)?;
      delivered_any = true;
      if !on_snapshot(&worktrees) {
        break;
      }
    }
    drop(send);
    if !delivered_any {
      return Err(GwmError::Other(
        "daemon: stream closed before the first snapshot".to_string(),
      ));
    }
    Ok(())
  }
}
