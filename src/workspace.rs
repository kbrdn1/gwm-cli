//! Workspace mode (issue #36): a bird's-eye view across every git repo that
//! sits one level below a workspace root (e.g. `~/Projects`).
//!
//! `gwm` is single-repo by default. Workspace mode is an orthogonal
//! dimension layered on top: discover every direct-child git repo under a
//! root, then merge their worktree listings into one table where each row
//! remembers which repo it belongs to. `.gwm.toml` stays per-repo — there is
//! no workspace-level config in this version of the feature.

use crate::error::Result;
use crate::worktree::{self, WorktreeInfo};
use git2::Repository;
use std::path::{Path, PathBuf};

/// One git repo discovered directly under the workspace root.
#[derive(Debug, Clone)]
pub struct WorkspaceRepo {
  /// Display name — the repo directory's basename.
  pub name: String,
  /// The repo's working directory (a direct child of the workspace root).
  pub path: PathBuf,
}

/// The set of repos found under a workspace root.
#[derive(Debug, Clone)]
pub struct Workspace {
  /// The root the user pointed `--workspace` at.
  pub root: PathBuf,
  /// Direct-child git repos, sorted alphabetically by name.
  pub repos: Vec<WorkspaceRepo>,
}

impl Workspace {
  /// True when no git repo was found directly under the root.
  pub fn is_empty(&self) -> bool {
    self.repos.is_empty()
  }
}

/// A merged worktree row: the owning repo plus the worktree info itself.
#[derive(Debug, Clone)]
pub struct WorkspaceRow {
  /// Display name of the repo this worktree belongs to.
  pub repo_name: String,
  /// Working directory of the owning repo (the workspace child dir).
  pub repo_path: PathBuf,
  /// The per-worktree listing, identical to single-repo `worktree::list`.
  pub info: WorktreeInfo,
}

/// Walk one level deep under `root`, opening each direct-child directory as a
/// git repo. Non-directories and non-repo directories are ignored; nested
/// repos two levels down are *not* discovered (workspace mode is intentionally
/// shallow). Repos are returned sorted by name for a stable listing.
///
/// Errors if `root` cannot be read (missing / not a directory / no
/// permission). An existing but repo-free root is *not* an error — it yields
/// an empty [`Workspace`]; callers decide whether that is worth surfacing.
pub fn discover(root: &Path) -> Result<Workspace> {
  let entries = std::fs::read_dir(root)?;

  let mut repos: Vec<WorkspaceRepo> = Vec::new();
  for entry in entries.flatten() {
    let path = entry.path();
    if !path.is_dir() {
      continue;
    }
    // `Repository::open` (not `discover`) so a non-repo child can't make us
    // walk *up* and latch onto an unrelated ancestor repo: the child dir
    // itself must be the repo root.
    let Ok(repo) = Repository::open(&path) else {
      continue;
    };
    if repo.is_bare() {
      continue;
    }
    let name = path
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_else(|| "repo".into());
    repos.push(WorkspaceRepo { name, path });
  }

  repos.sort_by(|a, b| a.name.cmp(&b.name));
  Ok(Workspace {
    root: root.to_path_buf(),
    repos,
  })
}

/// Merge every repo's worktree listing into one flat, repo-tagged table.
///
/// Rows are grouped by repo in `workspace.repos` (alphabetical) order; within
/// a repo the order is `worktree::list`'s (main worktree first). A repo whose
/// listing fails (corrupt `.git`, transient git error) is skipped rather than
/// aborting the whole table — the bird's-eye view is best-effort across repos.
pub fn merge_worktrees(workspace: &Workspace) -> Result<Vec<WorkspaceRow>> {
  let mut rows: Vec<WorkspaceRow> = Vec::new();
  for repo in &workspace.repos {
    let Ok(handle) = Repository::open(&repo.path) else {
      continue;
    };
    let Ok(trees) = worktree::list(&handle) else {
      continue;
    };
    for info in trees {
      rows.push(WorkspaceRow {
        repo_name: repo.name.clone(),
        repo_path: repo.path.clone(),
        info,
      });
    }
  }
  Ok(rows)
}
