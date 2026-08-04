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
  // The entry is written after the removal succeeds, so a refused removal
  // never shows up in `gwm history` as something to undo.
  let entry = journal_entry(repo, found, delete_branch);

  if let Err(e) = worktree::remove_verified(repo, &found.id, expected_path, delete_branch) {
    return Err(Box::new(RemovalFailure { outcome, error: e }));
  }
  outcome.removed = true;
  if let Err(e) = history::record(entry) {
    outcome.journal_warning = Some(format!(
      "failed to record undo journal entry: {} (the worktree is still removed)",
      e
    ));
  }

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

/// The undo-journal entry for a removal that is about to happen.
fn journal_entry(repo: &Repository, found: &WorktreeInfo, delete_branch: bool) -> OpEntry {
  let branch_oid = found.branch.as_deref().and_then(|b| {
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
    branch: found.branch.clone(),
    branch_oid,
    path: found.path.clone(),
    deleted_branch: delete_branch,
    repo_root,
    undone: false,
  }
}
