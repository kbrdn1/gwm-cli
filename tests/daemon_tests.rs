//! Cross-platform unit tests for the daemon's pure RPC core (issue #38,
//! phase 2). These exercise `handle_line` / `dispatch` against a real
//! repo workdir but never open a socket, so they run identically on every
//! platform (including the Windows CI runner where the socket server is
//! `cfg`-compiled out).

mod common;

use common::init_repo;
use gwm::daemon::{
  error as rpc_error, handle_line, parse_list_result, parse_worktrees_changed, success, worktrees_changed_notification,
  worktrees_differ, INTERNAL_ERROR, INVALID_PARAMS, LIST_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, SUBSCRIBE_REQUEST,
};
use gwm::json_api::{JsonStatus, JsonWorktree};
use serde_json::Value;

fn sample(name: &str) -> JsonWorktree {
  JsonWorktree {
    name: name.into(),
    id: name.into(),
    path: format!("/wt/{name}"),
    branch: Some(format!("feat/{name}")),
    head: Some("abc1234".into()),
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: JsonStatus {
      is_dirty: false,
      has_upstream: true,
      ahead: 0,
      behind: 0,
      unknown: false,
    },
    age_seconds: Some(10),
    issue: None,
    pr: None,
    agents: None,
  }
}

fn call(workdir: &std::path::Path, line: &str) -> Value {
  let resp = handle_line(workdir, line).expect("a request with an id must get a response");
  serde_json::from_str(&resp).expect("response must be valid JSON")
}

#[test]
fn list_method_returns_worktree_array_and_echoes_id() {
  let (dir, _repo) = init_repo();
  let v = call(dir.path(), r#"{"jsonrpc":"2.0","method":"list","id":7}"#);
  assert_eq!(v["jsonrpc"], serde_json::json!("2.0"));
  assert_eq!(v["id"], serde_json::json!(7), "response must echo the request id");
  let arr = v["result"].as_array().expect("result must be an array");
  assert_eq!(arr.len(), 1, "fresh repo has exactly the main worktree");
  assert_eq!(arr[0]["is_main"], serde_json::json!(true));
  assert!(v.get("error").is_none(), "success must not carry an error");
}

#[test]
fn doctor_method_returns_report_with_severity_and_exit_code() {
  let (dir, _repo) = init_repo();
  let v = call(dir.path(), r#"{"method":"doctor","id":"d1"}"#);
  assert_eq!(v["id"], serde_json::json!("d1"), "string ids round-trip");
  let result = &v["result"];
  assert!(result["checks"].is_array());
  let sev = result["severity"].as_str().unwrap();
  assert!(matches!(sev, "ok" | "warning" | "failed"));
  assert!(result["exit_code"].is_i64());
}

#[test]
fn path_method_missing_pattern_is_invalid_params() {
  let (dir, _repo) = init_repo();
  let v = call(dir.path(), r#"{"method":"path","id":1}"#);
  assert_eq!(v["error"]["code"], serde_json::json!(INVALID_PARAMS));
  assert!(v.get("result").is_none());
}

#[test]
fn path_method_unknown_pattern_is_an_error_not_a_crash() {
  let (dir, _repo) = init_repo();
  let v = call(
    dir.path(),
    r#"{"method":"path","params":{"pattern":"does-not-exist"},"id":2}"#,
  );
  // find_fuzzy surfaces a not-found error -> internal error envelope,
  // connection stays alive (we got a well-formed response back).
  assert!(v.get("error").is_some(), "unknown pattern yields an error envelope");
  assert_eq!(v["id"], serde_json::json!(2));
}

#[test]
fn unknown_method_is_method_not_found() {
  let (dir, _repo) = init_repo();
  let v = call(dir.path(), r#"{"method":"frobnicate","id":3}"#);
  assert_eq!(v["error"]["code"], serde_json::json!(METHOD_NOT_FOUND));
  assert!(
    v["error"]["message"].as_str().unwrap().contains("frobnicate"),
    "the message names the offending method"
  );
}

#[test]
fn subscribe_over_request_response_is_rejected_with_invalid_params() {
  // `subscribe` only makes sense on a streaming connection; reached via
  // the request/response dispatch it must be rejected, not silently
  // dropped.
  let (dir, _repo) = init_repo();
  let v = call(dir.path(), r#"{"method":"subscribe","id":4}"#);
  assert_eq!(v["error"]["code"], serde_json::json!(INVALID_PARAMS));
}

#[test]
fn malformed_line_is_parse_error_with_null_id() {
  let (dir, _repo) = init_repo();
  let v = call(dir.path(), "this is not json {");
  assert_eq!(v["error"]["code"], serde_json::json!(PARSE_ERROR));
  assert_eq!(v["id"], Value::Null, "a parse error can't know the id");
}

#[test]
fn notification_without_id_gets_no_response() {
  // JSON-RPC 2.0: a request object with no `id` member is a notification
  // and MUST NOT be answered (issue #38 review).
  let (dir, _repo) = init_repo();
  assert!(
    handle_line(dir.path(), r#"{"jsonrpc":"2.0","method":"list"}"#).is_none(),
    "a notification (no id) must produce no response"
  );
}

#[test]
fn explicit_null_id_is_a_request_and_is_answered() {
  // An explicit `"id": null` is a request (not a notification) and is
  // answered, echoing null.
  let (dir, _repo) = init_repo();
  let v = call(dir.path(), r#"{"jsonrpc":"2.0","method":"list","id":null}"#);
  assert_eq!(v["id"], Value::Null);
  assert!(v["result"].is_array());
}

// --- subscribe change detection (issue #38 review) --------------------

#[test]
fn worktrees_differ_ignores_age_seconds() {
  // The core of the spurious-notification fix: age ticks every poll for a
  // non-trunk branch, so a change on age ALONE must not register.
  use std::slice::from_ref;
  let a = sample("x");
  let mut b = a.clone();
  b.age_seconds = Some(a.age_seconds.unwrap() + 5);
  assert!(
    !worktrees_differ(from_ref(&a), from_ref(&b)),
    "age-only delta is not a change"
  );
}

#[test]
fn worktrees_differ_detects_real_changes() {
  use std::slice::from_ref;
  let base = sample("x");

  // Different length.
  assert!(worktrees_differ(from_ref(&base), &[]));

  // A new linked PR.
  let mut pr = base.clone();
  pr.pr = Some(7);
  assert!(worktrees_differ(from_ref(&base), from_ref(&pr)));

  // A dirty-state flip.
  let mut dirty = base.clone();
  dirty.status.is_dirty = true;
  assert!(worktrees_differ(from_ref(&base), from_ref(&dirty)));

  // A renamed worktree.
  let renamed = sample("y");
  assert!(worktrees_differ(from_ref(&base), from_ref(&renamed)));

  // Identical (age equal too) -> no change.
  assert!(!worktrees_differ(from_ref(&base), from_ref(&base)));
}

#[test]
fn worktrees_differ_notices_agent_changes_but_not_activity_churn() {
  // Codex review round C: subscribers must hear a session appear, vanish,
  // or flip active→idle — but the raw last_activity mtime moves on every
  // poll while an agent works, and comparing it would fire a spurious
  // `worktrees.changed` each tick (the age_seconds lesson again).
  use gwm::json_api::{JsonAgentSession, JsonWorktreeAgents};
  use std::slice::from_ref;
  let session = JsonAgentSession {
    kind: "claude".into(),
    freshness: "active".into(),
    last_activity: 100,
    id: "s1".into(),
    name: None,
  };
  let base = sample("x");
  let mut with = base.clone();
  with.agents = Some(JsonWorktreeAgents {
    top: session.clone(),
    sessions: vec![session],
  });

  // Appearing / vanishing.
  assert!(worktrees_differ(from_ref(&base), from_ref(&with)));
  assert!(worktrees_differ(from_ref(&with), from_ref(&base)));

  // active → idle flip.
  let mut idle = with.clone();
  if let Some(a) = idle.agents.as_mut() {
    a.sessions[0].freshness = "idle".into();
  }
  assert!(worktrees_differ(from_ref(&with), from_ref(&idle)));

  // Activity-timestamp churn ALONE is not a change.
  let mut churn = with.clone();
  if let Some(a) = churn.agents.as_mut() {
    a.top.last_activity += 5;
    a.sessions[0].last_activity += 5;
  }
  assert!(
    !worktrees_differ(from_ref(&with), from_ref(&churn)),
    "mtime churn must stay quiet"
  );
}

#[test]
fn subscription_push_skips_phantom_empty_on_transient_error() {
  // Issue #341: a transient `run_list` error must NOT push an empty
  // `worktrees.changed`. The pre-fix `unwrap_or_default()` turned the Err
  // into `[]`, which `worktrees_differ` read as "everything vanished" and
  // streamed to subscribers (flicker, then self-heal next poll).
  use gwm::daemon::next_subscription_push;
  use gwm::error::GwmError;

  let snapshot = vec![sample("a"), sample("b")];

  // The bug: a non-empty last snapshot + a transient error → stay quiet.
  assert_eq!(
    next_subscription_push(
      &Some(snapshot.clone()),
      Err(GwmError::Other("transient git lock".into()))
    ),
    None,
    "a transient error must not emit a phantom-empty snapshot"
  );

  // First successful snapshot (last is None) is always pushed.
  assert_eq!(
    next_subscription_push(&None, Ok(snapshot.clone())),
    Some(snapshot.clone())
  );

  // A genuine change is pushed.
  let grown = vec![sample("a"), sample("b"), sample("c")];
  assert_eq!(
    next_subscription_push(&Some(snapshot.clone()), Ok(grown.clone())),
    Some(grown)
  );

  // A genuine `Ok(empty)` — the last worktree really removed — is a real
  // change and IS pushed (only the error path is suppressed).
  assert_eq!(
    next_subscription_push(&Some(snapshot.clone()), Ok(vec![])),
    Some(vec![])
  );

  // No change → stay quiet.
  assert_eq!(next_subscription_push(&Some(snapshot.clone()), Ok(snapshot)), None);
}

// --- Client-side parsers (issue #309) --------------------------------------

#[test]
fn parse_list_result_round_trips_a_success_envelope() {
  // Exactly what the server writes for `list`: a success envelope whose
  // `result` is the worktree array. The client must recover the vec.
  let wts = vec![sample("a"), sample("b")];
  let envelope = success(&serde_json::json!(1), serde_json::to_value(&wts).unwrap());
  let line = envelope.to_string();

  let parsed = parse_list_result(&line).expect("a success envelope must parse");
  assert_eq!(parsed, wts, "the decoded list must equal what the server sent");
}

#[test]
fn parse_list_result_surfaces_a_server_error_envelope() {
  let envelope = rpc_error(&serde_json::json!(1), INTERNAL_ERROR, "boom");
  let err = parse_list_result(&envelope.to_string()).expect_err("an error envelope must not yield an empty list");
  assert!(
    err.to_string().contains("boom"),
    "the server message must surface: {err}"
  );
}

#[test]
fn parse_list_result_rejects_a_malformed_line() {
  assert!(parse_list_result("not json").is_err());
  // A well-formed object with no `result` and no `error` is still invalid.
  assert!(parse_list_result(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
}

#[test]
fn parse_worktrees_changed_round_trips_a_notification() {
  // Built by the server's own helper, so the test pins the exact wire shape.
  let wts = vec![sample("x")];
  let note = worktrees_changed_notification(&wts);
  let line = note.to_string();

  let parsed = parse_worktrees_changed(&line).expect("a notification must parse");
  assert_eq!(parsed, wts);
}

#[test]
fn parse_worktrees_changed_rejects_a_line_without_params() {
  assert!(parse_worktrees_changed("nonsense").is_err());
  assert!(parse_worktrees_changed(r#"{"method":"worktrees.changed"}"#).is_err());
}

#[test]
fn client_request_lines_are_valid_rpc_for_their_methods() {
  // The canonical request constants must parse as the methods they name —
  // a typo here would silently break every client.
  let list: Value = serde_json::from_str(LIST_REQUEST).unwrap();
  assert_eq!(list["method"], serde_json::json!("list"));
  let sub: Value = serde_json::from_str(SUBSCRIBE_REQUEST).unwrap();
  assert_eq!(sub["method"], serde_json::json!("subscribe"));
}
