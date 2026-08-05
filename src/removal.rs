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

  let pre_ctx = HookContext::for_worktree(repo, workdir, &found.path, &found.path, found.branch.as_deref(), config);
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

  // Issue #29: capture the branch OID via libgit2 BEFORE the branch is
  // deleted, so `gwm undo` can resurrect it at the tip that was dropped.
  //
  // The removal itself decides *when*: `remove_verified_recording` calls back
  // at its point of no return, once the worktree is gone and before the
  // branch delete. Nothing between the last thing that can refuse and the
  // write, which is what a second `verify_path` here used to be — it narrowed
  // the window rather than closing it, and an entry that slipped through
  // wedges `gwm undo` for the whole repo until `history.toml` is edited by
  // hand (issue #531). The `HeadBranch` handed back is the observation the
  // deletion acts on, so the entry cannot name a different branch.
  let mut journal_warning = None;
  let removal = worktree::remove_verified_recording(repo, &found.id, expected_path, delete_branch, |head| {
    let entry = journal_entry(repo, found, head, delete_branch);
    if let Err(e) = history::record(entry) {
      journal_warning = Some(format!(
        "failed to record undo journal entry: {} (continuing with the removal anyway)",
        e
      ));
    }
  });
  outcome.journal_warning = journal_warning;
  if let Err(e) = removal {
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

/// The branch to record, from the observation the removal itself acted on.
///
/// Not `WorktreeInfo::branch`, which comes from a listing the caller may have
/// taken a while ago: a batch resolves its listing once, so a `pre_remove`
/// hook on an earlier target can check out another branch in a later one
/// without moving it, and recording the stale name would have `gwm undo`
/// restore a ref that was never deleted while the deleted one stayed lost
/// (Codex review on PR #526).
///
/// The three answers stay distinct. `Detached` records no branch, because the
/// removal deleted none and `gwm undo` refuses such an entry rather than
/// re-attaching the worktree to a branch it was never on. `Unreadable` is the
/// only case that falls back to the listing: nothing was observed, so the
/// caller's older name is better than no name at all — it is what `gwm undo`
/// re-attaches to, and no branch was deleted for it to contradict.
fn branch_to_record(found: &WorktreeInfo, head: &worktree::HeadBranch) -> Option<String> {
  match head {
    worktree::HeadBranch::Attached(b) => Some(b.clone()),
    worktree::HeadBranch::Detached => None,
    worktree::HeadBranch::Unreadable => found.branch.clone(),
  }
}

/// The undo-journal entry for a removal that has just passed its point of no
/// return, built from the same `head` the removal acted on.
fn journal_entry(repo: &Repository, found: &WorktreeInfo, head: &worktree::HeadBranch, delete_branch: bool) -> OpEntry {
  let branch = branch_to_record(found, head);
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
