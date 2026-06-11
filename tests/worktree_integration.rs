//! Integration tests for the `worktree` module. Each test spins up a real
//! git repository in a tempdir, performs the operation under test, and asserts
//! the on-disk and libgit2 state.

mod common;

use common::{init_repo, paths_equal};
use git2::{Repository, Signature, Time};
use gwm::worktree;
use std::path::Path;
use std::process::Command;
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
  // find_fuzzy matches on the display name (the directory basename, #290), so
  // the dirs must carry the searched substring — as gwm-created worktrees do
  // (basename == slug).
  worktree::add(
    &repo,
    "feat-1-foo",
    &wt_root.path().join("feat-1-foo"),
    "feat/#1-foo",
    false,
  )
  .unwrap();
  worktree::add(
    &repo,
    "feat-2-foo",
    &wt_root.path().join("feat-2-foo"),
    "feat/#2-foo",
    false,
  )
  .unwrap();

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

// ---- run_git (issue #237) ---------------------------------------------------
// The shared `git -C <dir> <args>` helper that the sidebar/PR shell-outs and
// `gwm sync` both route through. These pin the two-branch contract the dedup
// must preserve: stdout verbatim on success, and a non-zero git exit mapped to
// `GwmError::CommandFailed` carrying git's own stderr.

#[test]
fn run_git_returns_stdout_on_success() {
  let (dir, _) = init_repo();
  let out = worktree::run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).unwrap();
  assert_eq!(
    out.trim(),
    "true",
    "rev-parse stdout should be returned verbatim, got: {:?}",
    out
  );
}

#[test]
fn run_git_maps_nonzero_exit_to_command_failed_with_stderr() {
  let (dir, _) = init_repo();
  // `git` runs and exits non-zero (the ref does not exist) — this exercises
  // the status-failure branch, not the spawn-failure branch.
  let err = worktree::run_git(dir.path(), &["rev-parse", "--verify", "definitely-missing-ref"]);
  match err {
    Err(gwm::error::GwmError::CommandFailed(msg)) => {
      assert!(
        msg.contains("fatal"),
        "CommandFailed must carry git's stderr (expected 'fatal'), got: {}",
        msg
      );
    }
    other => panic!("expected GwmError::CommandFailed, got: {:?}", other),
  }
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

#[test]
fn branch_age_prefers_persisted_local_branch_creation_time() {
  let (_dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("feat/#285-local-created", &head, false).unwrap();

  let created = chrono::Utc::now().timestamp() - 3 * 86_400;
  repo
    .config()
    .unwrap()
    .set_str("branch.feat/#285-local-created.gwm-created-at", &created.to_string())
    .unwrap();

  let age = worktree::branch_age(&repo, "feat/#285-local-created")
    .expect("persisted local creation timestamp should define branch age");
  let drift = age.as_secs().abs_diff(3 * 86_400);
  assert!(
    drift < 300,
    "expected ~3 days from local branch creation, got {}s (drift {}s)",
    age.as_secs(),
    drift
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

#[test]
fn resolve_trunk_falls_back_to_master_when_no_main() {
  // A repo whose only trunk-ish branch is `master` (no `main`) must
  // resolve to "master" via the COMMON_TRUNKS fallback even when the
  // caller passes no configured trunks.
  let (_dir, repo) = init_repo();

  // init_repo seeds `main`; create `master` at the same commit, point
  // HEAD at it, then drop `main` so `master` is the sole local branch.
  let head_oid = repo.head().unwrap().target().unwrap();
  let head_commit = repo.find_commit(head_oid).unwrap();
  repo.branch("master", &head_commit, false).unwrap();
  repo.set_head("refs/heads/master").unwrap();
  repo
    .find_branch("main", git2::BranchType::Local)
    .unwrap()
    .delete()
    .unwrap();

  assert!(
    repo.find_branch("main", git2::BranchType::Local).is_err(),
    "main should be gone for this test"
  );

  let resolved = worktree::resolve_trunk(&repo, &[]);
  assert_eq!(resolved.as_deref(), Some("master"));
}

#[test]
fn resolve_trunk_prefers_configured_over_fallback() {
  // With both `dev` and `main` present, a configured trunk of `dev`
  // must win over the COMMON_TRUNKS fallback (which would otherwise
  // pick `main` first).
  let (_dir, repo) = init_repo();

  let head_oid = repo.head().unwrap().target().unwrap();
  let head_commit = repo.find_commit(head_oid).unwrap();
  repo.branch("dev", &head_commit, false).unwrap();

  assert!(
    repo.find_branch("main", git2::BranchType::Local).is_ok(),
    "main should exist for this test"
  );
  assert!(
    repo.find_branch("dev", git2::BranchType::Local).is_ok(),
    "dev should exist for this test"
  );

  let resolved = worktree::resolve_trunk(&repo, &["dev".to_string()]);
  assert_eq!(resolved.as_deref(), Some("dev"));
}

// ---- git_diff_stat_vs_base / parse_diff_shortstat (issue #287) -------------

/// Run a `git` CLI command in `dir`, asserting success. Used to author
/// commits with controlled line counts so the shortstat diff is
/// deterministic — the helper itself shells out to `git diff`, so driving
/// the fixture through the same CLI mirrors the production path.
fn git_in(dir: &Path, args: &[&str]) {
  let out = std::process::Command::new("git")
    .current_dir(dir)
    .args(args)
    .env("GIT_AUTHOR_NAME", "gwm-test")
    .env("GIT_AUTHOR_EMAIL", "gwm@test")
    .env("GIT_COMMITTER_NAME", "gwm-test")
    .env("GIT_COMMITTER_EMAIL", "gwm@test")
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "git {:?} failed: {}",
    args,
    String::from_utf8_lossy(&out.stderr)
  );
}

#[test]
fn parse_diff_shortstat_reads_both_clauses() {
  let s = worktree::parse_diff_shortstat(" 3 files changed, 12 insertions(+), 4 deletions(-)");
  assert_eq!(s.insertions, 12);
  assert_eq!(s.deletions, 4);
}

#[test]
fn parse_diff_shortstat_handles_missing_clause_and_singular() {
  // All-additions diff omits the deletions clause entirely.
  let add_only = worktree::parse_diff_shortstat(" 1 file changed, 5 insertions(+)");
  assert_eq!(add_only.insertions, 5);
  assert_eq!(add_only.deletions, 0);

  // All-deletions, and the singular `1 deletion(-)` form.
  let del_only = worktree::parse_diff_shortstat(" 1 file changed, 1 deletion(-)");
  assert_eq!(del_only.insertions, 0);
  assert_eq!(del_only.deletions, 1);

  // Empty diff → empty string → zeroed stat.
  let empty = worktree::parse_diff_shortstat("");
  assert!(empty.is_empty());
}

#[test]
fn diff_stat_vs_base_counts_branch_insertions_and_deletions() {
  // init_repo seeds an empty commit on `main`. Author a base file there,
  // then branch and change it so the three-dot diff is deterministic.
  let (dir, _repo) = init_repo();
  let path = dir.path();

  std::fs::write(path.join("f.txt"), "a\nb\nc\n").unwrap();
  git_in(path, &["add", "f.txt"]);
  git_in(path, &["commit", "-m", "base file"]);

  // Branch off main and replace 1 of the 3 lines: +1 insertion, -1 deletion
  // versus the merge-base.
  git_in(path, &["checkout", "-b", "feat/#287-x"]);
  std::fs::write(path.join("f.txt"), "a\nB\nc\n").unwrap();
  git_in(path, &["commit", "-am", "tweak"]);

  let stat = worktree::git_diff_stat_vs_base(path, &["main".to_string()])
    .unwrap()
    .expect("a feature branch off main must yield a diff stat");
  assert_eq!(stat.insertions, 1, "one line replaced → one insertion");
  assert_eq!(stat.deletions, 1, "one line replaced → one deletion");
}

#[test]
fn diff_stat_vs_base_is_none_when_head_is_the_trunk() {
  // The main worktree resting on its trunk has no base distinct from
  // itself — the helper short-circuits to `None` so the sidebar paints no
  // Diff line rather than a misleading `+0 -0`.
  let (dir, _repo) = init_repo();
  let stat = worktree::git_diff_stat_vs_base(dir.path(), &["main".to_string()]).unwrap();
  assert!(stat.is_none(), "HEAD on the trunk must yield None, got {stat:?}");
}

#[test]
fn diff_stat_vs_base_is_none_for_a_later_trunk_worktree() {
  // Issue #287 review (P2): with the default `["dev", "main"]`, a worktree
  // whose HEAD is on `main` resolves its base to `dev` (the earlier
  // candidate). A naive `head == resolved_base` check would leak a
  // `dev...main` diff onto a trunk worktree; HEAD being *any* trunk must
  // suppress the row.
  let (dir, repo) = init_repo(); // seeds `main`
  let head_oid = repo.head().unwrap().target().unwrap();
  let head_commit = repo.find_commit(head_oid).unwrap();
  repo.branch("dev", &head_commit, false).unwrap();

  let stat = worktree::git_diff_stat_vs_base(dir.path(), &["dev".to_string(), "main".to_string()]).unwrap();
  assert!(
    stat.is_none(),
    "a worktree on the `main` trunk must yield None even when `dev` resolves as base, got {stat:?}"
  );
}

#[test]
fn is_trunk_branch_matches_configured_and_common_defaults() {
  assert!(worktree::is_trunk_branch("main", &[]));
  assert!(worktree::is_trunk_branch("develop", &[]));
  assert!(worktree::is_trunk_branch("release", &["release".to_string()]));
  assert!(!worktree::is_trunk_branch("feat/#287-x", &[]));
}

// ---- rename_worktree (#290) ----------------------------------------------

#[test]
fn rename_worktree_renames_local_branch_and_moves_dir() {
  // No origin remote: rename_worktree renames the local branch and moves the
  // worktree directory on disk, and reports remote_renamed == false.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-1-old");
  worktree::add(&repo, "feat-1-old", &old_path, "feat/#1-old", false).unwrap();

  let new_path = wt_root.path().join("feat-1-new");
  let remote_renamed =
    worktree::rename_worktree(dir.path(), &old_path, "feat/#1-old", &new_path, "feat/#1-new").unwrap();

  assert!(!remote_renamed, "no origin remote → remote branch not renamed");
  assert!(new_path.exists(), "worktree directory must move to the new path");
  assert!(!old_path.exists(), "old worktree directory must be gone");
  assert!(
    repo.find_branch("feat/#1-new", git2::BranchType::Local).is_ok(),
    "local branch must be renamed to feat/#1-new"
  );
  assert!(
    repo.find_branch("feat/#1-old", git2::BranchType::Local).is_err(),
    "old local branch must no longer exist"
  );
}

#[test]
fn rename_worktree_records_its_git_steps_in_the_command_log() {
  // The rename action (`c`, #290) shells out to `git worktree move` +
  // `git branch -m` through the captured-output path; those mutating steps
  // must surface in the Command Logs modal. Before this fix they ran through
  // an unlogged helper, so the user could not find them in the log.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-99-cmdlog-old");
  worktree::add(&repo, "feat-99-cmdlog-old", &old_path, "feat/#99-cmdlog-old", false).unwrap();

  let new_path = wt_root.path().join("feat-99-cmdlog-new");
  worktree::rename_worktree(
    dir.path(),
    &old_path,
    "feat/#99-cmdlog-old",
    &new_path,
    "feat/#99-cmdlog-new",
  )
  .unwrap();

  // Presence by the unique branch name: a sibling test cannot collide.
  let recorded = gwm::command_log::snapshot();
  assert!(
    recorded
      .iter()
      .any(|e| e.command.starts_with("git worktree move") && e.command.contains("feat-99-cmdlog-new")),
    "the `git worktree move` step must be recorded; got: {:?}",
    recorded.iter().map(|e| &e.command).collect::<Vec<_>>()
  );
  assert!(
    recorded
      .iter()
      .any(|e| e.command.starts_with("git branch -m") && e.command.contains("feat/#99-cmdlog-new")),
    "the `git branch -m` step must be recorded"
  );
}

#[test]
fn rename_worktree_renames_remote_branch_when_pushed() {
  // With the old branch pushed to a bare origin, rename_worktree also renames
  // the remote branch (delete old ref + push new) and reports remote_renamed.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();

  let remote_dir = TempDir::new().unwrap();
  let ok = Command::new("git")
    .args(["init", "--bare", &remote_dir.path().to_string_lossy()])
    .output()
    .unwrap()
    .status
    .success();
  assert!(ok, "bare remote init must succeed");
  Command::new("git")
    .args(["remote", "add", "origin", &remote_dir.path().to_string_lossy()])
    .current_dir(dir.path())
    .output()
    .unwrap();

  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-2-old");
  worktree::add(&repo, "feat-2-old", &old_path, "feat/#2-old", false).unwrap();
  let pushed = Command::new("git")
    .args(["push", "origin", "feat/#2-old"])
    .current_dir(&old_path)
    .output()
    .unwrap();
  assert!(
    pushed.status.success(),
    "push of the old branch must succeed: {}",
    String::from_utf8_lossy(&pushed.stderr)
  );

  let new_path = wt_root.path().join("feat-2-new");
  let remote_renamed =
    worktree::rename_worktree(dir.path(), &old_path, "feat/#2-old", &new_path, "feat/#2-new").unwrap();

  assert!(remote_renamed, "origin had the branch → remote_renamed must be true");
  let ls = Command::new("git")
    .args(["ls-remote", "--heads", "origin"])
    .current_dir(&new_path)
    .output()
    .unwrap();
  let refs = String::from_utf8_lossy(&ls.stdout);
  assert!(refs.contains("feat/#2-new"), "remote must carry the new branch: {refs}");
  assert!(!refs.contains("feat/#2-old"), "remote must drop the old branch: {refs}");
}

#[test]
fn rename_worktree_remote_push_carries_a_force_with_lease() {
  // Codex review on PR #292 (P1): the remote rename must lease the delete
  // refspec against the fetched old tip so a commit pushed by someone else in
  // the fetch→push window flips the lease and gets rejected instead of being
  // silently dropped. The exact tip race isn't reachable from the public API
  // (the fetch is internal), so assert the *intent*: the recorded push argv
  // carries `--force-with-lease=feat/#7-lease:<oid>`.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();

  let remote_dir = TempDir::new().unwrap();
  assert!(Command::new("git")
    .args(["init", "--bare", &remote_dir.path().to_string_lossy()])
    .output()
    .unwrap()
    .status
    .success());
  Command::new("git")
    .args(["remote", "add", "origin", &remote_dir.path().to_string_lossy()])
    .current_dir(dir.path())
    .output()
    .unwrap();

  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-7-lease");
  worktree::add(&repo, "feat-7-lease", &old_path, "feat/#7-lease", false).unwrap();
  assert!(Command::new("git")
    .args(["push", "origin", "feat/#7-lease"])
    .current_dir(&old_path)
    .output()
    .unwrap()
    .status
    .success());

  let new_path = wt_root.path().join("feat-7-leased");
  worktree::rename_worktree(dir.path(), &old_path, "feat/#7-lease", &new_path, "feat/#7-leased").unwrap();

  let recorded = gwm::command_log::snapshot();
  let push = recorded
    .iter()
    .find(|e| e.command.starts_with("git push") && e.command.contains("feat/#7-lease"))
    .expect("the remote rename push must be recorded");
  assert!(
    push.command.contains("--force-with-lease=feat/#7-lease:"),
    "the push must lease the delete against the fetched old tip: {}",
    push.command
  );
  assert!(
    push.command.contains("--atomic"),
    "the push must stay atomic: {}",
    push.command
  );
}

#[test]
fn rename_worktree_refuses_when_remote_target_branch_already_exists() {
  // Codex review on PR #292 (P1): if `origin/<new>` already exists, the rename
  // push would fast-forward/move that pre-existing remote branch AND delete
  // `origin/<old>` — overwriting someone else's branch. The rename must prove
  // the destination ref is absent before pushing, and roll back otherwise.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();

  let remote_dir = TempDir::new().unwrap();
  assert!(Command::new("git")
    .args(["init", "--bare", &remote_dir.path().to_string_lossy()])
    .output()
    .unwrap()
    .status
    .success());
  Command::new("git")
    .args(["remote", "add", "origin", &remote_dir.path().to_string_lossy()])
    .current_dir(dir.path())
    .output()
    .unwrap();

  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-8-old");
  worktree::add(&repo, "feat-8-old", &old_path, "feat/#8-old", false).unwrap();
  assert!(Command::new("git")
    .args(["push", "origin", "feat/#8-old"])
    .current_dir(&old_path)
    .output()
    .unwrap()
    .status
    .success());

  // Pre-create `origin/feat/#8-new` (remote only): branch at main HEAD, push,
  // then drop the local ref so the local `branch -m` doesn't collide.
  for args in [
    &["branch", "feat/#8-new"][..],
    &["push", "origin", "feat/#8-new"][..],
    &["branch", "-D", "feat/#8-new"][..],
  ] {
    assert!(Command::new("git")
      .args(args)
      .current_dir(dir.path())
      .output()
      .unwrap()
      .status
      .success());
  }

  let new_path = wt_root.path().join("feat-8-new");
  let err = worktree::rename_worktree(dir.path(), &old_path, "feat/#8-old", &new_path, "feat/#8-new").unwrap_err();
  assert!(
    matches!(err, gwm::error::GwmError::CommandFailed(_)),
    "an existing remote target must refuse the rename, got: {err:?}"
  );

  // Full rollback: local old branch + dir restored, new dir gone.
  assert!(
    repo.find_branch("feat/#8-old", git2::BranchType::Local).is_ok(),
    "old local branch must be restored"
  );
  assert!(old_path.exists(), "old worktree dir must be restored");
  assert!(!new_path.exists(), "new worktree dir must not linger");
  // The remote is untouched: both branches still present, none overwritten.
  let ls = Command::new("git")
    .args(["ls-remote", "--heads", "origin"])
    .current_dir(dir.path())
    .output()
    .unwrap();
  let refs = String::from_utf8_lossy(&ls.stdout);
  assert!(
    refs.contains("feat/#8-old"),
    "origin must still carry the old branch: {refs}"
  );
  assert!(
    refs.contains("feat/#8-new"),
    "the pre-existing remote target must be untouched: {refs}"
  );
}

#[test]
fn rename_worktree_refuses_preexisting_target_without_touching_refs() {
  // Codex review on PR #292: the directory move runs first and is preflighted,
  // so a pre-existing target is rejected before any ref is renamed — the repo
  // is never left half-renamed.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-5-old");
  worktree::add(&repo, "feat-5-old", &old_path, "feat/#5-old", false).unwrap();

  // A directory already sitting at the target path makes the move impossible.
  let new_path = wt_root.path().join("feat-5-new");
  std::fs::create_dir(&new_path).unwrap();

  let err = worktree::rename_worktree(dir.path(), &old_path, "feat/#5-old", &new_path, "feat/#5-new").unwrap_err();
  assert!(matches!(err, gwm::error::GwmError::CommandFailed(_)));

  // No ref touched: old branch survives, new branch absent, old dir intact.
  assert!(
    repo.find_branch("feat/#5-old", git2::BranchType::Local).is_ok(),
    "old branch must survive a rejected rename"
  );
  assert!(
    repo.find_branch("feat/#5-new", git2::BranchType::Local).is_err(),
    "new branch must not be created when the move is rejected"
  );
  assert!(old_path.exists(), "old worktree directory must stay put");
}

#[test]
fn rename_worktree_moves_dir_only_when_branch_unchanged() {
  // Codex review on PR #292: a path-only edit (same branch name, different
  // directory) must move the dir without running `git branch -m old old`
  // (which git rejects). The branch stays intact and the move succeeds.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-6-old");
  worktree::add(&repo, "feat-6-old", &old_path, "feat/#6-keep", false).unwrap();

  let new_path = wt_root.path().join("feat-6-new");
  let remote_renamed =
    worktree::rename_worktree(dir.path(), &old_path, "feat/#6-keep", &new_path, "feat/#6-keep").unwrap();

  assert!(!remote_renamed, "no branch change → nothing remote");
  assert!(new_path.exists(), "directory must move to the new path");
  assert!(!old_path.exists(), "old directory must be gone");
  assert!(
    repo.find_branch("feat/#6-keep", git2::BranchType::Local).is_ok(),
    "the unchanged branch must survive the path-only move"
  );
}

#[test]
fn rename_worktree_rejected_remote_push_is_atomic_and_rolls_back() {
  // Codex review on PR #292 (P1): the two-refspec remote rename must be
  // atomic, so a rejected push can never leave origin without the old branch.
  // We force a rejection with `receive.denyDeletes` on the bare remote: the
  // `:old` delete is refused, and `--atomic` makes that reject the whole push
  // (so `new` is never created and `old` survives). The local branch + dir
  // must roll back to their original state.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();

  let remote_dir = TempDir::new().unwrap();
  Command::new("git")
    .args(["init", "--bare", &remote_dir.path().to_string_lossy()])
    .output()
    .unwrap();
  Command::new("git")
    .args(["remote", "add", "origin", &remote_dir.path().to_string_lossy()])
    .current_dir(dir.path())
    .output()
    .unwrap();

  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-8-old");
  worktree::add(&repo, "feat-8-old", &old_path, "feat/#8-old", false).unwrap();
  Command::new("git")
    .args(["push", "origin", "feat/#8-old"])
    .current_dir(&old_path)
    .output()
    .unwrap();

  // Make the remote refuse branch deletions → the `:old` refspec is rejected.
  Command::new("git")
    .args(["config", "receive.denyDeletes", "true"])
    .current_dir(remote_dir.path())
    .output()
    .unwrap();

  let new_path = wt_root.path().join("feat-8-new");
  let err = worktree::rename_worktree(dir.path(), &old_path, "feat/#8-old", &new_path, "feat/#8-new").unwrap_err();
  assert!(matches!(err, gwm::error::GwmError::CommandFailed(_)));

  // Remote integrity: old branch still there, new branch never created.
  let ls = Command::new("git")
    .args(["ls-remote", "--heads", &remote_dir.path().to_string_lossy()])
    .output()
    .unwrap();
  let refs = String::from_utf8_lossy(&ls.stdout);
  assert!(
    refs.contains("feat/#8-old"),
    "atomic reject must keep the old remote branch: {refs}"
  );
  assert!(
    !refs.contains("feat/#8-new"),
    "the new remote branch must never have been created: {refs}"
  );

  // Local rollback: branch back to old, directory back to old_path.
  assert!(
    repo.find_branch("feat/#8-old", git2::BranchType::Local).is_ok(),
    "local branch must roll back to feat/#8-old"
  );
  assert!(
    repo.find_branch("feat/#8-new", git2::BranchType::Local).is_err(),
    "the new local branch must be rolled back"
  );
  assert!(old_path.exists(), "the worktree directory must roll back to old_path");
  assert!(!new_path.exists(), "the new directory must not survive a failed rename");
}

#[test]
fn rename_worktree_aborts_and_rolls_back_on_remote_lookup_failure() {
  // Codex review on PR #292: when `origin` is set but `git ls-remote` fails
  // (here: it points at a non-existent repo, so the lookup errors rather than
  // returning exit 2 "absent"), the rename must abort and roll back rather
  // than silently reporting local-only success.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  Command::new("git")
    .args(["remote", "add", "origin", "/nonexistent/path/to/repo.git"])
    .current_dir(dir.path())
    .output()
    .unwrap();

  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-9-old");
  worktree::add(&repo, "feat-9-old", &old_path, "feat/#9-old", false).unwrap();
  let new_path = wt_root.path().join("feat-9-new");

  let err = worktree::rename_worktree(dir.path(), &old_path, "feat/#9-old", &new_path, "feat/#9-new").unwrap_err();
  assert!(matches!(err, gwm::error::GwmError::CommandFailed(_)));

  // Full rollback: local branch + directory restored to their original state.
  assert!(
    repo.find_branch("feat/#9-old", git2::BranchType::Local).is_ok(),
    "local branch must roll back to feat/#9-old after a remote lookup failure"
  );
  assert!(
    repo.find_branch("feat/#9-new", git2::BranchType::Local).is_err(),
    "the new branch must not survive an aborted rename"
  );
  assert!(old_path.exists(), "the worktree directory must roll back to old_path");
  assert!(
    !new_path.exists(),
    "the new directory must not survive an aborted rename"
  );
}

#[test]
fn list_uses_new_slug_as_display_name_after_rename_but_keeps_id() {
  // Codex review on PR #292: `git worktree move` updates the path but not the
  // internal `.git/worktrees/<id>` entry. After a rename, `WorktreeInfo.name`
  // (display) must track the new directory slug, while `id` stays the original
  // — and `remove` must still resolve via that id.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-10-old");
  worktree::add(&repo, "feat-10-old", &old_path, "feat/#10-old", false).unwrap();

  let new_path = wt_root.path().join("feat-10-new");
  worktree::rename_worktree(dir.path(), &old_path, "feat/#10-old", &new_path, "feat/#10-new").unwrap();

  let trees = worktree::list(&repo).unwrap();
  let renamed = trees
    .iter()
    .find(|w| !w.is_main)
    .expect("the renamed worktree must be listed");
  assert_eq!(
    renamed.name, "feat-10-new",
    "display name must follow the moved directory slug"
  );
  assert_eq!(
    renamed.id, "feat-10-old",
    "internal git id is unchanged by `git worktree move`"
  );

  // Remove must still resolve the worktree via its (unchanged) id.
  worktree::remove(&repo, &renamed.id, false).unwrap();
  assert!(!new_path.exists(), "remove via id must delete the moved worktree");
}

#[test]
fn rename_worktree_aborts_when_remote_has_unfetched_commits() {
  // Codex review on PR #292 (P1): the remote rename deletes origin/<old> and
  // recreates it from the LOCAL tip. If origin/<old> advanced with commits the
  // worktree never fetched, that would drop them — so refuse and roll back.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();

  let remote_dir = TempDir::new().unwrap();
  Command::new("git")
    .args(["init", "--bare", &remote_dir.path().to_string_lossy()])
    .output()
    .unwrap();
  Command::new("git")
    .args(["remote", "add", "origin", &remote_dir.path().to_string_lossy()])
    .current_dir(dir.path())
    .output()
    .unwrap();

  let wt_root = TempDir::new().unwrap();
  let old_path = wt_root.path().join("feat-12-old");
  worktree::add(&repo, "feat-12-old", &old_path, "feat/#12-old", false).unwrap();
  // Push the branch at its current tip A, then advance origin/feat/#12-old to a
  // commit B (made in the main repo) that the worktree never fetches.
  Command::new("git")
    .args(["push", "origin", "feat/#12-old"])
    .current_dir(&old_path)
    .output()
    .unwrap();
  std::fs::write(dir.path().join("extra.txt"), "x").unwrap();
  Command::new("git")
    .args(["add", "extra.txt"])
    .current_dir(dir.path())
    .output()
    .unwrap();
  Command::new("git")
    .args(["commit", "-m", "B"])
    .current_dir(dir.path())
    .output()
    .unwrap();
  let pushed = Command::new("git")
    .args(["push", "origin", "HEAD:feat/#12-old"])
    .current_dir(dir.path())
    .output()
    .unwrap();
  assert!(
    pushed.status.success(),
    "advancing the remote branch must succeed: {}",
    String::from_utf8_lossy(&pushed.stderr)
  );

  let new_path = wt_root.path().join("feat-12-new");
  let err = worktree::rename_worktree(dir.path(), &old_path, "feat/#12-old", &new_path, "feat/#12-new").unwrap_err();
  assert!(matches!(err, gwm::error::GwmError::CommandFailed(_)));

  // Remote integrity: the old branch is still there (not deleted/rewound).
  let ls = Command::new("git")
    .args(["ls-remote", "--heads", &remote_dir.path().to_string_lossy()])
    .output()
    .unwrap();
  let refs = String::from_utf8_lossy(&ls.stdout);
  assert!(
    refs.contains("feat/#12-old"),
    "stale rename must leave the old remote branch intact: {refs}"
  );
  assert!(
    !refs.contains("feat/#12-new"),
    "the new remote branch must not be created: {refs}"
  );

  // Local rollback.
  assert!(repo.find_branch("feat/#12-old", git2::BranchType::Local).is_ok());
  assert!(repo.find_branch("feat/#12-new", git2::BranchType::Local).is_err());
  assert!(old_path.exists());
  assert!(!new_path.exists());
}

#[test]
fn find_fuzzy_reports_ambiguous_duplicate_display_names() {
  // Codex review on PR #292 (P2): since #290 derives the display name from the
  // path basename, two worktrees in different parents can share a name. An
  // exact match on a duplicated name must be ambiguous, not "take the first".
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  let a = wt_root.path().join("a").join("dup");
  let b = wt_root.path().join("b").join("dup");
  std::fs::create_dir_all(a.parent().unwrap()).unwrap();
  std::fs::create_dir_all(b.parent().unwrap()).unwrap();
  worktree::add(&repo, "feat-1-dup", &a, "feat/#1-dup", false).unwrap();
  worktree::add(&repo, "feat-2-dup", &b, "feat/#2-dup", false).unwrap();

  let err = worktree::find_fuzzy(&repo, "dup").unwrap_err();
  assert!(
    matches!(err, gwm::error::GwmError::Other(ref m) if m.contains("ambiguous")),
    "duplicate display names must resolve as ambiguous, got: {err:?}"
  );

  // ...but each duplicate is still reachable by its unique internal id.
  let by_id = worktree::find_fuzzy(&repo, "feat-1-dup").unwrap();
  assert_eq!(by_id.id, "feat-1-dup");
  assert!(
    by_id.path.ends_with("a/dup"),
    "id match must resolve the right worktree: {:?}",
    by_id.path
  );
}

#[test]
fn find_fuzzy_treats_display_name_equal_to_another_id_as_ambiguous() {
  // Codex review on PR #292 (P2): a `git worktree move` leaves the stable `id`
  // as the old slug while `name` tracks the new basename. If a *different*
  // worktree's display name happens to equal that old slug, the exact-name
  // shortcut would return the wrong row before ever consulting the id. The
  // token is genuinely ambiguous (it is one worktree's id and another's name)
  // and must be reported as such, not silently resolved.
  let (dir, _) = init_repo();
  let repo = worktree::discover_repo(Some(dir.path())).unwrap();
  let wt_root = TempDir::new().unwrap();
  // A: internal id "shared", display name "adir" (basename of its path).
  let a = wt_root.path().join("pa").join("adir");
  // B: internal id "bid", display name "shared" (basename of its path).
  let b = wt_root.path().join("pb").join("shared");
  std::fs::create_dir_all(a.parent().unwrap()).unwrap();
  std::fs::create_dir_all(b.parent().unwrap()).unwrap();
  worktree::add(&repo, "shared", &a, "feat/#1-a", false).unwrap();
  worktree::add(&repo, "bid", &b, "feat/#2-b", false).unwrap();

  let err = worktree::find_fuzzy(&repo, "shared").unwrap_err();
  assert!(
    matches!(err, gwm::error::GwmError::Other(ref m) if m.contains("ambiguous")),
    "a token that is one worktree's id and another's name must be ambiguous, got: {err:?}"
  );

  // Each worktree stays reachable by its own unambiguous id.
  assert_eq!(worktree::find_fuzzy(&repo, "bid").unwrap().id, "bid");
}
