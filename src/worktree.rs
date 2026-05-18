use crate::error::{GwmError, Result};
use git2::{BranchType, Repository, StatusOptions, WorktreeAddOptions, WorktreePruneOptions};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
  pub name: String,
  pub path: PathBuf,
  pub branch: Option<String>,
  pub head: Option<String>,
  pub is_main: bool,
  pub is_locked: bool,
  pub is_prunable: bool,
  pub status: BranchStatus,
}

/// Cheap snapshot of "where are we vs. clean / upstream".
#[derive(Debug, Clone, Default)]
pub struct BranchStatus {
  /// At least one tracked / untracked change in the work tree or index.
  pub is_dirty: bool,
  /// Upstream is configured for the current branch.
  pub has_upstream: bool,
  /// Commits on local not on upstream.
  pub ahead: usize,
  /// Commits on upstream not on local.
  pub behind: usize,
  /// Status couldn't be computed (e.g. detached HEAD, unborn branch).
  pub unknown: bool,
}

impl BranchStatus {
  pub fn synced(&self) -> bool {
    self.has_upstream && self.ahead == 0 && self.behind == 0
  }
}

/// Compute the working-tree + upstream status of a single repo / linked worktree.
fn compute_status(repo: &Repository) -> BranchStatus {
  let mut out = BranchStatus::default();

  // Dirty check
  let mut opts = StatusOptions::new();
  opts
    .include_untracked(true)
    .include_ignored(false)
    .recurse_untracked_dirs(true);
  match repo.statuses(Some(&mut opts)) {
    Ok(s) => out.is_dirty = !s.is_empty(),
    Err(_) => out.unknown = true,
  }

  // Ahead / behind vs upstream
  if let Ok(head_ref) = repo.head() {
    if let Some(shorthand) = head_ref.shorthand() {
      if let Ok(local_branch) = repo.find_branch(shorthand, BranchType::Local) {
        if let Ok(upstream) = local_branch.upstream() {
          if let (Some(local_oid), Some(up_oid)) = (head_ref.target(), upstream.into_reference().target()) {
            out.has_upstream = true;
            if let Ok((ahead, behind)) = repo.graph_ahead_behind(local_oid, up_oid) {
              out.ahead = ahead;
              out.behind = behind;
            }
          }
        }
      }
    }
  }

  out
}

/// Find the main repository starting from CWD, walking upwards.
pub fn discover_repo(start: Option<&Path>) -> Result<Repository> {
  let from = match start {
    Some(p) => p.to_path_buf(),
    None => std::env::current_dir()?,
  };
  let repo = Repository::discover(&from).map_err(|_| GwmError::NotInGitRepo)?;
  // If we're inside a linked worktree, walk back to the main repo working dir.
  // `repo.path()` for a linked worktree returns `<main>/.git/worktrees/<name>/`.
  // Two parents up = `<main>/.git`, three up = `<main>` (the main workdir).
  if repo.is_worktree() {
    let wt_admin = repo.path().to_path_buf();
    if let Some(git_dir) = wt_admin.parent().and_then(|p| p.parent()) {
      if let Some(main_workdir) = git_dir.parent() {
        if let Ok(main) = Repository::open(main_workdir) {
          return Ok(main);
        }
      }
    }
  }
  Ok(repo)
}

/// Name of the repo derived from the working dir path.
pub fn repo_name(repo: &Repository) -> String {
  repo
    .workdir()
    .and_then(|p| p.file_name())
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_else(|| "repo".into())
}

pub fn list(repo: &Repository) -> Result<Vec<WorktreeInfo>> {
  let mut out = Vec::new();

  // The main worktree is not listed by git2::Repository::worktrees(); add it manually.
  if let Some(workdir) = repo.workdir() {
    let head_ref = repo.head().ok();
    let branch = head_ref.as_ref().and_then(|r| r.shorthand().map(|s| s.to_string()));
    let head = head_ref.as_ref().and_then(|r| r.target().map(|o| o.to_string()));
    out.push(WorktreeInfo {
      name: workdir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".into()),
      path: workdir.to_path_buf(),
      branch,
      head,
      is_main: true,
      is_locked: false,
      is_prunable: false,
      status: compute_status(repo),
    });
  }

  let names = repo.worktrees()?;
  for name in names.iter().flatten() {
    let wt = match repo.find_worktree(name) {
      Ok(w) => w,
      Err(_) => continue,
    };
    let path = wt.path().to_path_buf();
    let is_locked = matches!(wt.is_locked(), Ok(git2::WorktreeLockStatus::Locked(_)));
    let is_prunable = matches!(wt.is_prunable(None), Ok(p) if p);

    // Open the worktree as a repo to read its HEAD + status.
    let (branch, head, status) = match Repository::open(&path) {
      Ok(sub) => {
        let head_ref = sub.head().ok();
        let b = head_ref.as_ref().and_then(|r| r.shorthand().map(|s| s.to_string()));
        let h = head_ref.as_ref().and_then(|r| r.target().map(|o| o.to_string()));
        let s = compute_status(&sub);
        (b, h, s)
      }
      Err(_) => (
        None,
        None,
        BranchStatus {
          unknown: true,
          ..Default::default()
        },
      ),
    };

    out.push(WorktreeInfo {
      name: name.to_string(),
      path,
      branch,
      head,
      is_main: false,
      is_locked,
      is_prunable,
      status,
    });
  }

  Ok(out)
}

/// Create a new worktree with a brand-new branch off of HEAD.
pub fn add(repo: &Repository, name: &str, target_path: &Path, branch_name: &str) -> Result<PathBuf> {
  // Refuse to clobber an existing directory.
  if target_path.exists() {
    return Err(GwmError::WorktreeExists(name.into(), target_path.display().to_string()));
  }

  // Ensure parent dir exists.
  if let Some(parent) = target_path.parent() {
    std::fs::create_dir_all(parent)?;
  }

  // Create branch ref if it doesn't already exist.
  let head_commit = repo.head()?.peel_to_commit()?;
  let branch = match repo.find_branch(branch_name, git2::BranchType::Local) {
    Ok(b) => b,
    Err(_) => repo.branch(branch_name, &head_commit, false)?,
  };
  let reference = branch.into_reference();

  let mut opts = WorktreeAddOptions::new();
  opts.reference(Some(&reference));

  repo.worktree(name, target_path, Some(&opts))?;
  Ok(target_path.to_path_buf())
}

/// Remove a worktree directory and prune its admin files. Optionally delete the branch.
pub fn remove(repo: &Repository, name: &str, delete_branch: bool) -> Result<()> {
  let wt = repo
    .find_worktree(name)
    .map_err(|_| GwmError::WorktreeNotFound(name.into()))?;
  let path = wt.path().to_path_buf();

  // Capture the branch (if any) so we can drop it after pruning.
  let branch_name = match Repository::open(&path) {
    Ok(sub) => sub.head().ok().and_then(|r| r.shorthand().map(|s| s.to_string())),
    Err(_) => None,
  };

  // Physical removal — git2's prune does NOT delete the work tree directory itself.
  if path.exists() {
    std::fs::remove_dir_all(&path)?;
  }

  // Force prune (admin files in .git/worktrees/<name>).
  let mut opts = WorktreePruneOptions::new();
  opts.valid(true).locked(true).working_tree(true);
  wt.prune(Some(&mut opts))?;

  if delete_branch {
    if let Some(b) = branch_name {
      if let Ok(mut branch) = repo.find_branch(&b, git2::BranchType::Local) {
        let _ = branch.delete();
      }
    }
  }

  Ok(())
}

/// Prune stale worktree admin entries (gwq cleanup equivalent).
pub fn prune(repo: &Repository) -> Result<usize> {
  let names = repo.worktrees()?;
  let mut pruned = 0usize;
  for name in names.iter().flatten() {
    let wt = match repo.find_worktree(name) {
      Ok(w) => w,
      Err(_) => continue,
    };
    let prunable = matches!(wt.is_prunable(None), Ok(p) if p);
    if !prunable {
      continue;
    }
    let mut opts = WorktreePruneOptions::new();
    opts.valid(true).locked(true).working_tree(true);
    if wt.prune(Some(&mut opts)).is_ok() {
      pruned += 1;
    }
  }
  Ok(pruned)
}

/// Resolve a worktree by exact name first, then by substring (case-insensitive) within the dir name.
pub fn find_fuzzy(repo: &Repository, pattern: &str) -> Result<WorktreeInfo> {
  let all = list(repo)?;
  if let Some(exact) = all.iter().find(|w| w.name == pattern && !w.is_main) {
    return Ok(exact.clone());
  }
  let pat = pattern.to_lowercase();
  let mut matches: Vec<&WorktreeInfo> = all
    .iter()
    .filter(|w| !w.is_main && w.name.to_lowercase().contains(&pat))
    .collect();
  match matches.len() {
    0 => Err(GwmError::WorktreeNotFound(pattern.into())),
    1 => Ok(matches.remove(0).clone()),
    _ => Err(GwmError::Other(format!(
      "pattern '{}' is ambiguous, candidates: {}",
      pattern,
      matches.iter().map(|w| w.name.as_str()).collect::<Vec<_>>().join(", ")
    ))),
  }
}
