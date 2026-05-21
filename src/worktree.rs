use crate::error::{GwmError, Result};
use crate::github::{self, BranchLink};
use git2::{BranchType, Repository, StatusOptions, WorktreeAddOptions, WorktreePruneOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Trunk branches treated as "merge destinations" when measuring how
/// long a branch has been alive. Order matters: the first match wins,
/// so `main` (modern default) beats `master` (legacy) beats `dev` (gwm
/// convention). Hardcoded here because `branch_age` is also reachable
/// from contexts that don't carry a `Config` (CLI smoke paths).
const TRUNK_CANDIDATES: &[&str] = &["main", "master", "dev"];

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
  /// Issue/PR link resolved at list time, so the table marker column
  /// can show `●` on rows that carry GitHub context without each frame
  /// re-shelling `git config`. Empty link = no marker dot. See
  /// `tui/ui.rs::table_marker`.
  pub link: BranchLink,
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
    let link = branch
      .as_deref()
      .and_then(|b| github::read_link(repo, b).ok())
      .unwrap_or_else(BranchLink::empty);
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
      link,
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

    let link = branch
      .as_deref()
      .and_then(|b| github::read_link(repo, b).ok())
      .unwrap_or_else(BranchLink::empty);
    out.push(WorktreeInfo {
      name: name.to_string(),
      path,
      branch,
      head,
      is_main: false,
      is_locked,
      is_prunable,
      status,
      link,
    });
  }

  Ok(out)
}

/// Create a new worktree with a brand-new branch off of HEAD.
///
/// Records the HEAD ref's short name into `branch.<branch_name>.gwm-base`
/// so the review launcher (issue #75) can recover the original parent
/// ref later — even on branches without an upstream. The write is
/// best-effort: a config-write error does not roll the worktree back.
pub fn add(repo: &Repository, name: &str, target_path: &Path, branch_name: &str) -> Result<PathBuf> {
  // Refuse to clobber an existing directory.
  if target_path.exists() {
    return Err(GwmError::WorktreeExists(name.into(), target_path.display().to_string()));
  }

  // Ensure parent dir exists.
  if let Some(parent) = target_path.parent() {
    std::fs::create_dir_all(parent)?;
  }

  // Capture HEAD's short name BEFORE creating the new branch so the
  // record points at the actual parent (`main` / `dev` / a release
  // train), not the freshly-created `branch_name` itself.
  let head_ref = repo.head()?;
  let head_short = head_ref.shorthand().map(|s| s.to_string());
  let head_commit = head_ref.peel_to_commit()?;
  let branch = match repo.find_branch(branch_name, git2::BranchType::Local) {
    Ok(b) => b,
    Err(_) => repo.branch(branch_name, &head_commit, false)?,
  };
  let reference = branch.into_reference();

  let mut opts = WorktreeAddOptions::new();
  opts.reference(Some(&reference));

  repo.worktree(name, target_path, Some(&opts))?;

  // Record the parent ref for the launcher's base resolution chain.
  if let Some(parent_ref) = head_short {
    let _ = crate::launcher::write_gwm_base(repo, branch_name, &parent_ref);
  }

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

/// A commit row pulled from `git log` for the Recent Commits sidebar block.
/// Mirrors lazygit's columnar layout (hash + author + subject) so the
/// renderer can lay out one commit per visual line. Hashes are full
/// 40-char SHAs; the renderer trims them on display to a fixed length
/// (the `COMMIT_HASH_DISPLAY_LEN` constant in `src/tui/ui.rs`, currently
/// 8 chars, matching lazygit's `Gui.CommitHashLength` default). Not
/// user-configurable today — change the constant to retune.
/// `parents.len() >= 2` flags a merge commit, which the renderer marks
/// with `◎` instead of `○`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRow {
  pub hash: String,
  pub author: String,
  pub parents: Vec<String>,
  pub subject: String,
}

/// Shell out to `git log --format='%H%x00%aN%x00%P%x00%s' -n <n>` inside
/// `path` and parse the NUL-separated output into [`CommitRow`]s. The TUI
/// sidebar uses this for the Recent Commits block — the NUL separator
/// avoids ambiguity when a subject contains the usual ` `, `|`, or tab
/// characters lazygit also relies on. The `%P` field carries the
/// space-separated list of parent SHAs (empty for the seed commit, one for
/// a normal commit, two-plus for a merge).
pub fn git_log_with_author(path: &Path, n: usize) -> Result<Vec<CommitRow>> {
  let output = Command::new("git")
    .arg("-C")
    .arg(path)
    .args(["log", "--format=%H%x00%aN%x00%P%x00%s", "-n"])
    .arg(n.to_string())
    .output()
    .map_err(|e| GwmError::Other(format!("git log failed to spawn: {}", e)))?;
  if !output.status.success() {
    return Err(GwmError::Other(format!(
      "git log exited {}: {}",
      output.status,
      String::from_utf8_lossy(&output.stderr).trim()
    )));
  }
  let raw = String::from_utf8_lossy(&output.stdout).into_owned();
  let mut rows = Vec::new();
  for line in raw.lines() {
    let mut parts = line.splitn(4, '\u{0}');
    let hash = parts.next().unwrap_or("").to_string();
    let author = parts.next().unwrap_or("").to_string();
    let parents_field = parts.next().unwrap_or("");
    let subject = parts.next().unwrap_or("").to_string();
    if hash.is_empty() {
      continue;
    }
    let parents: Vec<String> = parents_field.split_whitespace().map(|s| s.to_string()).collect();
    rows.push(CommitRow {
      hash,
      author,
      parents,
      subject,
    });
  }
  Ok(rows)
}

/// Shell out to `git log --oneline -n <n>` inside `path` and return raw stdout.
/// Used by the TUI sidebar to preview recent commits of the selected worktree.
pub fn git_log_oneline(path: &Path, n: usize) -> Result<String> {
  let output = Command::new("git")
    .arg("-C")
    .arg(path)
    .args(["log", "--oneline", "-n"])
    .arg(n.to_string())
    .output()
    .map_err(|e| GwmError::Other(format!("git log failed to spawn: {}", e)))?;
  if !output.status.success() {
    return Err(GwmError::Other(format!(
      "git log exited {}: {}",
      output.status,
      String::from_utf8_lossy(&output.stderr).trim()
    )));
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Shell out to `git status --short` inside `path` and return raw stdout.
/// Used by the TUI sidebar to preview the working-tree state.
pub fn git_status_short(path: &Path) -> Result<String> {
  let output = Command::new("git")
    .arg("-C")
    .arg(path)
    .args(["status", "--short"])
    .output()
    .map_err(|e| GwmError::Other(format!("git status failed to spawn: {}", e)))?;
  if !output.status.success() {
    return Err(GwmError::Other(format!(
      "git status exited {}: {}",
      output.status,
      String::from_utf8_lossy(&output.stderr).trim()
    )));
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Time elapsed since the *oldest* commit on `branch` that's not also on a
/// known trunk (main / master / dev). Returns `None` when no such commit
/// exists — i.e. the branch is the trunk itself, has no divergence yet,
/// or `branch` cannot be resolved. The "oldest commit" rule mirrors the
/// lazygit branch-age semantics (pkg/utils/date.go::UnixToTimeAgo on the
/// branch's founding commit) and is more meaningful for a worktree-manager
/// than `git log -1`: it answers "how long has this branch been alive?"
/// rather than "when did someone last touch it?".
pub fn branch_age(repo: &Repository, branch: &str) -> Option<Duration> {
  // The trunk itself has no "branch age" — there's no founding-commit
  // distinct from the repository's initial commit, and the natural
  // answer ("since forever") is more usefully encoded as `None` so the
  // UI can render a dash instead of a misleadingly precise duration.
  if TRUNK_CANDIDATES.contains(&branch) {
    return None;
  }

  let local = repo.find_branch(branch, BranchType::Local).ok()?;
  let head_oid = local.into_reference().target()?;

  let mut walker = repo.revwalk().ok()?;
  walker.push(head_oid).ok()?;
  // Track whether any trunk baseline was actually hidden. Without one,
  // the revwalk degenerates into "all commits reachable from HEAD" and
  // the oldest one is the repo's initial commit — i.e. the branch's
  // age becomes the repo's lifetime. PR #74 review caught this: when
  // no trunk candidate resolves locally, return `None` so the UI
  // renders `-` instead of a misleadingly large duration.
  let mut hidden_any = false;
  for trunk in TRUNK_CANDIDATES {
    if let Ok(t) = repo.find_branch(trunk, BranchType::Local) {
      if let Some(oid) = t.into_reference().target() {
        if walker.hide(oid).is_ok() {
          hidden_any = true;
        }
      }
    }
  }
  if !hidden_any {
    return None;
  }

  let mut oldest_secs: Option<i64> = None;
  for oid in walker.flatten() {
    if let Ok(commit) = repo.find_commit(oid) {
      let t = commit.time().seconds();
      oldest_secs = Some(oldest_secs.map_or(t, |x| x.min(t)));
    }
  }
  let oldest = oldest_secs?;
  let now = chrono::Utc::now().timestamp();
  let elapsed = (now - oldest).max(0) as u64;
  Some(Duration::from_secs(elapsed))
}

/// Render a `Duration` as a lazygit-style compact relative label
/// (`2d`, `3w`, `1M`, `5y`). Mirrors `pkg/utils/date.go::formatSecondsAgo`
/// from lazygit: single-character suffix, no plural, capital `M` to
/// disambiguate from minutes. Bounded at 4 chars for two-digit values in
/// each unit, which is enough for any realistic branch age.
pub fn format_relative_duration(d: Duration) -> String {
  const MINUTE: u64 = 60;
  const HOUR: u64 = 60 * MINUTE;
  const DAY: u64 = 24 * HOUR;
  const WEEK: u64 = 7 * DAY;
  // Month = 30.25 days, year = 365.25 days (matches lazygit `pkg/utils/date.go`).
  const MONTH: u64 = 30 * DAY + 6 * HOUR;
  const YEAR: u64 = 365 * DAY + 6 * HOUR;

  let s = d.as_secs();
  if s < MINUTE {
    format!("{}s", s)
  } else if s < HOUR {
    format!("{}m", s / MINUTE)
  } else if s < DAY {
    format!("{}h", s / HOUR)
  } else if s < WEEK {
    format!("{}d", s / DAY)
  } else if s < MONTH {
    format!("{}w", s / WEEK)
  } else if s < YEAR {
    format!("{}M", s / MONTH)
  } else {
    format!("{}y", s / YEAR)
  }
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
