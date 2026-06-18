//! `gwm exec` (issue #313): run a command across worktrees and roll up the
//! results.
//!
//! The CLI handler in `cli.rs` resolves which worktrees to target and prints
//! the output; everything testable lives here: the spawn primitive
//! ([`exec_in_dir`]), the aggregate exit code ([`rollup_exit_code`]), and the
//! per-worktree line formatter ([`format_outcome`]). Execution is sequential
//! — deterministic, readable output for the MVP; parallel fan-out is a
//! deliberate follow-up.

use crate::config::ExecConfig;
use crate::error::{GwmError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

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

/// One parallel worktree run: its [`ExecOutcome`] plus the captured
/// stdout+stderr bytes printed as a block by the caller.
pub type CapturedRun = (ExecOutcome, Vec<u8>);

/// Resolve the argv `gwm exec` should run, from exactly one source: an
/// inline `-- <cmd>` or a `--profile <name>` (issue #324).
///
/// The two are mutually exclusive and exactly one is required:
/// - both given → error (the profile *carries* the command);
/// - a `--profile` naming an entry absent from `[exec.profiles]` → error;
/// - a profile whose `command` is empty → error (degenerate config);
/// - neither given → error (nothing to run).
///
/// Every error path is a user-facing [`GwmError`] (exit 1), never a panic.
pub fn resolve_exec_command(profile: Option<&str>, inline: &[String], cfg: &ExecConfig) -> Result<Vec<String>> {
  match (profile, inline.is_empty()) {
    (Some(_), false) => Err(GwmError::Other(
      "exec: --profile and an inline `-- <cmd>` are mutually exclusive — the profile carries the command".into(),
    )),
    (Some(name), true) => {
      let p = cfg
        .profiles
        .get(name)
        .ok_or_else(|| GwmError::Config(format!("exec: no profile named `{name}` in [exec.profiles]")))?;
      validate_exec_profile_command(name, &p.command)?;
      Ok(p.command.clone())
    }
    (None, false) => Ok(inline.to_vec()),
    (None, true) => Err(GwmError::Other(
      "exec: provide a command after `--` (e.g. `gwm exec -- cargo test`) or pass `--profile <name>`".into(),
    )),
  }
}

/// Validate a `[exec.profiles.<name>]` entry's `command`: it must be a
/// non-empty argv array. Surfaced for the config validation path so
/// `gwm config validate` / `gwm doctor` reject what `gwm exec --profile`
/// would (issue #324 review).
pub fn validate_exec_profile_command(profile: &str, command: &[String]) -> Result<()> {
  if command.is_empty() {
    return Err(GwmError::Config(format!(
      "exec: profile `{profile}` has an empty `command` — give it an argv array like `command = [\"cargo\", \"test\"]`"
    )));
  }
  Ok(())
}

/// Run `program args…` with the working directory set to `dir`.
///
/// The child inherits the parent's stdio so its output streams to the user
/// live (sequential execution keeps the streams from interleaving). Only the
/// resolved exit status is captured and returned — a spawn failure (missing
/// binary, permission denied) maps to [`ExecStatus::SpawnError`] rather than
/// aborting the whole fan-out.
pub fn exec_in_dir(dir: &Path, program: &str, args: &[String]) -> ExecStatus {
  let resolved = resolve_program(dir, program);
  match Command::new(&resolved).args(args).current_dir(dir).status() {
    Ok(status) => match status.code() {
      Some(0) => ExecStatus::Ok,
      Some(code) => ExecStatus::Failed(code),
      None => ExecStatus::Signal,
    },
    Err(e) => ExecStatus::SpawnError(e.to_string()),
  }
}

/// Resolve the effective parallelism for `gwm exec` (issue #324): the `--jobs`
/// flag wins, then the selected profile's `jobs`, then the global `[exec]
/// jobs`, else `1`. A resolved `0` (or absent) means sequential. Always
/// returns a worker count `>= 1`.
pub fn resolve_jobs(flag: Option<u32>, profile: Option<&str>, cfg: &ExecConfig) -> usize {
  let n = flag
    .or_else(|| profile.and_then(|p| cfg.profiles.get(p)).and_then(|p| p.jobs))
    .or(cfg.jobs)
    .unwrap_or(1);
  n.max(1) as usize
}

/// Like [`exec_in_dir`], but CAPTURE stdout+stderr (stdout then stderr)
/// instead of inheriting the parent's stdio. Used by [`run_in_dirs_parallel`]
/// so concurrent worktrees don't interleave their output — each block is
/// printed whole, in worktree order, after the fan-out completes.
pub fn exec_capture_in_dir(dir: &Path, program: &str, args: &[String]) -> (ExecStatus, Vec<u8>) {
  let resolved = resolve_program(dir, program);
  match Command::new(&resolved).args(args).current_dir(dir).output() {
    Ok(out) => {
      let mut buf = out.stdout;
      buf.extend_from_slice(&out.stderr);
      let status = match out.status.code() {
        Some(0) => ExecStatus::Ok,
        Some(code) => ExecStatus::Failed(code),
        None => ExecStatus::Signal,
      };
      (status, buf)
    }
    Err(e) => (ExecStatus::SpawnError(e.to_string()), Vec::new()),
  }
}

/// Run `program args…` in each `(name, dir)` of `items` with up to `jobs`
/// concurrent workers, capturing each one's output. Returns one
/// `(ExecOutcome, captured_output)` per item **in input order** (not
/// completion order), so the caller prints deterministic per-worktree blocks
/// regardless of which finished first. `jobs` is clamped to `[1, items.len()]`.
pub fn run_in_dirs_parallel(
  jobs: usize,
  items: &[(String, PathBuf)],
  program: &str,
  args: &[String],
) -> Vec<CapturedRun> {
  if items.is_empty() {
    return Vec::new();
  }
  let workers = jobs.clamp(1, items.len());
  let next = AtomicUsize::new(0);
  let slots: Vec<Mutex<Option<CapturedRun>>> = (0..items.len()).map(|_| Mutex::new(None)).collect();
  std::thread::scope(|s| {
    for _ in 0..workers {
      s.spawn(|| loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= items.len() {
          break;
        }
        let (name, path) = &items[i];
        let (status, output) = exec_capture_in_dir(path, program, args);
        // `.lock()` never poisons: the worker body cannot panic (the spawn
        // primitive returns `SpawnError` instead of unwinding).
        *slots[i].lock().expect("exec worker mutex never poisoned") = Some((
          ExecOutcome {
            name: name.clone(),
            status,
          },
          output,
        ));
      });
    }
  });
  slots
    .into_iter()
    .map(|m| m.into_inner().expect("exec worker mutex never poisoned"))
    .map(|slot| slot.expect("every worktree slot filled by a worker"))
    .collect()
}

/// Resolve `program` for execution inside `dir`.
///
/// A relative program that contains a path separator (e.g. `./build.sh`,
/// `scripts/run`) is a *path*, and the command's contract is "run in each
/// worktree" — so it is joined onto `dir`. This pins the resolution to the
/// target worktree regardless of whether the platform resolves a relative
/// executable against the parent's or the child's cwd (the order differs
/// across OSes for `std::process::Command` + `current_dir`). Bare names
/// (no separator) stay `PATH` lookups, and absolute paths are left as-is.
pub fn resolve_program(dir: &Path, program: &str) -> PathBuf {
  let p = Path::new(program);
  if p.is_relative() && has_path_separator(program) {
    dir.join(p)
  } else {
    p.to_path_buf()
  }
}

/// Whether `program` contains a path separator — `/` everywhere, plus `\` on
/// Windows. Such a token is a path, not a `PATH`-resolved command name.
fn has_path_separator(program: &str) -> bool {
  program.contains('/') || (cfg!(windows) && program.contains('\\'))
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
