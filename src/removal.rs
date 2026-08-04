//! The one removal sequence: `pre_remove` hooks, undo-journal entry,
//! destruction, `post_remove` hooks.
//!
//! Issue #521. It used to live inside `cli::remove_one`, which meant the TUI
//! `d` — calling `worktree::remove` on its own — ran no hooks and recorded
//! nothing, so an interactive delete was unrecoverable and a `pre_remove`
//! guard only held for whoever used the CLI.
//!
//! Nothing here prints. The CLI renders the returned reports, and the TUI,
//! which owns the alternate screen, renders a summary in its status bar; the
//! hooks' own output is captured by `command_log::run_logged` either way, so
//! it reaches the TUI's Command Logs modal (`3`) rather than the screen.

use crate::bootstrap::BootstrapReport;
use crate::config::Config;
use crate::error::GwmError;
use crate::history::{self, OpEntry, OpKind};
use crate::lifecycle::{self, HookContext, HookPhase, HookSkips};
use crate::worktree::{self, WorktreeInfo};
use git2::Repository;
use std::path::Path;

/// Everything a removal produced that a caller may want to show. Populated
/// as the sequence advances, so a failure hands back what ran before it.
#[derive(Debug)]
pub struct RemovalOutcome {
  /// Steps run by the `pre_remove` phase.
  pub pre: BootstrapReport,
  /// `true` once the worktree is actually gone.
  pub removed: bool,
  /// The journal write failed and was swallowed. Losing recoverability is
  /// unfortunate; failing a destruction the user asked for because
  /// `history.toml` is unwritable would be worse.
  pub journal_warning: Option<String>,
  /// Steps run by the `post_remove` phase.
  pub post: BootstrapReport,
}

impl Default for RemovalOutcome {
  fn default() -> Self {
    Self {
      pre: BootstrapReport { steps: Vec::new() },
      removed: false,
      journal_warning: None,
      post: BootstrapReport { steps: Vec::new() },
    }
  }
}

/// A removal that stopped somewhere, with what it had done up to that point.
#[derive(Debug)]
pub struct RemovalFailure {
  pub outcome: RemovalOutcome,
  pub error: GwmError,
}

impl From<Box<RemovalFailure>> for GwmError {
  fn from(f: Box<RemovalFailure>) -> Self {
    f.error
  }
}

/// Remove one resolved worktree through the full lifecycle.
///
/// `expected_path` is what the caller committed to destroying — the path
/// printed by `--dry-run`, or the one the TUI confirm overlay displayed. It
/// is deliberately a separate argument from `found.path`: a worktree id is
/// just the `.git/worktrees/<id>` entry name, and git hands it back to
/// whoever recreates a worktree with that basename, so a caller that
/// re-resolves `found` between the decision and the call (the TUI worker
/// does, to build the journal entry) must still pass the path it committed
/// to. Deriving `expected_path` from a fresh `found` compares live state
/// with itself and can never refuse.
pub fn remove_with_lifecycle(
  repo: &Repository,
  workdir: &Path,
  config: &Config,
  skips: &HookSkips,
  found: &WorktreeInfo,
  expected_path: &Path,
  delete_branch: bool,
) -> std::result::Result<RemovalOutcome, Box<RemovalFailure>> {
  let mut outcome = RemovalOutcome::default();

  // Before anything runs. A caller that re-resolves its target between the
  // decision and this call (the TUI worker, to name the branch in the journal
  // entry) hands us a `found` describing where the worktree is NOW, and the
  // `pre_remove` hook runs with its cwd there. Leaving the check to
  // `remove_verified` meant a destructive hook had already executed against a
  // directory the user never confirmed (Codex review on PR #526).
  if let Err(e) = worktree::verify_path(repo, &found.id, expected_path) {
    return Err(Box::new(RemovalFailure { outcome, error: e }));
  }

  let pre_ctx = HookContext::for_worktree(repo, workdir, &found.path, &found.path, found.branch.as_deref());
  match lifecycle::run_phase_quiet(config, HookPhase::PreRemove, &pre_ctx, skips, false) {
    Ok(report) => outcome.pre = report,
    Err(f) => {
      outcome.pre = f.report;
      return Err(Box::new(RemovalFailure {
        outcome,
        error: f.error,
      }));
    }
  }

  // Issue #29: capture the branch OID via libgit2 BEFORE the destructive
  // call so `gwm undo` can resurrect the branch at the tip that was deleted.
  //
  // Written here, after every refusal has had its say and before the first
  // destructive effect. `worktree::remove` prunes the admin entry BEFORE
  // deleting the directory (#98), so a filesystem failure mid-removal leaves
  // the repo mutated, and with `--delete-branch` the branch tip gone: writing
  // the entry afterwards would leave that case with no recovery anchor at all
  // (Codex review on PR #526). Everything that refuses without mutating
  // anything — the path check above, a `pre_remove` hook, the trust gate —
  // happens earlier, so a refusal still records nothing.
  // Checked again, because the hooks just ran and one of them can move the
  // very worktree it was given: the advance check above passed, so without
  // this the entry would claim a removal `remove_verified` is about to refuse,
  // and `gwm undo` would pop that instead of a recoverable one (Codex review
  // on PR #526).
  if let Err(e) = worktree::verify_path(repo, &found.id, expected_path) {
    return Err(Box::new(RemovalFailure { outcome, error: e }));
  }

  let entry = journal_entry(repo, found, delete_branch);
  if let Err(e) = history::record(entry) {
    outcome.journal_warning = Some(format!(
      "failed to record undo journal entry: {} (continuing with the removal anyway)",
      e
    ));
  }

  if let Err(e) = worktree::remove_verified(repo, &found.id, expected_path, delete_branch) {
    return Err(Box::new(RemovalFailure { outcome, error: e }));
  }
  outcome.removed = true;

  let post_ctx = pre_ctx.with_cwd(workdir);
  match lifecycle::run_phase_quiet(config, HookPhase::PostRemove, &post_ctx, skips, false) {
    Ok(report) => outcome.post = report,
    Err(f) => {
      outcome.post = f.report;
      return Err(Box::new(RemovalFailure {
        outcome,
        error: f.error,
      }));
    }
  }

  Ok(outcome)
}

/// The branch a removal will actually delete: the one checked out in the
/// worktree right now, which is what `worktree::remove` reads.
///
/// Not `WorktreeInfo::branch`, which comes from a listing the caller may have
/// taken a while ago. A batch resolves its listing once, so a `pre_remove`
/// hook on an earlier target can check out another branch in a later one
/// without moving it; recording the stale name would have `gwm undo` restore
/// a ref that was never deleted while the deleted one stayed lost (Codex
/// review on PR #526). Falls back to the listing when the worktree cannot be
/// opened, and yields `None` on a detached HEAD, matching what the removal
/// does with it.
fn branch_at_removal_time(found: &WorktreeInfo) -> Option<String> {
  let live = Repository::open(&found.path).ok().and_then(|wt_repo| {
    let head = wt_repo.head().ok()?;
    if !head.is_branch() {
      return None;
    }
    head.shorthand().ok().map(String::from)
  });
  live.or_else(|| found.branch.clone())
}

/// The undo-journal entry for a removal that is about to happen.
fn journal_entry(repo: &Repository, found: &WorktreeInfo, delete_branch: bool) -> OpEntry {
  let branch = branch_at_removal_time(found);
  let branch_oid = branch.as_deref().and_then(|b| {
    repo
      .find_branch(b, git2::BranchType::Local)
      .ok()
      .and_then(|br| br.into_reference().target())
      .map(|o| o.to_string())
  });
  let repo_root = repo
    .workdir()
    .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
    .unwrap_or_default();
  OpEntry {
    ts: chrono::Utc::now(),
    kind: OpKind::Remove,
    worktree: found.name.clone(),
    branch,
    branch_oid,
    path: found.path.clone(),
    deleted_branch: delete_branch,
    repo_root,
    undone: false,
  }
}
