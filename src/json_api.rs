//! Stable, machine-readable JSON surface shared by the `--format=json`
//! CLI flags (issue #38, phase 1) and the daemon's JSON-RPC methods
//! (phase 2).
//!
//! The DTOs here are deliberately decoupled from the internal
//! [`crate::worktree::WorktreeInfo`] / [`crate::doctor::DoctorReport`]
//! types. Those structs carry TUI-runtime baggage (loaded GitHub issue /
//! PR state, cached branch age as a `Duration`, the `BranchLink` graph)
//! whose shape churns as the TUI evolves. Pinning the documented schema
//! (see `docs/schema/`) to a dedicated set of `Serialize` DTOs means a
//! refactor of `WorktreeInfo` can't silently break a downstream editor
//! plugin. Conversions are one-directional (`From<&Internal>`); the JSON
//! surface is output-only.
//!
//! Key convention: `snake_case`, matching the hand-built
//! `print_status_json` in [`crate::cli`].

use crate::doctor::{CheckStatus, DoctorReport};
use crate::error::Result;
use crate::worktree::{self, BranchStatus, WorktreeInfo};
use serde::Serialize;

/// Working-tree + upstream status, the stable projection of
/// [`BranchStatus`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonStatus {
  pub is_dirty: bool,
  pub has_upstream: bool,
  pub ahead: usize,
  pub behind: usize,
  /// Status couldn't be computed (detached HEAD, unborn branch).
  pub unknown: bool,
}

impl From<&BranchStatus> for JsonStatus {
  fn from(s: &BranchStatus) -> Self {
    Self {
      is_dirty: s.is_dirty,
      has_upstream: s.has_upstream,
      ahead: s.ahead,
      behind: s.behind,
      unknown: s.unknown,
    }
  }
}

/// One worktree as exposed to scripting / editor integrations. Mirrors
/// the columns of `gwm list` plus the machine-only fields a consumer
/// needs (absolute `path`, raw `age_seconds`, linked issue/PR numbers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonWorktree {
  /// Display name — the basename of the worktree directory.
  pub name: String,
  /// Internal git worktree id (`.git/worktrees/<id>`); diverges from
  /// `name` after a `git worktree move`.
  pub id: String,
  /// Absolute path to the worktree working directory.
  pub path: String,
  pub branch: Option<String>,
  /// Short HEAD oid, when resolvable.
  pub head: Option<String>,
  pub is_main: bool,
  pub is_locked: bool,
  pub is_prunable: bool,
  pub status: JsonStatus,
  /// Branch age relative to the trunk baseline, in whole seconds.
  /// `null` for trunk branches and unresolvable repos.
  pub age_seconds: Option<u64>,
  /// Linked issue number (branch-name inferred or explicit), if any.
  pub issue: Option<u64>,
  /// Linked PR number (inferred, explicit, or auto-detected), if any.
  pub pr: Option<u64>,
}

impl From<&WorktreeInfo> for JsonWorktree {
  fn from(w: &WorktreeInfo) -> Self {
    Self {
      name: w.name.clone(),
      id: w.id.clone(),
      path: w.path.to_string_lossy().into_owned(),
      branch: w.branch.clone(),
      head: w.head.clone(),
      is_main: w.is_main,
      is_locked: w.is_locked,
      is_prunable: w.is_prunable,
      status: JsonStatus::from(&w.status),
      age_seconds: w.age.map(|d| d.as_secs()),
      issue: w.link.issue,
      pr: w.link.pr,
    }
  }
}

/// The `{ name, path, branch }` triple returned by `gwm path --format=json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonPath {
  pub name: String,
  pub path: String,
  pub branch: Option<String>,
}

impl From<&WorktreeInfo> for JsonPath {
  fn from(w: &WorktreeInfo) -> Self {
    Self {
      name: w.name.clone(),
      path: w.path.to_string_lossy().into_owned(),
      branch: w.branch.clone(),
    }
  }
}

/// Stable lowercase string for a [`CheckStatus`], used as the `status`
/// field of [`JsonCheck`] and the `severity` of [`JsonDoctorReport`].
pub fn check_status_str(status: &CheckStatus) -> &'static str {
  match status {
    CheckStatus::Ok => "ok",
    CheckStatus::Warning => "warning",
    CheckStatus::Failed => "failed",
  }
}

/// One diagnostic check, the stable projection of [`crate::doctor::Check`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonCheck {
  pub name: String,
  /// `"ok"`, `"warning"`, or `"failed"`.
  pub status: String,
  pub detail: String,
  pub fix_hint: Option<String>,
}

/// A full doctor run, carrying the per-check list plus the aggregate
/// `severity` and the process `exit_code` (`0`/`1`/`2`) so a consumer
/// doesn't have to re-derive them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonDoctorReport {
  pub checks: Vec<JsonCheck>,
  /// Highest severity present: `"ok"`, `"warning"`, or `"failed"`.
  pub severity: String,
  pub exit_code: i32,
}

impl From<&DoctorReport> for JsonDoctorReport {
  fn from(r: &DoctorReport) -> Self {
    Self {
      checks: r
        .checks
        .iter()
        .map(|c| JsonCheck {
          name: c.name.clone(),
          status: check_status_str(&c.status).to_string(),
          detail: c.detail.clone(),
          fix_hint: c.fix_hint.clone(),
        })
        .collect(),
      severity: check_status_str(&r.severity()).to_string(),
      exit_code: r.exit_code(),
    }
  }
}

/// Build the stable JSON worktree list for `repo`. Shared by
/// `gwm list --format=json` and the daemon's `list` RPC method so both
/// surfaces stay byte-identical.
pub fn worktrees(repo: &git2::Repository) -> Result<Vec<JsonWorktree>> {
  Ok(worktree::list(repo)?.iter().map(JsonWorktree::from).collect())
}
