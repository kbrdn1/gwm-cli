//! `gwm sync` (issue #24) — fetch + rebase / merge a worktree's branch
//! onto its configured upstream.
//!
//! The read-side inspection (dirty check, upstream resolution,
//! ahead/behind) goes through libgit2; the mutating steps (`fetch`,
//! `rebase`, `merge`) shell out to the `git` binary. That split is
//! deliberate: libgit2's fetch needs the caller to wire credential
//! callbacks (SSH agents, tokens, helpers) to talk to a real remote,
//! whereas the user's `git` already has all of that configured. The
//! existing sidebar previews (`git_log_oneline`, `git_status_short`)
//! shell out for the same reason, so this stays consistent.

use crate::error::{GwmError, Result};
use crate::worktree;
use git2::{BranchType, Repository};
use std::path::Path;
use std::process::Command;

/// How `gwm sync` reconciles the local branch when it is behind its
/// upstream. Defaults to rebase (linear history, the repo convention);
/// `--merge` opts into a merge commit instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStrategy {
  Rebase,
  Merge,
}

impl SyncStrategy {
  /// The `git` subcommand verb (`rebase` / `merge`).
  fn verb(self) -> &'static str {
    match self {
      SyncStrategy::Rebase => "rebase",
      SyncStrategy::Merge => "merge",
    }
  }
}

/// What `sync` actually did once preconditions passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
  /// The branch was already level with (or ahead of) upstream — no
  /// integration was needed.
  UpToDate,
  /// `behind_before` upstream commits were integrated via the chosen
  /// strategy.
  Integrated,
}

/// Outcome of a successful `sync` run. Conflicts, dirty trees, and
/// missing upstreams surface as `GwmError` instead — only the
/// non-error paths produce a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
  /// Local branch shorthand that was synced (e.g. `feat/#24-sync`).
  pub branch: String,
  /// Upstream tracking ref shorthand (e.g. `origin/main`).
  pub upstream: String,
  /// Strategy used (rebase / merge).
  pub strategy: SyncStrategy,
  /// Commits the local branch had that upstream did not, measured
  /// before integration.
  pub ahead_before: usize,
  /// Commits upstream had that the local branch did not, measured
  /// after the fetch but before integration.
  pub behind_before: usize,
  /// What happened.
  pub action: SyncAction,
}

/// Fetch `start`'s upstream, then rebase (or merge) the local branch
/// onto it. `start` may be any path inside the target worktree — the
/// repository is discovered upwards from it.
///
/// Refuses up front when:
/// - the working tree is dirty (uncommitted changes),
/// - HEAD is detached / unborn (no branch shorthand),
/// - the branch has no upstream configured.
///
/// On a conflicting rebase/merge the operation is aborted so the
/// worktree is left usable, and a conflict error is returned telling
/// the user to reconcile by hand.
pub fn sync(start: &Path, strategy: SyncStrategy) -> Result<SyncReport> {
  let repo = Repository::discover(start).map_err(|_| GwmError::NotInGitRepo)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();

  // 1. Refuse to touch a dirty tree — a rebase/merge on top of
  //    uncommitted work is how people lose changes.
  if worktree::is_dirty(&repo)? {
    return Err(GwmError::Other(
      "worktree has uncommitted changes; commit or stash before syncing".into(),
    ));
  }

  // 2. Resolve the current branch and its upstream.
  let head = repo.head().map_err(|_| GwmError::UnbornHead {
    reason: "sync: cannot read HEAD (unborn or unreadable)".into(),
  })?;
  if !head.is_branch() {
    return Err(GwmError::UnbornHead {
      reason: "sync: HEAD is detached — check out a branch first".into(),
    });
  }
  let branch_short = head
    .shorthand()
    .ok_or_else(|| GwmError::UnbornHead {
      reason: "sync: HEAD has no branch name".into(),
    })?
    .to_string();
  let head_refname = head.name().map(|s| s.to_string());

  let local = repo
    .find_branch(&branch_short, BranchType::Local)
    .map_err(|_| GwmError::Other(format!("sync: local branch '{branch_short}' not found")))?;
  let upstream = local.upstream().map_err(|_| {
    GwmError::Other(format!(
      "branch '{branch_short}' has no upstream configured; set one with `git branch --set-upstream-to=<remote>/{branch_short}`"
    ))
  })?;
  let upstream_short = upstream
    .name()
    .ok()
    .flatten()
    .ok_or_else(|| GwmError::Other("sync: upstream tracking ref has no name".into()))?
    .to_string();

  // The remote to fetch. `branch_upstream_remote` wants the full
  // refname (`refs/heads/<branch>`). If the upstream is a local
  // branch (no remote), fall back to a bare `git fetch`.
  let remote = head_refname
    .as_deref()
    .and_then(|rn| repo.branch_upstream_remote(rn).ok())
    .and_then(|buf| buf.as_str().map(|s| s.to_string()));

  // 3. Fetch. After this the in-memory `repo` ref cache is stale, so
  //    everything past here re-resolves against a freshly opened repo.
  match &remote {
    Some(r) => run_git(&workdir, &["fetch", r])?,
    None => run_git(&workdir, &["fetch"])?,
  };

  // 4. Recompute ahead/behind against the now-updated upstream.
  let repo = Repository::discover(start).map_err(|_| GwmError::NotInGitRepo)?;
  let (ahead_before, behind_before) = ahead_behind(&repo, &branch_short)?;

  if behind_before == 0 {
    return Ok(SyncReport {
      branch: branch_short,
      upstream: upstream_short,
      strategy,
      ahead_before,
      behind_before,
      action: SyncAction::UpToDate,
    });
  }

  // 5. Integrate. On failure (conflicts), abort so the worktree is
  //    not left mid-rebase/merge, then surface a conflict error.
  let integrate = match strategy {
    SyncStrategy::Rebase => run_git(&workdir, &["rebase", &upstream_short]),
    SyncStrategy::Merge => run_git(&workdir, &["merge", "--no-edit", &upstream_short]),
  };
  if let Err(e) = integrate {
    // Distinguish a genuine conflict from any other failure (a failing
    // hook, a missing committer identity, a strategy/config error) by
    // inspecting the index for conflict stages BEFORE aborting —
    // language-independent, unlike grepping git's output. Either way we
    // abort so the worktree is left usable.
    let conflicted = Repository::discover(start)
      .ok()
      .and_then(|r| r.index().ok())
      .map(|idx| idx.has_conflicts())
      .unwrap_or(false);
    let _ = run_git(&workdir, &[strategy.verb(), "--abort"]);
    if conflicted {
      return Err(GwmError::Other(format!(
        "{} onto {} hit conflicts and was aborted; reconcile manually with `git {} {}`",
        strategy.verb(),
        upstream_short,
        strategy.verb(),
        upstream_short
      )));
    }
    // Not a conflict — surface the underlying git failure verbatim so
    // the user isn't sent down the wrong recovery path.
    return Err(GwmError::Other(format!(
      "git {} onto {} failed and was aborted: {}",
      strategy.verb(),
      upstream_short,
      e
    )));
  }

  Ok(SyncReport {
    branch: branch_short,
    upstream: upstream_short,
    strategy,
    ahead_before,
    behind_before,
    action: SyncAction::Integrated,
  })
}

/// Ahead / behind counts of `branch` versus its upstream, resolved
/// fresh from disk. Returns `(ahead, behind)`.
fn ahead_behind(repo: &Repository, branch: &str) -> Result<(usize, usize)> {
  let local = repo
    .find_branch(branch, BranchType::Local)
    .map_err(|_| GwmError::Other(format!("sync: local branch '{branch}' not found")))?;
  let upstream = local
    .upstream()
    .map_err(|_| GwmError::Other(format!("branch '{branch}' has no upstream configured")))?;
  let local_oid = local
    .get()
    .target()
    .ok_or_else(|| GwmError::Other(format!("sync: branch '{branch}' has no commit")))?;
  let up_oid = upstream
    .get()
    .target()
    .ok_or_else(|| GwmError::Other("sync: upstream has no commit".into()))?;
  let (ahead, behind) = repo.graph_ahead_behind(local_oid, up_oid)?;
  Ok((ahead, behind))
}

/// Run `git -C <dir> <args>`, returning stdout on success or a
/// `CommandFailed` error carrying the verb and stderr on failure.
fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .map_err(|e| GwmError::CommandFailed(format!("git {} failed to spawn: {}", args.join(" "), e)))?;
  if !out.status.success() {
    return Err(GwmError::CommandFailed(format!(
      "git {} exited {}: {}",
      args.join(" "),
      out.status,
      String::from_utf8_lossy(&out.stderr).trim()
    )));
  }
  Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
