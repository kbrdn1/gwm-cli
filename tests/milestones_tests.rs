//! Unit tests for the `milestones` module: date normalisation, state
//! parsing, and the declared-vs-remote diff. The `gh`-backed fetch /
//! push code is mocked at the JSON parsing boundary; the CLI flow lives
//! in `tests/cli_binary.rs`.

use gwm::config::MilestoneConfig;
use gwm::milestones::{
  diff_milestones, normalize_due_on, parse_state, resolve_milestones, MilestoneAction, MilestoneDiff, MilestoneSpec,
  MilestoneState, MilestoneUpdate, RemoteMilestone,
};

// --- normalize_due_on ----------------------------------------------------

#[test]
fn normalize_due_on_accepts_short_date_form() {
  // `YYYY-MM-DD` is the form the issue spec calls out as "common-sense
  // semantic for a due date". We materialise it as 23:59:59 UTC of
  // that day — anything else means a milestone "due Friday" closes
  // mid-day, which surprises users.
  let n = normalize_due_on("2026-07-15").unwrap();
  assert_eq!(n, "2026-07-15T23:59:59Z");
}

#[test]
fn normalize_due_on_passes_rfc3339_through() {
  // Full RFC3339 round-trips verbatim — users who already type the
  // long form (e.g. copy-pasted from GitHub) should get exactly what
  // they wrote.
  let n = normalize_due_on("2026-07-15T17:00:00Z").unwrap();
  assert_eq!(n, "2026-07-15T17:00:00Z");
}

#[test]
fn normalize_due_on_rejects_garbage() {
  // A typo (`2026/07/15` with slashes) must blow up at config-resolve
  // time, not silently get coerced into a wrong date.
  assert!(normalize_due_on("2026/07/15").is_err());
  assert!(normalize_due_on("tomorrow").is_err());
  assert!(normalize_due_on("").is_err());
}

#[test]
fn normalize_due_on_rejects_impossible_calendar_date() {
  // `2026-02-30` is shaped right but isn't a real date. chrono's
  // strict parser catches this; the test pins that contract so a
  // future refactor doesn't silently accept it.
  assert!(normalize_due_on("2026-02-30").is_err());
}

// --- parse_state ---------------------------------------------------------

#[test]
fn parse_state_accepts_open_and_closed() {
  assert_eq!(parse_state("open").unwrap(), MilestoneState::Open);
  assert_eq!(parse_state("closed").unwrap(), MilestoneState::Closed);
}

#[test]
fn parse_state_is_case_sensitive() {
  // GitHub stores `state` lowercase; we don't try to be clever about
  // `Open` / `OPEN` because the cost (silent typo) outweighs the
  // benefit (one less character of friction).
  assert!(parse_state("Open").is_err());
  assert!(parse_state("OPEN").is_err());
}

#[test]
fn parse_state_rejects_unknown() {
  // Anything that isn't `open` or `closed` is a config typo we surface
  // verbatim so the user can grep their `.gwm.toml`.
  let err = parse_state("draft").unwrap_err();
  assert!(err.to_string().contains("draft"));
}

// --- resolve_milestones --------------------------------------------------

#[test]
fn resolve_milestones_defaults_state_to_open() {
  // Omitted `state` → Open. The CLI summary line ("would update 1 → open")
  // depends on this default being explicit, not None.
  let declared = vec![MilestoneConfig {
    title: "v0.7.0".into(),
    description: None,
    due_on: None,
    state: None,
  }];
  let resolved = resolve_milestones(&declared).unwrap();
  assert_eq!(resolved.len(), 1);
  assert_eq!(resolved[0].state, MilestoneState::Open);
}

#[test]
fn resolve_milestones_keeps_declared_state() {
  let declared = vec![MilestoneConfig {
    title: "v0.6.0".into(),
    description: None,
    due_on: None,
    state: Some("closed".into()),
  }];
  let resolved = resolve_milestones(&declared).unwrap();
  assert_eq!(resolved[0].state, MilestoneState::Closed);
}

#[test]
fn resolve_milestones_normalises_short_due_on() {
  let declared = vec![MilestoneConfig {
    title: "v0.7.0".into(),
    description: None,
    due_on: Some("2026-07-15".into()),
    state: None,
  }];
  let resolved = resolve_milestones(&declared).unwrap();
  assert_eq!(resolved[0].due_on.as_deref(), Some("2026-07-15T23:59:59Z"));
}

#[test]
fn resolve_milestones_collapses_empty_description() {
  // Empty `description = ""` and absent are equivalent. Same
  // contract as labels (see `norm_desc`).
  let declared = vec![MilestoneConfig {
    title: "v0.7.0".into(),
    description: Some("".into()),
    due_on: None,
    state: None,
  }];
  let resolved = resolve_milestones(&declared).unwrap();
  assert_eq!(resolved[0].description, None);
}

#[test]
fn resolve_milestones_surfaces_invalid_state_with_title() {
  // A bad `state` is caught at resolve time with the milestone title
  // in the error message — otherwise the user has to grep their
  // config to find the offending entry.
  let declared = vec![MilestoneConfig {
    title: "v0.7.0".into(),
    description: None,
    due_on: None,
    state: Some("draft".into()),
  }];
  let err = resolve_milestones(&declared).unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("v0.7.0"), "should mention the title: {}", msg);
  assert!(msg.contains("draft"), "should mention the bad value: {}", msg);
}

#[test]
fn resolve_milestones_surfaces_invalid_due_on_with_title() {
  let declared = vec![MilestoneConfig {
    title: "v0.7.0".into(),
    description: None,
    due_on: Some("not-a-date".into()),
    state: None,
  }];
  let err = resolve_milestones(&declared).unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("v0.7.0"), "should mention the title: {}", msg);
}

// --- Diff against the remote --------------------------------------------

fn spec(title: &str, due: Option<&str>, desc: Option<&str>, state: MilestoneState) -> MilestoneSpec {
  MilestoneSpec {
    title: title.into(),
    description: desc.map(|s| s.into()),
    due_on: due.map(|s| s.into()),
    state,
  }
}

fn rmilestone(
  number: u64,
  title: &str,
  due: Option<&str>,
  desc: Option<&str>,
  state: MilestoneState,
) -> RemoteMilestone {
  RemoteMilestone {
    number,
    title: title.into(),
    description: desc.map(|s| s.into()),
    due_on: due.map(|s| s.into()),
    state,
  }
}

#[test]
fn diff_empty_declared_yields_only_extras() {
  let declared = vec![];
  let remote = vec![rmilestone(1, "old-sprint", None, None, MilestoneState::Closed)];
  let diff = diff_milestones(&declared, &remote);
  assert!(diff.to_create.is_empty());
  assert!(diff.to_update.is_empty());
  assert!(diff.matching.is_empty());
  assert_eq!(diff.extra_on_remote.len(), 1);
  assert_eq!(diff.extra_on_remote[0].title, "old-sprint");
}

#[test]
fn diff_declared_not_on_remote_yields_create() {
  let declared = vec![spec(
    "v0.7.0",
    Some("2026-07-15T23:59:59Z"),
    Some("sprint"),
    MilestoneState::Open,
  )];
  let remote = vec![];
  let diff = diff_milestones(&declared, &remote);
  assert_eq!(diff.to_create.len(), 1);
  assert_eq!(diff.to_create[0].title, "v0.7.0");
  assert!(diff.to_update.is_empty());
  assert!(diff.matching.is_empty());
  assert!(diff.extra_on_remote.is_empty());
}

#[test]
fn diff_due_on_mismatch_yields_update_carrying_number() {
  // The `gh api PATCH …/milestones/{number}` call needs the remote
  // milestone number; the diff has to carry it through so the push
  // step doesn't re-fetch.
  let declared = vec![spec("v0.7.0", Some("2026-07-15T23:59:59Z"), None, MilestoneState::Open)];
  let remote = vec![rmilestone(
    42,
    "v0.7.0",
    Some("2026-07-01T23:59:59Z"),
    None,
    MilestoneState::Open,
  )];
  let diff = diff_milestones(&declared, &remote);
  assert!(diff.to_create.is_empty());
  assert_eq!(diff.to_update.len(), 1);
  assert_eq!(diff.to_update[0].action, MilestoneAction::Update);
  assert_eq!(diff.to_update[0].number, 42);
  assert_eq!(diff.to_update[0].spec.due_on.as_deref(), Some("2026-07-15T23:59:59Z"));
  assert_eq!(
    diff.to_update[0].previous_due_on.as_deref(),
    Some("2026-07-01T23:59:59Z")
  );
}

#[test]
fn diff_state_mismatch_yields_update() {
  let declared = vec![spec("v0.6.0", None, None, MilestoneState::Closed)];
  let remote = vec![rmilestone(7, "v0.6.0", None, None, MilestoneState::Open)];
  let diff = diff_milestones(&declared, &remote);
  assert_eq!(diff.to_update.len(), 1);
  assert_eq!(diff.to_update[0].spec.state, MilestoneState::Closed);
  assert_eq!(diff.to_update[0].previous_state, Some(MilestoneState::Open));
}

#[test]
fn diff_description_mismatch_yields_update() {
  let declared = vec![spec("v0.7.0", None, Some("sprint"), MilestoneState::Open)];
  let remote = vec![rmilestone(7, "v0.7.0", None, Some("old desc"), MilestoneState::Open)];
  let diff = diff_milestones(&declared, &remote);
  assert_eq!(diff.to_update.len(), 1);
  assert_eq!(diff.to_update[0].spec.description.as_deref(), Some("sprint"));
}

#[test]
fn diff_full_match_yields_matching() {
  let declared = vec![spec(
    "v0.7.0",
    Some("2026-07-15T23:59:59Z"),
    Some("sprint"),
    MilestoneState::Open,
  )];
  let remote = vec![rmilestone(
    9,
    "v0.7.0",
    Some("2026-07-15T23:59:59Z"),
    Some("sprint"),
    MilestoneState::Open,
  )];
  let diff = diff_milestones(&declared, &remote);
  assert_eq!(diff.matching.len(), 1);
  assert_eq!(diff.matching[0].title, "v0.7.0");
  assert!(diff.to_create.is_empty());
  assert!(diff.to_update.is_empty());
  assert!(diff.extra_on_remote.is_empty());
}

#[test]
fn diff_summary_counts_match_buckets() {
  // CLI summary line ("would create 2, update 1, leave 3 untouched,
  // ignore 1 extra-on-remote") — counts must stay stable across
  // MilestoneDiff struct changes.
  let diff = MilestoneDiff {
    to_create: vec![
      spec("a", None, None, MilestoneState::Open),
      spec("b", None, None, MilestoneState::Open),
    ],
    to_update: vec![MilestoneUpdate {
      action: MilestoneAction::Update,
      spec: spec("c", None, None, MilestoneState::Open),
      number: 1,
      previous_due_on: None,
      previous_description: None,
      previous_state: Some(MilestoneState::Closed),
    }],
    matching: vec![
      spec("d", None, None, MilestoneState::Open),
      spec("e", None, None, MilestoneState::Open),
      spec("f", None, None, MilestoneState::Open),
    ],
    extra_on_remote: vec![rmilestone(99, "g", None, None, MilestoneState::Open)],
  };
  let (c, u, m, x) = diff.counts();
  assert_eq!(c, 2);
  assert_eq!(u, 1);
  assert_eq!(m, 3);
  assert_eq!(x, 1);
}
