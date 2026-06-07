//! In-memory transcript of the external commands gwm runs (issue #226).
//!
//! gwm shells out to `gh` (GitHub status), bootstrap shell steps, and
//! lifecycle hooks. The single-line statusbar action log (#217) shows only
//! the most recent action and is ephemeral; this module keeps a bounded,
//! scrollable history behind the lazygit-style Command Logs modal.
//!
//! ## Why a process-global ring
//!
//! The exec chokepoints live in library modules with no `App` handle
//! (`github::run_gh_with` runs on a worker thread, #217), so the sink has
//! to be reachable without threading a logger through every call site. A
//! `LazyLock<Mutex<…>>` ring mirrors the existing static caches
//! (`naming`'s compiled regexes, the recent-commits cache) and tolerates
//! cross-thread writes from the off-thread GitHub fetch.
//!
//! The TUI never renders straight off this global: `App` takes a
//! [`snapshot`] into `state::command_logs` so the modal renders from owned
//! `App` state. That keeps the render path testable without the global and
//! is the boundary the modal-render tests inject through.
//!
//! ## Scope (deliberately narrow for the first cut)
//!
//! Only the **captured-output** shell commands are logged: `gh`, bootstrap
//! steps, hooks. The read-only sidebar previews (`worktree::run_git` for
//! `git log` / `git status`) are *not* — they fire on every selection
//! change and would bury the real operations in noise. Interactive
//! launchers (`.status()` / `.spawn()`, which inherit the terminal and have
//! no captured output) and the libgit2 worktree ops (which run no
//! subprocess) are out of scope here; `gwm sync` joins once it has a TUI
//! action (#258).

use std::collections::VecDeque;
use std::process::{Command, Output};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Upper bound on retained entries. Old entries are evicted FIFO once the
/// ring is full — a transcript, not an audit trail (the operation journal
/// behind `gwm undo` / `gwm history` is the durable record).
pub const MAX_ENTRIES: usize = 256;

/// Cap on the captured output stored per entry. Keeps a chatty command
/// (a verbose hook, a large `gh … --json`) from ballooning the in-memory
/// log; the tail is kept since that is where errors surface.
const MAX_OUTPUT_BYTES: usize = 8 * 1024;

/// How a logged command finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandStatus {
  /// The process ran to completion. `Some(code)` is its exit status;
  /// `None` means it was terminated by a signal (no code on Unix).
  Exited(Option<i32>),
  /// The process could not be spawned at all (binary missing, etc.).
  Spawn,
}

impl CommandStatus {
  /// `true` only for a clean `exit 0`. A signal death or spawn failure is
  /// never a success.
  pub fn is_success(&self) -> bool {
    matches!(self, CommandStatus::Exited(Some(0)))
  }
}

/// One executed command in the transcript: the resolved command line, how
/// long it took, how it finished, and its captured (bounded) output.
#[derive(Debug, Clone)]
pub struct CommandLogEntry {
  /// Human-readable resolved command line, e.g. `gh issue view 226 --json …`.
  pub command: String,
  /// Wall-clock duration of the call.
  pub duration: Duration,
  /// How the command finished.
  pub status: CommandStatus,
  /// Captured stdout (or stderr when stdout was empty), trimmed and
  /// tail-bounded to [`MAX_OUTPUT_BYTES`]. Empty when nothing was captured.
  pub output: String,
}

impl CommandLogEntry {
  /// `true` when the command exited cleanly. Drives the green/red colour
  /// of the status line in the modal.
  pub fn is_success(&self) -> bool {
    self.status.is_success()
  }
}

/// Bounded FIFO ring of [`CommandLogEntry`]. Pure state — no global, no
/// I/O — so its push/evict/snapshot contract is unit-testable in
/// isolation.
#[derive(Debug, Default)]
pub struct CommandLog {
  entries: VecDeque<CommandLogEntry>,
}

impl CommandLog {
  /// An empty log.
  pub fn new() -> Self {
    Self::default()
  }

  /// Append `entry`, evicting the oldest when the ring is already at
  /// [`MAX_ENTRIES`].
  pub fn push(&mut self, entry: CommandLogEntry) {
    if self.entries.len() == MAX_ENTRIES {
      self.entries.pop_front();
    }
    self.entries.push_back(entry);
  }

  /// Number of retained entries.
  pub fn len(&self) -> usize {
    self.entries.len()
  }

  /// `true` when nothing has been logged yet.
  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  /// Oldest-first iterator over the retained entries.
  pub fn iter(&self) -> impl Iterator<Item = &CommandLogEntry> {
    self.entries.iter()
  }

  /// Owned, oldest-first clone of the retained entries — the boundary the
  /// TUI copies across so it never renders off the live global.
  pub fn snapshot(&self) -> Vec<CommandLogEntry> {
    self.entries.iter().cloned().collect()
  }

  /// Drop every entry.
  pub fn clear(&mut self) {
    self.entries.clear();
  }
}

/// The process-global transcript. Written from the exec chokepoints
/// (including the off-thread GitHub fetch worker), read by `App` via
/// [`snapshot`].
static GLOBAL: LazyLock<Mutex<CommandLog>> = LazyLock::new(|| Mutex::new(CommandLog::new()));

/// Record a finished command on the global log. Lock-poison-safe: a
/// poisoned mutex drops the entry rather than panicking — logging must
/// never take down a real operation.
pub fn record(entry: CommandLogEntry) {
  if let Ok(mut log) = GLOBAL.lock() {
    log.push(entry);
  }
}

/// Oldest-first snapshot of the global log. Returns empty on a poisoned
/// lock rather than panicking.
pub fn snapshot() -> Vec<CommandLogEntry> {
  GLOBAL.lock().map(|log| log.snapshot()).unwrap_or_default()
}

/// Clear the global log. Used by the (future) in-TUI "clear logs" action
/// and by tests that need a clean slate.
pub fn reset() {
  if let Ok(mut log) = GLOBAL.lock() {
    log.clear();
  }
}

/// Trim and tail-bound captured output for storage on an entry.
fn bound_output(stdout: &[u8], stderr: &[u8]) -> String {
  let stdout = String::from_utf8_lossy(stdout);
  let stdout = stdout.trim();
  let stderr = String::from_utf8_lossy(stderr);
  let stderr = stderr.trim();
  // Keep BOTH streams: a command that writes progress to stdout and its
  // diagnostics to stderr before failing must not lose the error text in
  // the transcript — the modal is for troubleshooting (Codex review #259).
  // Stdout first, stderr after, joined when both are present.
  let combined = match (stdout.is_empty(), stderr.is_empty()) {
    (true, true) => String::new(),
    (false, true) => stdout.to_string(),
    (true, false) => stderr.to_string(),
    (false, false) => format!("{stdout}\n{stderr}"),
  };
  if combined.len() <= MAX_OUTPUT_BYTES {
    return combined;
  }
  // Keep the tail (where errors land), snapped to a char boundary.
  let cut = combined.len() - MAX_OUTPUT_BYTES;
  let start = (cut..combined.len())
    .find(|&i| combined.is_char_boundary(i))
    .unwrap_or(combined.len());
  combined[start..].to_string()
}

/// Run `cmd`, recording an entry on the global log, and return the raw
/// [`Output`] exactly as [`Command::output`] would.
///
/// `command` is the resolved, human-readable command line stored on the
/// entry (the caller builds it so the transcript shows the real argv, not
/// an opaque `sh -c`). Times the call, captures stdout/stderr + exit, and
/// records the entry whether the command succeeded, failed, or could not be
/// spawned. The returned `Result` is the caller's to handle — this wrapper
/// only observes, it never swallows the error.
pub fn run_logged(cmd: &mut Command, command: String) -> std::io::Result<Output> {
  let start = Instant::now();
  let result = cmd.output();
  let duration = start.elapsed();
  match &result {
    Ok(out) => record(CommandLogEntry {
      command,
      duration,
      status: CommandStatus::Exited(out.status.code()),
      output: bound_output(&out.stdout, &out.stderr),
    }),
    Err(_) => record(CommandLogEntry {
      command,
      duration,
      status: CommandStatus::Spawn,
      output: String::new(),
    }),
  }
  result
}
