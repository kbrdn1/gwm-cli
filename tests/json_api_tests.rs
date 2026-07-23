//! Unit tests for the stable JSON DTOs (issue #38, phase 1). These pin
//! the documented schema's field names, types, and the mapping from the
//! internal TUI-coupled structs — in particular the fields the CLI E2E
//! tests can't reach on a fresh repo (branch `age_seconds`, linked
//! issue/PR numbers).

use gwm::doctor::{Check, DoctorReport};
use gwm::github::BranchLink;
use gwm::json_api::{self, check_status_str, JsonDoctorReport, JsonPath, JsonStatus, JsonWorktree};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use std::path::PathBuf;
use std::time::Duration;

fn sample_worktree() -> WorktreeInfo {
  let mut link = BranchLink::empty();
  link.issue = Some(42);
  link.pr = Some(99);
  WorktreeInfo {
    name: "feat-38".into(),
    id: "feat-38-internal".into(),
    path: PathBuf::from("/home/u/cc-worktree/repo/feat-38"),
    branch: Some("feat/#38-json-api".into()),
    head: Some("abc1234".into()),
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus {
      is_dirty: true,
      has_upstream: true,
      ahead: 2,
      behind: 1,
      unknown: false,
    },
    link,
    issue_state: None,
    pr_state: None,
    age: Some(Duration::from_secs(3600)),
  }
}

#[test]
fn json_status_projects_every_branch_status_field() {
  let bs = BranchStatus {
    is_dirty: true,
    has_upstream: true,
    ahead: 3,
    behind: 4,
    unknown: false,
  };
  let dto = JsonStatus::from(&bs);
  let v = serde_json::to_value(&dto).unwrap();
  assert_eq!(v["is_dirty"], serde_json::json!(true));
  assert_eq!(v["has_upstream"], serde_json::json!(true));
  assert_eq!(v["ahead"], serde_json::json!(3));
  assert_eq!(v["behind"], serde_json::json!(4));
  assert_eq!(v["unknown"], serde_json::json!(false));
}

#[test]
fn json_worktree_maps_age_and_link_numbers() {
  let w = sample_worktree();
  let dto = JsonWorktree::from(&w);
  let v = serde_json::to_value(&dto).unwrap();

  assert_eq!(v["name"], serde_json::json!("feat-38"));
  assert_eq!(v["id"], serde_json::json!("feat-38-internal"));
  assert_eq!(v["path"], serde_json::json!("/home/u/cc-worktree/repo/feat-38"));
  assert_eq!(v["branch"], serde_json::json!("feat/#38-json-api"));
  assert_eq!(v["head"], serde_json::json!("abc1234"));
  assert_eq!(v["is_main"], serde_json::json!(false));
  // Duration -> whole seconds.
  assert_eq!(v["age_seconds"], serde_json::json!(3600));
  // Link numbers lifted out of the BranchLink graph.
  assert_eq!(v["issue"], serde_json::json!(42));
  assert_eq!(v["pr"], serde_json::json!(99));
  // Nested status object.
  assert_eq!(v["status"]["is_dirty"], serde_json::json!(true));
  assert_eq!(v["status"]["ahead"], serde_json::json!(2));
  assert_eq!(v["status"]["behind"], serde_json::json!(1));
}

#[test]
fn json_worktree_emits_null_for_absent_optionals() {
  let mut w = sample_worktree();
  w.age = None;
  w.link = BranchLink::empty();
  w.branch = None;
  w.head = None;
  let v = serde_json::to_value(JsonWorktree::from(&w)).unwrap();
  assert_eq!(v["age_seconds"], serde_json::Value::Null);
  assert_eq!(v["issue"], serde_json::Value::Null);
  assert_eq!(v["pr"], serde_json::Value::Null);
  assert_eq!(v["branch"], serde_json::Value::Null);
  assert_eq!(v["head"], serde_json::Value::Null);
}

#[test]
fn json_path_is_the_name_path_branch_triple() {
  let w = sample_worktree();
  let dto = JsonPath::from(&w);
  let v = serde_json::to_value(&dto).unwrap();
  // Exactly three keys, no more.
  let obj = v.as_object().unwrap();
  assert_eq!(obj.len(), 3, "JsonPath must be exactly {{name, path, branch}}");
  assert_eq!(v["name"], serde_json::json!("feat-38"));
  assert_eq!(v["path"], serde_json::json!("/home/u/cc-worktree/repo/feat-38"));
  assert_eq!(v["branch"], serde_json::json!("feat/#38-json-api"));
}

#[test]
fn check_status_strings_are_stable_lowercase() {
  use gwm::doctor::CheckStatus;
  assert_eq!(check_status_str(&CheckStatus::Ok), "ok");
  assert_eq!(check_status_str(&CheckStatus::Warning), "warning");
  assert_eq!(check_status_str(&CheckStatus::Failed), "failed");
}

#[test]
fn json_doctor_report_carries_checks_severity_and_exit_code() {
  let mut report = DoctorReport::new();
  report.checks.push(Check::ok("config", "found .gwm.toml"));
  report
    .checks
    .push(Check::warning("upstream", "no upstream set").with_hint("git push -u"));
  let dto = JsonDoctorReport::from(&report);
  let v = serde_json::to_value(&dto).unwrap();

  assert_eq!(v["checks"][0]["name"], serde_json::json!("config"));
  assert_eq!(v["checks"][0]["status"], serde_json::json!("ok"));
  assert_eq!(v["checks"][0]["detail"], serde_json::json!("found .gwm.toml"));
  assert_eq!(v["checks"][0]["fix_hint"], serde_json::Value::Null);
  assert_eq!(v["checks"][1]["status"], serde_json::json!("warning"));
  assert_eq!(v["checks"][1]["fix_hint"], serde_json::json!("git push -u"));

  // A warning present, no failure -> severity "warning", exit 1.
  assert_eq!(v["severity"], serde_json::json!("warning"));
  assert_eq!(v["exit_code"], serde_json::json!(1));
}

#[test]
fn json_doctor_report_failed_wins_exit_two() {
  let mut report = DoctorReport::new();
  report.checks.push(Check::ok("a", "ok"));
  report.checks.push(Check::warning("b", "warn"));
  report.checks.push(Check::failed("c", "boom"));
  let v = serde_json::to_value(JsonDoctorReport::from(&report)).unwrap();
  assert_eq!(v["severity"], serde_json::json!("failed"));
  assert_eq!(v["exit_code"], serde_json::json!(2));
}

// --- Deserialize round-trip (issue #309) -----------------------------------
// `JsonWorktree` / `JsonStatus` gained `Deserialize` so a daemon *client*
// (the statusline consumer) can decode the very lines the server serialises.
// This pins that the derive is present and the round-trip is lossless.

#[test]
fn json_worktree_serialize_deserialize_round_trips() {
  let wt = JsonWorktree {
    name: "feat-309".into(),
    id: "feat-309".into(),
    path: "/wt/feat-309".into(),
    branch: Some("feat/#309-daemon-consumer".into()),
    head: Some("a".repeat(40)),
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: JsonStatus {
      is_dirty: true,
      has_upstream: true,
      ahead: 2,
      behind: 1,
      unknown: false,
    },
    age_seconds: Some(42),
    issue: Some(309),
    pr: Some(310),
    agents: None,
  };
  let line = serde_json::to_string(&wt).unwrap();
  let back: JsonWorktree = serde_json::from_str(&line).unwrap();
  assert_eq!(back, wt, "a serialised worktree must decode back identically");
}

#[test]
fn json_worktree_vec_decodes_from_a_daemon_list_result() {
  // The shape a client pulls out of a `list` response's `result` array.
  let json = r#"[
    {"name":"main","id":"main","path":"/repo","branch":"main","head":null,
     "is_main":true,"is_locked":false,"is_prunable":false,
     "status":{"is_dirty":false,"has_upstream":false,"ahead":0,"behind":0,"unknown":true},
     "age_seconds":null,"issue":null,"pr":null}
  ]"#;
  let wts: Vec<JsonWorktree> = serde_json::from_str(json).unwrap();
  assert_eq!(wts.len(), 1);
  assert!(wts[0].is_main);
  assert!(wts[0].status.unknown);
}

// -- Agent detection cache (issue #408, Codex review round A) ---------------

/// The daemon re-lists every poll tick; identical inputs within the TTL must
/// reuse the last detection instead of re-walking the artefact stores. The
/// probe: seed a session, attach once, delete the artefacts, attach again —
/// a cache hit still carries the agents (staleness ≤ 30 s is the documented
/// trade-off). Runs in its own process-global slot; this file has no other
/// attach_agents caller, so the env var and the cache stay uncontended.
#[test]
fn attach_agents_reuses_detection_within_the_ttl() {
  let home = tempfile::TempDir::new().unwrap();
  // SAFETY-of-intent: single test touching this var in this binary.
  std::env::set_var("GWM_AGENTS_HOME", home.path());
  let dir = home
    .path()
    .join(".codex/sessions")
    .join(gwm::agent_sessions::codex_day_dir(std::time::SystemTime::now()));
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(
    dir.join("rollout-cache.jsonl"),
    r#"{"type":"session_meta","payload":{"session_id":"cache-1","cwd":"/work/cached"}}
"#,
  )
  .unwrap();

  let row = || JsonWorktree {
    name: "cached".into(),
    id: "cached".into(),
    path: "/work/cached".into(),
    branch: Some("main".into()),
    head: None,
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: JsonStatus {
      is_dirty: false,
      has_upstream: false,
      ahead: 0,
      behind: 0,
      unknown: false,
    },
    age_seconds: None,
    issue: None,
    pr: None,
    agents: None,
  };

  let reals = vec![std::path::PathBuf::from("/work/cached")];
  let mut rows = vec![row()];
  json_api::attach_agents(&mut rows, &reals, &[]);
  assert!(rows[0].agents.is_some(), "seeded session must be detected");

  // Remove the artefacts: a cache hit still serves the previous summary.
  std::fs::remove_dir_all(home.path().join(".codex")).unwrap();
  let mut rows2 = vec![row()];
  json_api::attach_agents(&mut rows2, &reals, &[]);
  assert!(
    rows2[0].agents.is_some(),
    "within the TTL the cached detection must be reused (no re-walk)"
  );
}

#[cfg(unix)]
#[test]
fn json_worktree_path_stays_the_plain_lossy_absolute_path() {
  // Codex review round U (P2, undoing round T's key reuse): the public
  // `path` is the schema value consumers open and compare — it must NEVER
  // grow disambiguation suffixes, even for non-UTF-8 paths (where it is
  // lossy by JSON's nature). Agent association uses a separate INTERNAL
  // lossless key built from the caller-kept real PathBufs instead.
  use std::ffi::OsStr;
  use std::os::unix::ffi::OsStrExt;
  let mk = |bytes: &[u8]| gwm::worktree::WorktreeInfo {
    name: "wt".into(),
    id: "wt".into(),
    path: std::path::PathBuf::from("/repo").join(OsStr::from_bytes(bytes)),
    branch: Some("main".into()),
    head: None,
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: Default::default(),
    link: gwm::github::BranchLink::empty(),
    issue_state: None,
    pr_state: None,
    age: None,
  };
  let a = JsonWorktree::from(&mk(b"wt-\xff"));
  let b = JsonWorktree::from(&mk(b"wt-\xfe"));
  assert_eq!(
    a.path,
    mk(b"wt-\xff").path.to_string_lossy(),
    "no suffix, plain lossy value"
  );
  assert_eq!(
    a.path, b.path,
    "lossy public paths may collide — the internal key disambiguates"
  );
}
