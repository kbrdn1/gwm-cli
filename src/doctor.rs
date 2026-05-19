//! Environment + worktree diagnostics. Aggregates a series of cheap checks
//! into a single report so users (and CI) can answer "is my setup sane?"
//! without running a dozen ad-hoc commands.

use crate::config::{expand_placeholders, Config, CONFIG_FILE};
use crate::error::Result;
use crate::naming::parse_branch;
use crate::worktree;
use git2::BranchType;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct DoctorReport {
  pub checks: Vec<Check>,
}

impl DoctorReport {
  pub fn new() -> Self {
    Self::default()
  }

  /// Highest severity present in the report.
  pub fn severity(&self) -> Severity {
    let mut s = Severity::Ok;
    for c in &self.checks {
      match c.status {
        CheckStatus::Failed => return Severity::Failed,
        CheckStatus::Warning if s == Severity::Ok => s = Severity::Warning,
        _ => {}
      }
    }
    s
  }

  /// Process exit code derived from `severity()`:
  /// `0` = all green, `1` = at least one warning, `2` = at least one failure.
  /// Suitable for wiring into CI / pre-commit.
  pub fn exit_code(&self) -> i32 {
    match self.severity() {
      Severity::Ok => 0,
      Severity::Warning => 1,
      Severity::Failed => 2,
    }
  }
}

#[derive(Debug, Clone)]
pub struct Check {
  pub name: String,
  pub status: CheckStatus,
  pub detail: String,
  /// One-line user-facing remediation, displayed under the check when set.
  pub fix_hint: Option<String>,
}

impl Check {
  pub fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      status: CheckStatus::Ok,
      detail: detail.into(),
      fix_hint: None,
    }
  }

  pub fn warning(name: impl Into<String>, detail: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      status: CheckStatus::Warning,
      detail: detail.into(),
      fix_hint: None,
    }
  }

  pub fn failed(name: impl Into<String>, detail: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      status: CheckStatus::Failed,
      detail: detail.into(),
      fix_hint: None,
    }
  }

  pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
    self.fix_hint = Some(hint.into());
    self
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
  Ok,
  Warning,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
  Ok,
  Warning,
  Failed,
}

pub struct DoctorCtx<'a> {
  pub repo_workdir: &'a Path,
  pub repo: &'a git2::Repository,
  pub config: &'a Config,
}

pub fn run(ctx: &DoctorCtx<'_>) -> Result<DoctorReport> {
  let mut report = DoctorReport::new();
  report.checks.push(check_config_parses(ctx));
  report.checks.push(check_guard_references(ctx));
  report.checks.push(check_when_predicates(ctx));
  report.checks.push(check_binaries_on_path(ctx));
  report.checks.push(check_prunable_worktrees(ctx));
  report.checks.push(check_orphan_branches(ctx));
  report.checks.push(check_base_dir_writable(ctx));
  Ok(report)
}

/// Check #1: `.gwm.toml` parses cleanly. Missing config is fine — defaults
/// are documented and identical to what `gwm init` writes out. Invalid TOML
/// is a hard failure since it would crash every other subcommand.
fn check_config_parses(ctx: &DoctorCtx<'_>) -> Check {
  let path = ctx.repo_workdir.join(CONFIG_FILE);
  let name = ".gwm.toml parses";

  if !path.exists() {
    return Check::ok(name, "no .gwm.toml present — defaults assumed");
  }

  let raw = match std::fs::read_to_string(&path) {
    Ok(s) => s,
    Err(e) => {
      return Check::failed(name, format!("could not read {}: {}", path.display(), e));
    }
  };

  match toml::from_str::<Config>(&raw) {
    Ok(_) => Check::ok(name, format!("{} parses cleanly", path.display())),
    Err(e) => Check::failed(name, format!("invalid TOML in {}: {}", path.display(), e))
      .with_hint("fix the syntax or back it up and re-run `gwm init`"),
  }
}

/// Check #2: every `[[bootstrap.copy]].guards = [...]` entry references a
/// `[[bootstrap.guard]].name` that actually exists. Dangling references are
/// silent footguns — the copy step would proceed unchecked and the guard
/// would never trip.
fn check_guard_references(ctx: &DoctorCtx<'_>) -> Check {
  let name = "guard references resolve";
  let bs = &ctx.config.bootstrap;

  let mut dangling: Vec<String> = Vec::new();
  for copy in &bs.copy {
    for guard_name in &copy.guards {
      if ctx.config.guard_by_name(guard_name).is_none() {
        dangling.push(format!(
          "{} (referenced from copy {} -> {})",
          guard_name, copy.from, copy.to
        ));
      }
    }
  }

  if dangling.is_empty() {
    let count: usize = bs.copy.iter().map(|c| c.guards.len()).sum();
    return Check::ok(name, format!("{} guard reference(s) resolve", count));
  }

  Check::failed(name, format!("dangling guard reference(s): {}", dangling.join("; ")))
    .with_hint("declare the missing `[[bootstrap.guard]]` block(s) or drop the reference")
}

/// Recognised `when:` predicates. Update this list when a new keyword
/// lands in `bootstrap.rs::evaluate_when`.
const SUPPORTED_WHEN_PREFIXES: &[&str] = &["file_exists:"];

/// Check #3: every `[[bootstrap.command]].when` predicate uses one of the
/// supported keywords. Unknown predicates default to `true` in
/// `bootstrap::check_when`, so the command runs anyway and the user's
/// intended gating condition is silently ignored — that's still a footgun
/// worth flagging, just not "command never runs".
fn check_when_predicates(ctx: &DoctorCtx<'_>) -> Check {
  let name = "`when` predicates supported";
  let bs = &ctx.config.bootstrap;

  let mut unknown: Vec<String> = Vec::new();
  let mut checked: usize = 0;
  for cmd in &bs.command {
    let Some(w) = &cmd.when else { continue };
    checked += 1;
    if !SUPPORTED_WHEN_PREFIXES.iter().any(|p| w.starts_with(p)) {
      unknown.push(format!("{} (on command `{}`)", w, cmd.name));
    }
  }

  if unknown.is_empty() {
    let detail = if checked == 0 {
      "no `when:` predicates configured".to_string()
    } else {
      format!("{} predicate(s) recognised", checked)
    };
    return Check::ok(name, detail);
  }

  Check::failed(name, format!("unknown `when` predicate(s): {}", unknown.join("; ")))
    .with_hint(format!("supported keywords: {}", SUPPORTED_WHEN_PREFIXES.join(", ")))
}

/// Extract the executable name from a shell command string. Tokenises
/// via `shell_words` so quoted args (`"my tool" --flag`) and escaped
/// whitespace are handled the way the shell would, then skips leading
/// `FOO=bar` env assignments and returns the first token that isn't
/// `KEY=VAL`. Returns `None` for empty strings or strings that fail
/// to parse (unbalanced quotes — better to surface nothing than a
/// garbage binary name that would produce a confusing PATH warning).
fn extract_binary(run: &str) -> Option<String> {
  let tokens = shell_words::split(run).ok()?;
  tokens.into_iter().find(|t| !t.contains('='))
}

/// Check #4: every binary referenced by the bootstrap commands resolves on
/// `$PATH`. `lazygit` (the TUI's `l` keybinding) and `direnv` (only if the
/// repo has an `.envrc`) are also checked because they're the two "ambient"
/// dependencies whose absence routinely confuses new users.
///
/// Missing binaries are surfaced as Warning, not Failed — the user may not
/// rely on that step at all, but the visibility matters.
fn check_binaries_on_path(ctx: &DoctorCtx<'_>) -> Check {
  let name = "external binaries on PATH";
  let mut needed: BTreeSet<String> = BTreeSet::new();

  // Ambient deps the rest of the CLI uses.
  needed.insert("lazygit".into());
  if ctx.repo_workdir.join(".envrc").exists() {
    needed.insert("direnv".into());
  }

  // Whatever the user's own bootstrap commands invoke.
  for cmd in &ctx.config.bootstrap.command {
    if let Some(bin) = extract_binary(&cmd.run) {
      needed.insert(bin);
    }
  }

  let mut missing: Vec<String> = Vec::new();
  let mut found: usize = 0;
  for bin in &needed {
    if which::which(bin).is_ok() {
      found += 1;
    } else {
      missing.push(bin.clone());
    }
  }

  if missing.is_empty() {
    return Check::ok(name, format!("{}/{} binaries found", found, needed.len()));
  }

  Check::warning(name, format!("not on PATH: {}", missing.join(", ")))
    .with_hint("install the missing binaries or remove the steps that need them")
}

/// Check #7: the configured worktree `base` directory exists and is
/// writable. Absence is fine when the parent is writable (gwm creates the
/// base lazily on `gwm create`); a non-writable base is a Failed because
/// every future `create` would error out.
fn check_base_dir_writable(ctx: &DoctorCtx<'_>) -> Check {
  let name = "base directory writable";
  let repo_name = worktree::repo_name(ctx.repo);
  let base_expanded = match expand_placeholders(&ctx.config.worktree.base, &repo_name, None, None, None) {
    Ok(s) => s,
    Err(e) => return Check::failed(name, format!("could not expand base placeholders: {}", e)),
  };
  let base = Path::new(&base_expanded);

  if base.exists() {
    return if is_writable_dir(base) {
      Check::ok(name, format!("{} is writable", base.display()))
    } else {
      Check::failed(name, format!("{} exists but is not writable", base.display()))
        .with_hint("fix the permissions, or set `[worktree].base` to a writable path")
    };
  }

  // Base doesn't exist yet — gwm will create it. Check the parent instead.
  let parent = match base.parent() {
    Some(p) if !p.as_os_str().is_empty() => p,
    _ => {
      return Check::ok(
        name,
        format!("{} will be created on first `gwm create`", base.display()),
      )
    }
  };
  if !parent.exists() {
    return Check::warning(
      name,
      format!(
        "neither {} nor its parent {} exists yet",
        base.display(),
        parent.display()
      ),
    )
    .with_hint("create the parent directory, or pick a different `[worktree].base`");
  }
  if is_writable_dir(parent) {
    Check::ok(
      name,
      format!(
        "{} will be created on first `gwm create` (parent writable)",
        base.display()
      ),
    )
  } else {
    Check::failed(name, format!("parent {} is not writable", parent.display()))
      .with_hint("fix the permissions, or set `[worktree].base` to a writable path")
  }
}

/// Check #5: no prunable worktree entries left in `.git/worktrees/`. These
/// happen when a worktree's working directory is deleted manually without
/// going through `gwm remove` — the admin record stays and confuses future
/// `gwm list` invocations.
fn check_prunable_worktrees(ctx: &DoctorCtx<'_>) -> Check {
  let name = "no prunable worktrees";
  let trees = match worktree::list(ctx.repo) {
    Ok(t) => t,
    Err(e) => return Check::failed(name, format!("could not list worktrees: {}", e)),
  };

  let prunable: Vec<String> = trees.iter().filter(|w| w.is_prunable).map(|w| w.name.clone()).collect();
  if prunable.is_empty() {
    return Check::ok(name, format!("{} worktree(s) tracked, none prunable", trees.len()));
  }

  let noun = if prunable.len() == 1 { "entry" } else { "entries" };
  Check::warning(
    name,
    format!("{} prunable {}: {}", prunable.len(), noun, prunable.join(", ")),
  )
  .with_hint("run `gwm prune` to clear them")
}

/// Trunk branches the doctor considers "merged into" — branches fully
/// reachable from one of these are deliberately preserved per
/// CONTRIBUTING.md (`Never delete the source branch after merge`), so we
/// don't flag them as orphan even when their worktree is gone.
const TRUNK_BRANCHES: &[&str] = &["dev", "main"];

/// Check #6: every local branch matching the `<type>/#<issue>-<desc>`
/// shape has a worktree pointing at it. A branch without a worktree was
/// likely created by `gwm create` and lost its worktree without a
/// `--delete-branch` — purely cosmetic dead weight, hence Warning not Failed.
///
/// Branches already fully merged into one of the trunk branches
/// (`dev`, `main`) are filtered out: keeping them is the project
/// convention, and surfacing them would make the check produce N false
/// positives on every successful release.
fn check_orphan_branches(ctx: &DoctorCtx<'_>) -> Check {
  let name = "no orphan gwm branches";

  let trees = match worktree::list(ctx.repo) {
    Ok(t) => t,
    Err(e) => return Check::failed(name, format!("could not list worktrees: {}", e)),
  };
  let claimed: BTreeSet<String> = trees.iter().filter_map(|w| w.branch.clone()).collect();

  // Resolve the trunk OIDs once. Missing trunks (e.g. a repo without `dev`)
  // are silently skipped — we only check against what exists.
  let trunk_oids: Vec<git2::Oid> = TRUNK_BRANCHES
    .iter()
    .filter_map(|t| {
      ctx
        .repo
        .find_branch(t, BranchType::Local)
        .ok()
        .and_then(|b| b.get().target())
    })
    .collect();

  let branches = match ctx.repo.branches(Some(BranchType::Local)) {
    Ok(b) => b,
    Err(e) => return Check::failed(name, format!("could not list local branches: {}", e)),
  };

  let mut orphans: Vec<String> = Vec::new();
  let mut merged_count: usize = 0;
  for entry in branches.flatten() {
    let (branch, _) = entry;
    let Ok(Some(branch_name)) = branch.name() else { continue };
    if parse_branch(branch_name).is_none() {
      continue; // user-managed branch, leave it alone
    }
    if claimed.contains(branch_name) {
      continue; // has a worktree — not orphan in any sense
    }
    let Some(branch_oid) = branch.get().target() else {
      continue;
    };
    match is_merged_into_any(ctx.repo, branch_oid, &trunk_oids) {
      Ok(true) => {
        merged_count += 1;
        continue; // preserved on purpose per CONTRIBUTING — not flagged
      }
      Ok(false) => {
        // Real orphan — fall through.
      }
      Err(e) => {
        // libgit2 couldn't walk the graph (missing objects, shallow
        // clone, repo corruption). Surface this loudly: silently
        // assuming "not merged" and recommending `git branch -d` would
        // be actively dangerous.
        return Check::failed(
          name,
          format!("could not determine merge status for {}: {}", branch_name, e),
        )
        .with_hint("check the repository integrity (`git fsck`) or re-fetch missing objects");
      }
    }
    orphans.push(branch_name.to_string());
  }

  if orphans.is_empty() {
    let detail = if merged_count == 0 {
      "every gwm-style branch has a matching worktree".to_string()
    } else {
      format!(
        "{} merged gwm-style branch(es) preserved per CONTRIBUTING, no unmerged orphans",
        merged_count
      )
    };
    return Check::ok(name, detail);
  }

  let suggestions: Vec<String> = orphans.iter().map(|b| format!("git branch -d {}", b)).collect();
  Check::warning(
    name,
    format!("{} unmerged orphan branch(es): {}", orphans.len(), orphans.join(", ")),
  )
  .with_hint(suggestions.join(" && "))
}

/// Returns `Ok(true)` iff `branch_oid` is fully reachable from at least
/// one of `trunks` — i.e. the branch is merged into one of the trunks
/// (or is equal to it). Implemented via libgit2's descendant check:
/// trunk is a descendant of the branch iff the branch is reachable
/// from trunk. Propagates `git2::Error` so callers can distinguish
/// "definitively unmerged" from "could not tell" — silently swallowing
/// the error would let a misclassification lead to a destructive
/// `git branch -d` suggestion.
fn is_merged_into_any(
  repo: &git2::Repository,
  branch_oid: git2::Oid,
  trunks: &[git2::Oid],
) -> std::result::Result<bool, git2::Error> {
  for trunk_oid in trunks {
    if *trunk_oid == branch_oid {
      return Ok(true);
    }
    if repo.graph_descendant_of(*trunk_oid, branch_oid)? {
      return Ok(true);
    }
  }
  Ok(false)
}

/// Probe a directory for write access by creating and deleting a unique
/// sentinel file. More reliable across platforms than parsing Unix mode
/// bits. Uses `tempfile::Builder` so concurrent `gwm doctor` runs don't
/// collide on a fixed filename, and so a SIGKILL mid-probe doesn't leak
/// a stray sentinel into the user's worktree base — `NamedTempFile`
/// RAII-cleans on drop.
fn is_writable_dir(dir: &Path) -> bool {
  tempfile::Builder::new()
    .prefix(".gwm-doctor-probe-")
    .rand_bytes(8)
    .tempfile_in(dir)
    .is_ok()
}
