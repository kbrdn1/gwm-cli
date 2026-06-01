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
  worktree::add(&repo, "feat-1-foo", &target, "feat/#1-foo", false).unwrap();

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
  worktree::add(&repo, "feat-7-launcher", &target, "feat/#7-launcher", false).unwrap();

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

  let err = worktree::add(&repo, "clash", &target, "feat/#9-x", false).unwrap_err();
  assert!(matches!(err, gwm::error::GwmError::WorktreeExists(_, _)));
}

#[test]
fn remove_deletes_dir_and_prunes() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-2-bar");
  worktree::add(&repo, "feat-2-bar", &target, "feat/#2-bar", false).unwrap();
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
  worktree::add(&repo, "feat-3-baz", &target, "feat/#3-baz", false).unwrap();

  worktree::remove(&repo, "feat-3-baz", true).unwrap();
  assert!(repo.find_branch("feat/#3-baz", git2::BranchType::Local).is_err());
}

#[test]
fn find_fuzzy_matches_substring() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-99-auth");
  worktree::add(&repo, "feat-99-auth", &target, "feat/#99-auth", false).unwrap();

  let found = worktree::find_fuzzy(&repo, "auth").unwrap();
  assert_eq!(found.name, "feat-99-auth");
}

#[test]
fn find_fuzzy_errors_on_ambiguous() {
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  worktree::add(&repo, "feat-1-foo", &wt_root.path().join("a"), "feat/#1-foo", false).unwrap();
  worktree::add(&repo, "feat-2-foo", &wt_root.path().join("b"), "feat/#2-foo", false).unwrap();

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

// --- Issue #31: dry-run plans for remove + prune -----------------------------

#[test]
fn prunable_worktrees_returns_empty_when_clean() {
  // Empty case for `prunable_worktrees`: a brand-new repo has no
  // worktree admin entries at all, so the plan list is empty. This
  // backs `gwm prune --dry-run` reporting "0 worktree(s) to prune".
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let plan = worktree::prunable_worktrees(&repo).unwrap();
  assert!(plan.is_empty());
}

#[test]
fn prunable_worktrees_lists_orphaned_admin_entry() {
  // When the working directory of a linked worktree is deleted out
  // from under the admin entry (a "ghost worktree"), libgit2 flags it
  // as prunable. `prunable_worktrees` must surface it with name, path,
  // and reason — the three columns the `--dry-run` CLI prints.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-31-ghost");
  worktree::add(&repo, "feat-31-ghost", &target, "feat/#31-ghost", false).unwrap();
  std::fs::remove_dir_all(&target).unwrap();

  let plan = worktree::prunable_worktrees(&repo).unwrap();
  assert_eq!(plan.len(), 1, "ghost worktree must appear in the prune plan");
  assert_eq!(plan[0].name, "feat-31-ghost");
  assert!(
    !plan[0].reason.is_empty(),
    "every prunable entry must carry a human reason"
  );

  // Sanity: the dry-run plan must not have mutated libgit2's state.
  assert!(
    repo.find_worktree("feat-31-ghost").is_ok(),
    "prunable_worktrees is read-only — the admin entry must still resolve"
  );
}

#[test]
fn prunable_worktrees_sorted_by_name() {
  // Deterministic output is a hard requirement of the `--dry-run`
  // contract — scripted callers diff stdout across runs. We pin the
  // sort order to ascending by `name`.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let zeta = wt_root.path().join("feat-99-zeta");
  let alpha = wt_root.path().join("feat-99-alpha");
  worktree::add(&repo, "feat-99-zeta", &zeta, "feat/#99-zeta", false).unwrap();
  worktree::add(&repo, "feat-99-alpha", &alpha, "feat/#99-alpha", false).unwrap();
  std::fs::remove_dir_all(&zeta).unwrap();
  std::fs::remove_dir_all(&alpha).unwrap();

  let plan = worktree::prunable_worktrees(&repo).unwrap();
  let names: Vec<&str> = plan.iter().map(|e| e.name.as_str()).collect();
  assert_eq!(names, vec!["feat-99-alpha", "feat-99-zeta"]);
}

#[test]
fn remove_with_dry_run_keeps_worktree_and_branch_intact() {
  // The libgit2-level pin for `worktree::remove(.., dry_run=true)`:
  // resolution still happens (the caller hands us a `name` that
  // matched), but no admin prune, no rmdir, no branch deletion. The
  // function must return Ok(()) so the CLI prints the plan and exits 0.
  let (_dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-31-keep");
  worktree::add(&repo, "feat-31-keep", &target, "feat/#31-keep", false).unwrap();
  assert!(target.exists());

  worktree::remove_dry_run(&repo, "feat-31-keep").unwrap();

  assert!(target.exists(), "dry-run must not delete the worktree dir");
  assert!(
    repo.find_branch("feat/#31-keep", git2::BranchType::Local).is_ok(),
    "dry-run must not delete the local branch"
  );
  assert!(
    repo.find_worktree("feat-31-keep").is_ok(),
    "dry-run must leave libgit2's worktree admin entry in place"
  );
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
  worktree::add(&repo, "feat-1-foo", &target, "feat/#1-foo", false).unwrap();

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

#[test]
fn branch_age_returns_none_when_no_trunk_candidate_exists_locally() {
  // PR #74 Copilot review: if none of the trunk candidates (main /
  // master / dev) resolves as a local branch, the revwalk hides nothing
  // and `branch_age` falls back to the repo's initial commit — turning
  // every branch into a misleadingly large age (the repo's lifetime).
  // The intent is "branch age relative to a trunk baseline"; without
  // a baseline, we must surface `None` so the UI renders `-`.
  let dir = TempDir::new().unwrap();
  let repo = Repository::init(dir.path()).unwrap();
  // Initialise the repo on a branch that's *not* a trunk candidate so
  // the seed commit lives on `feat/standalone`, not `main`/`master`/`dev`.
  repo.set_head("refs/heads/feat/standalone").ok();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

  assert!(
    worktree::branch_age(&repo, "feat/standalone").is_none(),
    "no trunk baseline → branch_age must be None, not the repo's lifetime"
  );
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

// --------------------------------------------------------------------------
// Issue #99 — refuse/reuse contract for `worktree::add` on pre-existing
// local branches. Issue #101 ships the E2E coverage; this block holds
// the libgit2-level pair.
// --------------------------------------------------------------------------
//
// These complement the CLI-level E2E tests in `tests/cli_binary.rs` by
// pinning the libgit2-level contract of `worktree::add` / `worktree::remove`.
// The pair below covers #99: with `reuse_branch: false` (the new default)
// `worktree::add` refuses to attach to a pre-existing local branch — the
// caller has to opt back into the historical reuse behaviour explicitly.

#[test]
fn add_refuses_stale_branch_without_reuse_flag() {
  // Issue #99 contract. Default (`reuse_branch: false`) must refuse to
  // resurrect a stale branch silently — the previous behaviour pointed
  // the new worktree at whatever commit the stale ref referenced, which
  // is invisible to the user until they run `git log` inside the new
  // worktree. The error carries the offending OID so the CLI can render
  // it in the message.
  let (_dir, repo) = init_repo();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();

  let main_oid = repo.head().unwrap().target().unwrap();
  let main_commit = repo.find_commit(main_oid).unwrap();
  let stale_branch = repo.branch("feat/#99-stale", &main_commit, false).unwrap();
  let stale_oid = stale_branch.into_reference().target().unwrap();

  // Advance main so HEAD diverges from the stale branch tip — that
  // divergence is what made the silent reuse a foot-gun.
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo
    .commit(Some("HEAD"), &sig, &sig, "advance main", &tree, &[&main_commit])
    .unwrap();

  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-99-stale");
  let err = worktree::add(&repo, "feat-99-stale", &target, "feat/#99-stale", false).unwrap_err();

  match err {
    gwm::error::GwmError::BranchExists { name, oid } => {
      assert_eq!(name, "feat/#99-stale");
      assert_eq!(
        oid,
        stale_oid.to_string(),
        "error must surface the stale branch tip so the user can audit it"
      );
    }
    other => panic!("expected BranchExists, got {:?}", other),
  }

  assert!(
    !target.exists(),
    "worktree dir must not be created when the branch is refused"
  );
}

#[test]
fn add_attaches_to_stale_branch_with_reuse_flag() {
  // Companion to `add_refuses_stale_branch_without_reuse_flag`: when the
  // caller passes `reuse_branch: true` (the explicit opt-in plumbed
  // through `--reuse-branch` on the CLI), the legacy attach-to-existing
  // behaviour applies — the new worktree comes up on whatever commit the
  // pre-existing branch references, and the branch tip is NOT moved to
  // HEAD. This pins the only escape hatch for #99.
  let (_dir, repo) = init_repo();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();

  let main_oid = repo.head().unwrap().target().unwrap();
  let main_commit = repo.find_commit(main_oid).unwrap();
  let stale_branch = repo.branch("feat/#99-stale", &main_commit, false).unwrap();
  let stale_oid = stale_branch.into_reference().target().unwrap();

  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  let new_head = repo
    .commit(Some("HEAD"), &sig, &sig, "advance main", &tree, &[&main_commit])
    .unwrap();
  assert_ne!(new_head, stale_oid, "precondition: HEAD must diverge from stale branch");

  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-99-stale");
  worktree::add(&repo, "feat-99-stale", &target, "feat/#99-stale", true).unwrap();

  let resolved = repo
    .find_branch("feat/#99-stale", git2::BranchType::Local)
    .unwrap()
    .into_reference()
    .target()
    .unwrap();
  assert_eq!(
    resolved, stale_oid,
    "with reuse_branch=true the existing branch tip is kept as-is"
  );
  assert!(target.exists(), "worktree dir must be created when reuse is opt-in");
}

#[test]
fn remove_prunes_admin_files_on_happy_path() {
  // Companion characterization for #98 — the happy path. After `remove`
  // succeeds, the admin directory under `.git/worktrees/<name>` must be
  // gone so `find_worktree` can no longer resolve a phantom entry. This
  // pins the post-condition that #98's fix must preserve (the fix
  // reorders prune-before-rmdir; the post-condition itself doesn't
  // change).
  let (dir, _repo) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-98-prune");
  worktree::add(&repo, "feat-98-prune", &target, "feat/#98-prune", false).unwrap();

  let admin_dir = dir.path().join(".git").join("worktrees").join("feat-98-prune");
  assert!(admin_dir.exists(), "precondition: admin entry exists after add");

  worktree::remove(&repo, "feat-98-prune", false).unwrap();

  assert!(!target.exists(), "remove must delete the worktree dir");
  assert!(
    !admin_dir.exists(),
    "remove must also prune the admin entry under .git/worktrees/"
  );
  assert!(
    repo.find_worktree("feat-98-prune").is_err(),
    "libgit2 must no longer resolve the pruned worktree by name"
  );
}

#[test]
#[cfg(unix)]
fn remove_failed_filesystem_unlink_still_prunes_metadata() {
  // Issue #98: `worktree::remove` must prune the admin metadata BEFORE
  // calling `fs::remove_dir_all`. Otherwise, a mid-way filesystem failure
  // leaves a "phantom worktree": directory gone, libgit2 metadata still
  // listing the name. `gwm list` shows a ghost row and `gwm bootstrap`
  // fails confusingly until the user runs `gwm prune` manually.
  //
  // We force `remove_dir_all` to fail by stripping `w` from the worktree's
  // PARENT (the final `rmdir(target)` needs write on its parent). With the
  // fix, prune ran first → the admin entry is already gone. With the
  // buggy ordering, prune never runs → `find_worktree` still resolves
  // the ghost name.
  use std::os::unix::fs::PermissionsExt;
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-98-ghost");
  worktree::add(&repo, "feat-98-ghost", &target, "feat/#98-ghost", false).unwrap();

  // Capture the original mode so we can restore EXACTLY what TempDir
  // gave us (mac defaults to 0o700, linux 0o755, umask-dependent on
  // both). Hard-coding 0o755 in the restore would widen permissions
  // on macOS, which is harmless for cleanup but a needless mutation —
  // and would mask any future regression where `set_mode` itself
  // misbehaves on a quirky tmpfs.
  let original_mode = std::fs::metadata(wt_root.path()).unwrap().permissions().mode();
  let mut parent_perms = std::fs::metadata(wt_root.path()).unwrap().permissions();
  parent_perms.set_mode(0o555);
  std::fs::set_permissions(wt_root.path(), parent_perms).unwrap();

  let result = worktree::remove(&repo, "feat-98-ghost", false);

  // Restore the exact original mode so tempdir cleanup succeeds even
  // if the assertions below panic.
  let mut restore = std::fs::metadata(wt_root.path()).unwrap().permissions();
  restore.set_mode(original_mode);
  std::fs::set_permissions(wt_root.path(), restore).unwrap();

  assert!(
    result.is_err(),
    "remove must surface the filesystem failure as an error"
  );
  assert!(
    repo.find_worktree("feat-98-ghost").is_err(),
    "prune must run BEFORE remove_dir_all so a failed unlink cannot leave a phantom worktree"
  );
}

// --------------------------------------------------------------------------
// Issue #103 — `WorktreeInfo.age` pre-computed at list time so the TUI
// render loop no longer opens a fresh `git2::Repository` per row per frame.
// --------------------------------------------------------------------------

#[test]
fn list_populates_age_on_feature_worktree() {
  // Issue #103: the TUI used to call `branch_age_for(w)` per row per frame,
  // which opened a `git2::Repository` and ran a revwalk every time. The fix
  // moves that computation into `worktree::list()` so the render path becomes
  // pure read-only struct field access. Asserting `WorktreeInfo.age` is
  // populated by `list()` pins the new contract: the TUI is no longer
  // permitted to open libgit2 handles on the render path.
  let (dir, repo) = init_repo();

  // Pin a `feat/#103-age` branch with one commit dated 2 days ago so the
  // formatter has something stable to read.
  let two_days_ago = chrono::Utc::now().timestamp() - 2 * 86_400;
  let main_oid = repo.head().unwrap().target().unwrap();
  let main_commit = repo.find_commit(main_oid).unwrap();
  repo.branch("feat/#103-age", &main_commit, false).unwrap();
  commit_with_time(
    dir.path(),
    &repo,
    "refs/heads/feat/#103-age",
    "branch-old",
    two_days_ago,
  );

  // Attach a worktree on that branch and list. The branch was created
  // above, so `reuse_branch=true` is required (the #99 stale-branch
  // refusal would otherwise reject this `add`).
  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("feat-103-age");
  worktree::add(&repo, "feat-103-age", &target, "feat/#103-age", true).unwrap();

  let trees = worktree::list(&repo).unwrap();
  let feature = trees
    .iter()
    .find(|w| w.name == "feat-103-age")
    .expect("feature worktree must appear in list");

  let age = feature
    .age
    .expect("WorktreeInfo.age must be Some on a feature branch with divergence");
  let drift = age.as_secs().abs_diff(2 * 86_400);
  assert!(
    drift < 300,
    "expected ~2 days on the cached age field, got {}s (drift {}s)",
    age.as_secs(),
    drift
  );
}

#[test]
fn list_returns_none_age_for_main_worktree() {
  // Trunk branches (`main` / `master` / `dev`) have no meaningful "branch
  // age" — `worktree::list()` must surface `None` so the TUI renders `-`,
  // matching the prior `branch_age_for` semantics.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let trees = worktree::list(&repo).unwrap();
  let main = trees
    .iter()
    .find(|w| w.is_main)
    .expect("main worktree must appear in list");
  assert!(
    main.age.is_none(),
    "main worktree on a trunk branch must report age = None, got {:?}",
    main.age
  );
}

// --- git_stash_list (issue #34) -----------------------------------------

/// Create one stash on the given tempdir repo by writing a tracked file,
/// staging an edit, then `git stash push -m <subject>`. Returns once the
/// stash has been created so the caller can immediately `git_stash_list`.
fn create_stash(path: &Path, file_rel: &str, subject: &str) {
  // Seed a tracked file (commit) then mutate it so `git stash push`
  // has a non-empty diff to capture. `git stash` on an empty diff is a
  // no-op and would make the test pin the wrong contract.
  let abs = path.join(file_rel);
  std::fs::write(&abs, "v1\n").unwrap();
  let run = |args: &[&str]| {
    let status = std::process::Command::new("git")
      .arg("-C")
      .arg(path)
      .args(args)
      .status()
      .unwrap();
    assert!(status.success(), "git {:?} failed", args);
  };
  run(&["add", file_rel]);
  run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "seed"]);
  std::fs::write(&abs, "v2 dirty\n").unwrap();
  run(&["stash", "push", "-m", subject]);
}

#[test]
fn git_stash_list_empty_returns_empty_vec() {
  // A fresh repo has no stashes; the helper must report that as an
  // empty `Vec`, not an error. The sidebar renderer relies on this to
  // distinguish "(no stashes)" from "git stash list failed".
  let (dir, _) = init_repo();
  let entries = worktree::git_stash_list(dir.path(), 10).unwrap();
  assert!(entries.is_empty());
}

#[test]
fn git_stash_list_parses_canonical_output() {
  let (dir, _) = init_repo();
  create_stash(dir.path(), "a.txt", "wip on auth refactor");
  create_stash(dir.path(), "b.txt", "wip on docs");

  let entries = worktree::git_stash_list(dir.path(), 10).unwrap();
  assert_eq!(entries.len(), 2);
  // git stashes are LIFO — the most recent push is `stash@{0}`.
  assert_eq!(entries[0].ref_name, "stash@{0}");
  assert!(
    entries[0].subject.contains("wip on docs"),
    "expected the latest stash subject, got: {}",
    entries[0].subject
  );
  assert_eq!(entries[1].ref_name, "stash@{1}");
  assert!(
    entries[1].subject.contains("wip on auth refactor"),
    "expected the earlier stash subject, got: {}",
    entries[1].subject
  );
}

#[test]
fn git_stash_list_respects_limit() {
  // The helper caps the returned vec at `limit` so the sidebar
  // doesn't allocate an unbounded list on a repo with hundreds of
  // stashes. The full list is still available through `git stash`
  // directly — this is a preview-only cap.
  let (dir, _) = init_repo();
  create_stash(dir.path(), "a.txt", "first");
  create_stash(dir.path(), "b.txt", "second");
  create_stash(dir.path(), "c.txt", "third");

  let limited = worktree::git_stash_list(dir.path(), 2).unwrap();
  assert_eq!(limited.len(), 2, "limit must cap the result vec");
}
