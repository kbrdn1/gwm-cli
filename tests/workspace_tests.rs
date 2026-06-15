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
fn discover_skips_a_linked_worktree_sibling() {
  // A linked worktree of `main` sitting as a sibling under the root must NOT
  // be admitted as its own repo: `main`'s `worktree::list` already emits it,
  // so admitting it would duplicate rows (Codex review #303 P2).
  let root = TempDir::new().unwrap();
  let main = init_repo_at(&root.path().join("main"));
  // Create a linked worktree at root/wt (a sibling of main, one level deep).
  main
    .worktree("wt", &root.path().join("wt"), None)
    .expect("create linked worktree");

  let ws = workspace::discover(root.path()).unwrap();
  let names: Vec<&str> = ws.repos.iter().map(|r| r.name.as_str()).collect();
  assert_eq!(
    names,
    vec!["main"],
    "the linked worktree sibling is skipped, got {names:?}"
  );
}

#[test]
fn discover_includes_an_orphan_linked_worktree_via_its_owner() {
  // A linked worktree under the root whose *owner* main checkout lives OUTSIDE
  // the root must not be dropped: it resolves to (and is listed as) its owner,
  // so the checkout stays reachable (issue #304).
  let owner_dir = TempDir::new().unwrap();
  let owner = init_repo_at(owner_dir.path());
  let root = TempDir::new().unwrap();
  owner
    .worktree("wt", &root.path().join("wt"), None)
    .expect("create linked worktree under the root");

  let ws = workspace::discover(root.path()).unwrap();
  assert_eq!(
    ws.repos.len(),
    1,
    "the orphan worktree resolves to one owner repo, got {:?}",
    ws.repos
  );
  let got = ws.repos[0]
    .path
    .canonicalize()
    .unwrap_or_else(|_| ws.repos[0].path.clone());
  let want = owner_dir
    .path()
    .canonicalize()
    .unwrap_or_else(|_| owner_dir.path().to_path_buf());
  assert_eq!(got, want, "the workspace repo is the owner checkout");
}

#[test]
fn discover_skips_the_root_repos_own_git_dir() {
  // When the root is itself a git repo, `read_dir` surfaces its `.git/`, which
  // `Repository::open` resolves back to the root repo. That must NOT appear as
  // a bogus `.git` child — only the genuine child repo does (Codex review
  // #303 P2).
  let root = TempDir::new().unwrap();
  init_repo_at(root.path()); // the root is itself a repo
  init_repo_at(&root.path().join("kid"));

  let ws = workspace::discover(root.path()).unwrap();
  let names: Vec<&str> = ws.repos.iter().map(|r| r.name.as_str()).collect();
  assert_eq!(names, vec!["kid"], "only the real child repo, no `.git`, got {names:?}");
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
