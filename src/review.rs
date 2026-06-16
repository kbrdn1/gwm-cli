//! `gwm review <PR#>` (issue #308) — materialise an existing GitHub PR
//! into an isolated worktree.
//!
//! Every other worktree⇄GitHub path in gwm is **outbound** (it assumes
//! *you* authored a `<type>/#<issue>-<desc>` branch). This module covers
//! the **inbound** case: check out a teammate's PR — including one opened
//! from a fork — into a clean worktree to review / test / fix it.
//!
//! The fetch leans on GitHub's universal `refs/pull/<N>/head` ref, which
//! the **base** repo (`origin`) exposes for *every* PR regardless of
//! which fork the head lives on. That sidesteps resolving the
//! contributor's fork URL (and its credentials) entirely, and works for
//! open, draft, closed, and merged PRs alike. The mutating `git fetch`
//! shells out to the user's `git` for the same credential reason
//! [`crate::sync`] does; everything else (branch existence, worktree
//! attach, PR link) goes through libgit2.

use crate::bootstrap::{self, BootstrapCtx, BootstrapReport};
use crate::config::Config;
use crate::error::{GwmError, Result};
use crate::lifecycle::{self, HookContext, HookPhase, HookSkips};
use crate::naming::kebab;
use crate::{github, launcher, worktree};
use git2::Repository;
use std::path::{Path, PathBuf};

/// Derive a ref-/filesystem-safe slug from a PR head ref name. Only the
/// last path segment is kept (`alice/feat/spike-x` → `spike-x`) and then
/// kebab-cased, so a noisy contributor branch collapses to a short tail.
pub fn head_slug(head_ref: &str) -> String {
  let last = head_ref.rsplit('/').next().unwrap_or(head_ref);
  kebab(last)
}

/// Join `pr-<N>`, the kebab-cased author, and the kebab-cased slug with
/// `-`, skipping any empty segment so a missing author / slug never
/// produces a `--` run. The shared tail behind both the branch name and
/// the worktree directory name.
fn review_tail(number: u64, author: &str, slug: &str) -> String {
  let mut tail = format!("pr-{number}");
  let author = kebab(author);
  if !author.is_empty() {
    tail.push('-');
    tail.push_str(&author);
  }
  let slug = kebab(slug);
  if !slug.is_empty() {
    tail.push('-');
    tail.push_str(&slug);
  }
  tail
}

/// Local review branch name: `review/pr-<N>-<author>-<slug>`. Deliberately
/// *not* the `<type>/#<issue>-<desc>` shape — the branch isn't ours, and
/// the `review/` prefix keeps it out of the issue-number auto-link path.
pub fn review_branch_name(number: u64, author: &str, slug: &str) -> String {
  format!("review/{}", review_tail(number, author, slug))
}

/// Worktree directory name: `review-pr-<N>-<author>-<slug>`. Mirrors the
/// branch tail with `-` joins so `gwm path review-pr-<N>` fuzzy-resolves it.
pub fn review_dirname(number: u64, author: &str, slug: &str) -> String {
  format!("review-{}", review_tail(number, author, slug))
}

/// Derive a worktree directory name from an explicit `--name` branch
/// override: slashes (illegal in a path segment) collapse to dashes so a
/// `review/pr-9-x` override still lands in a flat `review-pr-9-x` dir.
pub fn dirname_from_branch(branch: &str) -> String {
  branch.replace('/', "-")
}

/// Fetch the PR's head commit into `branch` via origin's
/// `refs/pull/<N>/head` ref. The RHS is written as an explicit
/// `refs/heads/<branch>` so git never has to guess where a bare name
/// lands. Logged so the call surfaces in the Command Logs modal.
pub fn fetch_pr_head_ref(workdir: &Path, number: u64, branch: &str) -> Result<()> {
  let refspec = format!("pull/{number}/head:refs/heads/{branch}");
  worktree::run_git_logged(workdir, &["fetch", "origin", &refspec])?;
  Ok(())
}

/// Everything `gwm review` needs to know once the PR metadata is resolved.
#[derive(Debug, Clone)]
pub struct ReviewSpec<'a> {
  /// PR number — feeds the `refs/pull/<N>/head` fetch and the PR link.
  pub number: u64,
  /// Local review branch to create (`review/pr-<N>-<author>-<slug>`).
  pub branch: &'a str,
  /// Worktree directory name (`review-pr-<N>-<author>-<slug>`).
  pub dirname: &'a str,
  /// Absolute worktree path on disk.
  pub target: &'a Path,
  /// The diff base recorded as `branch.<name>.gwm-base` so the launcher's
  /// `{base}`/`{diff}` placeholders compare against the PR's merge target.
  /// Callers should pass a reliably resolvable ref — a remote-tracking
  /// `origin/<base>` rather than a bare local `<base>` that may be stale or
  /// absent. `None` falls back to the parent ref `worktree::add` records.
  pub base_ref: Option<&'a str>,
}

/// The behavioural seam: fetch the PR head, attach a worktree to it, link
/// the PR, and record the diff base. Does **not** bootstrap or run
/// lifecycle hooks — the CLI layer wraps those around this call so the
/// ordering matches `gwm create`.
///
/// Pre-flights both the local branch and the target directory *before*
/// the fetch, so the common "re-run after a half-finished attempt" path
/// fails cleanly instead of tripping over its own orphaned branch. As a
/// belt-and-suspenders, a fetched branch is deleted again if the worktree
/// attach fails downstream.
pub fn materialize(repo: &Repository, workdir: &Path, spec: &ReviewSpec) -> Result<PathBuf> {
  if repo.find_branch(spec.branch, git2::BranchType::Local).is_ok() {
    return Err(GwmError::Other(format!(
      "review branch '{}' already exists; remove the existing review worktree first (e.g. `gwm remove {} --delete-branch`)",
      spec.branch, spec.dirname
    )));
  }
  if spec.target.exists() {
    return Err(GwmError::WorktreeExists(
      spec.dirname.into(),
      spec.target.display().to_string(),
    ));
  }

  fetch_pr_head_ref(workdir, spec.number, spec.branch)?;

  // The fetch just minted `branch`; reuse it rather than branching from
  // HEAD. If the attach fails, drop the branch so a retry isn't blocked.
  let created = match worktree::add(repo, spec.dirname, spec.target, spec.branch, true) {
    Ok(path) => path,
    Err(e) => {
      if let Ok(mut b) = repo.find_branch(spec.branch, git2::BranchType::Local) {
        let _ = b.delete();
      }
      return Err(e);
    }
  };

  // Explicit link — a `review/…` branch matches neither the issue-number
  // pattern nor `gh pr list --head <branch>` (that keys on the PR's head
  // ref, not our local name), so auto-detection can't wire this up.
  github::link_pr(repo, spec.branch, spec.number)?;

  // Point the diff base at the PR's real base ref when we know it, so the
  // `R` review launcher diffs against the right merge target.
  if let Some(base) = spec.base_ref {
    let _ = launcher::write_gwm_base(repo, spec.branch, base);
  }

  Ok(created)
}

/// The ordered reports produced when `gwm review` runs setup against a
/// materialised worktree, mirroring `gwm create`'s bootstrap sequence.
#[derive(Debug)]
pub struct ReviewSetupReports {
  pub pre_bootstrap: BootstrapReport,
  pub bootstrap: BootstrapReport,
  pub post_bootstrap: BootstrapReport,
  pub post_create: BootstrapReport,
}

/// Run the post-materialise bootstrap + lifecycle-hook sequence — the steps
/// `gwm review` shares with `gwm create` — **only when `bootstrap` is true**.
///
/// This gate is a security boundary, not a convenience toggle. Unlike
/// `gwm create` (which sets up a branch off *your* HEAD), `gwm review`
/// materialises a worktree full of a contributor's PR code, possibly from an
/// untrusted fork. The bootstrap commands and the `pre_bootstrap` /
/// `post_bootstrap` / `post_create` hooks all run with their cwd inside that
/// worktree, so `npm install` (preinstall scripts), `composer install`,
/// `direnv allow` (the PR's `.envrc`), a `Makefile` target, etc. would
/// execute attacker-controlled code. The repo's own `.gwm.toml` being trusted
/// (the TOFU ledger) does not cover the PR-controlled files those commands
/// evaluate — so review is **safe-by-default**, and this whole sequence is
/// opt-in via `gwm review --bootstrap`. Returns `None` (nothing executed)
/// when `bootstrap` is false.
///
/// `ctx` must already point its cwd at the materialised worktree (build it
/// with `HookContext::with_cwd`).
pub fn run_post_setup(
  config: &Config,
  ctx: &HookContext,
  main_repo: &Path,
  worktree: &Path,
  skips: &HookSkips,
  bootstrap: bool,
) -> Result<Option<ReviewSetupReports>> {
  if !bootstrap {
    return Ok(None);
  }

  let pre_bootstrap = lifecycle::run_phase(config, HookPhase::PreBootstrap, ctx, skips, false)?;
  let bctx = BootstrapCtx {
    main_repo,
    worktree,
    config,
  };
  let bootstrap = bootstrap::run_core(&bctx)?;
  let post_bootstrap = lifecycle::run_phase(config, HookPhase::PostBootstrap, ctx, skips, false)?;
  // `true`: legacy `[[bootstrap.command]]` is folded into post_create when no
  // explicit hook block exists — matches `cmd_create`'s bootstrapped path.
  let post_create = lifecycle::run_phase(config, HookPhase::PostCreate, ctx, skips, true)?;

  Ok(Some(ReviewSetupReports {
    pre_bootstrap,
    bootstrap,
    post_bootstrap,
    post_create,
  }))
}
