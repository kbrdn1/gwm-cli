//! Integration tests for the `worktree` module. Each test spins up a real
//! git repository in a tempdir, performs the operation under test, and asserts
//! the on-disk and libgit2 state.

mod common;

use common::{init_repo, paths_equal};
use git2::{Repository, Signature, Time};
use gwm::worktree;
use std::path::Path;
use std::time::Duration;
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

// Issue #73: relative-duration formatter + branch age. The formatter is a
// pure function (table-driven tests below); `branch_age` walks the commit
// graph and needs a real repo with controlled commit timestamps.

#[test]
fn format_relative_duration_under_one_minute_renders_seconds() {
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(0)), "0s");
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(1)), "1s");
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(59)), "59s");
}

#[test]
fn format_relative_duration_steps_through_units() {
  // Anchor cases at the lazygit boundary (>= unit threshold → render in that unit).
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(60)), "1m");
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(60 * 59)), "59m");
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(3600)), "1h");
  assert_eq!(
    worktree::format_relative_duration(Duration::from_secs(3600 * 23)),
    "23h"
  );
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(86_400)), "1d");
  assert_eq!(
    worktree::format_relative_duration(Duration::from_secs(86_400 * 6)),
    "6d"
  );
  assert_eq!(
    worktree::format_relative_duration(Duration::from_secs(86_400 * 7)),
    "1w"
  );
  // 4 weeks is still rendered as weeks; the month cutoff sits at ~30 days.
  assert_eq!(
    worktree::format_relative_duration(Duration::from_secs(86_400 * 28)),
    "4w"
  );
}

#[test]
fn format_relative_duration_handles_months_and_years() {
  // Month uses a 30.25-day approximation (lazygit `pkg/utils/date.go`).
  let one_month = 30 * 86_400 + 6 * 3600;
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(one_month)), "1M");
  assert_eq!(
    worktree::format_relative_duration(Duration::from_secs(one_month * 11)),
    "11M"
  );
  let one_year = 365 * 86_400 + 6 * 3600;
  assert_eq!(worktree::format_relative_duration(Duration::from_secs(one_year)), "1y");
  assert_eq!(
    worktree::format_relative_duration(Duration::from_secs(one_year * 3 + 86_400 * 10)),
    "3y"
  );
}

#[test]
fn format_relative_duration_output_stays_under_four_chars_for_realistic_inputs() {
  // Lazygit's recency column is documented as "always three characters";
  // gwm cell is slightly more lenient (4) but the lazygit promise must hold
  // for every value below 100 of any unit.
  for secs in [
    0, 1, 59, 60, 3599, 3600, 86_399, 86_400, 604_799, 604_800, 2_595_600, 2_595_601,
  ] {
    let out = worktree::format_relative_duration(Duration::from_secs(secs));
    assert!(
      out.len() <= 4,
      "format_relative_duration({}s) = {:?} exceeded 4 chars",
      secs,
      out
    );
  }
}

#[test]
fn branch_age_returns_none_for_main_only_repo() {
  // Repo with a single branch (`main`) and no divergence has no "branch
  // creation" date — `branch_age` returns None so the UI can fall back to
  // a dash.
  let (dir, _) = init_repo();
  let repo = Repository::open(dir.path()).unwrap();
  assert!(worktree::branch_age(&repo, "main").is_none());
}

#[test]
fn branch_age_reflects_oldest_branch_commit() {
  // Build a `feat/age` branch with two commits — `branch_age` must return
  // the elapsed time since the *oldest* commit on that branch (the one
  // that pinned the branch creation), not the latest tip commit.
  let (dir, repo) = init_repo();
  // Anchor "branch creation" at a known instant — 3 days ago, so the
  // formatter rendering layer can also be sanity-checked downstream.
  let three_days_ago = chrono::Utc::now().timestamp() - 3 * 86_400;
  let one_hour_ago = chrono::Utc::now().timestamp() - 3600;

  let main_oid = repo.head().unwrap().target().unwrap();
  let main_commit = repo.find_commit(main_oid).unwrap();
  repo.branch("feat/age", &main_commit, false).unwrap();

  // First commit on the branch — dated 3 days ago.
  commit_with_time(dir.path(), &repo, "refs/heads/feat/age", "branch-old", three_days_ago);
  // Second commit on the branch — dated 1 hour ago.
  commit_with_time(dir.path(), &repo, "refs/heads/feat/age", "branch-recent", one_hour_ago);

  let age = worktree::branch_age(&repo, "feat/age").expect("branch must have an age");
  let three_days_secs = 3 * 86_400;
  // Allow a 5-minute wiggle for test execution time.
  let drift = age.as_secs().abs_diff(three_days_secs);
  assert!(
    drift < 300,
    "expected ~{} seconds, got {} (drift {}s)",
    three_days_secs,
    age.as_secs(),
    drift
  );
}

#[test]
fn branch_age_treats_master_and_dev_as_trunks() {
  // A `feat/work` branch must still get a non-None age even when the
  // default trunk is `dev` (not `main`). Verifies the trunk-candidates
  // list covers the common conventions.
  let dir = TempDir::new().unwrap();
  let repo = Repository::init(dir.path()).unwrap();
  repo.set_head("refs/heads/dev").ok();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
  let dev_oid = repo.head().unwrap().target().unwrap();
  let dev_commit = repo.find_commit(dev_oid).unwrap();
  repo.branch("feat/work", &dev_commit, false).unwrap();

  let two_days_ago = chrono::Utc::now().timestamp() - 2 * 86_400;
  commit_with_time(dir.path(), &repo, "refs/heads/feat/work", "branch-commit", two_days_ago);

  let age = worktree::branch_age(&repo, "feat/work").expect("dev-rooted branch must have an age");
  let drift = age.as_secs().abs_diff(2 * 86_400);
  assert!(drift < 300, "expected ~2 days, got {}s", age.as_secs());
}

/// Helper: append a commit (empty tree, configurable timestamp) on top of
/// the given ref. The committer / author share the same timestamp so
/// `branch_age` (which reads committer time) is deterministic.
fn commit_with_time(workdir: &Path, repo: &Repository, ref_name: &str, message: &str, unix_secs: i64) {
  let _ = workdir; // currently unused but reserved if we later need to touch the index
  let time = Time::new(unix_secs, 0);
  let sig = Signature::new("gwm-test", "gwm@test", &time).unwrap();
  let parent_oid = repo.find_reference(ref_name).unwrap().target().unwrap();
  let parent = repo.find_commit(parent_oid).unwrap();
  let tree_id = parent.tree_id();
  let tree = repo.find_tree(tree_id).unwrap();
  repo
    .commit(Some(ref_name), &sig, &sig, message, &tree, &[&parent])
    .unwrap();
}
