//! Unit tests for the `gitlab` backend (issue #419).
//!
//! Every test here is fixture-driven against the **pure** parsers and argv
//! builders — CI runners have no `glab`, exactly as they have no `gh`, so
//! nothing in this file spawns a process. The JSON fixtures mirror the
//! GitLab REST objects that `glab … --output json` / `glab api` pass
//! through unchanged.

use gwm::forge::{CheckOutcome, CiState, IssueState, PrState};
use gwm::gitlab;
use gwm::labels::LabelSpec;
use gwm::milestones::{MilestoneSpec, MilestoneState};

// --- issues ---------------------------------------------------------------

/// `glab issue view <iid> -R <slug> --output json`.
const ISSUE_JSON: &str = r#"{
  "id": 76,
  "iid": 42,
  "project_id": 1,
  "title": "TUI: fuzzy search",
  "state": "opened",
  "labels": ["feature", "tui"],
  "updated_at": "2026-05-19T10:00:00.000Z",
  "web_url": "https://gitlab.com/group/proj/-/issues/42"
}"#;

#[test]
fn parse_issue_json_maps_iid_not_id() {
  // The load-bearing mapping: `iid` is the project-scoped number the user
  // sees and the URL uses; `id` is a global counter. Picking `id` fails
  // silently — wrong URLs, wrong follow-up fetches, no error.
  let issue = gitlab::parse_issue_json(ISSUE_JSON).unwrap();

  assert_eq!(issue.number, 42);
}

#[test]
fn parse_issue_json_maps_opened_to_open() {
  let issue = gitlab::parse_issue_json(ISSUE_JSON).unwrap();

  assert_eq!(issue.state, IssueState::Open);
  assert_eq!(issue.title, "TUI: fuzzy search");
  assert_eq!(issue.url, "https://gitlab.com/group/proj/-/issues/42");
  assert_eq!(issue.updated_at, "2026-05-19T10:00:00.000Z");
}

#[test]
fn parse_issue_json_reads_labels_as_a_bare_string_array() {
  // GitLab returns `["bug"]`, not GitHub's `[{"name": "bug"}]`.
  let issue = gitlab::parse_issue_json(ISSUE_JSON).unwrap();

  assert_eq!(issue.labels, vec!["feature", "tui"]);
}

#[test]
fn parse_issue_json_handles_closed() {
  let json = r#"{"id":1,"iid":7,"title":"old","state":"closed","labels":[],
    "updated_at":"2025-01-01T00:00:00Z","web_url":"https://gitlab.com/g/p/-/issues/7"}"#;

  let issue = gitlab::parse_issue_json(json).unwrap();

  assert_eq!(issue.state, IssueState::Closed);
}

#[test]
fn parse_issue_json_rejects_an_unknown_state() {
  let json = r#"{"id":1,"iid":7,"title":"x","state":"quantum","labels":[],
    "updated_at":"","web_url":""}"#;

  let err = gitlab::parse_issue_json(json).unwrap_err();

  assert!(err.to_string().contains("quantum"), "should name the state: {}", err);
}

// --- merge requests -------------------------------------------------------

/// `glab mr view <iid> -R <slug> --output json`.
const MR_JSON: &str = r#"{
  "id": 155016530,
  "iid": 61,
  "title": "feat(tui): fuzzy search",
  "state": "opened",
  "draft": false,
  "work_in_progress": false,
  "web_url": "https://gitlab.com/group/proj/-/merge_requests/61",
  "updated_at": "2026-05-19T10:00:00.000Z",
  "source_branch": "feat/fuzzy",
  "target_branch": "main",
  "author": {"username": "alice", "name": "Alice"},
  "head_pipeline": {
    "id": 900,
    "iid": 12,
    "status": "success",
    "web_url": "https://gitlab.com/group/proj/-/pipelines/900",
    "created_at": "2026-05-19T09:00:00.000Z",
    "started_at": "2026-05-19T09:01:00.000Z",
    "finished_at": "2026-05-19T09:09:00.000Z"
  }
}"#;

#[test]
fn parse_mr_json_maps_iid_state_and_url() {
  let pr = gitlab::parse_mr_json(MR_JSON).unwrap();

  assert_eq!(pr.number, 61);
  assert_eq!(pr.state, PrState::Open);
  assert_eq!(pr.title, "feat(tui): fuzzy search");
  assert_eq!(pr.url, "https://gitlab.com/group/proj/-/merge_requests/61");
}

#[test]
fn parse_mr_json_maps_draft_to_the_draft_state() {
  let json = MR_JSON.replace("\"draft\": false", "\"draft\": true");

  let pr = gitlab::parse_mr_json(&json).unwrap();

  assert_eq!(pr.state, PrState::Draft);
}

#[test]
fn parse_mr_json_falls_back_to_the_legacy_work_in_progress_flag() {
  // `draft` replaced `work_in_progress`, but older self-hosted instances
  // still only send the legacy key.
  let json = r#"{"id":1,"iid":5,"title":"x","state":"opened","work_in_progress":true,
    "web_url":"https://gitlab.com/g/p/-/merge_requests/5","updated_at":"",
    "source_branch":"a","target_branch":"main"}"#;

  let pr = gitlab::parse_mr_json(json).unwrap();

  assert_eq!(pr.state, PrState::Draft);
}

#[test]
fn parse_mr_json_maps_merged_and_closed_and_locked() {
  for (raw, want) in [
    ("merged", PrState::Merged),
    ("closed", PrState::Closed),
    // `locked` is a transient GitLab-only state during a merge; it is
    // still an open MR from the user's point of view.
    ("locked", PrState::Open),
  ] {
    let json = MR_JSON.replace("\"state\": \"opened\"", &format!("\"state\": \"{raw}\""));

    let pr = gitlab::parse_mr_json(&json).unwrap();

    assert_eq!(pr.state, want, "state {raw}");
  }
}

#[test]
fn parse_mr_json_rejects_an_unknown_state() {
  let json = MR_JSON.replace("\"state\": \"opened\"", "\"state\": \"teleported\"");

  let err = gitlab::parse_mr_json(&json).unwrap_err();

  assert!(err.to_string().contains("teleported"), "should name the state: {}", err);
}

// --- pipelines ------------------------------------------------------------

#[test]
fn parse_mr_json_synthesises_one_check_from_the_head_pipeline() {
  // GitLab hangs a single `head_pipeline` off the MR, not GitHub's
  // per-check `statusCheckRollup` array. One synthetic check keeps
  // `PrStatus` identical across forges; per-job granularity would need a
  // second request and is deliberately out of scope here.
  let pr = gitlab::parse_mr_json(MR_JSON).unwrap();

  assert_eq!(pr.checks_total, 1);
  assert_eq!(pr.checks_passed, 1);
  assert_eq!(pr.ci, CiState::Passing);
  assert_eq!(pr.checks.len(), 1);
  assert_eq!(pr.checks[0].outcome, CheckOutcome::Passing);
  assert_eq!(
    pr.checks[0].url.as_deref(),
    Some("https://gitlab.com/group/proj/-/pipelines/900")
  );
  assert_eq!(pr.checks[0].started_at.as_deref(), Some("2026-05-19T09:01:00.000Z"));
  assert_eq!(pr.checks[0].completed_at.as_deref(), Some("2026-05-19T09:09:00.000Z"));
}

#[test]
fn parse_mr_json_without_a_pipeline_reports_no_ci() {
  let json = r#"{"id":1,"iid":5,"title":"x","state":"opened","draft":false,
    "web_url":"","updated_at":"","source_branch":"a","target_branch":"main"}"#;

  let pr = gitlab::parse_mr_json(json).unwrap();

  assert_eq!(pr.ci, CiState::None);
  assert_eq!(pr.checks_total, 0);
  assert!(pr.checks.is_empty());
}

#[test]
fn pipeline_status_classification_covers_the_gitlab_vocabulary() {
  for (status, want) in [
    ("success", CheckOutcome::Passing),
    // Mirrors the GitHub side treating NEUTRAL / SKIPPED as accepted.
    ("skipped", CheckOutcome::Passing),
    ("manual", CheckOutcome::Passing),
    ("failed", CheckOutcome::Failing),
    ("canceled", CheckOutcome::Failing),
    ("canceling", CheckOutcome::Failing),
    ("created", CheckOutcome::Running),
    ("waiting_for_resource", CheckOutcome::Running),
    ("preparing", CheckOutcome::Running),
    ("pending", CheckOutcome::Running),
    ("running", CheckOutcome::Running),
    ("scheduled", CheckOutcome::Running),
  ] {
    assert_eq!(
      gitlab::classify_pipeline_status(status),
      want,
      "pipeline status {status}"
    );
  }
}

#[test]
fn an_unrecognised_pipeline_status_is_unknown_not_green() {
  // The silent-failure guard from issue #419: a `_ => success` arm would
  // report a green CI that is not green. A new GitLab status must land on
  // `Unknown` and never on `Passing`.
  assert_eq!(
    gitlab::classify_pipeline_status("some_future_status"),
    CheckOutcome::Unknown
  );
}

#[test]
fn an_unknown_pipeline_status_does_not_aggregate_to_passing() {
  let json = MR_JSON.replace("\"status\": \"success\"", "\"status\": \"some_future_status\"");

  let pr = gitlab::parse_mr_json(&json).unwrap();

  assert_ne!(pr.ci, CiState::Passing, "an unknown status must never read as green");
  assert_eq!(pr.checks_passed, 0);
  assert_eq!(pr.checks[0].outcome, CheckOutcome::Unknown);
}

// --- MR head (gwm review) -------------------------------------------------

#[test]
fn parse_mr_head_json_extracts_author_and_branches() {
  let head = gitlab::parse_mr_head_json(MR_JSON).unwrap();

  assert_eq!(head.number, 61);
  assert_eq!(head.author, "alice");
  assert_eq!(head.head_ref_name, "feat/fuzzy");
  assert_eq!(head.base_ref_name, "main");
}

#[test]
fn parse_mr_head_json_tolerates_a_deleted_author() {
  let json = r#"{"id":1,"iid":5,"state":"opened","author":null,
    "source_branch":"a","target_branch":"main","web_url":"","updated_at":"","title":"x"}"#;

  let head = gitlab::parse_mr_head_json(json).unwrap();

  assert_eq!(head.author, "");
}

// --- MR lookup by source branch -------------------------------------------

#[test]
fn parse_mr_list_number_returns_the_first_iid() {
  let json = r#"[{"id":900,"iid":61},{"id":901,"iid":62}]"#;

  assert_eq!(gitlab::parse_mr_list_number(json).unwrap(), Some(61));
}

#[test]
fn parse_mr_list_number_returns_none_on_an_empty_array() {
  assert_eq!(gitlab::parse_mr_list_number("[]").unwrap(), None);
}

#[test]
fn mr_list_argv_pins_the_source_branch_and_all_states() {
  let argv = gitlab::mr_list_argv("group/proj", "feat/x");

  assert_eq!(
    argv,
    vec![
      "mr",
      "list",
      "--repo",
      "group/proj",
      "--source-branch",
      "feat/x",
      "--all",
      "--output",
      "json",
      "--per-page",
      "1",
    ]
  );
}

#[test]
fn issue_view_argv_pins_the_json_output_flag() {
  let argv = gitlab::issue_view_argv("group/proj", 42);

  assert_eq!(
    argv,
    vec!["issue", "view", "42", "--repo", "group/proj", "--output", "json"]
  );
}

#[test]
fn mr_view_argv_pins_the_json_output_flag() {
  let argv = gitlab::mr_view_argv("group/proj", 61);

  assert_eq!(
    argv,
    vec!["mr", "view", "61", "--repo", "group/proj", "--output", "json"]
  );
}

// --- create ---------------------------------------------------------------

#[test]
fn issue_create_argv_passes_the_body_inline_and_skips_the_editor() {
  // `glab issue create` has no `--body-file`; the backend reads the
  // rendered body and passes it via `--description`, and must suppress
  // both the editor and the confirmation prompt or the call blocks on a
  // TTY read.
  let argv = gitlab::issue_create_argv("group/proj", "My title", "body text", &["bug".into()]);

  assert_eq!(
    argv,
    vec![
      "issue",
      "create",
      "--repo",
      "group/proj",
      "--title",
      "My title",
      "--description",
      "body text",
      "--label",
      "bug",
      "--no-editor",
      "--yes",
    ]
  );
}

#[test]
fn mr_create_argv_carries_branches_and_draft() {
  let argv = gitlab::mr_create_argv("group/proj", "T", "B", "feat/x", Some("main"), true);

  assert_eq!(
    argv,
    vec![
      "mr",
      "create",
      "--repo",
      "group/proj",
      "--title",
      "T",
      "--description",
      "B",
      "--source-branch",
      "feat/x",
      "--target-branch",
      "main",
      "--draft",
      "--no-editor",
      "--yes",
    ]
  );
}

#[test]
fn create_argv_omits_repo_entirely_when_the_slug_is_empty() {
  // `gwm new` / `gwm pr` have always tolerated a repo with no `origin`,
  // letting the forge CLI infer the project from the local git context.
  // An empty slug must drop the flag rather than pass `--repo ""`, which
  // glab would reject.
  let issue = gitlab::issue_create_argv("", "T", "B", &[]);
  let mr = gitlab::mr_create_argv("", "T", "B", "feat/x", None, false);

  assert!(!issue.iter().any(|a| a == "--repo"), "issue argv: {:?}", issue);
  assert!(!mr.iter().any(|a| a == "--repo"), "mr argv: {:?}", mr);
  assert_eq!(issue[..2], ["issue", "create"]);
  assert_eq!(mr[..2], ["mr", "create"]);
}

#[test]
fn mr_create_argv_omits_the_target_branch_when_absent() {
  let argv = gitlab::mr_create_argv("g/p", "T", "B", "feat/x", None, false);

  assert!(!argv.iter().any(|a| a == "--target-branch"));
  assert!(!argv.iter().any(|a| a == "--draft"));
}

#[test]
fn created_number_is_parsed_from_a_merge_requests_url() {
  // GitLab's URL shape is `/-/merge_requests/N`, not GitHub's `/pull/N`.
  let out = "https://gitlab.com/group/proj/-/merge_requests/61";

  assert_eq!(gitlab::parse_created_mr_number(out).unwrap(), 61);
}

#[test]
fn created_number_is_parsed_from_an_issues_url() {
  let out = "https://gitlab.com/group/proj/-/issues/42";

  assert_eq!(gitlab::parse_created_issue_number(out).unwrap(), 42);
}

#[test]
fn created_number_parsing_scans_multi_line_output() {
  // glab prints a banner line before the URL.
  let out = "!61 My title (feat/x)\n https://gitlab.com/group/proj/-/merge_requests/61\n";

  assert_eq!(gitlab::parse_created_mr_number(out).unwrap(), 61);
}

#[test]
fn created_number_parsing_errors_without_a_url() {
  let err = gitlab::parse_created_mr_number("something went sideways").unwrap_err();

  assert!(err.to_string().contains("URL"), "should mention the URL: {}", err);
}

// --- labels ---------------------------------------------------------------

#[test]
fn parse_labels_json_strips_the_leading_hash_and_lowercases() {
  // GitLab serialises `"#D9534F"`; the diff engine compares against bare
  // lowercase 6-hex, so a raw pass-through would flag every label as a
  // colour mismatch on every run.
  let json = r##"[{"id":1,"name":"bug","color":"#D9534F","description":"Something broke"},
                  {"id":2,"name":"tui","color":"#5319e7","description":null}]"##;

  let labels = gitlab::parse_labels_json(json).unwrap();

  assert_eq!(labels.len(), 2);
  assert_eq!(labels[0].name, "bug");
  assert_eq!(labels[0].color, "d9534f");
  assert_eq!(labels[0].description.as_deref(), Some("Something broke"));
  assert_eq!(labels[1].color, "5319e7");
  assert_eq!(labels[1].description, None);
}

#[test]
fn label_create_argv_re_adds_the_hash_gitlab_expects() {
  let spec = LabelSpec {
    name: "bug".into(),
    description: Some("Something broke".into()),
    color: "d9534f".into(),
  };

  let argv = gitlab::label_create_argv("group/proj", &spec);

  assert_eq!(
    argv,
    vec![
      "api",
      "-X",
      "POST",
      "projects/group%2Fproj/labels",
      "--raw-field",
      "name=bug",
      "--raw-field",
      "color=#d9534f",
      "--raw-field",
      "description=Something broke",
    ]
  );
}

#[test]
fn label_create_argv_omits_an_empty_description() {
  let spec = LabelSpec {
    name: "bug".into(),
    description: None,
    color: "d9534f".into(),
  };

  let argv = gitlab::label_create_argv("g/p", &spec);

  assert!(!argv.iter().any(|a| a.starts_with("description=")));
}

#[test]
fn label_update_argv_keys_on_the_name_not_a_numeric_id() {
  // GitLab's `PUT /projects/:id/labels/:label_id` accepts the label
  // *title* as well as the numeric id, which is what lets `RemoteLabel`
  // stay id-free and shared with the GitHub backend.
  let spec = LabelSpec {
    name: "good first issue".into(),
    description: None,
    color: "7057ff".into(),
  };

  let argv = gitlab::label_update_argv("group/proj", &spec);

  assert_eq!(
    argv,
    vec![
      "api",
      "-X",
      "PUT",
      "projects/group%2Fproj/labels/good%20first%20issue",
      "--raw-field",
      "color=#7057ff",
    ]
  );
}

#[test]
fn label_delete_argv_url_encodes_the_name() {
  let argv = gitlab::label_delete_argv("group/proj", "good first issue");

  assert_eq!(
    argv,
    vec![
      "api",
      "-X",
      "DELETE",
      "projects/group%2Fproj/labels/good%20first%20issue"
    ]
  );
}

// --- milestones -----------------------------------------------------------

#[test]
fn parse_milestones_json_maps_active_to_open_and_normalises_the_due_date() {
  // Two divergences in one payload: GitLab says `active` (not `open`) and
  // ships a bare `due_date` (`YYYY-MM-DD`) where GitHub ships an RFC3339
  // `due_on`. Both are normalised at the parse boundary so the shared
  // diff engine never sees a spurious change.
  let json = r#"[{"id":12,"iid":3,"title":"v1.5.0","description":"next",
                  "state":"active","due_date":"2026-07-15"},
                 {"id":13,"iid":4,"title":"v1.0.0","description":null,
                  "state":"closed","due_date":null}]"#;

  let ms = gitlab::parse_milestones_json(json).unwrap();

  assert_eq!(ms.len(), 2);
  assert_eq!(ms[0].number, 12, "the update path keys on the global `id`");
  assert_eq!(ms[0].title, "v1.5.0");
  assert_eq!(ms[0].state, MilestoneState::Open);
  assert_eq!(ms[0].due_on.as_deref(), Some("2026-07-15T23:59:59Z"));
  assert_eq!(ms[1].state, MilestoneState::Closed);
  assert_eq!(ms[1].due_on, None);
}

#[test]
fn parse_milestones_json_rejects_an_unknown_state() {
  let json = r#"[{"id":1,"iid":1,"title":"x","state":"paused","due_date":null}]"#;

  let err = gitlab::parse_milestones_json(json).unwrap_err();

  assert!(err.to_string().contains("paused"), "should name the state: {}", err);
}

#[test]
fn milestone_create_argv_sends_a_bare_due_date() {
  let spec = MilestoneSpec {
    title: "v1.5.0".into(),
    description: Some("next".into()),
    due_on: Some("2026-07-15T23:59:59Z".into()),
    state: MilestoneState::Open,
  };

  let argv = gitlab::milestone_create_argv("group/proj", &spec);

  assert_eq!(
    argv,
    vec![
      "api",
      "-X",
      "POST",
      "projects/group%2Fproj/milestones",
      "--raw-field",
      "title=v1.5.0",
      "--raw-field",
      "description=next",
      "--raw-field",
      "due_date=2026-07-15",
    ]
  );
}

#[test]
fn milestone_update_argv_uses_state_event_not_state() {
  // GitLab has no `state` field on write: closing is a `state_event`
  // transition. Sending `state=closed` is silently ignored.
  let spec = MilestoneSpec {
    title: "v1.0.0".into(),
    description: None,
    due_on: None,
    state: MilestoneState::Closed,
  };

  let argv = gitlab::milestone_update_argv("group/proj", 13, &spec);

  assert_eq!(
    argv,
    vec![
      "api",
      "-X",
      "PUT",
      "projects/group%2Fproj/milestones/13",
      "--raw-field",
      "title=v1.0.0",
      "--raw-field",
      "state_event=close",
    ]
  );
}

#[test]
fn milestone_update_argv_reopens_with_activate() {
  let spec = MilestoneSpec {
    title: "v1.0.0".into(),
    description: None,
    due_on: None,
    state: MilestoneState::Open,
  };

  let argv = gitlab::milestone_update_argv("g/p", 13, &spec);

  assert!(argv.iter().any(|a| a == "state_event=activate"));
}

#[test]
fn milestone_list_argv_paginates() {
  // GitLab caps `per_page` at 100; without `--paginate` a repo with more
  // milestones would diff against a truncated set and `--prune` would
  // propose deleting the ones that fell off the page.
  let argv = gitlab::milestone_list_argv("group/proj");

  assert_eq!(
    argv,
    vec!["api", "--paginate", "projects/group%2Fproj/milestones?per_page=100"]
  );
}

#[test]
fn milestone_delete_argv_targets_the_numeric_id() {
  let argv = gitlab::milestone_delete_argv("group/proj", 13);

  assert_eq!(argv, vec!["api", "-X", "DELETE", "projects/group%2Fproj/milestones/13"]);
}

#[test]
fn label_list_argv_paginates() {
  let argv = gitlab::label_list_argv("group/proj");

  assert_eq!(
    argv,
    vec!["api", "--paginate", "projects/group%2Fproj/labels?per_page=100"]
  );
}
