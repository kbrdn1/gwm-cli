//! Integration tests for `gwm review`'s behavioural seam (issue #308):
//! fetch a PR head via origin's universal `refs/pull/<N>/head` ref, attach
//! a worktree to it, link the PR, and record the diff base — all without
//! `gh`, by standing up a local `origin` that advertises a `refs/pull/1/head`
//! ref exactly as GitHub does.

use git2::Repository;
use gwm::config::{Config, HookStep};
use gwm::lifecycle::{HookContext, HookSkips};
use gwm::{github, review};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Run `git -C <dir> <args…>` with a deterministic identity, panicking with
/// captured stderr on failure. Returns trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> String {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args([
      "-c",
      "user.name=gwm-test",
      "-c",
      "user.email=gwm@test",
      "-c",
      "commit.gpgsign=false",
      "-c",
      "init.defaultBranch=main",
    ])
    .args(args)
    .output()
    .expect("git spawns");
  assert!(
    out.status.success(),
    "git {:?} failed: {}",
    args,
    String::from_utf8_lossy(&out.stderr)
  );
  String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Build an `origin` repo on `main` with one base commit, plus a second
/// commit published only under `refs/pull/1/head` (the contributor's PR
/// head — never on `main`). Returns `(origin_tempdir, pr_head_sha)`.
fn origin_with_pr() -> (TempDir, String) {
  let origin = TempDir::new().unwrap();
  let p = origin.path();
  git(p, &["init", "-q"]);
  git(p, &["checkout", "-q", "-B", "main"]);
  std::fs::write(p.join("base.txt"), "base\n").unwrap();
  git(p, &["add", "."]);
  git(p, &["commit", "-q", "-m", "base"]);

  // A PR-head commit that lives only at refs/pull/1/head.
  git(p, &["checkout", "-q", "-b", "pr-head"]);
  std::fs::write(p.join("feature.txt"), "feature\n").unwrap();
  git(p, &["add", "."]);
  git(p, &["commit", "-q", "-m", "the PR change"]);
  let pr_sha = git(p, &["rev-parse", "HEAD"]);
  git(p, &["update-ref", "refs/pull/1/head", &pr_sha]);

  // Park HEAD back on main and drop the helper branch so the PR head is
  // reachable *only* through refs/pull/1/head — like a real fork PR.
  git(p, &["checkout", "-q", "main"]);
  git(p, &["branch", "-q", "-D", "pr-head"]);
  (origin, pr_sha)
}

/// Clone `origin` into a fresh tempdir (so an `origin` remote exists) and
/// return `(clone_tempdir, repo)`.
fn clone_of(origin: &Path) -> (TempDir, Repository) {
  let clone = TempDir::new().unwrap();
  // Clone into the existing tempdir path.
  git(clone.path(), &["clone", "-q", origin.to_str().unwrap(), "."]);
  let repo = Repository::open(clone.path()).unwrap();
  (clone, repo)
}

#[test]
fn fetch_pr_head_ref_creates_local_branch_at_pr_head() {
  let (origin, pr_sha) = origin_with_pr();
  let (clone, repo) = clone_of(origin.path());

  review::fetch_pr_head_ref(clone.path(), 1, "review/pr-1-alice-x").unwrap();

  let branch = repo
    .find_branch("review/pr-1-alice-x", git2::BranchType::Local)
    .expect("fetched review branch exists");
  let oid = branch.get().target().unwrap().to_string();
  assert_eq!(oid, pr_sha, "review branch points at the PR head commit");
}

#[test]
fn materialize_attaches_worktree_links_pr_and_records_base() {
  let (origin, pr_sha) = origin_with_pr();
  let (clone, repo) = clone_of(origin.path());

  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("review-pr-1-alice-x");
  let spec = review::ReviewSpec {
    number: 1,
    branch: "review/pr-1-alice-x",
    dirname: "review-pr-1-alice-x",
    target: &target,
    base_ref: Some("main"),
  };

  let created = review::materialize(&repo, clone.path(), &spec).unwrap();

  // Worktree on disk, checked out at the PR head.
  assert!(created.exists(), "review worktree dir exists");
  let wt_repo = Repository::open(&target).unwrap();
  assert_eq!(
    wt_repo.head().unwrap().peel_to_commit().unwrap().id().to_string(),
    pr_sha
  );

  // The PR is explicitly linked so the sidebar / CI indicator light up.
  let link = github::read_link(&repo, "review/pr-1-alice-x").unwrap();
  assert_eq!(link.pr, Some(1));

  // The diff base points at the PR's base ref, not the main checkout's HEAD.
  let base = repo
    .config()
    .unwrap()
    .get_string("branch.review/pr-1-alice-x.gwm-base")
    .unwrap();
  assert_eq!(base, "main");
}

/// A `HookContext` whose cwd/path point at the (pretend) review worktree.
/// Public fields, so no `git2` fixture is needed.
fn review_ctx(worktree: &Path) -> HookContext {
  HookContext {
    main_repo: worktree.to_path_buf(),
    cwd: worktree.to_path_buf(),
    path: worktree.to_path_buf(),
    branch: "review/pr-1-alice-x".into(),
    branch_type: "review".into(),
    issue: "1".into(),
    desc: "x".into(),
    user: "tester".into(),
    owner: "kbrdn1".into(),
    repo: "gwm-cli".into(),
  }
}

fn config_with_post_create(sentinel: &str) -> Config {
  let mut cfg = Config::default();
  cfg.hooks.post_create.push(HookStep {
    name: "sentinel".into(),
    run: format!("echo {sentinel}"),
    when: None,
    env: HashMap::new(),
    on_fail: gwm::config::HookOnFail::default(),
  });
  cfg
}

#[test]
fn run_post_setup_skips_all_execution_without_bootstrap_flag() {
  // SECURITY (issue #308): a review worktree holds a contributor's possibly
  // untrusted code. Without `--bootstrap`, `run_post_setup` must execute
  // NOTHING — no bootstrap commands, no `post_create` hook — so a fork PR
  // can't run code on `gwm review`. Unique sentinel ⇒ absence is meaningful.
  let sentinel = "gwm-review-nobootstrap-9f3c";
  let wt = TempDir::new().unwrap();
  let cfg = config_with_post_create(sentinel);
  let ctx = review_ctx(wt.path());

  let out = review::run_post_setup(&cfg, &ctx, wt.path(), wt.path(), &HookSkips::default(), false).unwrap();

  assert!(out.is_none(), "no setup runs without --bootstrap");
  let recorded = gwm::command_log::snapshot();
  assert!(
    !recorded.iter().any(|e| e.command.contains(sentinel)),
    "the post_create hook must NOT execute when bootstrap is off"
  );
}

#[test]
fn run_post_setup_runs_hooks_with_bootstrap_flag() {
  // The opt-in path: `--bootstrap` runs the full create-parity sequence, so
  // the configured `post_create` hook executes against the worktree.
  let sentinel = "gwm-review-bootstrap-7a21";
  let wt = TempDir::new().unwrap();
  let cfg = config_with_post_create(sentinel);
  let ctx = review_ctx(wt.path());

  let out = review::run_post_setup(&cfg, &ctx, wt.path(), wt.path(), &HookSkips::default(), true).unwrap();

  assert!(out.is_some(), "setup runs and reports back with --bootstrap");
  let recorded = gwm::command_log::snapshot();
  let mine = recorded
    .iter()
    .find(|e| e.command.contains(sentinel))
    .expect("the post_create hook executes when bootstrap is on");
  assert!(mine.is_success(), "the sentinel hook exits cleanly");
}

#[test]
fn materialize_refuses_when_review_branch_already_exists() {
  let (origin, _) = origin_with_pr();
  let (clone, repo) = clone_of(origin.path());

  // Pre-create the branch the review would mint.
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("review/pr-1-alice-x", &head, false).unwrap();

  let wt_root = TempDir::new().unwrap();
  let target = wt_root.path().join("review-pr-1-alice-x");
  let spec = review::ReviewSpec {
    number: 1,
    branch: "review/pr-1-alice-x",
    dirname: "review-pr-1-alice-x",
    target: &target,
    base_ref: Some("main"),
  };

  let err = review::materialize(&repo, clone.path(), &spec).unwrap_err();
  assert!(err.to_string().contains("already exists"), "got: {err}");
  assert!(!target.exists(), "no worktree dir is left behind");
}
