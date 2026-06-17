//! `gwm exec` (issue #313): run a command across worktrees and roll up the
//! results.
//!
//! The CLI handler in `cli.rs` resolves which worktrees to target and prints
//! the output; everything testable lives here: the spawn primitive
//! ([`exec_in_dir`]), the aggregate exit code ([`rollup_exit_code`]), and the
//! per-worktree line formatter ([`format_outcome`]). Execution is sequential
//! — deterministic, readable output for the MVP; parallel fan-out is a
//! deliberate follow-up.

use std::path::Path;
use std::process::Command;

/// Outcome of running the command inside one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecStatus {
  /// The command exited 0.
  Ok,
  /// The command exited with a non-zero code.
  Failed(i32),
  /// The command was terminated by a signal (no exit code available).
  Signal,
  /// The program could not be spawned at all (e.g. not found on `PATH`).
  SpawnError(String),
}

/// A worktree's display name paired with its command outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
  pub name: String,
  pub status: ExecStatus,
}

/// Run `program args…` with the working directory set to `dir`.
///
/// The child inherits the parent's stdio so its output streams to the user
/// live (sequential execution keeps the streams from interleaving). Only the
/// resolved exit status is captured and returned — a spawn failure (missing
/// binary, permission denied) maps to [`ExecStatus::SpawnError`] rather than
/// aborting the whole fan-out.
pub fn exec_in_dir(dir: &Path, program: &str, args: &[String]) -> ExecStatus {
  match Command::new(program).args(args).current_dir(dir).status() {
    Ok(status) => match status.code() {
      Some(0) => ExecStatus::Ok,
      Some(code) => ExecStatus::Failed(code),
      None => ExecStatus::Signal,
    },
    Err(e) => ExecStatus::SpawnError(e.to_string()),
  }
}

/// Aggregate exit code for the whole fan-out: `0` only when every worktree
/// succeeded, else `1`. Mirrors the repo's doctor/CI convention of a single
/// non-zero "something failed" code rather than trying to reconcile multiple
/// distinct child codes into one.
pub fn rollup_exit_code(outcomes: &[ExecOutcome]) -> i32 {
  if outcomes.iter().all(|o| o.status == ExecStatus::Ok) {
    0
  } else {
    1
  }
}

/// Render one rollup line for a worktree using the repo's ✓ / ✗ sigils,
/// e.g. `✓ feat-1` or `✗ fix-2 (exit 2)`.
pub fn format_outcome(o: &ExecOutcome) -> String {
  match &o.status {
    ExecStatus::Ok => format!("✓ {}", o.name),
    ExecStatus::Failed(code) => format!("✗ {} (exit {})", o.name, code),
    ExecStatus::Signal => format!("✗ {} (killed by signal)", o.name),
    ExecStatus::SpawnError(msg) => format!("✗ {} (spawn error: {})", o.name, msg),
  }
}
