//! Unit tests for the `github` module: link storage (git branch config),
//! repo-slug extraction from the `origin` remote, and JSON parsing of
//! `gh issue view` / `gh pr view --json` payloads.

mod common;

use common::init_repo;
use gwm::github::{
  self, parse_issue_json, parse_pr_head_json, parse_pr_json, BranchLink, CheckOutcome, CiState, IssueState, LinkSource,
  PrState,
};

fn make_branch(repo: &git2::Repository, name: &str) {
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch(name, &head, false).unwrap();
}

#[test]
fn read_link_returns_none_when_branch_name_has_no_issue() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "random-branch");

  let link = github::read_link(&repo, "random-branch").unwrap();

  assert_eq!(link.issue, None);
  assert_eq!(link.pr, None);
  assert_eq!(link.issue_source, LinkSource::None);
  assert_eq!(link.pr_source, LinkSource::None);
}

#[test]
fn read_link_auto_detects_issue_from_branch_name() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_source, LinkSource::BranchName);
  assert_eq!(link.pr, None);
  assert_eq!(link.pr_source, LinkSource::None);
}

#[test]
fn link_issue_writes_branch_config_overriding_auto_detect() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  github::link_issue(&repo, "feat/#42-tui-search", 99).unwrap();
  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.issue, Some(99));
  assert_eq!(link.issue_source, LinkSource::Explicit);
}

#[test]
fn unlink_issue_removes_explicit_override_and_falls_back_to_branch_name() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  github::link_issue(&repo, "feat/#42-tui-search", 99).unwrap();
  github::unlink_issue(&repo, "feat/#42-tui-search").unwrap();
  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  // Auto-detection from branch name kicks back in.
  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_source, LinkSource::BranchName);
}

#[test]
fn unlink_issue_on_unlinked_branch_is_idempotent() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "random-branch");

  // Should not error even if nothing to unlink.
  github::unlink_issue(&repo, "random-branch").unwrap();
  github::unlink_issue(&repo, "random-branch").unwrap();
}

#[test]
fn link_pr_writes_branch_config() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.pr, Some(61));
  assert_eq!(link.pr_source, LinkSource::Explicit);
}

#[test]
fn unlink_pr_clears_the_pr_link_only() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  github::link_issue(&repo, "feat/#42-tui-search", 99).unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  github::unlink_pr(&repo, "feat/#42-tui-search").unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.issue, Some(99));
  assert_eq!(link.pr, None);
}

// --- Persisted PR detection (issue #283) ---------------------------------

#[test]
fn persist_detected_pr_is_read_back_as_detected_source() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  // The detected PR lives in its own key so the no-fetch table read path
  // can surface it on every row while staying distinguishable from an
  // explicit link.
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.pr, Some(77));
  assert_eq!(link.pr_source, LinkSource::Detected);
}

#[test]
fn read_link_round_trips_persisted_issue_title() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo
    .config()
    .unwrap()
    .set_str("branch.feat/#42-tui-search.gwm-issue-title", "TUI title \"quoted\"")
    .unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_source, LinkSource::BranchName);
  assert_eq!(link.issue_title.as_deref(), Some("TUI title \"quoted\""));
}

#[test]
fn read_link_round_trips_explicit_and_detected_pr_titles() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/#42-tui-search.gwm-pr", "61").unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-title", "Explicit PR title")
      .unwrap();
    cfg.set_str("branch.feat/#42-tui-search.gwm-pr-detected", "77").unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-detected-title", "Detected PR title")
      .unwrap();
  }

  let explicit = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(explicit.pr, Some(61));
  assert_eq!(explicit.pr_source, LinkSource::Explicit);
  assert_eq!(explicit.pr_title.as_deref(), Some("Explicit PR title"));

  github::unlink_pr(&repo, "feat/#42-tui-search").unwrap();
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  repo
    .config()
    .unwrap()
    .set_str("branch.feat/#42-tui-search.gwm-pr-detected-title", "Detected PR title")
    .unwrap();
  let detected = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(detected.pr, Some(77));
  assert_eq!(detected.pr_source, LinkSource::Detected);
  assert_eq!(detected.pr_title.as_deref(), Some("Detected PR title"));
}

#[test]
fn read_link_round_trips_persisted_issue_state() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  github::persist_issue_state(&repo, "feat/#42-tui-search", IssueState::Closed).unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_source, LinkSource::BranchName);
  assert_eq!(link.issue_state, Some(IssueState::Closed));
}

#[test]
fn read_link_round_trips_explicit_and_detected_pr_states() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  github::persist_pr_state(&repo, "feat/#42-tui-search", PrState::Merged).unwrap();
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  github::persist_detected_pr_state(&repo, "feat/#42-tui-search", PrState::Draft).unwrap();

  let explicit = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(explicit.pr, Some(61));
  assert_eq!(explicit.pr_source, LinkSource::Explicit);
  assert_eq!(explicit.pr_state, Some(PrState::Merged));

  github::unlink_pr(&repo, "feat/#42-tui-search").unwrap();
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  github::persist_detected_pr_state(&repo, "feat/#42-tui-search", PrState::Draft).unwrap();
  let detected = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(detected.pr, Some(77));
  assert_eq!(detected.pr_source, LinkSource::Detected);
  assert_eq!(detected.pr_state, Some(PrState::Draft));
}

#[test]
fn unlink_issue_clears_persisted_issue_title() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "random-branch");
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.random-branch.gwm-issue", "99").unwrap();
    cfg
      .set_str("branch.random-branch.gwm-issue-title", "Stale issue title")
      .unwrap();
    cfg.set_str("branch.random-branch.gwm-issue-state", "closed").unwrap();
  }

  github::unlink_issue(&repo, "random-branch").unwrap();
  let link = github::read_link(&repo, "random-branch").unwrap();

  assert_eq!(link.issue, None);
  assert_eq!(link.issue_title, None);
  assert_eq!(link.issue_state, None);
}

#[test]
fn unlink_pr_clears_explicit_and_detected_pr_titles() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/#42-tui-search.gwm-pr", "61").unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-title", "Explicit PR title")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-state", "merged")
      .unwrap();
    cfg.set_str("branch.feat/#42-tui-search.gwm-pr-detected", "77").unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-detected-title", "Detected PR title")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-detected-state", "draft")
      .unwrap();
  }

  github::unlink_pr(&repo, "feat/#42-tui-search").unwrap();
  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.pr, None);
  assert_eq!(link.pr_title, None);
  assert_eq!(link.pr_state, None);
}

#[test]
fn clear_persisted_detected_pr_clears_detected_title() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/#42-tui-search.gwm-pr-detected", "77").unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-detected-title", "Detected PR title")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-detected-state", "draft")
      .unwrap();
  }

  github::clear_persisted_detected_pr(&repo, "feat/#42-tui-search").unwrap();
  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.pr, None);
  assert_eq!(link.pr_title, None);
  assert_eq!(link.pr_state, None);
}

#[test]
fn explicit_pr_overrides_persisted_detected_pr() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  // Both keys set: the explicit `gwm link --pr` must win, and its source
  // must read back as Explicit (not Detected) so the pane badge is right.
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.pr, Some(61));
  assert_eq!(link.pr_source, LinkSource::Explicit);
}

#[test]
fn persist_detected_pr_overwrites_a_previous_detection() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  // Re-detection (the branch's PR changed) refreshes the stored value.
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  github::persist_detected_pr_title(&repo, "feat/#42-tui-search", "Old detected title").unwrap();
  github::persist_detected_pr_state(&repo, "feat/#42-tui-search", PrState::Merged).unwrap();
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 88).unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.pr, Some(88));
  assert_eq!(link.pr_title, None);
  assert_eq!(link.pr_state, None);
  assert_eq!(link.pr_source, LinkSource::Detected);
}

#[test]
fn persist_detected_pr_keeps_title_when_number_is_unchanged() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  github::persist_detected_pr_title(&repo, "feat/#42-tui-search", "Detected PR title").unwrap();
  github::persist_detected_pr_state(&repo, "feat/#42-tui-search", PrState::Draft).unwrap();

  // A successful refresh that redetects the same PR should not throw away a
  // known title before the follow-up status fetch has a chance to update it.
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.pr, Some(77));
  assert_eq!(link.pr_title.as_deref(), Some("Detected PR title"));
  assert_eq!(link.pr_state, Some(PrState::Draft));
  assert_eq!(link.pr_source, LinkSource::Detected);
}

#[test]
fn unlink_pr_also_clears_a_persisted_detection() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  // A persisted detection plus an explicit link, then unlink: unlinking a PR
  // must not resurface a stale auto-detection from the detected key.
  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  github::unlink_pr(&repo, "feat/#42-tui-search").unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.pr, None);
  assert_eq!(link.pr_source, LinkSource::None);
}

#[test]
fn clear_persisted_detected_pr_removes_the_detected_link() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  github::clear_persisted_detected_pr(&repo, "feat/#42-tui-search").unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.pr, None);
  assert_eq!(link.pr_source, LinkSource::None);
}

// --- Repo-slug extraction ------------------------------------------------

fn set_origin(repo: &git2::Repository, url: &str) {
  // remote_set_url is a no-op when the remote doesn't exist, so create it first.
  let _ = repo.remote("origin", url);
}

#[test]
fn repo_slug_from_ssh_origin() {
  let (_dir, repo) = init_repo();
  set_origin(&repo, "git@github.com:kbrdn1/gwm-cli.git");

  let slug = github::repo_slug(&repo).unwrap();

  assert_eq!(slug, "kbrdn1/gwm-cli");
}

#[test]
fn repo_slug_from_https_origin() {
  let (_dir, repo) = init_repo();
  set_origin(&repo, "https://github.com/kbrdn1/gwm-cli.git");

  let slug = github::repo_slug(&repo).unwrap();

  assert_eq!(slug, "kbrdn1/gwm-cli");
}

#[test]
fn repo_slug_strips_trailing_dot_git_when_absent() {
  let (_dir, repo) = init_repo();
  set_origin(&repo, "https://github.com/kbrdn1/gwm-cli");

  let slug = github::repo_slug(&repo).unwrap();

  assert_eq!(slug, "kbrdn1/gwm-cli");
}

#[test]
fn repo_slug_handles_trailing_slash_after_dot_git() {
  // Copilot PR #68 review: `https://…/repo.git/` previously left ".git"
  // in the slug because we stripped `.git` before trimming `/`. The fix
  // is to normalise trailing slashes first, then strip `.git`.
  let (_dir, repo) = init_repo();
  set_origin(&repo, "https://github.com/kbrdn1/gwm-cli.git/");

  let slug = github::repo_slug(&repo).unwrap();

  assert_eq!(slug, "kbrdn1/gwm-cli");
}

#[test]
fn repo_slug_handles_trailing_slash_without_dot_git() {
  let (_dir, repo) = init_repo();
  set_origin(&repo, "https://github.com/kbrdn1/gwm-cli/");

  let slug = github::repo_slug(&repo).unwrap();

  assert_eq!(slug, "kbrdn1/gwm-cli");
}

#[test]
fn repo_slug_errors_when_no_origin_remote() {
  let (_dir, repo) = init_repo();

  let err = github::repo_slug(&repo).unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("origin"), "error should mention origin remote: {}", msg);
}

#[test]
fn repo_slug_accepts_a_non_github_origin_since_the_forge_split() {
  // Contract change from issue #419, replacing the pre-#419
  // `repo_slug_errors_when_origin_is_not_github`: slug extraction is now
  // host-agnostic, and *which* forge to talk to is decided separately by
  // `forge::resolve` (config key first, host inference second). Rejecting
  // non-github.com here would have made a GitLab remote unusable before
  // the backend ever got a say.
  let (_dir, repo) = init_repo();
  set_origin(&repo, "https://gitlab.com/kbrdn1/something.git");

  let slug = github::repo_slug(&repo).unwrap();

  assert_eq!(slug, "kbrdn1/something");
}

// --- JSON parsing --------------------------------------------------------

#[test]
fn parse_issue_json_extracts_open_state_and_labels() {
  let json = r#"{
    "number": 42,
    "title": "TUI: fuzzy search",
    "state": "OPEN",
    "url": "https://github.com/kbrdn1/gwm-cli/issues/42",
    "labels": [
      {"name": "feature", "color": "0e8a16"},
      {"name": "tui", "color": "5319e7"}
    ],
    "updatedAt": "2026-05-19T10:00:00Z"
  }"#;

  let issue = parse_issue_json(json).unwrap();

  assert_eq!(issue.number, 42);
  assert_eq!(issue.title, "TUI: fuzzy search");
  assert_eq!(issue.state, IssueState::Open);
  assert_eq!(issue.url, "https://github.com/kbrdn1/gwm-cli/issues/42");
  assert_eq!(issue.labels, vec!["feature", "tui"]);
}

#[test]
fn parse_issue_json_handles_closed_state() {
  let json = r#"{
    "number": 7,
    "title": "old bug",
    "state": "CLOSED",
    "url": "https://github.com/x/y/issues/7",
    "labels": [],
    "updatedAt": "2025-01-01T00:00:00Z"
  }"#;

  let issue = parse_issue_json(json).unwrap();

  assert_eq!(issue.state, IssueState::Closed);
  assert!(issue.labels.is_empty());
}

#[test]
fn parse_pr_json_extracts_state_draft_and_checks() {
  // Mirror of `gh pr view <N> --json state,title,isDraft,url,statusCheckRollup,updatedAt`.
  let json = r#"{
    "number": 61,
    "title": "feat(tui): fuzzy search",
    "state": "OPEN",
    "isDraft": true,
    "url": "https://github.com/kbrdn1/gwm-cli/pull/61",
    "statusCheckRollup": [
      {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
      {"name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS"},
      {"name": "fmt", "status": "IN_PROGRESS", "conclusion": null}
    ],
    "updatedAt": "2026-05-19T10:00:00Z"
  }"#;

  let pr = parse_pr_json(json).unwrap();

  assert_eq!(pr.number, 61);
  assert_eq!(pr.title, "feat(tui): fuzzy search");
  assert_eq!(pr.state, PrState::Draft);
  assert_eq!(pr.url, "https://github.com/kbrdn1/gwm-cli/pull/61");
  // 2 out of 3 checks completed (the IN_PROGRESS one is still running).
  assert_eq!(pr.checks_passed, 2);
  assert_eq!(pr.checks_total, 3);
}

#[test]
fn parse_pr_json_keeps_the_per_check_list() {
  // Issue #436: `RawCheck` used to drop the per-check name and URL when the
  // rollup collapsed into `CiState` — the CI checks overlay needs the full
  // classified list. Both rollup shapes must resolve: a `CheckRun` carries
  // `name` + `detailsUrl`, the legacy `StatusContext` carries `context` +
  // `targetUrl`.
  let json = r#"{
    "number": 61,
    "title": "feat(tui): fuzzy search",
    "state": "OPEN",
    "isDraft": false,
    "url": "https://github.com/kbrdn1/gwm-cli/pull/61",
    "statusCheckRollup": [
      {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS",
       "detailsUrl": "https://github.com/kbrdn1/gwm-cli/actions/runs/1/job/2",
       "workflowName": "ci", "startedAt": "2026-07-24T14:51:06Z",
       "completedAt": "2026-07-24T14:52:24Z"},
      {"context": "security/scan", "state": "FAILURE",
       "targetUrl": "https://scanner.example/run/9"},
      {"name": "fmt", "status": "IN_PROGRESS", "conclusion": null}
    ],
    "updatedAt": "2026-05-19T10:00:00Z"
  }"#;

  let pr = parse_pr_json(json).unwrap();

  assert_eq!(pr.checks.len(), 3, "one entry per rollup check, order preserved");
  assert_eq!(pr.checks[0].name, "ci");
  assert_eq!(pr.checks[0].outcome, CheckOutcome::Passing);
  assert_eq!(
    pr.checks[0].url.as_deref(),
    Some("https://github.com/kbrdn1/gwm-cli/actions/runs/1/job/2")
  );
  assert_eq!(
    pr.checks[1].name, "security/scan",
    "StatusContext name comes from `context`"
  );
  assert_eq!(pr.checks[1].outcome, CheckOutcome::Failing);
  assert_eq!(pr.checks[1].url.as_deref(), Some("https://scanner.example/run/9"));
  assert_eq!(pr.checks[2].name, "fmt");
  assert_eq!(pr.checks[2].outcome, CheckOutcome::Running);
  assert_eq!(pr.checks[2].url, None, "a CheckRun without detailsUrl yields no URL");
  // #436 validation feedback: keep the run metadata for the overlay's
  // right-aligned detail column (workflow + duration).
  assert_eq!(pr.checks[0].workflow_name.as_deref(), Some("ci"));
  assert_eq!(pr.checks[0].started_at.as_deref(), Some("2026-07-24T14:51:06Z"));
  assert_eq!(pr.checks[0].completed_at.as_deref(), Some("2026-07-24T14:52:24Z"));
  assert_eq!(pr.checks[1].workflow_name, None, "a StatusContext has no workflow");
  assert_eq!(pr.checks[1].started_at, None);
}

#[test]
fn parse_pr_json_merged_state_overrides_open() {
  let json = r#"{
    "number": 61,
    "title": "feat(tui): fuzzy search",
    "state": "MERGED",
    "isDraft": false,
    "url": "https://github.com/kbrdn1/gwm-cli/pull/61",
    "statusCheckRollup": [],
    "updatedAt": "2026-05-19T10:00:00Z"
  }"#;

  let pr = parse_pr_json(json).unwrap();

  assert_eq!(pr.state, PrState::Merged);
  assert_eq!(pr.checks_total, 0);
}

#[test]
fn parse_pr_json_handles_missing_status_check_rollup() {
  let json = r#"{
    "number": 5,
    "title": "x",
    "state": "OPEN",
    "isDraft": false,
    "url": "https://github.com/x/y/pull/5",
    "updatedAt": "2026-05-19T10:00:00Z"
  }"#;

  let pr = parse_pr_json(json).unwrap();

  assert_eq!(pr.checks_total, 0);
  assert_eq!(pr.checks_passed, 0);
  assert_eq!(pr.state, PrState::Open);
}

// --- CI state derivation (issue #299) -----------------------------------

/// Build a minimal PR JSON body with the given `statusCheckRollup` array
/// literal so the CI-state tests stay focused on the rollup.
fn pr_json_with_rollup(rollup: &str) -> String {
  format!(
    r#"{{
      "number": 7,
      "title": "x",
      "state": "OPEN",
      "isDraft": false,
      "url": "https://github.com/x/y/pull/7",
      "statusCheckRollup": {rollup},
      "updatedAt": "2026-06-15T10:00:00Z"
    }}"#
  )
}

#[test]
fn ci_state_is_passing_when_all_checks_succeed() {
  let json = pr_json_with_rollup(
    r#"[
      {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
      {"name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS"}
    ]"#,
  );
  assert_eq!(parse_pr_json(&json).unwrap().ci, CiState::Passing);
}

#[test]
fn ci_state_treats_neutral_and_skipped_as_passing() {
  let json = pr_json_with_rollup(
    r#"[
      {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
      {"name": "optional", "status": "COMPLETED", "conclusion": "NEUTRAL"},
      {"name": "deploy", "status": "COMPLETED", "conclusion": "SKIPPED"}
    ]"#,
  );
  assert_eq!(parse_pr_json(&json).unwrap().ci, CiState::Passing);
}

#[test]
fn ci_state_is_running_when_a_check_is_in_flight() {
  let json = pr_json_with_rollup(
    r#"[
      {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
      {"name": "fmt", "status": "IN_PROGRESS", "conclusion": null}
    ]"#,
  );
  assert_eq!(parse_pr_json(&json).unwrap().ci, CiState::Running);
}

#[test]
fn ci_state_treats_queued_and_pending_as_running() {
  let json = pr_json_with_rollup(
    r#"[
      {"name": "queued", "status": "QUEUED", "conclusion": null},
      {"name": "pending", "status": "PENDING", "conclusion": null}
    ]"#,
  );
  assert_eq!(parse_pr_json(&json).unwrap().ci, CiState::Running);
}

#[test]
fn ci_state_is_failing_on_any_failed_conclusion() {
  for conclusion in ["FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED"] {
    let json = pr_json_with_rollup(&format!(
      r#"[
        {{"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"}},
        {{"name": "broken", "status": "COMPLETED", "conclusion": "{conclusion}"}}
      ]"#
    ));
    assert_eq!(
      parse_pr_json(&json).unwrap().ci,
      CiState::Failing,
      "conclusion {conclusion} must read as Failing"
    );
  }
}

#[test]
fn ci_state_failing_outranks_running() {
  // A red check must never hide behind a still-running one.
  let json = pr_json_with_rollup(
    r#"[
      {"name": "still-going", "status": "IN_PROGRESS", "conclusion": null},
      {"name": "broken", "status": "COMPLETED", "conclusion": "FAILURE"}
    ]"#,
  );
  assert_eq!(parse_pr_json(&json).unwrap().ci, CiState::Failing);
}

#[test]
fn ci_state_is_none_when_there_are_no_checks() {
  let json = pr_json_with_rollup("[]");
  assert_eq!(parse_pr_json(&json).unwrap().ci, CiState::None);
}

#[test]
fn ci_state_classifies_legacy_status_context_state() {
  // A `StatusContext` (legacy commit-status API) carries `state`, not
  // `status` / `conclusion`. A failed external CI status must read red, not
  // be mistaken for a still-running check (Codex review #302).
  let failing = pr_json_with_rollup(r#"[{"context": "ci/ext", "state": "FAILURE"}]"#);
  assert_eq!(parse_pr_json(&failing).unwrap().ci, CiState::Failing);

  let error = pr_json_with_rollup(r#"[{"context": "ci/ext", "state": "ERROR"}]"#);
  assert_eq!(parse_pr_json(&error).unwrap().ci, CiState::Failing);

  let success = pr_json_with_rollup(r#"[{"context": "ci/ext", "state": "SUCCESS"}]"#);
  assert_eq!(parse_pr_json(&success).unwrap().ci, CiState::Passing);

  let pending = pr_json_with_rollup(r#"[{"context": "ci/ext", "state": "PENDING"}]"#);
  assert_eq!(parse_pr_json(&pending).unwrap().ci, CiState::Running);
}

#[test]
fn ci_state_treats_non_success_terminal_conclusions_as_failing() {
  // A completed check whose conclusion is a terminal failure GitHub doesn't
  // list among the "common" four (e.g. STARTUP_FAILURE, STALE) must still
  // read red rather than fall through to green (Codex review #302).
  for conclusion in ["STARTUP_FAILURE", "STALE"] {
    let json = pr_json_with_rollup(&format!(
      r#"[{{"name": "broken", "status": "COMPLETED", "conclusion": "{conclusion}"}}]"#
    ));
    assert_eq!(
      parse_pr_json(&json).unwrap().ci,
      CiState::Failing,
      "conclusion {conclusion} must read as Failing"
    );
  }
}

#[test]
fn checks_passed_counts_accepted_terminals_so_count_matches_ci_label() {
  // The N/M shown next to the CI label must use the same accepted terminals
  // as the state, else a green rollup renders "passing 1/3" (Codex review #302).
  let json = pr_json_with_rollup(
    r#"[
      {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
      {"name": "optional", "status": "COMPLETED", "conclusion": "NEUTRAL"},
      {"name": "deploy", "status": "COMPLETED", "conclusion": "SKIPPED"}
    ]"#,
  );
  let pr = parse_pr_json(&json).unwrap();
  assert_eq!(pr.ci, CiState::Passing);
  assert_eq!(pr.checks_passed, 3);
  assert_eq!(pr.checks_total, 3);
}

// --- Labels: gh label list --json contract (issue #81) ------------------

#[test]
fn parse_labels_json_returns_remote_labels() {
  // Mirror of `gh label list --json name,color,description --limit 1000`
  // — a JSON array, even when there's only one entry.
  let json = r#"[
    {"name": "bug", "color": "d73a4a", "description": "Something isn't working"},
    {"name": "enhancement", "color": "a2eeef", "description": ""},
    {"name": "good first issue", "color": "7057ff", "description": "Good for newcomers"}
  ]"#;
  let labels = github::parse_labels_json(json).unwrap();
  assert_eq!(labels.len(), 3);
  assert_eq!(labels[0].name, "bug");
  assert_eq!(labels[0].color, "d73a4a");
  assert_eq!(labels[0].description.as_deref(), Some("Something isn't working"));
  // Empty description must round-trip as `Some("")` — the labels diff
  // module normalises empty == None on its own.
  assert_eq!(labels[1].description.as_deref(), Some(""));
  // Whitespace in name preserved verbatim.
  assert_eq!(labels[2].name, "good first issue");
}

#[test]
fn parse_labels_json_handles_empty_array() {
  let json = r#"[]"#;
  let labels = github::parse_labels_json(json).unwrap();
  assert!(labels.is_empty());
}

#[test]
fn parse_labels_json_tolerates_missing_description_field() {
  // gh sometimes returns the field as absent rather than empty.
  let json = r#"[{"name": "wip", "color": "ededed"}]"#;
  let labels = github::parse_labels_json(json).unwrap();
  assert_eq!(labels[0].name, "wip");
  assert_eq!(labels[0].description, None);
}

#[test]
fn parse_labels_json_rejects_malformed_payload() {
  let err = github::parse_labels_json("not json").unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("labels"), "should mention labels: {}", msg);
}

#[test]
fn parse_labels_json_normalises_uppercase_color() {
  // GitHub sometimes serialises colours uppercase. The parsed
  // `RemoteLabel.color` must already be lowercase 6-hex so callers
  // (diff engine, printer) can rely on the invariant without re-
  // normalising at each call site. Copilot review on PR #90.
  let json = r#"[{"name": "bug", "color": "D73A4A", "description": "broken"}]"#;
  let labels = github::parse_labels_json(json).unwrap();
  assert_eq!(labels[0].color, "d73a4a");
}

#[test]
fn parse_labels_json_rejects_missing_color_field() {
  // Defensive contract check: gh's documented schema always carries
  // a `color`. If a future version dropped the field, a silent
  // `#[serde(default)]` would turn it into an empty string and the
  // diff would flag every remote label as a colour mismatch. Better
  // to fail loud at parse time. Copilot review on PR #90.
  let json = r#"[{"name": "bug", "description": "broken"}]"#;
  let err = github::parse_labels_json(json).unwrap_err();
  let msg = err.to_string();
  assert!(
    msg.contains("color") || msg.contains("missing"),
    "error should mention the missing color field: {}",
    msg
  );
}

// --- Argv contract for gh label commands --------------------------------

#[test]
fn label_create_argv_carries_name_color_description_and_force() {
  // We don't shell out in tests, but the argv builder is the contract
  // surface: name, --color, --description (when present), --force.
  use gwm::labels::LabelSpec;
  let spec = LabelSpec {
    name: "good first issue".into(),
    description: Some("Good for newcomers".into()),
    color: "7057ff".into(),
  };
  let argv = github::label_create_argv("kbrdn1/gwm-cli", &spec);
  // Order is not asserted strictly, but the elements must be present.
  let joined = argv.join(" ");
  assert!(argv.contains(&"label".to_string()));
  assert!(argv.contains(&"create".to_string()));
  assert!(argv.contains(&"good first issue".to_string()));
  assert!(argv.contains(&"--force".to_string()));
  assert!(joined.contains("--color 7057ff"), "color flag missing in {}", joined);
  assert!(
    joined.contains("--description Good for newcomers"),
    "description flag missing in {}",
    joined
  );
  assert!(joined.contains("--repo kbrdn1/gwm-cli"));
}

#[test]
fn label_create_argv_omits_description_when_absent() {
  use gwm::labels::LabelSpec;
  let spec = LabelSpec {
    name: "wip".into(),
    description: None,
    color: "ededed".into(),
  };
  let argv = github::label_create_argv("kbrdn1/gwm-cli", &spec);
  assert!(
    !argv.iter().any(|a| a == "--description"),
    "no --description flag when desc absent, got {:?}",
    argv
  );
}

#[test]
fn label_delete_argv_carries_name_repo_and_yes() {
  let argv = github::label_delete_argv("kbrdn1/gwm-cli", "wontfix");
  assert!(argv.contains(&"label".to_string()));
  assert!(argv.contains(&"delete".to_string()));
  assert!(argv.contains(&"wontfix".to_string()));
  // --yes skips the destructive-confirm prompt; without it `gh` blocks
  // on a TTY read and gwm hangs.
  assert!(argv.contains(&"--yes".to_string()));
  assert!(argv.join(" ").contains("--repo kbrdn1/gwm-cli"));
}

// --- Milestones: gh api …/milestones contract (issue #82) --------------

#[test]
fn parse_milestones_json_returns_remote_milestones() {
  // Mirror of `gh api repos/:owner/:repo/milestones?state=all` — an
  // array of objects with `number`, `title`, `state`, optional
  // `description` and `due_on`.
  let json = r#"[
    {"number": 1, "title": "v0.7.0", "state": "open", "description": "Configurability sprint", "due_on": "2026-07-15T23:59:59Z"},
    {"number": 2, "title": "v0.6.0", "state": "closed", "description": "", "due_on": null},
    {"number": 3, "title": "Backlog", "state": "open"}
  ]"#;
  let milestones = github::parse_milestones_json(json).unwrap();
  assert_eq!(milestones.len(), 3);

  assert_eq!(milestones[0].number, 1);
  assert_eq!(milestones[0].title, "v0.7.0");
  assert_eq!(milestones[0].state, gwm::milestones::MilestoneState::Open);
  assert_eq!(milestones[0].description.as_deref(), Some("Configurability sprint"));
  assert_eq!(milestones[0].due_on.as_deref(), Some("2026-07-15T23:59:59Z"));

  assert_eq!(milestones[1].state, gwm::milestones::MilestoneState::Closed);
  // Empty description round-trips as Some("") — milestones diff
  // collapses it to None on its own (same as labels).
  assert_eq!(milestones[1].description.as_deref(), Some(""));
  // `due_on: null` reads as None.
  assert_eq!(milestones[1].due_on, None);

  assert_eq!(milestones[2].title, "Backlog");
  assert_eq!(milestones[2].description, None);
  assert_eq!(milestones[2].due_on, None);
}

#[test]
fn parse_milestones_json_handles_empty_array() {
  let json = r#"[]"#;
  let milestones = github::parse_milestones_json(json).unwrap();
  assert!(milestones.is_empty());
}

#[test]
fn parse_milestones_json_rejects_unknown_state() {
  // GitHub only emits `open` / `closed`; anything else means the
  // contract changed under us and we want to know loud.
  let json = r#"[{"number": 1, "title": "x", "state": "draft"}]"#;
  let err = github::parse_milestones_json(json).unwrap_err();
  let msg = err.to_string();
  assert!(
    msg.contains("draft") || msg.contains("state"),
    "should mention state: {}",
    msg
  );
}

#[test]
fn parse_milestones_json_rejects_malformed_payload() {
  let err = github::parse_milestones_json("not json").unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("milestones"), "should mention milestones: {}", msg);
}

// --- Argv contract for gh api milestones ---------------------------------

#[test]
fn milestone_list_argv_uses_repos_endpoint_with_state_all() {
  // `gh api --paginate repos/<slug>/milestones?state=all&per_page=100`
  // — `state=all` is the key bit: without it, closed milestones
  // disappear from the diff and `gwm milestones push --prune` thinks
  // they're already gone.
  let argv = github::milestone_list_argv("kbrdn1/gwm-cli");
  let joined = argv.join(" ");
  assert!(argv.contains(&"api".to_string()), "expected 'api', got {:?}", argv);
  assert!(
    joined.contains("repos/kbrdn1/gwm-cli/milestones"),
    "expected milestones endpoint, got {}",
    joined
  );
  assert!(joined.contains("state=all"), "expected state=all, got {}", joined);
}

#[test]
fn milestone_list_argv_uses_paginate_for_repos_with_many_milestones() {
  // GitHub's milestones list endpoint caps `per_page` at 100. Without
  // `--paginate`, repos with more than 100 milestones would diff
  // against a truncated remote set, leading to bogus creates and a
  // dangerously confusing `--prune` (Copilot review on PR #92).
  let argv = github::milestone_list_argv("kbrdn1/gwm-cli");
  assert!(
    argv.contains(&"--paginate".to_string()),
    "expected --paginate flag, got {:?}",
    argv
  );
}

#[test]
fn milestone_create_argv_uses_post_with_title_and_state() {
  use gwm::milestones::{MilestoneSpec, MilestoneState};
  let spec = MilestoneSpec {
    title: "v0.7.0".into(),
    description: Some("Configurability sprint".into()),
    due_on: Some("2026-07-15T23:59:59Z".into()),
    state: MilestoneState::Open,
  };
  let argv = github::milestone_create_argv("kbrdn1/gwm-cli", &spec);
  let joined = argv.join(" ");
  assert!(argv.contains(&"api".to_string()));
  assert!(argv.contains(&"-X".to_string()));
  assert!(argv.contains(&"POST".to_string()));
  assert!(
    joined.contains("repos/kbrdn1/gwm-cli/milestones"),
    "expected milestones endpoint, got {}",
    joined
  );
  // `-f title=…` is gh's form-encoded body syntax. The flag must
  // appear exactly once per field.
  assert!(joined.contains("title=v0.7.0"), "missing title=…: {}", joined);
  assert!(
    joined.contains("description=Configurability sprint"),
    "missing description=…: {}",
    joined
  );
  assert!(
    joined.contains("due_on=2026-07-15T23:59:59Z"),
    "missing due_on=…: {}",
    joined
  );
  assert!(joined.contains("state=open"), "missing state=…: {}", joined);
}

#[test]
fn milestone_create_argv_omits_description_and_due_on_when_absent() {
  // Same defensive contract as label_create: skip the flag entirely
  // rather than send empty, so the remote isn't wiped of a value the
  // user didn't intend to touch.
  use gwm::milestones::{MilestoneSpec, MilestoneState};
  let spec = MilestoneSpec {
    title: "Backlog".into(),
    description: None,
    due_on: None,
    state: MilestoneState::Open,
  };
  let argv = github::milestone_create_argv("kbrdn1/gwm-cli", &spec);
  let joined = argv.join(" ");
  assert!(
    !joined.contains("description="),
    "no description= flag when desc absent, got {}",
    joined
  );
  assert!(
    !joined.contains("due_on="),
    "no due_on= flag when due_on absent, got {}",
    joined
  );
  // title and state still present (state always known).
  assert!(joined.contains("title=Backlog"));
  assert!(joined.contains("state=open"));
}

#[test]
fn milestone_update_argv_uses_patch_with_number_in_path() {
  use gwm::milestones::{MilestoneSpec, MilestoneState};
  let spec = MilestoneSpec {
    title: "v0.7.0".into(),
    description: None,
    due_on: Some("2026-07-15T23:59:59Z".into()),
    state: MilestoneState::Closed,
  };
  let argv = github::milestone_update_argv("kbrdn1/gwm-cli", 42, &spec);
  let joined = argv.join(" ");
  assert!(argv.contains(&"-X".to_string()));
  assert!(argv.contains(&"PATCH".to_string()));
  assert!(
    joined.contains("repos/kbrdn1/gwm-cli/milestones/42"),
    "expected number in path, got {}",
    joined
  );
  assert!(joined.contains("state=closed"));
  assert!(joined.contains("due_on=2026-07-15T23:59:59Z"));
}

#[test]
fn milestone_delete_argv_uses_delete_with_number_in_path() {
  let argv = github::milestone_delete_argv("kbrdn1/gwm-cli", 7);
  let joined = argv.join(" ");
  assert!(argv.contains(&"-X".to_string()));
  assert!(argv.contains(&"DELETE".to_string()));
  assert!(
    joined.contains("repos/kbrdn1/gwm-cli/milestones/7"),
    "expected number in path, got {}",
    joined
  );
}

#[test]
fn branch_link_summary_renders_human_readable() {
  let link = BranchLink {
    issue: Some(42),
    pr: Some(61),
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::BranchName,
    pr_source: LinkSource::Explicit,
  };
  let s = link.summary("PR");
  assert!(s.contains("#42"), "summary should mention issue #42: {}", s);
  assert!(s.contains("#61"), "summary should mention PR #61: {}", s);
}

// --- Issue #100: prune-path argv-injection guard ------------------------

#[test]
fn delete_label_refuses_dash_prefixed_remote_name_before_shelling_out() {
  // Companion to the declared-side `validate_label_name` guard. The
  // symmetric vector is the prune path: `gh label delete <name>`
  // takes the name positionally, so a remote label whose name starts
  // with `-` (planted by an attacker who controls the upstream label
  // set, or by a tool that predates the validator) would be parsed
  // as a flag — `-h` no-ops the delete with a help banner. The
  // validator runs BEFORE the shell-out so `delete_label` refuses
  // cleanly without ever invoking gh — meaning the test can rely on
  // the early return without needing `gh` on PATH.
  let err = github::delete_label("owner/repo", "-h").unwrap_err();
  let msg = format!("{}", err);
  assert!(
    msg.contains("labels (remote)"),
    "error must scope itself to the remote prune path; got: {}",
    msg
  );
  assert!(
    msg.contains("\"-h\"") || msg.contains("-h"),
    "error must name the offending label; got: {}",
    msg
  );
}

// --- PR auto-detection (issue #181) --------------------------------------

#[test]
fn apply_detected_pr_sets_pr_with_detected_source_when_none_linked() {
  // Branch carries an issue from its name but no PR link. Feeding a
  // detected PR number stamps it with the `Detected` provenance.
  let mut link = BranchLink {
    issue: Some(42),
    pr: None,
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::BranchName,
    pr_source: LinkSource::None,
  };

  github::apply_detected_pr(&mut link, Some(128));

  assert_eq!(link.pr, Some(128));
  assert_eq!(link.pr_source, LinkSource::Detected);
  // The issue side is left untouched.
  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_source, LinkSource::BranchName);
}

#[test]
fn apply_detected_pr_leaves_explicit_pr_untouched() {
  // An explicit `gwm link --pr` always wins: detection must not clobber it.
  let mut link = BranchLink {
    issue: None,
    pr: Some(61),
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::None,
    pr_source: LinkSource::Explicit,
  };

  github::apply_detected_pr(&mut link, Some(128));

  assert_eq!(link.pr, Some(61));
  assert_eq!(link.pr_source, LinkSource::Explicit);
}

#[test]
fn apply_detected_pr_is_noop_when_nothing_detected() {
  let mut link = BranchLink::empty();

  github::apply_detected_pr(&mut link, None);

  assert_eq!(link.pr, None);
  assert_eq!(link.pr_source, LinkSource::None);
}

#[test]
fn detected_pr_renders_in_summary_like_any_pr() {
  let mut link = BranchLink::empty();
  github::apply_detected_pr(&mut link, Some(128));
  assert_eq!(link.summary("PR"), "PR #128");
}

#[test]
fn summary_uses_the_forge_noun() {
  // Issue #419: `gwm status` on GitLab must not print "PR #128" for a
  // merge request. The noun is supplied by the resolved forge.
  let mut link = BranchLink::empty();
  github::apply_detected_pr(&mut link, Some(128));
  link.issue = Some(42);

  assert_eq!(link.summary("MR"), "issue #42 · MR #128");
}

#[test]
fn find_pr_argv_pins_the_gh_pr_list_contract() {
  let argv = github::find_pr_argv("kbrdn1/gwm-cli", "feat/#181-auto-detect-pr");
  assert_eq!(
    argv,
    vec![
      "pr",
      "list",
      "--repo",
      "kbrdn1/gwm-cli",
      "--head",
      "feat/#181-auto-detect-pr",
      "--state",
      "all",
      "--json",
      // `isCrossRepository` joins the field list so a fork's PR sharing
      // the branch name can be filtered out (Codex review #458), and the
      // limit rises so there is something to filter.
      "number,isCrossRepository",
      "--limit",
      "20",
    ]
  );
}

#[test]
fn parse_pr_list_number_returns_first_pr() {
  assert_eq!(github::parse_pr_list_number(r#"[{"number":128}]"#).unwrap(), Some(128));
}

#[test]
fn parse_pr_list_number_returns_none_for_empty_array() {
  assert_eq!(github::parse_pr_list_number("[]").unwrap(), None);
}

#[test]
fn parse_pr_list_number_errors_on_malformed_json() {
  assert!(github::parse_pr_list_number("not json").is_err());
}

#[test]
fn parse_pr_head_json_extracts_author_head_and_base() {
  // Issue #308: `gwm review` keys off the PR head ref (slug), the author
  // login (path component), and the base ref (diff base).
  let json = r#"{
    "number": 312,
    "author": { "login": "alice" },
    "headRefName": "feat/spike-x",
    "baseRefName": "main"
  }"#;
  let head = parse_pr_head_json(json).unwrap();
  assert_eq!(head.number, 312);
  assert_eq!(head.author, "alice");
  assert_eq!(head.head_ref_name, "feat/spike-x");
  assert_eq!(head.base_ref_name, "main");
}

#[test]
fn parse_pr_head_json_tolerates_a_null_author() {
  // A deleted GitHub account surfaces as `"author": null`; the default
  // keeps parsing from blowing up (the slug/branch fall back to empty).
  let json = r#"{ "number": 7, "author": null, "headRefName": "x", "baseRefName": "dev" }"#;
  let head = parse_pr_head_json(json).unwrap();
  assert_eq!(head.author, "");
  assert_eq!(head.base_ref_name, "dev");
}

#[test]
fn gh_command_line_uses_the_program_basename_and_joins_args() {
  use std::ffi::{OsStr, OsString};
  // Issue #226: the Command Logs transcript shows the resolved `gh <args…>`
  // (the user's chosen lazygit-style argv). A `GWM_GH` override pointing at
  // a full path must still read as `gh …`, not leak the path.
  let args: Vec<OsString> = ["issue", "view", "226", "--json", "title,body"]
    .iter()
    .map(OsString::from)
    .collect();
  assert_eq!(
    github::gh_command_line(OsStr::new("/opt/homebrew/bin/gh"), &args),
    "gh issue view 226 --json title,body"
  );
  // A bare program name is preserved as-is.
  assert_eq!(
    github::gh_command_line(OsStr::new("gh"), &args),
    "gh issue view 226 --json title,body"
  );
}

// -- Agent pin branch guard (issue #408, Codex review round B) --------------

/// libgit2 surfaces a detached HEAD either as `None` or as the literal
/// `Some("HEAD")` (same trap the statusline handles). Writing
/// `branch.HEAD.gwm-agent-pin` would share one pin across every detached
/// worktree, so `"HEAD"` must read as "no branch".
#[test]
fn pinnable_branch_rejects_none_and_literal_head() {
  use gwm::github::pinnable_branch;
  assert_eq!(pinnable_branch(None), None);
  assert_eq!(pinnable_branch(Some("HEAD")), None);
  assert_eq!(pinnable_branch(Some("feat/#408-x")), Some("feat/#408-x"));
  assert_eq!(pinnable_branch(Some("main")), Some("main"));
}

/// User feedback 2026-07-22: a worktree can host several agent sessions at
/// once, so pins are a multi-valued branch-config key — attach accumulates,
/// detach removes one specific pin, clear drops them all.
#[test]
fn agent_pins_accumulate_and_detach_individually() {
  use gwm::github::{add_agent_pin, agent_pins, clear_agent_pins, remove_agent_pin};
  let (_dir, repo) = init_repo();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();

  assert!(agent_pins(&repo, &branch).unwrap().is_empty());
  add_agent_pin(&repo, &branch, "sid-one").unwrap();
  add_agent_pin(&repo, &branch, "sid-two").unwrap();
  // Re-attaching the same id is a no-op, not a duplicate.
  add_agent_pin(&repo, &branch, "sid-one").unwrap();
  assert_eq!(agent_pins(&repo, &branch).unwrap(), vec!["sid-one", "sid-two"]);

  // Detach removes exactly the named pin.
  assert!(remove_agent_pin(&repo, &branch, "sid-one").unwrap());
  assert_eq!(agent_pins(&repo, &branch).unwrap(), vec!["sid-two"]);
  // Removing an absent pin reports false, never errors.
  assert!(!remove_agent_pin(&repo, &branch, "sid-one").unwrap());

  clear_agent_pins(&repo, &branch).unwrap();
  assert!(agent_pins(&repo, &branch).unwrap().is_empty());
}

#[test]
fn pr_detection_ignores_a_fork_with_the_same_branch_name() {
  // Same hazard as the GitLab side (Codex review #458): `--head <branch>`
  // matches on the branch name alone, so a fork's PR sharing the name
  // could be picked and persisted as this branch's detected PR.
  // `isCrossRepository` is GitHub's own marker for "opened from a fork".
  let json = r#"[{"number":900,"isCrossRepository":true},{"number":61,"isCrossRepository":false}]"#;

  assert_eq!(github::parse_pr_list_number(json).unwrap(), Some(61));
}

#[test]
fn pr_detection_keeps_a_pr_that_does_not_report_cross_repository() {
  let json = r#"[{"number":61}]"#;

  assert_eq!(github::parse_pr_list_number(json).unwrap(), Some(61));
}

// --- persisted links are scoped to the instance that produced them ---------

#[test]
fn a_persisted_link_is_dropped_when_the_origin_changes() {
  // The link keys (`gwm-pr`, `gwm-pr-detected`) are forge-neutral, which
  // was the right call for #419 — but the *number* they hold is not.
  // PR #128 on github.com and MR !128 on gitlab.com are different
  // objects, so once the forge became switchable a stored number could
  // be reinterpreted against a different instance and silently point
  // the worktree at a stranger's merge request.
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 128).unwrap();
  assert_eq!(github::read_link(&repo, "feat/#42-tui-search").unwrap().pr, Some(128));

  // The repo moves. The number stored a moment ago means nothing here.
  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();

  assert_eq!(
    github::read_link(&repo, "feat/#42-tui-search").unwrap().pr,
    None,
    "a number from another instance must not be resurfaced"
  );
}

#[test]
fn a_persisted_link_survives_when_the_origin_is_unchanged() {
  // The negative control. Invalidation that fires on every read would
  // "fix" this by breaking the feature.
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 128).unwrap();

  assert_eq!(github::read_link(&repo, "feat/#42-tui-search").unwrap().pr, Some(128));
}

#[test]
fn the_branch_name_issue_survives_a_change_of_origin() {
  // Only *persisted* values are instance-scoped. The issue number parsed
  // out of the branch name is the user's own naming and stays valid.
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 128).unwrap();
  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.issue, Some(42));
}

#[test]
fn a_link_written_without_an_origin_is_still_readable() {
  // Local-only repos have no origin to stamp. They must keep working
  // rather than having every link invalidated on the next read.
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");

  github::link_pr(&repo, "feat/#42-tui-search", 128).unwrap();

  assert_eq!(github::read_link(&repo, "feat/#42-tui-search").unwrap().pr, Some(128));
}

#[test]
fn writing_one_link_after_an_origin_change_does_not_revive_the_others() {
  // One stamp covers the issue, the explicit PR and the detected PR, so
  // rewriting it for a freshly created PR re-blessed whatever the
  // previous origin had left behind. The lazy check in `read_link` never
  // fires afterwards: the stamp matches again (Codex review #458).
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  github::link_issue(&repo, "feat/#42-tui-search", 900).unwrap();

  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 7).unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.pr, Some(7));
  assert_eq!(
    link.issue,
    Some(42),
    "the stale explicit issue must be gone, leaving only the branch-name number"
  );
}

#[test]
fn two_instances_on_one_host_with_different_ports_do_not_share_a_stamp() {
  // `<host>/<path>` collapses two self-hosted instances that differ only
  // by port, so their numbers were reinterpreted against each other
  // without ever tripping the mismatch check.
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo
    .remote("origin", "https://git.acme.internal:8443/team/proj.git")
    .unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 128).unwrap();

  repo.remote_delete("origin").unwrap();
  repo
    .remote("origin", "https://git.acme.internal:9443/team/proj.git")
    .unwrap();

  assert_eq!(github::read_link(&repo, "feat/#42-tui-search").unwrap().pr, None);
}

#[test]
fn the_same_repo_over_ssh_and_https_keeps_its_links() {
  // The negative control for the port fix: both spellings resolve to the
  // same web origin, so switching remote protocol must not throw the
  // persisted links away.
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "git@github.com:acme/widgets.git").unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 128).unwrap();

  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();

  assert_eq!(github::read_link(&repo, "feat/#42-tui-search").unwrap().pr, Some(128));
}

#[test]
fn a_foreign_stamp_also_hides_the_cached_title_and_state() {
  // The eager purge only runs on the next *write*. Until then — offline,
  // read-only, or simply before the next `gwm status` — `read_link` still
  // fell back to the branch-name issue number and then read the previous
  // tenant's cached title and state onto it, presenting one instance's
  // metadata as the other's (Codex review #458).
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  github::link_issue(&repo, "feat/#42-tui-search", 42).unwrap();
  github::persist_issue_title(&repo, "feat/#42-tui-search", "Title from the old tenant").unwrap();

  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();

  assert_eq!(link.issue, Some(42), "the branch name still names an issue");
  assert_eq!(link.issue_title, None, "but not with the other instance's title");
}

#[test]
fn a_link_predating_the_stamp_is_adopted_by_the_current_origin() {
  // Links written before `gwm-link-origin` existed have no stamp, and an
  // absent stamp is treated as safe — otherwise upgrading gwm would wipe
  // every existing link. But leaving them unstamped forever means a
  // later migration reinterprets their numbers against the new instance
  // (Codex review #458). The first read adopts them instead: no data
  // loss now, and a real invalidation later.
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  github::link_pr(&repo, "feat/#42-tui-search", 128).unwrap();
  // Simulate a pre-#419 link: the number is there, the stamp is not.
  repo
    .config()
    .unwrap()
    .remove("branch.feat/#42-tui-search.gwm-link-origin")
    .unwrap();

  // A read adopts it for the origin it currently has...
  assert_eq!(github::read_link(&repo, "feat/#42-tui-search").unwrap().pr, Some(128));

  // ...so moving the repo afterwards now invalidates it.
  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();

  assert_eq!(github::read_link(&repo, "feat/#42-tui-search").unwrap().pr, None);
}

#[test]
fn a_cached_title_alone_is_enough_to_adopt_the_origin() {
  // An issue derived from the branch name persists no number, only
  // `gwm-issue-title` / `gwm-issue-state`. The adoption trigger looked
  // for numbers, so those branches stayed unstamped forever and kept
  // showing the previous tenant's metadata after a move (Codex review
  // #458).
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  github::persist_issue_title(&repo, "feat/#42-tui-search", "Title from the old tenant").unwrap();
  repo
    .config()
    .unwrap()
    .remove("branch.feat/#42-tui-search.gwm-link-origin")
    .ok();

  // A read adopts it...
  assert_eq!(
    github::read_link(&repo, "feat/#42-tui-search")
      .unwrap()
      .issue_title
      .as_deref(),
    Some("Title from the old tenant")
  );

  // ...so the move is now caught.
  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();

  let link = github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.issue, Some(42), "the branch name still names an issue");
  assert_eq!(link.issue_title, None, "but not with the other instance's title");
}

#[test]
fn a_fork_pr_is_a_fallback_not_a_disqualification() {
  // `--head <branch>` matches the branch NAME only, so a stranger's fork
  // carrying the same name lands in the same list and its number could
  // be persisted as this branch's PR. Filtering `isCrossRepository` out
  // closed that — and closed the standard fork workflow with it: branch
  // locally in a clone of upstream, push to your own fork, open the PR
  // against upstream. That PR *is* cross-repository, and it stopped
  // being detected at all (Codex review #458).
  //
  // Prefer same-repo, fall back to cross-repo. Strictly better than
  // both: before the filter gwm took the first row whatever it was, and
  // the ambiguity that remains — no same-repo PR, and a fork PR that
  // may not be yours — needs the head owner to resolve. Filed as a
  // follow-up (issue #461) rather than grown here in round 27.
  let mixed = r#"[{"number":128,"isCrossRepository":true},{"number":61,"isCrossRepository":false}]"#;
  assert_eq!(
    github::parse_pr_list_number(mixed).unwrap(),
    Some(61),
    "a same-repo PR always wins"
  );

  let fork_only = r#"[{"number":128,"isCrossRepository":true}]"#;
  assert_eq!(
    github::parse_pr_list_number(fork_only).unwrap(),
    Some(128),
    "your own fork's PR is the only one there is"
  );

  assert_eq!(github::parse_pr_list_number("[]").unwrap(), None);
}

#[test]
fn a_refetched_title_is_stamped_with_the_origin_that_produced_it() {
  // `read_link` suppresses cached metadata written against a previous
  // origin. After a move, the refetch persists a fresh title and state —
  // but `persist_issue_title` / `persist_issue_state` wrote them without
  // touching `gwm-link-origin`, so the stamp still named the old origin
  // and the next read suppressed the new values too. Permanently blank
  // (Codex review #458). `persist_detected_pr` already stamped; the
  // title/state writers did not.
  let (dir, repo) = common::init_repo();
  repo.remote("origin", "https://github.com/old/proj.git").unwrap();
  // The branch NAME carries the number, which is the whole point: no
  // `gwm link` ever runs, so no writer that takes a number restamps.
  let branch = "feat/#42-fuzzy-search";
  github::persist_issue_title(&repo, branch, "before").unwrap();
  github::persist_issue_state(&repo, branch, github::IssueState::Closed).unwrap();

  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://github.com/new/proj.git").unwrap();
  let stale = github::read_link(&repo, branch).unwrap();
  assert_eq!(stale.issue, Some(42), "the branch name still carries it");
  assert_eq!(
    stale.issue_title, None,
    "precondition: the old origin's metadata is suppressed"
  );

  // The number here comes from the BRANCH NAME, not from the config, so
  // nothing re-links it and `stamp_link_origin` is never reached through
  // a writer that takes a number. Only the title/state writers run.
  github::persist_issue_title(&repo, branch, "after").unwrap();
  github::persist_issue_state(&repo, branch, github::IssueState::Open).unwrap();

  let link = github::read_link(&repo, branch).unwrap();
  assert_eq!(link.issue_title.as_deref(), Some("after"));
  assert_eq!(link.issue_state, Some(github::IssueState::Open));
  drop(dir);
}

// --- rich PR / issue payload (issue #420) ---------------------------------
//
// Two assertions per surface, deliberately: `pr_view_argv` (what gwm asks
// `gh` for) and `parse_pr_json` (what it keeps) are independently wrong-able.
// A fixture carrying `"body"` parses green even when the field list never
// requests it — feature dead in production, suite green.

#[test]
fn issue_view_argv_requests_the_rich_fields() {
  let argv = github::issue_view_argv("kbrdn1/gwm-cli", 420);
  let fields = argv
    .iter()
    .position(|a| a == "--json")
    .and_then(|i| argv.get(i + 1))
    .expect("--json <fields> pair")
    .split(',')
    .collect::<Vec<_>>();

  for f in ["number", "title", "state", "url", "labels", "updatedAt"] {
    assert!(fields.contains(&f), "summary field {f} must survive");
  }
  for f in ["body", "author", "comments"] {
    assert!(fields.contains(&f), "rich field {f} must be requested");
  }
}

#[test]
fn pr_view_argv_requests_the_rich_fields() {
  let argv = github::pr_view_argv("kbrdn1/gwm-cli", 519);
  let fields = argv
    .iter()
    .position(|a| a == "--json")
    .and_then(|i| argv.get(i + 1))
    .expect("--json <fields> pair")
    .split(',')
    .collect::<Vec<_>>();

  for f in [
    "number",
    "title",
    "state",
    "isDraft",
    "url",
    "updatedAt",
    "statusCheckRollup",
  ] {
    assert!(fields.contains(&f), "summary field {f} must survive");
  }
  for f in [
    "body",
    "author",
    "additions",
    "deletions",
    "baseRefName",
    "headRefName",
    "reviews",
    "comments",
  ] {
    assert!(fields.contains(&f), "rich field {f} must be requested");
  }
}

#[test]
fn parse_issue_json_extracts_the_rich_payload() {
  // Shape taken from a real `gh issue view 484 --repo kbrdn1/gwm-cli --json
  // number,…,body,author,comments` response, trimmed.
  let json = r###"{
    "number": 484,
    "title": "space toggles the active row",
    "state": "OPEN",
    "url": "https://github.com/kbrdn1/gwm-cli/issues/484",
    "labels": [{"name": "feature"}],
    "updatedAt": "2026-08-01T10:00:00Z",
    "body": "## Problem\n\nBulk cleanup needs a row mark.",
    "author": {"id": "MDQ6VXNlcjM=", "is_bot": false, "login": "sassman", "name": "Sven Kanoldt"},
    "comments": [
      {"author": {"login": "kbrdn1"}, "authorAssociation": "OWNER",
       "body": "Thanks Sven.", "createdAt": "2026-08-01T11:00:00Z",
       "url": "https://github.com/kbrdn1/gwm-cli/issues/484#issuecomment-1"},
      {"author": {"login": "coderabbitai"}, "authorAssociation": "NONE",
       "body": "Review skipped.", "createdAt": "2026-08-01T12:00:00Z",
       "url": "https://github.com/kbrdn1/gwm-cli/issues/484#issuecomment-2"}
    ]
  }"###;

  let issue = parse_issue_json(json).unwrap();

  assert_eq!(issue.detail.body, "## Problem\n\nBulk cleanup needs a row mark.");
  assert_eq!(issue.detail.author, "sassman", "the login, not the display name");
  assert_eq!(issue.detail.comments.len(), 2, "order preserved");
  assert_eq!(issue.detail.comments[0].author, "kbrdn1");
  assert_eq!(issue.detail.comments[0].body, "Thanks Sven.");
  assert_eq!(issue.detail.comments[0].created_at, "2026-08-01T11:00:00Z");
  assert_eq!(
    issue.detail.comments[0].url.as_deref(),
    Some("https://github.com/kbrdn1/gwm-cli/issues/484#issuecomment-1")
  );
  assert_eq!(issue.detail.comments[1].author, "coderabbitai");
}

#[test]
fn parse_issue_json_tolerates_a_summary_only_payload() {
  // The rich fields are additive: a response without them (a stubbed `gh`,
  // an older CLI) must still parse into the summary tier rather than error.
  let json = r#"{
    "number": 7,
    "title": "old bug",
    "state": "CLOSED",
    "url": "https://github.com/x/y/issues/7",
    "labels": [],
    "updatedAt": "2025-01-01T00:00:00Z"
  }"#;

  let issue = parse_issue_json(json).unwrap();

  assert!(issue.detail.body.is_empty());
  assert!(issue.detail.author.is_empty());
  assert!(issue.detail.comments.is_empty());
}

#[test]
fn parse_pr_json_extracts_the_rich_payload() {
  // Shape taken from a real `gh pr view 514 --repo kbrdn1/gwm-cli --json
  // …,body,author,additions,deletions,baseRefName,headRefName,reviews,comments`.
  let json = r###"{
    "number": 519,
    "title": "feat(config): Symfony preset",
    "state": "OPEN",
    "isDraft": false,
    "url": "https://github.com/kbrdn1/gwm-cli/pull/519",
    "updatedAt": "2026-08-04T13:00:00Z",
    "statusCheckRollup": [{"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"}],
    "body": "## Description\n\nA seventh preset.",
    "author": {"id": "U_kgD", "is_bot": false, "login": "kbrdn1", "name": "Kylian Bardini"},
    "additions": 1198,
    "deletions": 12,
    "baseRefName": "dev",
    "headRefName": "feat/#392-symfony-preset",
    "reviews": [
      {"author": {"login": "coderabbitai"}, "authorAssociation": "NONE",
       "body": "Actionable comments posted: 2", "state": "COMMENTED",
       "submittedAt": "2026-08-04T13:40:21Z"},
      {"author": {"login": "Copilot"}, "authorAssociation": "NONE",
       "body": "", "state": "APPROVED", "submittedAt": "2026-08-04T14:00:00Z"}
    ],
    "comments": [
      {"author": {"login": "kbrdn1"}, "body": "rebased", "createdAt": "2026-08-04T15:00:00Z",
       "url": "https://github.com/kbrdn1/gwm-cli/pull/519#issuecomment-3"}
    ]
  }"###;

  let pr = parse_pr_json(json).unwrap();

  assert_eq!(pr.detail.body, "## Description\n\nA seventh preset.");
  assert_eq!(pr.detail.author, "kbrdn1");
  assert_eq!(pr.detail.additions, 1198);
  assert_eq!(pr.detail.deletions, 12);
  assert_eq!(pr.detail.base_ref, "dev");
  assert_eq!(pr.detail.head_ref, "feat/#392-symfony-preset");
  assert_eq!(pr.detail.reviews.len(), 2, "order preserved");
  assert_eq!(pr.detail.reviews[0].author, "coderabbitai");
  assert_eq!(pr.detail.reviews[0].state, gwm::github::ReviewState::Commented);
  assert_eq!(pr.detail.reviews[0].body, "Actionable comments posted: 2");
  assert_eq!(pr.detail.reviews[0].submitted_at, "2026-08-04T13:40:21Z");
  assert_eq!(pr.detail.reviews[1].state, gwm::github::ReviewState::Approved);
  assert_eq!(pr.detail.comments.len(), 1);
  assert_eq!(pr.detail.comments[0].author, "kbrdn1");
  assert_eq!(pr.detail.comments[0].body, "rebased");
}

#[test]
fn parse_pr_json_tolerates_a_summary_only_payload() {
  let json = r#"{
    "number": 61,
    "title": "feat(tui): fuzzy search",
    "state": "OPEN",
    "isDraft": false,
    "url": "https://github.com/kbrdn1/gwm-cli/pull/61",
    "updatedAt": "2026-05-19T10:00:00Z"
  }"#;

  let pr = parse_pr_json(json).unwrap();

  assert!(pr.detail.body.is_empty());
  assert!(pr.detail.author.is_empty());
  assert_eq!(pr.detail.additions, 0);
  assert!(pr.detail.reviews.is_empty());
  assert!(pr.detail.comments.is_empty());
}

#[test]
fn review_state_classifies_every_github_variant() {
  use gwm::github::ReviewState;
  assert_eq!(ReviewState::classify("APPROVED"), ReviewState::Approved);
  assert_eq!(
    ReviewState::classify("CHANGES_REQUESTED"),
    ReviewState::ChangesRequested
  );
  assert_eq!(ReviewState::classify("COMMENTED"), ReviewState::Commented);
  assert_eq!(ReviewState::classify("DISMISSED"), ReviewState::Dismissed);
  assert_eq!(ReviewState::classify("PENDING"), ReviewState::Pending);
  // Named honestly rather than folded into `Commented` (same rule as
  // `CheckOutcome::Unknown`): a future state must not read as a verdict.
  assert_eq!(ReviewState::classify("SOMETHING_NEW"), ReviewState::Unknown);
}

#[test]
fn parse_pr_json_survives_null_string_fields() {
  // Codex review #529: `#[serde(default)]` covers an ABSENT key, not an
  // explicit `null`, so a single null string aborted the whole parse and
  // took the CI summary down with the rich view. GitHub sends
  // `submittedAt: null` for a review that has not been submitted, and any
  // of these can come back null on a deleted account or an empty body.
  let json = r#"{
    "number": 519,
    "title": "nulls everywhere",
    "state": "OPEN",
    "isDraft": false,
    "url": "https://github.com/kbrdn1/gwm-cli/pull/519",
    "updatedAt": null,
    "statusCheckRollup": [],
    "body": null,
    "author": null,
    "baseRefName": null,
    "headRefName": null,
    "reviews": [
      {"author": null, "body": null, "state": "PENDING", "submittedAt": null}
    ],
    "comments": [
      {"author": null, "body": null, "createdAt": null, "url": null}
    ]
  }"#;

  let pr = parse_pr_json(json).expect("a null must degrade, never abort the parse");

  assert_eq!(pr.number, 519);
  assert!(pr.detail.body.is_empty());
  assert!(pr.detail.author.is_empty());
  assert_eq!(pr.detail.reviews.len(), 1);
  assert_eq!(pr.detail.reviews[0].state, gwm::github::ReviewState::Pending);
  assert!(pr.detail.reviews[0].submitted_at.is_empty());
  assert_eq!(pr.detail.comments.len(), 1);
  assert!(pr.detail.comments[0].created_at.is_empty());
}

#[test]
fn parse_issue_json_survives_null_string_fields() {
  let json = r#"{
    "number": 420,
    "title": "nulls everywhere",
    "state": "OPEN",
    "url": "https://github.com/kbrdn1/gwm-cli/issues/420",
    "labels": [],
    "updatedAt": null,
    "body": null,
    "author": null,
    "comments": [{"author": null, "body": null, "createdAt": null, "url": null}]
  }"#;

  let issue = parse_issue_json(json).expect("a null must degrade, never abort the parse");

  assert!(issue.detail.body.is_empty());
  assert!(issue.detail.author.is_empty());
  assert_eq!(issue.detail.comments.len(), 1);
}

// ---- Inline review comments (issue #528) --------------------------------
//
// Two assertions per surface, deliberately: a fixture that contains
// `diffHunk` parses green even when the query never asks for it, so the
// query is pinned separately from the parse. The fixtures below mirror the
// shape of a real `gh api graphql` response (verified against PR #514 of
// this repo), with the bodies shortened.

#[test]
fn pr_threads_argv_asks_for_the_anchor_the_hunk_and_the_totals() {
  let argv = github::pr_threads_argv("kbrdn1/gwm-cli", 514).expect("an owner/repo slug resolves");

  assert_eq!(argv[0], "api", "inline comments are a GraphQL-only surface");
  assert_eq!(argv[1], "graphql");

  let query = argv
    .iter()
    .find(|a| a.starts_with("query="))
    .expect("the query is passed as -f query=…");

  // Every field the renderer reads has to be requested. Dropping one here
  // is invisible to the parse tests, which read a fixture rather than gh.
  for field in [
    "reviewThreads",
    "diffHunk",
    "path",
    "line",
    "startLine",
    "isResolved",
    "isOutdated",
    "totalCount",
  ] {
    assert!(query.contains(field), "the query must request `{field}`, got: {query}");
  }

  // GraphQL has no `--repo`, so owner and repo travel as separate
  // variables — as `-f` strings, since `-F` would read a leading `@` as a
  // file and coerce a numeric-looking owner to an Int.
  assert!(argv.iter().any(|a| a == "owner=kbrdn1"), "argv: {argv:?}");
  assert!(argv.iter().any(|a| a == "repo=gwm-cli"), "argv: {argv:?}");
  assert!(argv.iter().any(|a| a == "number=514"), "argv: {argv:?}");
}

#[test]
fn pr_threads_argv_refuses_a_slug_it_cannot_split() {
  // `GitHubForge::repo_selector` returns "" for an origin `gh` cannot be
  // pinned to, and `gh pr view` copes by resolving from the working
  // directory. A GraphQL query cannot: owner and repo are required
  // variables, so guessing them would send the request to whichever
  // instance is ambient — the #458 finding, replayed on a new transport.
  assert!(
    github::pr_threads_argv("", 1).is_err(),
    "an unpinned origin must refuse, never guess"
  );
  assert!(github::pr_threads_argv("gwm-cli", 1).is_err(), "a slug with no owner");
  assert!(
    github::pr_threads_argv("a/b/c", 1).is_err(),
    "a slug with too many parts"
  );
}

/// One resolved thread anchored to a range, one unresolved thread anchored
/// to a single line with a reply. Shape copied from a live response.
const THREADS_JSON: &str = r###"{
  "data": { "repository": { "pullRequest": { "reviewThreads": {
    "totalCount": 2,
    "nodes": [
      {
        "id": "PRRT_a",
        "isResolved": true,
        "isOutdated": false,
        "path": "src/tui/app.rs",
        "line": 11,
        "startLine": 7,
        "comments": {
          "totalCount": 1,
          "nodes": [
            {
              "author": { "login": "coderabbitai" },
              "body": "This drops the guard.",
              "diffHunk": "@@ -4,10 +4,11 @@\n context\n-old line\n+new line",
              "createdAt": "2026-08-04T13:40:21Z",
              "url": "https://github.com/kbrdn1/gwm-cli/pull/514#discussion_r1"
            }
          ]
        }
      },
      {
        "id": "PRRT_b",
        "isResolved": false,
        "isOutdated": true,
        "path": "docs/7.roadmap.md",
        "line": 14,
        "startLine": null,
        "comments": {
          "totalCount": 2,
          "nodes": [
            {
              "author": { "login": "kbrdn1" },
              "body": "Why this order?",
              "diffHunk": "@@ -1,3 +1,3 @@\n-a\n+b",
              "createdAt": "2026-08-04T14:00:00Z",
              "url": "https://github.com/kbrdn1/gwm-cli/pull/514#discussion_r2"
            },
            {
              "author": { "login": "copilot" },
              "body": "Because the anchor is the last line.",
              "diffHunk": "@@ -1,3 +1,3 @@\n-a\n+b",
              "createdAt": "2026-08-04T14:05:00Z",
              "url": "https://github.com/kbrdn1/gwm-cli/pull/514#discussion_r3"
            }
          ]
        }
      }
    ]
  } } } }
}"###;

#[test]
fn parse_pr_threads_json_keeps_the_anchor_the_hunk_and_the_reply_chain() {
  let parsed = github::parse_pr_threads_json(THREADS_JSON).expect("a live-shaped payload parses");
  let threads = parsed.threads();

  assert_eq!(threads.len(), 2);

  let first = &threads[0];
  assert_eq!(first.path, "src/tui/app.rs");
  assert_eq!(first.line, Some(11));
  assert_eq!(first.start_line, Some(7), "a range anchor keeps both ends");
  assert!(first.is_resolved);
  assert!(!first.is_outdated);
  assert!(
    first.diff_hunk.contains("+new line"),
    "the hunk is the context a review comment is about"
  );
  assert_eq!(first.comments.len(), 1);
  assert_eq!(first.comments[0].author, "coderabbitai");

  // A reply chain stays one thread, not two loose rows.
  let second = &threads[1];
  assert!(second.is_outdated);
  assert_eq!(second.comments.len(), 2);
  assert_eq!(second.comments[0].author, "kbrdn1");
  assert_eq!(second.comments[1].author, "copilot");
  assert_eq!(second.comments[1].body, "Because the anchor is the last line.");
}

#[test]
fn parse_pr_threads_json_survives_a_null_start_line_and_a_deleted_author() {
  // `startLine` is null on every single-line anchor — the common case, not
  // an edge one. `author` is null for a deleted account.
  let json = r###"{
    "data": { "repository": { "pullRequest": { "reviewThreads": {
      "totalCount": 1,
      "nodes": [
        {
          "id": "PRRT_c",
          "isResolved": false,
          "isOutdated": false,
          "path": "src/lib.rs",
          "line": 3,
          "startLine": null,
          "comments": {
            "totalCount": 1,
            "nodes": [
              { "author": null, "body": null, "diffHunk": null, "createdAt": null, "url": null }
            ]
          }
        }
      ]
    } } } }
  }"###;

  let parsed = github::parse_pr_threads_json(json).expect("a null must degrade, never abort the parse");
  let threads = parsed.threads();

  assert_eq!(threads.len(), 1);
  assert_eq!(threads[0].start_line, None);
  assert_eq!(threads[0].line, Some(3));
  assert!(threads[0].comments[0].author.is_empty());
  assert!(threads[0].diff_hunk.is_empty());
}

#[test]
fn parse_pr_threads_json_reports_totals_the_page_does_not_hold() {
  // The page is capped by the query, so `totalCount` is the only honest
  // source for an "… N more" row.
  let parsed = github::parse_pr_threads_json(THREADS_JSON).unwrap();

  assert_eq!(parsed.total(), 2, "threads reported by the forge");
  assert_eq!(parsed.threads()[1].total_comments, 2);
}

#[test]
fn parse_pr_threads_json_reads_an_empty_review_as_zero_threads_not_unsupported() {
  // A clean PR and a forge that cannot answer are different states, and
  // the view says something different for each.
  let json = r###"{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":0,"nodes":[]}}}}}"###;

  let parsed = github::parse_pr_threads_json(json).unwrap();

  assert!(parsed.threads().is_empty());
  assert_eq!(parsed.total(), 0);
  assert!(
    !matches!(parsed, gwm::forge::ReviewThreads::Unsupported),
    "GitHub answered; the answer was zero"
  );
}

#[test]
fn pr_merge_argv_names_its_method_and_never_deletes_the_branch() {
  // Validation feedback on #551. Two things this has to pin, and the
  // second is why the test exists at all:
  //
  // 1. The method flag is always explicit. Without one `gh` prompts, and a
  //    prompt from inside a TUI is a hang — the terminal belongs to the
  //    TUI, not to `gh`.
  // 2. No `--delete-branch`, ever. This repo's rules say the atomic commit
  //    history on the branch is the artefact, and a merge fired from a
  //    keypress is the last place to be inventive about that. Pinning the
  //    argv is cheap; debugging an accidental branch deletion is not.
  use gwm::forge::MergeMethod;
  use gwm::github::pr_merge_argv;

  assert_eq!(
    pr_merge_argv("kbrdn1/gwm-cli", 587, MergeMethod::Merge),
    vec!["pr", "merge", "587", "--merge", "--repo", "kbrdn1/gwm-cli"]
  );
  assert_eq!(
    pr_merge_argv("kbrdn1/gwm-cli", 587, MergeMethod::Squash),
    vec!["pr", "merge", "587", "--squash", "--repo", "kbrdn1/gwm-cli"]
  );
  assert_eq!(
    pr_merge_argv("kbrdn1/gwm-cli", 587, MergeMethod::Rebase),
    vec!["pr", "merge", "587", "--rebase", "--repo", "kbrdn1/gwm-cli"]
  );
  for method in MergeMethod::ALL {
    let argv = pr_merge_argv("owner/repo", 1, method);
    assert!(
      !argv.iter().any(|a| a == "--delete-branch" || a == "-d"),
      "{method:?} must not ask for a branch deletion: {argv:?}"
    );
  }
  // An empty slug means `origin` was unresolvable; `gh` then infers the
  // repo from the local git context, same as every other argv here.
  assert_eq!(
    pr_merge_argv("", 3, MergeMethod::Merge),
    vec!["pr", "merge", "3", "--merge"]
  );
}

// ---- Orphaned branch config (issue #633) ---------------------------------

/// Seed the four shapes the sweep has to tell apart: a live branch with
/// gwm keys, a dead one with gwm keys, a dead one with only git's own
/// keys, and a dead branch name that carries dots.
fn seed_branch_config(repo: &git2::Repository) {
  make_branch(repo, "feat/#1-live");
  let mut cfg = repo.config().unwrap();
  cfg.set_str("branch.feat/#1-live.gwm-issue", "1").unwrap();
  cfg.set_str("branch.feat/#1-live.gwm-pr", "11").unwrap();
  cfg.set_str("branch.feat/#2-dead.gwm-issue", "2").unwrap();
  cfg.set_str("branch.feat/#2-dead.gwm-pr", "22").unwrap();
  cfg.set_str("branch.feat/#2-dead.gwm-issue-title", "gone").unwrap();
  // Not ours: git's own branch keys stay put even on a dead branch.
  cfg.set_str("branch.feat/#2-dead.remote", "origin").unwrap();
  cfg.set_str("branch.feat/#3-untouched.merge", "refs/heads/x").unwrap();
  // A dotted branch name — the key splits on the LAST dot, not the first.
  cfg.set_str("branch.release/1.2.x.gwm-issue", "3").unwrap();
}

#[test]
fn orphan_branch_config_reports_dead_branches_and_spares_live_ones() {
  let (_dir, repo) = init_repo();
  seed_branch_config(&repo);

  let orphans = github::orphan_branch_config(&repo).unwrap();

  assert_eq!(
    orphans,
    vec![("feat/#2-dead".to_string(), 3), ("release/1.2.x".to_string(), 1)],
    "only gwm keys of branches that no longer exist, dotted names split on the last dot"
  );
}

#[test]
fn orphan_branch_config_is_empty_when_every_key_belongs_to_a_live_branch() {
  let (_dir, repo) = init_repo();
  make_branch(&repo, "feat/#1-live");
  let mut cfg = repo.config().unwrap();
  cfg.set_str("branch.feat/#1-live.gwm-issue", "1").unwrap();

  assert!(github::orphan_branch_config(&repo).unwrap().is_empty());
}

#[test]
fn purge_orphan_branch_config_drops_dead_keys_and_leaves_everything_else() {
  let (_dir, repo) = init_repo();
  seed_branch_config(&repo);

  let purged = github::purge_orphan_branch_config(&repo).unwrap();
  assert_eq!(
    purged,
    vec![("feat/#2-dead".to_string(), 3), ("release/1.2.x".to_string(), 1)]
  );

  let cfg = repo.config().unwrap();
  // Gone.
  for key in [
    "branch.feat/#2-dead.gwm-issue",
    "branch.feat/#2-dead.gwm-pr",
    "branch.feat/#2-dead.gwm-issue-title",
    "branch.release/1.2.x.gwm-issue",
  ] {
    assert!(cfg.get_string(key).is_err(), "{key} should have been purged");
  }
  // Untouched: the live branch's gwm keys, and git's own keys on a dead one.
  assert_eq!(cfg.get_string("branch.feat/#1-live.gwm-issue").unwrap(), "1");
  assert_eq!(cfg.get_string("branch.feat/#1-live.gwm-pr").unwrap(), "11");
  assert_eq!(cfg.get_string("branch.feat/#2-dead.remote").unwrap(), "origin");
  assert_eq!(
    cfg.get_string("branch.feat/#3-untouched.merge").unwrap(),
    "refs/heads/x"
  );

  // Idempotent: a second run finds nothing left to do.
  assert!(github::purge_orphan_branch_config(&repo).unwrap().is_empty());
}

#[test]
fn purge_orphan_branch_config_clears_every_value_of_a_multi_valued_key() {
  let (_dir, repo) = init_repo();
  let mut cfg = repo.config().unwrap();
  // `gwm-agent-pin` accumulates: `Config::remove` refuses a multivar, so
  // the purge has to go through `remove_multivar` or leave values behind.
  cfg
    .set_multivar("branch.feat/#9-gone.gwm-agent-pin", "^$", "session-a")
    .unwrap();
  cfg
    .set_multivar("branch.feat/#9-gone.gwm-agent-pin", "^$", "session-b")
    .unwrap();

  let purged = github::purge_orphan_branch_config(&repo).unwrap();
  assert_eq!(purged, vec![("feat/#9-gone".to_string(), 1)]);
  assert!(repo
    .config()
    .unwrap()
    .get_string("branch.feat/#9-gone.gwm-agent-pin")
    .is_err());
}
