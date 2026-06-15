//! Integration tests for workspace mode discovery + merge (issue #36).
//!
//! Workspace mode gives a bird's-eye view across every git repo that sits
//! one level below a workspace root (e.g. `~/Projects`). These tests pin the
//! discovery contract (one level deep, ignore non-repos, sort by name) and
//! the merged worktree listing (each row tagged with the owning repo) against
//! real `git2` repos created under a `tempfile::TempDir`.

use git2::{Repository, Signature};
use gwm::workspace;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Initialise a git repo at `path` (created if missing) on `main` with one
/// empty commit, mirroring `tests/common::init_repo` but at a caller-chosen
/// location so several repos can share one workspace-root tempdir.
fn init_repo_at(path: &Path) -> Repository {
  fs::create_dir_all(path).unwrap();
  let repo = Repository::init(path).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = {
    let mut index = repo.index().unwrap();
    index.write_tree().unwrap()
  };
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
  Repository::open(path).unwrap()
}

#[test]
fn discover_finds_child_repos_and_ignores_non_repos() {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));
  // A plain directory (no .git) must be ignored.
  fs::create_dir_all(root.path().join("notes")).unwrap();
  // A loose file at the root must be ignored.
  fs::write(root.path().join("README.md"), "hi").unwrap();

  let ws = workspace::discover(root.path()).unwrap();
  let names: Vec<&str> = ws.repos.iter().map(|r| r.name.as_str()).collect();
  assert_eq!(names, vec!["alpha", "beta"], "only the two git repos, got {names:?}");
}

#[test]
fn discover_sorts_repos_alphabetically() {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("zulu"));
  init_repo_at(&root.path().join("mike"));
  init_repo_at(&root.path().join("alfa"));

  let ws = workspace::discover(root.path()).unwrap();
  let names: Vec<&str> = ws.repos.iter().map(|r| r.name.as_str()).collect();
  assert_eq!(names, vec!["alfa", "mike", "zulu"], "alphabetical, got {names:?}");
}

#[test]
fn discover_does_not_recurse_below_one_level() {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  // A repo nested two levels deep must NOT be discovered (one level only).
  init_repo_at(&root.path().join("group").join("nested"));

  let ws = workspace::discover(root.path()).unwrap();
  let names: Vec<&str> = ws.repos.iter().map(|r| r.name.as_str()).collect();
  assert_eq!(names, vec!["alpha"], "nested repo excluded, got {names:?}");
}

#[test]
fn discover_empty_root_yields_no_repos() {
  let root = TempDir::new().unwrap();
  fs::create_dir_all(root.path().join("plain")).unwrap();

  let ws = workspace::discover(root.path()).unwrap();
  assert!(ws.repos.is_empty(), "no git repos under the root");
  assert!(ws.is_empty(), "is_empty() reflects an empty repo set");
}

#[test]
fn discover_missing_root_is_an_error() {
  let root = TempDir::new().unwrap();
  let missing = root.path().join("does-not-exist");
  assert!(workspace::discover(&missing).is_err(), "a missing root must error");
}

#[test]
fn autodetect_fires_when_cwd_is_not_a_repo_but_holds_child_repos() {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));

  let ws = workspace::autodetect(root.path()).expect("a repo-free dir with child repos triggers");
  let names: Vec<&str> = ws.repos.iter().map(|r| r.name.as_str()).collect();
  assert_eq!(
    names,
    vec!["alpha", "beta"],
    "auto-detected workspace lists the children"
  );
}

#[test]
fn autodetect_declines_when_cwd_is_itself_a_repo() {
  // Inside a git repo, single-repo mode wins — never auto-open a workspace,
  // even if the repo happens to contain nested child repos.
  let root = TempDir::new().unwrap();
  init_repo_at(root.path());
  init_repo_at(&root.path().join("vendored"));

  assert!(
    workspace::autodetect(root.path()).is_none(),
    "a directory that is itself a repo must not auto-open as a workspace"
  );
}

#[test]
fn autodetect_declines_when_no_child_repos() {
  let root = TempDir::new().unwrap();
  fs::create_dir_all(root.path().join("plain")).unwrap();
  assert!(
    workspace::autodetect(root.path()).is_none(),
    "no child repos → nothing to open as a workspace"
  );
}

#[test]
fn merge_worktrees_tags_each_row_with_its_repo() {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));

  let ws = workspace::discover(root.path()).unwrap();
  let rows = workspace::merge_worktrees(&ws).unwrap();

  // Each repo contributes at least its main worktree; rows are grouped by
  // repo in discovery (alphabetical) order.
  let repo_names: Vec<&str> = rows.iter().map(|r| r.repo_name.as_str()).collect();
  assert!(repo_names.contains(&"alpha"), "alpha rows present: {repo_names:?}");
  assert!(repo_names.contains(&"beta"), "beta rows present: {repo_names:?}");

  // The main worktree of each repo is its repo directory.
  let alpha_main = rows
    .iter()
    .find(|r| r.repo_name == "alpha" && r.info.is_main)
    .expect("alpha main worktree row");
  assert!(
    alpha_main.info.path.ends_with("alpha"),
    "main worktree path points at the repo dir, got {:?}",
    alpha_main.info.path
  );
  // alpha sorts before beta — its rows come first.
  let first_beta = repo_names.iter().position(|n| *n == "beta").unwrap();
  let last_alpha = repo_names.iter().rposition(|n| *n == "alpha").unwrap();
  assert!(last_alpha < first_beta, "alpha rows precede beta rows: {repo_names:?}");
}
