//! `gwm exec` (issue #313): run a command across worktrees and roll up the
//! results.
//!
//! The CLI handler in `cli.rs` resolves which worktrees to target and prints
//! the output; everything testable lives here: the spawn primitive
//! ([`exec_in_dir`]), the aggregate exit code ([`rollup_exit_code`]), and the
//! per-worktree line formatter ([`format_outcome`]). Execution is sequential
//! — deterministic, readable output for the MVP; parallel fan-out is a
//! deliberate follow-up.

use crate::config::{ContainerConfig, ExecConfig, ExecProfile};
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
      "exec: --profile and an inline `-- <cmd>` are mutually exclusive; the profile carries the command".into(),
    )),
    (Some(name), true) => {
      let p = cfg
        .profiles
        .get(name)
        .ok_or_else(|| GwmError::Config(format!("exec: no profile named `{name}` in [exec.profiles]")))?;
      validate_exec_profile(name, p)?;
      Ok(p.command.clone())
    }
    (None, false) => Ok(inline.to_vec()),
    (None, true) => Err(GwmError::Other(
      "exec: provide a command after `--` (e.g. `gwm exec -- cargo test`) or pass `--profile <name>`".into(),
    )),
  }
}

/// Validate one `[exec.profiles.<name>]` entry. Surfaced for the config
/// validation path so `gwm config validate` / `gwm doctor` reject what
/// `gwm exec --profile` would (issue #324 review).
///
/// The profile is destructured **exhaustively**: adding a field to
/// [`ExecProfile`] without deciding whether it needs validating no longer
/// compiles, so a checker can't silently read half the config (the `gwm
/// doctor` failure mode from #392).
pub fn validate_exec_profile(profile: &str, entry: &ExecProfile) -> Result<()> {
  let ExecProfile {
    command,
    jobs: _, // any u32 is valid; `resolve_jobs` clamps 0 to 1.
    container,
  } = entry;
  if command.is_empty() {
    return Err(GwmError::Config(format!(
      "exec: profile `{profile}` has an empty `command`; give it an argv array like `command = [\"cargo\", \"test\"]`"
    )));
  }
  if let Some(c) = container {
    validate_container(profile, c)?;
  }
  Ok(())
}

/// Validate a `[exec.profiles.<name>.container]` block (issue #421): a
/// non-empty `image` (the one field with no default), and `extra_args` that
/// does not take over `--name`.
///
/// `--name` is gwm's: the TUI overlay tears its container down **by name**,
/// and a runtime honours the last `--name` it is given, so an `extra_args`
/// one would leave the teardown removing a container that was never started
/// — possibly one belonging to something else.
pub fn validate_container(profile: &str, cfg: &ContainerConfig) -> Result<()> {
  let ContainerConfig {
    image,
    runtime: _, // any non-empty string is a candidate binary; a bad one fails at spawn.
    extra_args,
    selinux_relabel: _, // a bool cannot be invalid.
  } = cfg;
  if image.trim().is_empty() {
    return Err(GwmError::Config(format!(
      "exec: profile `{profile}` has a `[container]` with an empty `image`; give it one like `image = \"rust:1.90\"`"
    )));
  }
  if extra_args.iter().any(|a| a == "--name" || a.starts_with("--name=")) {
    return Err(GwmError::Config(format!(
      "exec: profile `{profile}` sets `--name` in `[container] extra_args`; gwm owns that flag, \
       because the TUI overlay removes its container by name when it closes. Drop it."
    )));
  }
  Ok(())
}

/// The container CLIs gwm auto-detects, **in preference order**. Docker
/// first: it is the CLI the ecosystem's tooling assumes, and every
/// Docker-compatible engine (OrbStack, Colima, Rancher Desktop, Docker
/// Desktop) exposes it. The reference implementation prefers podman; gwm
/// states its own order rather than inheriting one by accident (issue #421).
pub const CONTAINER_RUNTIMES: &[&str] = &["docker", "podman"];

/// Resolve the container block for this invocation: `Some` only when a
/// `--profile` carrying `[container]` was named. The inline
/// `gwm exec -- <cmd>` surface is **never** containerised, whatever the
/// config says — that surface is frozen (#319) and containerising it would
/// change what an unchanged command line does.
pub fn resolve_exec_container(profile: Option<&str>, cfg: &ExecConfig) -> Result<Option<ContainerConfig>> {
  let Some(name) = profile else {
    return Ok(None);
  };
  let Some(p) = cfg.profiles.get(name) else {
    return Ok(None); // `resolve_exec_command` already reports the unknown profile.
  };
  let Some(container) = p.container.clone() else {
    return Ok(None);
  };
  validate_container(name, &container)?;
  Ok(Some(container))
}

/// Resolve which container CLI to run: an explicit `runtime` wins outright
/// (it is honoured even when absent from `PATH` — the spawn failure reports
/// it better than a config error could), else the first of
/// [`CONTAINER_RUNTIMES`] that `available` accepts.
///
/// `available` is injected rather than read from `PATH` here so the caller
/// owns the environment lookup and the tests stay hermetic (CI runners have
/// neither docker nor podman).
pub fn resolve_container_runtime(configured: Option<&str>, available: impl Fn(&str) -> bool) -> Result<String> {
  if let Some(r) = configured.map(str::trim).filter(|r| !r.is_empty()) {
    return Ok(r.to_string());
  }
  CONTAINER_RUNTIMES
    .iter()
    .find(|bin| available(bin))
    .map(|bin| (*bin).to_string())
    .ok_or_else(|| {
      GwmError::Config(format!(
        "exec: no container runtime found on PATH (looked for {}); install one or set `runtime` in the profile's `[container]`",
        CONTAINER_RUNTIMES.join(", ")
      ))
    })
}

/// Everything a containerised fan-out needs, resolved once per repo: the
/// runtime binary, the profile's block, and the main checkout's gitdir to
/// mount. Held by the CLI across the whole run so the per-worktree wrap is a
/// pure [`build_container_argv`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerPlan {
  /// Resolved container CLI (`docker`, `podman`, …) — see
  /// [`resolve_container_runtime`].
  pub runtime: String,
  /// The profile's `[container]` block.
  pub config: ContainerConfig,
  /// `repo.commondir()` — `<main>/.git` for every worktree of the repo,
  /// normalised (no trailing separator) so the mount argument reads like a
  /// path rather than a directory listing.
  pub common_dir: PathBuf,
}

impl ContainerPlan {
  /// Resolve the runtime and normalise `common_dir`. `available` is the
  /// `PATH` lookup, injected so tests never depend on the runner having a
  /// container runtime installed.
  pub fn resolve(config: ContainerConfig, common_dir: &Path, available: impl Fn(&str) -> bool) -> Result<Self> {
    // Refused on Windows rather than half-honoured. The whole design mirrors
    // host paths, and a Windows path is not one a Linux container can mount
    // or `cd` into; worse, the `.git` file of a linked worktree would still
    // name a drive-letter path, so even a translated mount would leave git
    // unable to answer — the one thing this feature exists to guarantee.
    if cfg!(windows) {
      return Err(GwmError::Config(
        "exec: `[container]` is not supported on Windows; the wrapper mirrors host paths, \
         and a `C:\\…` path is neither mountable nor resolvable inside a Linux container \
         (the worktree's `.git` file would still name a Windows path). Run the profile on \
         the host, or drive the container from a Linux/macOS checkout."
          .into(),
      ));
    }
    let runtime = resolve_container_runtime(config.runtime.as_deref(), available)?;
    Ok(Self {
      runtime,
      config,
      // `components()` drops a trailing separator (git2 returns
      // `<main>/.git/`) and any `.` component, without touching symlinks —
      // canonicalising here would rewrite the path the host actually uses.
      common_dir: common_dir.components().collect(),
    })
  }

  /// Wrap `argv` for `worktree`, non-interactively. See
  /// [`build_container_argv`]. This is the fan-out form: `gwm exec` runs
  /// across N worktrees, where a TTY per container means nothing.
  pub fn wrap(&self, worktree: &Path, argv: &[String]) -> Result<Vec<String>> {
    build_container_argv(&self.runtime, &self.config, worktree, &self.common_dir, argv, None)
  }

  /// Wrap `argv` for `worktree` **with `-i -t` and a `--name`**, for a caller
  /// that already owns a terminal: the TUI exec overlay spawns into a real
  /// pty (`portable_pty::openpty`), so without the tty flags a REPL, a
  /// debugger or any prompting command would read EOF and see no terminal —
  /// capabilities the same command has when it runs on the host there.
  ///
  /// The name is what makes the container **stoppable**: killing the client
  /// leaves the container running (the daemon owns it, and `--rm` only fires
  /// once it exits), so the overlay tears it down by name on close. See
  /// [`container_teardown_argv`].
  pub fn wrap_interactive(&self, worktree: &Path, argv: &[String], name: &str) -> Result<Vec<String>> {
    build_container_argv(
      &self.runtime,
      &self.config,
      worktree,
      &self.common_dir,
      argv,
      Some(name),
    )
  }

  /// The argv that force-removes the named container, best-effort, when the
  /// TUI overlay closes.
  pub fn container_teardown_argv(&self, name: &str) -> Vec<String> {
    vec![self.runtime.clone(), "rm".into(), "-f".into(), name.to_string()]
  }
}

/// A container name for the TUI overlay: `gwm-<worktree>-<pid>-<seq>`, reduced
/// to the character class a container name accepts
/// (`[a-zA-Z0-9][a-zA-Z0-9_.-]*`).
///
/// Both halves are load-bearing. `seq` separates two overlays opened on the
/// same worktree within one session; `pid` separates two gwm processes, which
/// would otherwise both produce `…-1` for their first overlay — and there the
/// collision is not a failed `docker run` but the loser's teardown removing
/// the winner's container.
pub fn container_run_name(worktree: &Path, pid: u32, seq: u64) -> String {
  let stem: String = worktree
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_default()
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
        c
      } else {
        '-'
      }
    })
    .collect();
  // Leading `gwm-` guarantees the required alphanumeric first character even
  // when the worktree name starts with `.` or `-`, or is empty.
  format!("gwm-{stem}-{pid}-{seq}")
}

/// Build the argv that runs `argv` inside the container described by `cfg`.
///
/// **argv, never a shell string.** The result is handed to
/// [`exec_in_dir`] / [`exec_capture_in_dir`] as `Command` arguments, so no
/// token is ever quoted, joined or re-parsed by a shell. GHSA-fffq-vg6f-gxqm
/// was branch-name injection through a shell hook; this is stated as an
/// invariant rather than left as a happy consequence.
///
/// The shape:
///
/// ```text
/// <runtime> run --rm [-i -t] -v <wt>:<wt> -v <common>:<common> -w <wt> \
///   -e GIT_CONFIG_COUNT=<n> -e GIT_CONFIG_KEY_i=safe.directory -e GIT_CONFIG_VALUE_i=<path> \
///   <extra…> <image> <argv…>
/// ```
///
/// Three decisions are baked in:
///
/// - **Host paths are mirrored.** `/workspace` buys nothing once the gitdir
///   mount has to reproduce an absolute host path anyway, and mirroring keeps
///   `{path}` / `GWM_PATH` true on both sides.
/// - **`common_dir` is mounted too.** A linked worktree's `.git` is a *file*
///   holding the absolute host path of `<main>/.git/worktrees/<id>`; without
///   that mount the container has a worktree in which no git command answers.
///   It is skipped when it already lives inside the worktree (the main
///   checkout, reachable via an explicit slug), where the first mount covers
///   it.
/// - **Every mounted path is declared `safe.directory`.** With a rootful
///   Docker on Linux the process runs as uid 0 while the bind-mounted tree
///   belongs to the host user, and git refuses a repository it sees as
///   `dubious ownership` — which would undo the mount above. The declaration
///   travels as `GIT_CONFIG_*` environment, so nothing is written to any
///   config file, and it names **only the paths gwm mounts itself** rather
///   than the blanket `*` (the ownership check stays on for everything else).
///   It is the same fix CI providers apply to their own checkouts.
///
/// `argv` becomes the container's **command**, so an image's `ENTRYPOINT` (if
/// any) receives it as arguments; `extra_args = ["--entrypoint", ""]` opts
/// out. `extra_args` lands after gwm's own flags, so a repeated flag wins.
///
/// `name` is `Some` for the interactive (TUI overlay) form, which adds
/// `-i -t --name <name>`; see [`ContainerPlan::wrap_interactive`].
///
/// Errors when a path gwm must mount contains a `:`. That byte is legal in a
/// Unix path but is the field separator of `-v source:destination`, so such a
/// mount cannot be expressed; the runtime would reject the spec with a
/// message about neither the worktree nor gwm.
pub fn build_container_argv(
  runtime: &str,
  cfg: &ContainerConfig,
  worktree: &Path,
  common_dir: &Path,
  argv: &[String],
  name: Option<&str>,
) -> Result<Vec<String>> {
  let wt = mount_path(worktree);
  let mut out = vec![runtime.to_string(), "run".into(), "--rm".into()];
  if let Some(name) = name {
    out.push("-i".into());
    out.push("-t".into());
    out.push("--name".into());
    out.push(name.to_string());
  }
  // The paths gwm mounts itself, in mount order — also the exact set it
  // declares safe below.
  let mut mounted = vec![wt.clone()];
  if !common_dir.starts_with(worktree) {
    mounted.push(mount_path(common_dir));
  }
  if let Some(bad) = mounted.iter().find(|p| p.contains(':')) {
    return Err(GwmError::Config(format!(
      "exec: `[container]` cannot mount `{bad}`; a `:` in the path is the separator of \
       `-v source:destination`, so the mount cannot be expressed. Move the worktree (or the \
       repository) to a path without a colon, or run the profile on the host."
    )));
  }
  // `:z` relabels the bind mount for an SELinux-enforcing host. Opt-in: it
  // writes a shared label to the host tree, recursively.
  let suffix = if cfg.selinux_relabel { ":z" } else { "" };
  for path in &mounted {
    out.push("-v".into());
    out.push(format!("{path}:{path}{suffix}"));
  }
  out.push("-w".into());
  out.push(wt);
  out.push("-e".into());
  out.push(format!("GIT_CONFIG_COUNT={}", mounted.len()));
  for (i, path) in mounted.iter().enumerate() {
    out.push("-e".into());
    out.push(format!("GIT_CONFIG_KEY_{i}=safe.directory"));
    out.push("-e".into());
    out.push(format!("GIT_CONFIG_VALUE_{i}={path}"));
  }
  out.extend(cfg.extra_args.iter().cloned());
  out.push(cfg.image.clone());
  out.extend(argv.iter().cloned());
  Ok(out)
}

/// Render a path for a `-v` / `-w` argument: `components()` drops a trailing
/// separator (git2 hands out both `<main>/.git/` as a commondir and a main
/// worktree path with a trailing slash) and any `.` component, without
/// touching symlinks — canonicalising would rewrite the very path the host
/// uses, and the container has to see that one.
fn mount_path(p: &Path) -> String {
  p.components().collect::<PathBuf>().display().to_string()
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

/// Run each `(name, dir, argv)` of `items` with up to `jobs` concurrent
/// workers, capturing each one's output. Returns one
/// `(ExecOutcome, captured_output)` per item **in input order** (not
/// completion order), so the caller prints deterministic per-worktree blocks
/// regardless of which finished first. `jobs` is clamped to `[1, items.len()]`.
///
/// The argv is **per item**, not shared: a containerised run (issue #421)
/// mounts the worktree's own path, so no two worktrees run the same argv.
pub fn run_in_dirs_parallel(jobs: usize, items: &[(String, PathBuf, Vec<String>)]) -> Vec<CapturedRun> {
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
        let (name, path, argv) = &items[i];
        let (status, output) = match argv.split_first() {
          Some((program, args)) => exec_capture_in_dir(path, program, args),
          // Unreachable via the CLI (`exec_plan` guarantees a non-empty
          // argv), but a panic here would be user-facing.
          None => (ExecStatus::SpawnError("exec: no command resolved".into()), Vec::new()),
        };
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
