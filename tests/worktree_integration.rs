//! Integration tests for the `worktree` module. Each test spins up a real
//! git repository in a tempdir, performs the operation under test, and asserts
//! the on-disk and libgit2 state.

mod common;

use common::{init_repo, paths_equal};
use gwm::worktree;
use tempfile::TempDir;

#[test]
fn discover_finds_repo() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  assert!(paths_equal(repo.workdir().unwrap(), dir.path()));
}

#[test]
fn list_includes_main_worktree() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let trees = worktree::list(&repo).unwrap();
  assert_eq!(trees.len(), 1, "only the main worktree should exist");
  assert!(trees[0].is_main);
  assert!(paths_equal(&trees[0].path, dir.path()));
}

#[test]
fn add_creates_branch_and_worktree() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-1-foo");
  worktree::add(&repo, "feat-1-foo", &target, "feat/#1-foo").unwrap();

  assert!(target.exists(), "worktree dir should exist on disk");
  assert!(repo.find_branch("feat/#1-foo", git2::BranchType::Local).is_ok());

  let trees = worktree::list(&repo).unwrap();
  assert_eq!(trees.len(), 2);
  assert!(trees.iter().any(|w| w.name == "feat-1-foo" && !w.is_main));
}

#[test]
fn add_records_gwm_base_for_new_branch() {
  // Issue #75: `branch.<name>.gwm-base` is the second link in the
  // review base-resolution chain. `gwm create` (via `worktree::add`)
  // must set it to HEAD's short name so the review launcher can fall
  // back to the original parent even on branches without an upstream.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-7-launcher");
  worktree::add(&repo, "feat-7-launcher", &target, "feat/#7-launcher").unwrap();

  let cfg = repo.config().unwrap();
  let base = cfg.get_string("branch.feat/#7-launcher.gwm-base").unwrap();
  assert_eq!(
    base, "main",
    "worktree::add must record HEAD's short name as gwm-base for the review fallback"
  );
}

#[test]
fn add_refuses_to_clobber_existing_dir() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("clash");
  std::fs::create_dir(&target).unwrap();

  let err = worktree::add(&repo, "clash", &target, "feat/#9-x").unwrap_err();
  assert!(matches!(err, gwm::error::GwmError::WorktreeExists(_, _)));
}

#[test]
fn remove_deletes_dir_and_prunes() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-2-bar");
  worktree::add(&repo, "feat-2-bar", &target, "feat/#2-bar").unwrap();
  assert!(target.exists());

  worktree::remove(&repo, "feat-2-bar", false).unwrap();
  assert!(!target.exists(), "worktree dir should be deleted");

  let trees = worktree::list(&repo).unwrap();
  assert_eq!(trees.len(), 1, "only main should remain");
}

#[test]
fn remove_with_delete_branch_drops_branch() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-3-baz");
  worktree::add(&repo, "feat-3-baz", &target, "feat/#3-baz").unwrap();

  worktree::remove(&repo, "feat-3-baz", true).unwrap();
  assert!(repo.find_branch("feat/#3-baz", git2::BranchType::Local).is_err());
}

#[test]
fn find_fuzzy_matches_substring() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-99-auth");
  worktree::add(&repo, "feat-99-auth", &target, "feat/#99-auth").unwrap();

  let found = worktree::find_fuzzy(&repo, "auth").unwrap();
  assert_eq!(found.name, "feat-99-auth");
}

#[test]
fn find_fuzzy_errors_on_ambiguous() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  worktree::add(&repo, "feat-1-foo", &wt_root.path().join("a"), "feat/#1-foo").unwrap();
  worktree::add(&repo, "feat-2-foo", &wt_root.path().join("b"), "feat/#2-foo").unwrap();

  let err = worktree::find_fuzzy(&repo, "foo").unwrap_err();
  assert!(matches!(err, gwm::error::GwmError::Other(_)));
}

#[test]
fn prune_returns_zero_when_clean() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let n = worktree::prune(&repo).unwrap();
  assert_eq!(n, 0);
}

#[test]
fn repo_name_derives_from_workdir() {
  let parent = TempDir::new().unwrap();
  let workdir = parent.path().join("my-cool-repo");
  std::fs::create_dir(&workdir).unwrap();
  git2::Repository::init(&workdir).unwrap();
  let repo = worktree::discover_repo(Some(&workdir)).unwrap();
  assert_eq!(worktree::repo_name(&repo), "my-cool-repo");
}

#[test]
fn discover_from_inside_linked_worktree_walks_back_to_main() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-1-foo");
  worktree::add(&repo, "feat-1-foo", &target, "feat/#1-foo").unwrap();

  let main_again = worktree::discover_repo(Some(&target)).unwrap();
  assert!(paths_equal(main_again.workdir().unwrap(), dir.path()));
}

// ---- git_log_oneline / git_status_short -------------------------------------

#[test]
fn git_log_oneline_returns_seed_commit() {
  let (dir, _) = init_repo();
  let out = worktree::git_log_oneline(dir.path(), 10).unwrap();
  let lines: Vec<&str> = out.lines().collect();
  assert_eq!(lines.len(), 1, "init_repo seeds one commit, got: {:?}", lines);
  assert!(
    lines[0].contains("init"),
    "expected seed commit message 'init', got: {}",
    lines[0]
  );
}

#[test]
fn git_log_oneline_respects_limit() {
  use git2::Signature;
  let (dir, repo) = init_repo();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  // Add two extra commits on top of the seed → 3 total.
  for i in 0..2 {
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = repo.find_tree(repo.index().unwrap().write_tree().unwrap()).unwrap();
    repo
      .commit(Some("HEAD"), &sig, &sig, &format!("c{}", i), &tree, &[&parent])
      .unwrap();
  }
  let out = worktree::git_log_oneline(dir.path(), 2).unwrap();
  assert_eq!(out.lines().count(), 2);
}

#[test]
fn git_status_short_empty_on_clean_repo() {
  let (dir, _) = init_repo();
  let out = worktree::git_status_short(dir.path()).unwrap();
  assert!(
    out.trim().is_empty(),
    "clean repo should produce empty status, got: {:?}",
    out
  );
}

#[test]
fn git_status_short_lists_untracked_file() {
  let (dir, _) = init_repo();
  std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
  let out = worktree::git_status_short(dir.path()).unwrap();
  assert!(
    out.contains("new.txt"),
    "expected untracked new.txt in status, got: {:?}",
    out
  );
}

#[test]
fn git_log_oneline_errors_outside_repo() {
  let empty = TempDir::new().unwrap();
  let err = worktree::git_log_oneline(empty.path(), 5);
  assert!(err.is_err(), "expected error outside a git repo, got: {:?}", err);
}
