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
  // Canonical main-workdir of each repo already admitted, so two entries that
  // resolve to the same main repo (a main checkout and one of its linked
  // worktrees, both under the root) collapse to a single row.
  let mut seen: Vec<PathBuf> = Vec::new();
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
    // Resolve the entry to the *main* repo it belongs to: a normal repo is its
    // own main; a linked worktree resolves to the main checkout that owns it
    // (which may live outside the root). `None` ⇒ unresolvable, skip.
    let Some(main_workdir) = main_workdir(&repo) else {
      continue;
    };
    // A non-worktree entry must BE its own repo root. When `--workspace` points
    // at a directory that is itself a repo, `read_dir` surfaces the root's own
    // `.git/`; `Repository::open` succeeds on it but resolves to the *parent*
    // repo (main workdir = root, not `root/.git`), so this drops that bogus
    // `.git` row (Codex review #303 P2).
    if !repo.is_worktree() && !paths_equal(&main_workdir, &path) {
      continue;
    }
    // Dedupe by the resolved main workdir: a main checkout and a linked
    // worktree of it that both sit under the root collapse to one row (the
    // main repo's `worktree::list` already emits that worktree). A linked
    // worktree whose owner is NOT under the root resolves to its owner and is
    // still included — its checkout would otherwise be invisible (#304).
    let canon = main_workdir.canonicalize().unwrap_or_else(|_| main_workdir.clone());
    if seen.contains(&canon) {
      continue;
    }
    seen.push(canon);
    let name = main_workdir
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_else(|| "repo".into());
    repos.push(WorkspaceRepo {
      name,
      path: main_workdir,
    });
  }

  // Sort by name, then by path as a stable tie-breaker so two repos that share
  // a basename always order the same way across runs / filesystems — otherwise
  // the `-N` suffixing below would assign `main` / `main-2` non-deterministically
  // and `--repo main-2` could target a different physical repo per run (#304).
  repos.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
  // Display names come from the main workdir basename, which can collide for
  // distinct repos (a linked worktree resolving to an owner outside the root,
  // or symlinked children). `--repo <name>` and the TUI must address each repo
  // unambiguously, so suffix any duplicate with the smallest free `-N` (#304).
  let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
  for r in &mut repos {
    if used.insert(r.name.clone()) {
      continue;
    }
    let mut n = 2;
    loop {
      let candidate = format!("{}-{}", r.name, n);
      if used.insert(candidate.clone()) {
        r.name = candidate;
        break;
      }
      n += 1;
    }
  }
  Ok(Workspace {
    root: root.to_path_buf(),
    repos,
  })
}

/// The working directory of the *main* repo an opened entry belongs to. A
/// normal repo is its own main (`workdir()`); a linked worktree resolves to
/// the main checkout that owns it — its gitdir is
/// `<main>/.git/worktrees/<id>/`, so three parents up is `<main>`, which we
/// re-open to read its real workdir. `None` when the path layout can't be
/// resolved or the repo has no workdir (bare).
fn main_workdir(repo: &Repository) -> Option<PathBuf> {
  if repo.is_worktree() {
    let admin = repo.path();
    let main = admin.parent()?.parent()?.parent()?;
    Repository::open(main).ok()?.workdir().map(Path::to_path_buf)
  } else {
    repo.workdir().map(Path::to_path_buf)
  }
}

/// Compare two paths for the same on-disk location, canonicalizing first so a
/// trailing separator (libgit2 workdirs carry one) or a `/var`↔`/private/var`
/// symlink (macOS tempdirs) doesn't make equal paths compare unequal. Falls
/// back to the raw path when canonicalization fails (e.g. a not-yet-created
/// path), which is the conservative "compare as-is" behaviour.
fn paths_equal(a: &Path, b: &Path) -> bool {
  let ca = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
  let cb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
  ca == cb
}

/// Heuristic trigger for the auto-detect prompt (issue #36): when bare `gwm`
/// is run in a directory that is *not* itself inside a git repo but *does*
/// hold direct-child git repos, offer to open it as a workspace.
///
/// Returns the discovered [`Workspace`] when the heuristic fires, else `None`.
/// The interactive prompt lives in the CLI layer — this is the pure decision
/// so it can be tested without stdin. Being inside a repo (at `cwd` or any
/// ancestor) always loses to single-repo mode.
pub fn autodetect(cwd: &Path) -> Option<Workspace> {
  // `discover` here is libgit2's repo discovery (walks up); an `Ok` means we
  // are inside a repo, so single-repo mode wins and we never auto-workspace.
  if Repository::discover(cwd).is_ok() {
    return None;
  }
  let ws = discover(cwd).ok()?;
  if ws.is_empty() {
    None
  } else {
    Some(ws)
  }
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
