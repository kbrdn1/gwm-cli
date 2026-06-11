use crate::error::{GwmError, Result};
use crate::github::{self, BranchLink, IssueState, PrState};
use git2::{BranchType, Repository, StatusOptions, WorktreeAddOptions, WorktreePruneOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;

/// Trunk branches treated as "merge destinations" when measuring how
/// long a branch has been alive. Order matters: the first match wins,
/// so `main` (modern default) beats `master` (legacy) beats `dev` (gwm
/// convention). Hardcoded here because `branch_age` is also reachable
/// from contexts that don't carry a `Config` (CLI smoke paths).
const TRUNK_CANDIDATES: &[&str] = &["main", "master", "dev"];
/// Common trunk branch names tried (after any configured trunks) when
/// resolving a PR / diff base, and treated as "this branch is itself a
/// trunk" by [`is_trunk_branch`]. Superset of [`TRUNK_CANDIDATES`].
const COMMON_TRUNKS: &[&str] = &["main", "master", "dev", "develop", "trunk"];
const BRANCH_CREATED_AT_CONFIG_KEY: &str = "gwm-created-at";
const RECENT_COMMITS_CACHE_MAX_ENTRIES: usize = 64;
type RecentCommitCacheKey = (PathBuf, git2::Oid, usize);

static RECENT_COMMITS_CACHE: LazyLock<Mutex<HashMap<RecentCommitCacheKey, Vec<CommitRow>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
  /// Display name — the basename of the worktree directory on disk. This is
  /// what the user sees, yanks, and filters on, so after a `git worktree move`
  /// (the `c` rename, #290) it reflects the new slug rather than the stale
  /// internal id (Codex review on PR #292).
  pub name: String,
  /// Internal git worktree id — the `.git/worktrees/<id>` entry from
  /// `repo.worktrees()`. `git worktree move` does NOT rename it, so it can
  /// diverge from [`Self::name`] after a rename. Use this (not `name`) for
  /// `worktree::remove` / `find_worktree`, which resolve by id. Equal to
  /// `name` for a freshly created worktree and for the main worktree.
  pub id: String,
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
  /// Loaded GitHub issue state for the row, if the TUI has fetched it this
  /// session. `None` keeps the table on its no-fetch linked/unlinked colour.
  pub issue_state: Option<IssueState>,
  /// Loaded GitHub PR state for the row, if the TUI has fetched it this
  /// session. `None` keeps the table on its no-fetch linked/unlinked colour.
  pub pr_state: Option<PrState>,
  /// Branch age relative to the trunk baseline, pre-computed at list
  /// time so the TUI render path never opens a fresh `git2::Repository`
  /// per row per frame (issue #103). `None` for trunk branches and for
  /// worktrees whose repo can't be opened — the UI renders `-`.
  pub age: Option<Duration>,
}

#[cfg(test)]
mod tests {
  use super::parse_git_log_with_author_output;

  #[test]
  fn parse_git_log_error_includes_invalid_commit_oid_text() {
    let err = parse_git_log_with_author_output("not-an-oid\u{0}Ada\u{0}\u{0}subject\n").unwrap_err();
    let rendered = err.to_string();

    assert!(
      rendered.contains("not-an-oid"),
      "invalid commit oid should be included in the error, got: {}",
      rendered
    );
  }

  #[test]
  fn parse_git_log_error_includes_invalid_parent_oid_text() {
    let raw = "0123456789abcdef0123456789abcdef01234567\u{0}Ada\u{0}bad-parent\u{0}subject\n";
    let err = parse_git_log_with_author_output(raw).unwrap_err();
    let rendered = err.to_string();

    assert!(
      rendered.contains("bad-parent"),
      "invalid parent oid should be included in the error, got: {}",
      rendered
    );
  }
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

/// True when the worktree at `repo` carries staged, unstaged, or
/// untracked changes (ignored files excluded). Shares its
/// `StatusOptions` shape with [`compute_status`] so the status column
/// and `gwm sync`'s dirty-tree refusal (issue #24) agree on what
/// "dirty" means.
pub fn is_dirty(repo: &Repository) -> Result<bool> {
  let mut opts = StatusOptions::new();
  opts
    .include_untracked(true)
    .include_ignored(false)
    .recurse_untracked_dirs(true);
  let statuses = repo.statuses(Some(&mut opts))?;
  Ok(!statuses.is_empty())
}

/// Compute the working-tree + upstream status of a single repo / linked worktree.
fn compute_status(repo: &Repository) -> BranchStatus {
  let mut out = BranchStatus::default();

  // Dirty check — reuse the shared `is_dirty` scanner so the column
  // and `gwm sync` can never disagree on dirtiness.
  match is_dirty(repo) {
    Ok(dirty) => out.is_dirty = dirty,
    Err(_) => out.unknown = true,
  }

  // Ahead / behind vs upstream
  if let Ok(head_ref) = repo.head() {
    if let Ok(shorthand) = head_ref.shorthand() {
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
    let branch = head_ref
      .as_ref()
      .and_then(|r| r.shorthand().ok().map(|s| s.to_string()));
    let head = head_ref.as_ref().and_then(|r| r.target().map(|o| o.to_string()));
    let link = branch
      .as_deref()
      .and_then(|b| github::read_link(repo, b).ok())
      .unwrap_or_else(BranchLink::empty);
    let age = branch.as_deref().and_then(|b| branch_age(repo, b));
    let main_name = workdir
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_else(|| "main".into());
    out.push(WorktreeInfo {
      // The main worktree has no `.git/worktrees/<id>` entry; id == display.
      id: main_name.clone(),
      name: main_name,
      path: workdir.to_path_buf(),
      branch,
      head,
      is_main: true,
      is_locked: false,
      is_prunable: false,
      status: compute_status(repo),
      issue_state: link.issue_state,
      pr_state: link.pr_state,
      link,
      age,
    });
  }

  let names = repo.worktrees()?;
  // `StringArray::iter` yields `Result<Option<&str>, _>`; skip both the
  // `Err` (non-UTF-8 entry) and `None` arms so `name` is a plain `&str`.
  for name in names.iter().filter_map(|r| r.ok().flatten()) {
    let wt = match repo.find_worktree(name) {
      Ok(w) => w,
      Err(_) => continue,
    };
    let path = wt.path().to_path_buf();
    let is_locked = matches!(wt.is_locked(), Ok(git2::WorktreeLockStatus::Locked(_)));
    let is_prunable = matches!(wt.is_prunable(None), Ok(p) if p);

    // Open the worktree as a repo to read its HEAD + status + branch age.
    // Issue #103: piggyback the age computation onto this existing open so
    // the TUI render path no longer needs to call `Repository::open` per
    // row per frame. Cost is the same revwalk we'd otherwise do per frame.
    let (branch, head, status, age) = match Repository::open(&path) {
      Ok(sub) => {
        let head_ref = sub.head().ok();
        let b = head_ref
          .as_ref()
          .and_then(|r| r.shorthand().ok().map(|s| s.to_string()));
        let h = head_ref.as_ref().and_then(|r| r.target().map(|o| o.to_string()));
        let s = compute_status(&sub);
        // The trunk-baseline lookup must run against the main repo's
        // branch table; the linked worktree's `sub` has the same refs DB
        // either way (git2 shares the gitdir), so either handle works.
        let a = b.as_deref().and_then(|name| branch_age(&sub, name));
        (b, h, s, a)
      }
      Err(_) => (
        None,
        None,
        BranchStatus {
          unknown: true,
          ..Default::default()
        },
        None,
      ),
    };

    let link = branch
      .as_deref()
      .and_then(|b| github::read_link(repo, b).ok())
      .unwrap_or_else(BranchLink::empty);
    // Display name = basename of the on-disk path (tracks `git worktree move`);
    // id = the `repo.worktrees()` entry (stable, used for remove/find).
    let display_name = path
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_else(|| name.to_string());
    out.push(WorktreeInfo {
      name: display_name,
      id: name.to_string(),
      path,
      branch,
      head,
      is_main: false,
      is_locked,
      is_prunable,
      status,
      issue_state: link.issue_state,
      pr_state: link.pr_state,
      link,
      age,
    });
  }

  Ok(out)
}

/// Create a new worktree off of HEAD, attaching it either to a freshly
/// created branch (the default) or — when `reuse_branch` is true — to a
/// pre-existing local branch of the same name.
///
/// Records the HEAD ref's short name into `branch.<branch_name>.gwm-base`
/// so the review launcher (issue #75) can recover the original parent
/// ref later — even on branches without an upstream. The write is
/// best-effort: a config-write error does not roll the worktree back.
///
/// `reuse_branch` gates the "branch already exists" path (issue #99). The
/// historical default silently reused a stale branch at whatever commit
/// it referenced, resurrecting `git log` state the user never asked for.
/// The new default refuses with `GwmError::BranchExists`; pass `true`
/// (`--reuse-branch` on the CLI) to opt back into the legacy behaviour
/// when attaching to an existing branch is the intent.
pub fn add(
  repo: &Repository,
  name: &str,
  target_path: &Path,
  branch_name: &str,
  reuse_branch: bool,
) -> Result<PathBuf> {
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
  let head_short = head_ref.shorthand().ok().map(|s| s.to_string());
  let head_commit = head_ref.peel_to_commit()?;
  let (branch, created_branch) = match repo.find_branch(branch_name, git2::BranchType::Local) {
    Ok(b) => {
      if !reuse_branch {
        // Resolve the existing tip for the error message so the user
        // sees *where* the stale ref is pointing and can decide between
        // `--reuse-branch`, `git branch -D <name>`, or a different slug.
        let oid = b
          .get()
          .target()
          .map(|o| o.to_string())
          .unwrap_or_else(|| "<unresolved>".into());
        return Err(GwmError::BranchExists {
          name: branch_name.into(),
          oid,
        });
      }
      (b, false)
    }
    Err(_) => (repo.branch(branch_name, &head_commit, false)?, true),
  };
  if created_branch {
    let _ = write_branch_created_at(repo, branch_name, chrono::Utc::now().timestamp());
  }
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

fn branch_config_key(branch: &str, leaf: &str) -> String {
  format!("branch.{}.{}", branch, leaf)
}

fn write_branch_created_at(repo: &Repository, branch: &str, unix_secs: i64) -> Result<()> {
  let mut cfg = repo.config()?;
  cfg.set_str(
    &branch_config_key(branch, BRANCH_CREATED_AT_CONFIG_KEY),
    &unix_secs.to_string(),
  )?;
  Ok(())
}

fn branch_created_age(repo: &Repository, branch: &str) -> Option<Duration> {
  let cfg = repo.config().ok()?;
  let key = branch_config_key(branch, BRANCH_CREATED_AT_CONFIG_KEY);
  let raw = cfg.get_string(&key).ok()?;
  let created = raw.trim().parse::<i64>().ok()?;
  let now = chrono::Utc::now().timestamp();
  Some(Duration::from_secs((now - created).max(0) as u64))
}

/// Remove a worktree directory and prune its admin files. Optionally delete the branch.
pub fn remove(repo: &Repository, name: &str, delete_branch: bool) -> Result<()> {
  let wt = repo
    .find_worktree(name)
    .map_err(|_| GwmError::WorktreeNotFound(name.into()))?;
  let path = wt.path().to_path_buf();

  // Capture the branch (if any) so we can drop it after pruning.
  let branch_name = match Repository::open(&path) {
    Ok(sub) => sub.head().ok().and_then(|r| r.shorthand().ok().map(|s| s.to_string())),
    Err(_) => None,
  };

  // Prune admin files (.git/worktrees/<name>) FIRST so a subsequent
  // filesystem failure cannot leave a "phantom worktree" (issue #98):
  // directory gone but `repo.worktrees()` still listing the name. The
  // reverse ordering forced users into a manual `gwm prune` recovery
  // after any partial failure.
  let mut opts = WorktreePruneOptions::new();
  opts.valid(true).locked(true).working_tree(true);
  wt.prune(Some(&mut opts))?;

  // Physical removal — git2's prune does NOT delete the work tree directory itself.
  if path.exists() {
    std::fs::remove_dir_all(&path)?;
  }

  if delete_branch {
    if let Some(b) = branch_name {
      if let Ok(mut branch) = repo.find_branch(&b, git2::BranchType::Local) {
        let _ = branch.delete();
      }
    }
  }

  Ok(())
}

/// Run a `git` subcommand in `dir`, returning trimmed stdout on success or a
/// [`GwmError::CommandFailed`] carrying stderr on a non-zero exit. Shared by
/// the worktree-rename steps (#290) so each step reports a precise error.
fn git_in(dir: &Path, args: &[&str]) -> Result<String> {
  let mut cmd = Command::new("git");
  cmd.args(args).current_dir(dir);
  // Route through the command-log chokepoint so the rename's mutating steps
  // (`worktree move`, `branch -m`, the lease `fetch`, `push --atomic`) surface
  // in the Command Logs modal (#290). `git_in` is rename-only, so this never
  // spams the log with read-only sidebar previews.
  let out = crate::command_log::run_logged(&mut cmd, format!("git {}", args.join(" ")))?;
  if out.status.success() {
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
  } else {
    Err(GwmError::CommandFailed(
      String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
  }
}

/// Rename a worktree's branch (local + remote) and move its directory on
/// disk (`c` in the TUI, #290).
///
/// The directory move is the step most likely to fail (the row is the main
/// or a locked worktree, or the target path already exists), so it runs
/// **first** — a failure there leaves every ref untouched (Codex review on
/// PR #292). Only once the directory is in place are the refs renamed, and a
/// branch-rename failure rolls the move back so the worktree is never left
/// in a half-renamed state. Order of operations:
///
/// 1. Preflight: refuse if `<new_path>` already exists (the move would fail).
/// 2. `git worktree move <old_path> <new_path>` (run from `workdir`, the main
///    repo, so the CWD is never inside the moved dir). Skipped when the path
///    is unchanged.
/// 3. When the branch name changes, `git branch -m <old> <new>` from the
///    moved directory. On failure, roll the move back and return the error. A
///    path-only edit (same branch) skips this and every remote step.
/// 4. If `<old_branch>` exists on `origin`, `git push --atomic origin :<old>
///    <new>:<new>` renames the remote branch (the `--atomic` flag makes the
///    delete-old + create-new pair all-or-nothing, so a rejected push can't
///    leave the remote with neither branch), then `git branch
///    --set-upstream-to` re-points tracking (non-fatal). A rejected push rolls
///    back both the local rename and the move so the repo is never left
///    half-renamed.
///
/// Returns `true` when the remote branch was also renamed (it existed on
/// `origin`), `false` when only the local branch + directory changed (or a
/// path-only move with no branch change).
pub fn rename_worktree(
  workdir: &Path,
  old_path: &Path,
  old_branch: &str,
  new_path: &Path,
  new_branch: &str,
) -> Result<bool> {
  let moves = new_path != old_path;

  // 1. Preflight — a pre-existing target would make `git worktree move`
  //    fail anyway, so reject it up front before touching any ref.
  if moves && new_path.exists() {
    return Err(GwmError::CommandFailed(format!(
      "target path already exists: {}",
      new_path.display()
    )));
  }

  // 2. Move the worktree directory first: it is the most failure-prone step
  //    (main/locked worktree, busy dir), and failing here leaves all refs
  //    untouched.
  if moves {
    git_in(
      workdir,
      &[
        "worktree",
        "move",
        &old_path.to_string_lossy(),
        &new_path.to_string_lossy(),
      ],
    )
    .map_err(|e| GwmError::CommandFailed(format!("worktree move failed: {e}")))?;
  }
  // From here on the branch lives in `branch_dir`.
  let branch_dir = if moves { new_path } else { old_path };

  // Roll the directory move back to its original location. Used when a later
  // step fails so the worktree is never left moved-but-not-renamed.
  let rollback_move = || {
    if moves {
      let _ = git_in(
        workdir,
        &[
          "worktree",
          "move",
          &new_path.to_string_lossy(),
          &old_path.to_string_lossy(),
        ],
      );
    }
  };

  // A path-only edit (same branch, different dir — e.g. a changed
  // `[worktree].base`) must skip every ref mutation: `git branch -m old old`
  // is an error, which would roll a valid move back (Codex review on PR #292).
  let renames_branch = new_branch != old_branch;
  if !renames_branch {
    return Ok(false);
  }

  // 3. Local branch rename. Roll the directory move back on failure so the
  //    worktree is not left moved-but-not-renamed.
  if let Err(e) = git_in(branch_dir, &["branch", "-m", old_branch, new_branch]) {
    rollback_move();
    return Err(GwmError::CommandFailed(format!("local rename failed: {e}")));
  }

  // 4. Remote branch rename, only when the old branch is on origin.
  //    First decide whether an `origin` remote is even configured: with no
  //    remote a local-only rename is perfectly valid (don't abort). Only when
  //    `origin` exists do we look the branch up — and there, with
  //    `--exit-code`, `git ls-remote` exits 0 when the branch is found and 2
  //    when it is genuinely absent. Any other status (auth, network, server)
  //    is a lookup *failure*, not "absent": treating it as absent would skip
  //    the remote rename and report local-only success while `origin/<old>`
  //    lives on, so abort + roll back instead (Codex review on PR #292).
  let has_origin = Command::new("git")
    .args(["remote", "get-url", "origin"])
    .current_dir(branch_dir)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false);
  let remote_exists = if has_origin {
    let ls = Command::new("git")
      .args(["ls-remote", "--exit-code", "--heads", "origin", old_branch])
      .current_dir(branch_dir)
      .output();
    match ls {
      Ok(o) if o.status.success() => true,
      Ok(o) if o.status.code() == Some(2) => false,
      other => {
        let detail = match other {
          Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
          Err(e) => e.to_string(),
        };
        let _ = git_in(branch_dir, &["branch", "-m", new_branch, old_branch]);
        rollback_move();
        return Err(GwmError::CommandFailed(format!("remote lookup failed: {detail}")));
      }
    }
  } else {
    false
  };
  let mut remote_renamed = false;
  if remote_exists {
    // Lease check (Codex review on PR #292): the rename deletes `origin/<old>`
    // and recreates it from the LOCAL tip. If `origin/<old>` has commits this
    // worktree never fetched, that would silently drop them. Fetch the current
    // remote tip and refuse unless it is already contained in the local branch.
    let _ = git_in(branch_dir, &["fetch", "origin", old_branch]);
    let remote_tip = Command::new("git")
      .args(["rev-parse", "FETCH_HEAD"])
      .current_dir(branch_dir)
      .output();
    let up_to_date = match remote_tip {
      Ok(o) if o.status.success() => {
        let tip = String::from_utf8_lossy(&o.stdout).trim().to_string();
        // The remote tip must be an ancestor of (already contained in) the
        // local branch — otherwise origin carries commits we don't have.
        Command::new("git")
          .args(["merge-base", "--is-ancestor", &tip, new_branch])
          .current_dir(branch_dir)
          .output()
          .map(|o| o.status.success())
          .unwrap_or(false)
      }
      _ => false,
    };
    if !up_to_date {
      let _ = git_in(branch_dir, &["branch", "-m", new_branch, old_branch]);
      rollback_move();
      return Err(GwmError::CommandFailed(format!(
        "origin/{old_branch} has commits not in your local branch; fetch/merge before renaming"
      )));
    }
    // `--atomic` makes the two-refspec push all-or-nothing: without it git can
    // delete `origin/<old>` and then fail on `<new>`, leaving the remote with
    // neither branch — and the local rollback below can't restore a deleted
    // remote ref (Codex review on PR #292). With `--atomic`, a rejected push
    // leaves `origin/<old>` intact, so the local rollback fully restores state.
    if let Err(e) = git_in(
      branch_dir,
      &[
        "push",
        "--atomic",
        "origin",
        &format!(":{old_branch}"),
        &format!("{new_branch}:{new_branch}"),
      ],
    ) {
      // The remote push was rejected (protected branch, auth/network, or an
      // existing remote target). Undo the local branch rename and the move so
      // the repo is not left half-renamed (Codex review on PR #292).
      let _ = git_in(branch_dir, &["branch", "-m", new_branch, old_branch]);
      rollback_move();
      return Err(GwmError::CommandFailed(format!("remote rename failed: {e}")));
    }
    // Re-track the new upstream. Non-fatal: the rename is already done.
    let _ = git_in(
      branch_dir,
      &[
        "branch",
        "--set-upstream-to",
        &format!("origin/{new_branch}"),
        new_branch,
      ],
    );
    remote_renamed = true;
  }

  Ok(remote_renamed)
}

/// One prunable worktree entry as surfaced by `gwm prune --dry-run`
/// (issue #31). The `reason` field is a human-readable rationale that
/// is currently hard-coded to "working dir missing" — that is the only
/// case `is_prunable(None)` flags today (working tree removed out from
/// under the admin entry). Kept as a `String` rather than a literal
/// in the CLI so future libgit2 versions can surface richer reasons
/// (locked worktrees, broken HEAD, …) without breaking the CLI
/// rendering contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunableEntry {
  pub name: String,
  pub path: PathBuf,
  pub reason: String,
}

/// Compute (without mutating) the list of worktree admin entries that
/// `gwm prune` would drop. Used by `gwm prune --dry-run` (issue #31)
/// and consumed by [`prune`] so the dry-run preview and the destructive
/// pass can never drift on what "prunable" means. Output is sorted by
/// name for deterministic stdout — scripted callers diff across runs.
pub fn prunable_worktrees(repo: &Repository) -> Result<Vec<PrunableEntry>> {
  let names = repo.worktrees()?;
  let mut out = Vec::new();
  // `StringArray::iter` yields `Result<Option<&str>, _>`; skip both the
  // `Err` (non-UTF-8 entry) and `None` arms so `name` is a plain `&str`.
  for name in names.iter().filter_map(|r| r.ok().flatten()) {
    let wt = match repo.find_worktree(name) {
      Ok(w) => w,
      Err(_) => continue,
    };
    if !matches!(wt.is_prunable(None), Ok(p) if p) {
      continue;
    }
    out.push(PrunableEntry {
      name: name.to_string(),
      path: wt.path().to_path_buf(),
      reason: "working dir missing".to_string(),
    });
  }
  out.sort_by(|a, b| a.name.cmp(&b.name));
  Ok(out)
}

/// Prune stale worktree admin entries (gwq cleanup equivalent).
/// Consumes [`prunable_worktrees`] so what `--dry-run` shows is exactly
/// what this destructive pass acts on — the two surfaces share the
/// scanner, by construction.
pub fn prune(repo: &Repository) -> Result<usize> {
  let plan = prunable_worktrees(repo)?;
  let mut pruned = 0usize;
  for entry in plan {
    let wt = match repo.find_worktree(&entry.name) {
      Ok(w) => w,
      Err(_) => continue,
    };
    let mut opts = WorktreePruneOptions::new();
    opts.valid(true).locked(true).working_tree(true);
    if wt.prune(Some(&mut opts)).is_ok() {
      pruned += 1;
    }
  }
  Ok(pruned)
}

/// Read-only check that `name` resolves to a removable worktree —
/// the libgit2 half of `gwm remove --dry-run` (issue #31). Errors on
/// the same "worktree not found" path as `remove` so the dry-run
/// surface and the destructive surface share an error contract;
/// returns `Ok(())` when the worktree exists. The caller (the CLI)
/// is responsible for rendering the plan; this function intentionally
/// touches no filesystem state and emits no output.
pub fn remove_dry_run(repo: &Repository, name: &str) -> Result<()> {
  repo
    .find_worktree(name)
    .map_err(|_| GwmError::WorktreeNotFound(name.into()))?;
  Ok(())
}

/// A commit row pulled from `git log` for the Recent Commits sidebar block.
/// Mirrors lazygit's columnar layout (hash + author + subject) so the
/// renderer can lay out one commit per visual line. Hashes are parsed
/// into binary OIDs once, then formatted on display to a fixed length (the
/// `COMMIT_HASH_DISPLAY_LEN` constant in `src/tui/ui.rs`, currently 8
/// chars, matching lazygit's `Gui.CommitHashLength` default). Not
/// user-configurable today — change the constant to retune.
/// `parents.len() >= 2` flags a merge commit, which the renderer marks
/// with `◎` instead of `○`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRow {
  pub hash: git2::Oid,
  pub author: String,
  pub parents: Vec<git2::Oid>,
  pub subject: String,
}

/// Return recent commits for the sidebar using libgit2. This is the uncached
/// compatibility entry point; the TUI should call [`recent_commits_cached`]
/// so repeated sidebar rebuilds for the same branch tip are a hash lookup.
pub fn git_log_with_author(path: &Path, n: usize) -> Result<Vec<CommitRow>> {
  let repo = Repository::open(path)?;
  let tip = repo.head()?.target().ok_or_else(|| GwmError::UnbornHead {
    reason: "HEAD does not point at a commit".into(),
  })?;
  recent_commits_revwalk(&repo, tip, n)
}

/// Return recent commits for one worktree, memoised by branch-tip OID and
/// limit. `WorktreeInfo.head` is populated by [`list`], so normal TUI sidebar
/// refreshes can hit the cache without reopening the repo. Fixtures and older
/// callers with `head = None` fall back to opening the worktree once.
pub fn recent_commits_cached(w: &WorktreeInfo, limit: usize) -> Result<Vec<CommitRow>> {
  let tip = worktree_head_oid(w)?;
  let key = (recent_commits_cache_repo_key(&w.path), tip, limit);
  if let Some(rows) = recent_commits_cache().get(&key).cloned() {
    return Ok(rows);
  }

  let repo = Repository::open(&w.path)?;
  let rows = recent_commits_revwalk(&repo, tip, limit)?;
  let mut cache = recent_commits_cache();
  if cache.len() >= RECENT_COMMITS_CACHE_MAX_ENTRIES {
    if let Some(oldest_key) = cache.keys().next().cloned() {
      cache.remove(&oldest_key);
    }
  }
  cache.insert(key, rows.clone());
  Ok(rows)
}

fn recent_commits_cache() -> MutexGuard<'static, HashMap<RecentCommitCacheKey, Vec<CommitRow>>> {
  match RECENT_COMMITS_CACHE.lock() {
    Ok(cache) => cache,
    Err(poisoned) => poisoned.into_inner(),
  }
}

fn recent_commits_cache_repo_key(path: &Path) -> PathBuf {
  std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn worktree_head_oid(w: &WorktreeInfo) -> Result<git2::Oid> {
  if let Some(head) = &w.head {
    return git2::Oid::from_str(head)
      .map_err(|e| GwmError::Other(format!("cached worktree head '{}' is not an oid: {}", head, e)));
  }

  let repo = Repository::open(&w.path)?;
  let head_ref = repo.head()?;
  head_ref.target().ok_or_else(|| GwmError::UnbornHead {
    reason: "HEAD does not point at a commit".into(),
  })
}

fn recent_commits_revwalk(repo: &Repository, tip: git2::Oid, limit: usize) -> Result<Vec<CommitRow>> {
  let mut walker = repo.revwalk()?;
  walker.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)?;
  walker.push(tip)?;

  let mut rows = Vec::new();
  for oid in walker.take(limit) {
    let oid = oid?;
    let commit = repo.find_commit(oid)?;
    rows.push(CommitRow {
      hash: oid,
      author: commit.author().name().unwrap_or("").to_string(),
      parents: commit.parent_ids().collect(),
      subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
    });
  }
  Ok(rows)
}

#[cfg(test)]
fn parse_git_log_with_author_output(raw: &str) -> Result<Vec<CommitRow>> {
  let mut rows = Vec::new();
  for line in raw.lines() {
    let mut parts = line.splitn(4, '\u{0}');
    let hash = parts.next().unwrap_or("");
    let author = parts.next().unwrap_or("").to_string();
    let parents_field = parts.next().unwrap_or("");
    let subject = parts.next().unwrap_or("").to_string();
    if hash.is_empty() {
      continue;
    }
    let hash = git2::Oid::from_str(hash)
      .map_err(|e| GwmError::CommandFailed(format!("git log returned invalid commit oid '{}': {}", hash, e)))?;
    let parents: Vec<git2::Oid> = parents_field
      .split_whitespace()
      .map(|s| {
        git2::Oid::from_str(s)
          .map_err(|e| GwmError::CommandFailed(format!("git log returned invalid parent oid '{}': {}", s, e)))
      })
      .collect::<Result<Vec<_>>>()?;
    rows.push(CommitRow {
      hash,
      author,
      parents,
      subject,
    });
  }
  Ok(rows)
}

/// Run `git -C <dir> <args>`, returning stdout verbatim on success or a
/// [`GwmError::CommandFailed`] carrying the verb and git's stderr on a
/// non-zero exit (or the spawn error if `git` could not be launched).
///
/// This is the single shell-out helper for the read-side git invocations
/// (sidebar previews, PR-body fillers). Read-only previews fire on every
/// selection change, so this variant is deliberately **not** logged — see
/// [`run_git_logged`] for the mutating-step counterpart used by `gwm sync`.
/// Callers that need trimming, truncation, or field parsing post-process the
/// returned `String` themselves.
pub fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
  run_git_inner(dir, args, false)
}

/// Like [`run_git`] but records the call on the process-global command log so
/// it surfaces in the Command Logs modal (#290). Used for `gwm sync`'s
/// mutating steps (`fetch` / `rebase` / `merge` / `--abort`), which are
/// user-triggered operations the user expects to find in the transcript —
/// unlike the read-only previews that go through [`run_git`].
pub fn run_git_logged(dir: &Path, args: &[&str]) -> Result<String> {
  run_git_inner(dir, args, true)
}

fn run_git_inner(dir: &Path, args: &[&str], log: bool) -> Result<String> {
  let mut cmd = Command::new("git");
  cmd.arg("-C").arg(dir).args(args);
  let out = if log {
    crate::command_log::run_logged(&mut cmd, format!("git {}", args.join(" ")))
  } else {
    cmd.output()
  }
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

/// Shell out to `git log --oneline -n <n>` inside `path` and return raw stdout.
/// Used by the TUI sidebar to preview recent commits of the selected worktree.
pub fn git_log_oneline(path: &Path, n: usize) -> Result<String> {
  let n = n.to_string();
  run_git(path, &["log", "--oneline", "-n", &n])
}

/// Shell out to `git log --pretty=- %s <base>..<head>` inside `path`
/// and return raw stdout. Used by `gwm pr` to fill the `{commits}`
/// placeholder in PR templates (issue #84) — each commit becomes a
/// Markdown bullet so a list of commit subjects drops straight into a
/// PR body without extra formatting.
pub fn git_log_subject_between(path: &Path, base: &str, head: &str) -> Result<String> {
  let range = format!("{}..{}", base, head);
  let out = run_git(path, &["log", "--pretty=format:- %s", &range])?;
  Ok(out.trim_end().to_string())
}

/// Shell out to `git diff --stat <base>..<head>` inside `path`. The
/// output is truncated to `max_lines` lines so a sprawling diff stat
/// doesn't blow up the PR body (issue #84: 30-line cap by convention).
pub fn git_diff_stat_between(path: &Path, base: &str, head: &str, max_lines: usize) -> Result<String> {
  let range = format!("{}..{}", base, head);
  let raw = run_git(path, &["diff", "--stat", &range])?;
  let mut lines: Vec<&str> = raw.lines().collect();
  let truncated = lines.len() > max_lines;
  if truncated {
    lines.truncate(max_lines);
  }
  let mut out = lines.join("\n");
  if truncated {
    out.push_str(&format!(
      "\n… ({} more line{} trimmed)",
      raw.lines().count() - max_lines,
      if raw.lines().count() - max_lines == 1 { "" } else { "s" }
    ));
  }
  Ok(out)
}

/// Insertion / deletion line counts of a branch versus its base trunk
/// (issue #287). Populated from `git diff --shortstat <base>...HEAD` — the
/// three-dot merge-base form, so the figures reflect only what the branch
/// itself contributed (the GitHub-PR view), not divergence that landed on
/// the trunk after the fork.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffLineStat {
  /// Lines added by the branch relative to the merge-base with its trunk.
  pub insertions: usize,
  /// Lines removed by the branch relative to the merge-base with its trunk.
  pub deletions: usize,
}

impl DiffLineStat {
  /// True when the branch carries no committed diff against its base — a
  /// fresh branch with no commits past the fork point, or one whose net
  /// change is empty. The sidebar hides the `Diff` line in that case.
  pub fn is_empty(&self) -> bool {
    self.insertions == 0 && self.deletions == 0
  }
}

/// Parse a `git diff --shortstat` summary line into a [`DiffLineStat`]
/// (issue #287). The line looks like
/// ` 3 files changed, 12 insertions(+), 4 deletions(-)`, but either the
/// insertions or the deletions clause can be absent — an all-additions or
/// all-deletions diff omits the empty side, and an empty diff yields an
/// empty string. Any clause that's missing counts as zero; the singular
/// `1 insertion(+)` / `1 deletion(-)` forms are handled too.
pub fn parse_diff_shortstat(raw: &str) -> DiffLineStat {
  let mut out = DiffLineStat::default();
  for part in raw.split(',') {
    let part = part.trim();
    if let Some(n) = part
      .strip_suffix("insertions(+)")
      .or_else(|| part.strip_suffix("insertion(+)"))
    {
      out.insertions = n.trim().parse().unwrap_or(0);
    } else if let Some(n) = part
      .strip_suffix("deletions(-)")
      .or_else(|| part.strip_suffix("deletion(-)"))
    {
      out.deletions = n.trim().parse().unwrap_or(0);
    }
  }
  out
}

/// True when `branch` is itself a trunk — present in the configured trunk
/// list or in the [`COMMON_TRUNKS`] defaults. Used to suppress the Status
/// pane's diff row on trunk worktrees regardless of which trunk
/// `resolve_trunk` would pick as the base (issue #287).
pub fn is_trunk_branch(branch: &str, configured: &[String]) -> bool {
  configured.iter().any(|t| t == branch) || COMMON_TRUNKS.contains(&branch)
}

/// Committed diff size of the worktree's current branch versus its base
/// trunk (issue #287), via `git diff --shortstat <base>...HEAD`. Returns
/// `Ok(None)` when the path is not a readable repo, when HEAD is itself a
/// trunk (no meaningful base to diff against — see [`is_trunk_branch`]), or
/// when no base trunk resolves locally. `trunks` is the configured
/// trunk-priority list (`config.doctor.trunks`) so the figure matches the
/// base `gwm pr` would target — `resolve_trunk` walks it before falling
/// back to the common defaults.
pub fn git_diff_stat_vs_base(path: &Path, trunks: &[String]) -> Result<Option<DiffLineStat>> {
  let repo = match Repository::open(path) {
    Ok(r) => r,
    Err(_) => return Ok(None),
  };
  // HEAD sitting on *any* trunk has no meaningful base to diff against —
  // suppress the row so a trunk worktree never shows a `Diff`. This must
  // check the whole trunk universe, not just the resolved base: with the
  // default `["dev", "main"]`, a worktree on `main` resolves its base to
  // `dev` (the earlier candidate), and a `head == base` check alone would
  // leak a `main...dev` diff onto a trunk worktree (issue #287 review).
  if let Ok(head) = repo.head() {
    if let Ok(branch) = head.shorthand() {
      if is_trunk_branch(branch, trunks) {
        return Ok(None);
      }
    }
  }
  let base = match resolve_trunk(&repo, trunks) {
    Some(b) => b,
    None => return Ok(None),
  };
  let range = format!("{}...HEAD", base);
  let raw = run_git(path, &["diff", "--shortstat", &range])?;
  Ok(Some(parse_diff_shortstat(&raw)))
}

/// One row of `git stash list` (issue #34). Surfaced by the sidebar
/// in stashes mode. Kept deliberately minimal — `ref_name` so the user
/// can copy `stash@{N}` to the status bar, `subject` so they can tell
/// which stash is which. Per-file diff numbers (`+/-`) live in a
/// follow-up — the v1 contract is just "name + subject".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
  /// Canonical git stash reference (e.g. `stash@{0}`). Stable for the
  /// lifetime of the panel — the user can paste it into `git stash
  /// apply <ref>` from the surrounding shell.
  pub ref_name: String,
  /// Human-readable subject as written by `git stash push -m <msg>`
  /// (or the auto-generated `WIP on <branch>: …` when no `-m` was
  /// supplied).
  pub subject: String,
}

/// Parse the worktree's stash list (issue #34). Returns up to `limit`
/// entries in `git stash list` order (LIFO — `stash@{0}` is the most
/// recent push).
///
/// Uses `--pretty=format:%gd<US>%s` (with `\x1f` as the unit
/// separator) so subjects containing spaces, colons, or `:` round-trip
/// safely. An empty stash list returns `Ok(Vec::new())`; only spawn /
/// non-zero-exit failures surface as `GwmError::CommandFailed`.
pub fn git_stash_list(path: &Path, limit: usize) -> Result<Vec<StashEntry>> {
  // ASCII Unit Separator (0x1F) cannot occur in a normal shell argv
  // or git ref name, so it's a safe per-field delimiter — same
  // technique `git_log_with_author` uses with `\x1c` for record
  // separation.
  //
  // Pass `-n <limit>` (a `git log` option `stash list` forwards
  // through) so a repo with hundreds of stashes doesn't materialise
  // the full list in stdout just for the panel to drop everything
  // past the cap. Pre-review the limit was applied client-side after
  // the full stdout was read.
  let limit_arg = format!("-n{}", limit);
  let raw = run_git(path, &["stash", "list", "--pretty=format:%gd\x1f%s", &limit_arg])?;
  let entries = raw
    .lines()
    .filter(|line| !line.is_empty())
    .take(limit)
    .filter_map(|line| {
      let mut parts = line.splitn(2, '\x1f');
      let ref_name = parts.next()?.to_string();
      let subject = parts.next().unwrap_or("").to_string();
      Some(StashEntry { ref_name, subject })
    })
    .collect();
  Ok(entries)
}

/// Shell out to `git status --short` inside `path` and return raw stdout.
/// Used by the TUI sidebar to preview the working-tree state.
pub fn git_status_short(path: &Path) -> Result<String> {
  run_git(path, &["status", "--short"])
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

  if let Some(age) = branch_created_age(repo, branch) {
    return Some(age);
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
  // Exact display-name match. Since #290 derives `name` from the path basename,
  // it is no longer guaranteed unique (two worktrees in different parent dirs
  // can share a basename), so an exact match that hits more than one row is
  // ambiguous rather than "take the first" (Codex review on PR #292).
  let exact: Vec<&WorktreeInfo> = all.iter().filter(|w| w.name == pattern && !w.is_main).collect();
  match exact.len() {
    1 => return Ok(exact[0].clone()),
    n if n > 1 => {
      // Duplicate display names: let the caller still target one by its unique
      // internal id (the original slug), and list those ids so they know what
      // to type (Codex review on PR #292).
      if let Some(by_id) = all.iter().find(|w| w.id == pattern && !w.is_main) {
        return Ok(by_id.clone());
      }
      let ids = exact.iter().map(|w| w.id.as_str()).collect::<Vec<_>>().join(", ");
      return Err(GwmError::Other(format!(
        "name '{}' is ambiguous ({} worktrees share it); target one by id: {}",
        pattern, n, ids
      )));
    }
    // Unique display name not found: allow an exact id match before falling
    // back to substring search, so a renamed worktree stays reachable by id.
    _ => {
      if let Some(by_id) = all.iter().find(|w| w.id == pattern && !w.is_main) {
        return Ok(by_id.clone());
      }
    }
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

/// Pick a base ref for `gwm pr` by walking the `configured` trunks list
/// first, then the common defaults (`main`, `master`, `dev`, `develop`,
/// `trunk`) so a repo whose local trunk is `master` and which hasn't
/// customised `[doctor]` doesn't fall back to a non-existent `"main"`.
/// Returns `None` only if none of the candidates resolve to a local
/// branch — the caller then uses `"main"` as a last resort so the
/// downstream `gh pr create --base main` produces a clean error message
/// instead of a panic.
pub fn resolve_trunk(repo: &Repository, configured: &[String]) -> Option<String> {
  for trunk in configured {
    if repo.find_branch(trunk, BranchType::Local).is_ok() {
      return Some(trunk.clone());
    }
  }
  for trunk in COMMON_TRUNKS {
    if configured.iter().any(|t| t == trunk) {
      continue; // already tried as a configured trunk above
    }
    if repo.find_branch(trunk, BranchType::Local).is_ok() {
      return Some((*trunk).to_string());
    }
  }
  None
}
