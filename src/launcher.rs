//! Configurable command launchers for the TUI `l` (git_tui) and `R`
//! (review) keybindings — issue #75.
//!
//! Both keybindings share the same mini-API: take a `command` template
//! string from `.gwm.toml`, substitute placeholders, split with
//! `shell-words`, and exec'd with `cwd = worktree.path`. The only
//! per-keybinding difference is the placeholder set: `[git_tui]` only
//! cares about `{path}`, while `[review]` also exposes `{base}`,
//! `{head}`, and `{diff}` (a lazily-materialised tempfile carrying
//! `git diff {base}..{head}`).
//!
//! This module owns the shared machinery (placeholder expansion,
//! base resolution, missing-binary probe) so the TUI event loop and
//! `gwm doctor` can each consume it through the same surface.

use crate::config::ResolvedLauncher;
use crate::error::{GwmError, Result};
use git2::Repository;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Placeholder substitution context. `base` and `head` are only required
/// by the review launcher; the git_tui launcher passes `None` for both.
/// `worktree_path` is mandatory because both launchers `cd` into it
/// before exec'ing the command.
#[derive(Debug, Clone)]
pub struct LauncherContext<'a> {
  pub worktree_path: &'a Path,
  pub base: Option<&'a str>,
  pub head: Option<&'a str>,
  /// Repo handle used to materialise the `{diff}` tempfile on demand.
  /// `None` disables the `{diff}` placeholder (status-bar error if the
  /// template references it).
  pub repo_workdir: Option<&'a Path>,
}

/// Output of expanding a launcher template. `argv[0]` is the binary,
/// `argv[1..]` are the arguments. `diff_file` is `Some` only when the
/// template referenced `{diff}` — its `Drop` impl cleans the tempfile
/// up once the spawned process has consumed it.
#[derive(Debug)]
pub struct ExpandedCommand {
  pub argv: Vec<String>,
  /// Kept alive for the duration of the spawned process so the tempfile
  /// the `{diff}` placeholder points at is not unlinked before the
  /// reviewer reads it. `None` when the template didn't use `{diff}`.
  pub diff_file: Option<tempfile::NamedTempFile>,
}

impl ExpandedCommand {
  /// Binary name (`argv[0]`), or `None` for an empty argv (parser
  /// returned `[]`, which means the user typed e.g. `command = ""`).
  pub fn binary(&self) -> Option<&str> {
    self.argv.first().map(|s| s.as_str())
  }
}

/// Substitute `{base}`, `{head}`, `{path}`, `{diff}` in `template` and
/// split the result with `shell-words`. Materialises a tempfile holding
/// `git diff {base}..{head}` iff the template references `{diff}` —
/// this is the lazy contract from the issue body: a template that
/// doesn't use `{diff}` must not spawn `git diff`.
///
/// Errors:
/// - `Config` — `{base}` / `{head}` / `{diff}` referenced without the
///   matching context field set.
/// - `Other` — `shell-words` parse failure (unbalanced quotes etc.).
pub fn expand_command(template: &str, ctx: &LauncherContext<'_>) -> Result<ExpandedCommand> {
  let uses_base = template.contains("{base}");
  let uses_head = template.contains("{head}");
  let uses_diff = template.contains("{diff}");

  if uses_base && ctx.base.is_none() {
    return Err(GwmError::Config(
      "template uses {base} but no base ref was resolved".into(),
    ));
  }
  if uses_head && ctx.head.is_none() {
    return Err(GwmError::Config(
      "template uses {head} but no head ref was resolved".into(),
    ));
  }

  let path_str = ctx.worktree_path.to_string_lossy();
  let mut expanded = template.replace("{path}", &path_str);
  if let Some(b) = ctx.base {
    expanded = expanded.replace("{base}", b);
  }
  if let Some(h) = ctx.head {
    expanded = expanded.replace("{head}", h);
  }

  let diff_file = if uses_diff {
    let (b, h) = match (ctx.base, ctx.head) {
      (Some(b), Some(h)) => (b, h),
      _ => {
        return Err(GwmError::Config(
          "template uses {diff} but {base}/{head} could not be resolved".into(),
        ))
      }
    };
    let workdir = ctx.repo_workdir.ok_or_else(|| {
      GwmError::Config("template uses {diff} but no repo workdir was provided to the launcher".into())
    })?;
    let tmp = materialise_diff(workdir, b, h)?;
    let diff_path = tmp.path().to_string_lossy().to_string();
    expanded = expanded.replace("{diff}", &diff_path);
    Some(tmp)
  } else {
    None
  };

  let argv =
    shell_words::split(&expanded).map_err(|e| GwmError::Other(format!("invalid shell line '{}': {}", expanded, e)))?;
  Ok(ExpandedCommand { argv, diff_file })
}

/// Shell out to `git diff <base>..<head>` from `workdir`, write the
/// output into a tempfile, and return the handle. The caller keeps
/// the handle alive (the tempfile is unlinked on drop) so the consumer
/// process can read it via the `{diff}` substitution.
///
/// Failures of `git diff` are surfaced as `CommandFailed` rather than
/// silently producing an empty file — the user pressed `R` expecting a
/// diff, so an empty buffer would mask a real configuration problem.
fn materialise_diff(workdir: &Path, base: &str, head: &str) -> Result<tempfile::NamedTempFile> {
  let output = Command::new("git")
    .arg("-C")
    .arg(workdir)
    .args(["diff", &format!("{}..{}", base, head)])
    .output()
    .map_err(|e| GwmError::CommandFailed(format!("git diff failed to spawn: {}", e)))?;
  if !output.status.success() {
    return Err(GwmError::CommandFailed(format!(
      "git diff {}..{} exited with status {:?}: {}",
      base,
      head,
      output.status.code(),
      String::from_utf8_lossy(&output.stderr).trim()
    )));
  }
  let mut tmp = tempfile::Builder::new()
    .prefix("gwm-review-")
    .suffix(".diff")
    .tempfile()
    .map_err(GwmError::Io)?;
  use std::io::Write as _;
  tmp.write_all(&output.stdout).map_err(GwmError::Io)?;
  tmp.flush().map_err(GwmError::Io)?;
  Ok(tmp)
}

/// Resolve the review base for `branch` following the chain documented
/// in issue #75:
///
/// 1. `branch.<name>.merge` (the upstream tracking ref).
/// 2. `branch.<name>.gwm-base` — set by `gwm create` on the new branch
///    so the original parent is recoverable even without an upstream.
/// 3. `[review].default_base` from `.gwm.toml`.
/// 4. `"dev"` (gwm's project convention).
/// 5. `"main"` (universal git default).
///
/// Returns the first non-empty hit; never errors (the final `"main"`
/// is a guaranteed sentinel). The string is the user-facing ref name
/// — the launcher passes it to `git diff` / `git rev-list` directly,
/// so it must be a name git understands.
pub fn resolve_review_base(repo: &Repository, branch: &str, default_base: Option<&str>) -> String {
  if let Some(upstream) = read_branch_merge(repo, branch) {
    return upstream;
  }
  if let Some(gwm_base) = read_branch_config(repo, branch, "gwm-base") {
    return gwm_base;
  }
  if let Some(d) = default_base.map(str::trim).filter(|s| !s.is_empty()) {
    return d.to_string();
  }
  "dev".to_string()
}

/// Persist `branch.<name>.gwm-base = <base>` so the review base chain
/// can recover the parent ref even if the upstream is dropped. Called
/// by `gwm create` after a successful `git worktree add`.
pub fn write_gwm_base(repo: &Repository, branch: &str, base: &str) -> Result<()> {
  let mut cfg = repo.config()?;
  cfg.set_str(&format!("branch.{}.gwm-base", branch), base)?;
  Ok(())
}

fn read_branch_merge(repo: &Repository, branch: &str) -> Option<String> {
  let cfg = repo.config().ok()?;
  let raw = cfg.get_string(&format!("branch.{}.merge", branch)).ok()?;
  let trimmed = raw.trim();
  if trimmed.is_empty() {
    return None;
  }
  // `branch.<name>.merge` is stored as a refspec like `refs/heads/dev`;
  // surface the short name so the value is fit for `git diff` / `git
  // rev-list` without further massaging.
  Some(trimmed.strip_prefix("refs/heads/").unwrap_or(trimmed).to_string())
}

fn read_branch_config(repo: &Repository, branch: &str, leaf: &str) -> Option<String> {
  let cfg = repo.config().ok()?;
  let raw = cfg.get_string(&format!("branch.{}.{}", branch, leaf)).ok()?;
  let trimmed = raw.trim();
  if trimmed.is_empty() {
    None
  } else {
    Some(trimmed.to_string())
  }
}

/// Count commits in `head` not in `base` (`git rev-list --count
/// {base}..{head}` shelled out from `workdir`). Returns `0` when the
/// shell-out fails — the `R` keybinding defers to the caller's
/// `skip_when_no_changes` knob to decide what to do with the count.
pub fn count_commits_ahead(workdir: &Path, base: &str, head: &str) -> u32 {
  let output = Command::new("git")
    .arg("-C")
    .arg(workdir)
    .args(["rev-list", "--count", &format!("{}..{}", base, head)])
    .output();
  let Ok(out) = output else { return 0 };
  if !out.status.success() {
    return 0;
  }
  String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().unwrap_or(0)
}

/// Probe `$PATH` for the binary in an [`ExpandedCommand`]. Returns the
/// absolute path on hit, `None` otherwise — the status bar can then
/// surface a precise error without trying to spawn a missing file.
pub fn locate_binary(expanded: &ExpandedCommand) -> Option<PathBuf> {
  let bin = expanded.binary()?;
  which::which(bin).ok()
}

/// Probe whether the resolved launcher's binary exists on $PATH. Used
/// by [`gwm doctor`](crate::doctor) to emit a warning (exit code 1)
/// when the configured review/git_tui binary is missing — the launcher
/// itself is opt-in, so this is advisory, not a hard failure.
///
/// Returns the binary name on miss so the doctor check can include it
/// in the message verbatim ("not on PATH: lumen"). Returns `None` if
/// the binary resolves or if the launcher has no parseable argv.
pub fn missing_binary_for(launcher: &ResolvedLauncher) -> Option<String> {
  // Strip the placeholders before tokenising — `shell_words` would
  // happily eat the `{path}` literal, but we only care about argv[0]
  // here so a quick replace keeps things simple. The launcher itself
  // does proper expansion at exec time.
  let cleaned = launcher
    .command
    .replace("{base}", "BASE")
    .replace("{head}", "HEAD")
    .replace("{path}", "PATH")
    .replace("{diff}", "/tmp/diff");
  let tokens = shell_words::split(&cleaned).ok()?;
  let bin = tokens.into_iter().find(|t| !t.contains('='))?;
  if which::which(&bin).is_ok() {
    None
  } else {
    Some(bin)
  }
}
