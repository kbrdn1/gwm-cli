//! Unit tests for the `github` module: link storage (git branch config),
//! repo-slug extraction from the `origin` remote, and JSON parsing of
//! `gh issue view` / `gh pr view --json` payloads.

mod common;

use common::init_repo;
use gwm::github::{self, parse_issue_json, parse_pr_json, BranchLink, IssueState, LinkSource, PrState};

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
fn repo_slug_errors_when_origin_is_not_github() {
  let (_dir, repo) = init_repo();
  set_origin(&repo, "https://gitlab.com/kbrdn1/something.git");

  let err = github::repo_slug(&repo).unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("github"), "error should mention github: {}", msg);
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
  // `gh api repos/<slug>/milestones?state=all&per_page=100` — `state=all`
  // is the key bit: without it, closed milestones disappear from the
  // diff and `gwm milestones push --prune` thinks they're already
  // gone.
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
    issue_source: LinkSource::BranchName,
    pr_source: LinkSource::Explicit,
  };
  let s = link.summary();
  assert!(s.contains("#42"), "summary should mention issue #42: {}", s);
  assert!(s.contains("#61"), "summary should mention PR #61: {}", s);
}
