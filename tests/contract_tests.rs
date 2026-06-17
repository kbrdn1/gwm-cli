//! Contract freeze tests for the 1.0 machine surface (issue #317).
//!
//! These pin the three machine-readable contracts so a rename or removal
//! of a *stable* field fails CI and becomes a conscious breaking decision
//! (a `contract::SCHEMA_VERSION` bump) rather than an accident:
//!
//! 1. **JSON output schemas** — `docs/schema/*.json` vs the
//!    [`gwm::json_api`] DTOs they document.
//! 2. **Daemon JSON-RPC protocol** — method/notification names, error
//!    codes, and the `schema_version` carried in `worktrees.changed`.
//! 3. **`.gwm.toml` config schema** — the top-level section set.
//!
//! Anchor choice: the committed `docs/schema/*.json` files are the source
//! of truth (they are what external consumers fetch), so the parity tests
//! read them off disk and compare against the live serialized DTOs. The
//! relation is deliberately a **subset** check, not exact equality — the
//! `repo` field is workspace-only and present in `properties` but not in
//! `required`, so `required ⊆ serialized ⊆ properties` is the correct shape
//! (an `assert_eq!` would false-fail on it).
//!
//! Proven to bite: temporarily renaming a `JsonWorktree` field, or dropping
//! a `required` entry from a schema file, turns the relevant parity test
//! red (verified during development of #317).

use gwm::contract;
use gwm::json_api::{JsonCheck, JsonDoctorReport, JsonPath, JsonStatus, JsonWorktree};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

// --- helpers ---------------------------------------------------------------

fn schema_dir() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/schema")
}

fn read_schema(name: &str) -> Value {
  let path = schema_dir().join(name);
  let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
  serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Keys of the JSON object at `pointer` within `schema`.
fn object_keys(schema: &Value, pointer: &str) -> BTreeSet<String> {
  schema
    .pointer(pointer)
    .unwrap_or_else(|| panic!("schema has no object at {pointer}"))
    .as_object()
    .unwrap_or_else(|| panic!("{pointer} is not an object"))
    .keys()
    .cloned()
    .collect()
}

/// Entries of the `required` string array at `pointer` within `schema`.
fn required_set(schema: &Value, pointer: &str) -> BTreeSet<String> {
  schema
    .pointer(pointer)
    .unwrap_or_else(|| panic!("schema has no `required` at {pointer}"))
    .as_array()
    .unwrap_or_else(|| panic!("{pointer} is not an array"))
    .iter()
    .map(|v| v.as_str().expect("required entry must be a string").to_string())
    .collect()
}

/// Top-level keys of a serialized value (must serialize to an object).
fn serialized_keys<T: serde::Serialize>(value: &T) -> BTreeSet<String> {
  serde_json::to_value(value)
    .expect("serialize")
    .as_object()
    .expect("DTO must serialize to an object")
    .keys()
    .cloned()
    .collect()
}

/// `required ⊆ serialized ⊆ properties` — the field-set contract between a
/// schema object and the DTO it documents. `what` labels failures.
fn assert_field_contract(
  what: &str,
  required: &BTreeSet<String>,
  serialized: &BTreeSet<String>,
  properties: &BTreeSet<String>,
) {
  let missing_required: Vec<_> = required.difference(serialized).collect();
  assert!(
    missing_required.is_empty(),
    "{what}: stable field(s) {missing_required:?} are `required` in the schema but no longer serialized — a rename/removal that breaks the contract"
  );
  let undocumented: Vec<_> = serialized.difference(properties).collect();
  assert!(
    undocumented.is_empty(),
    "{what}: serialized field(s) {undocumented:?} are not in the schema `properties` — an undocumented field leaking into the frozen output"
  );
}

fn sample_worktree() -> JsonWorktree {
  JsonWorktree {
    name: "feat-317".into(),
    id: "feat-317".into(),
    path: "/wt/feat-317".into(),
    branch: Some("feat/#317-freeze-machine-contracts".into()),
    head: Some("a".repeat(40)),
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: JsonStatus {
      is_dirty: true,
      has_upstream: true,
      ahead: 1,
      behind: 0,
      unknown: false,
    },
    age_seconds: Some(10),
    issue: Some(317),
    pr: Some(318),
  }
}

// --- 1. JSON output schema parity ------------------------------------------

#[test]
fn worktree_list_schema_matches_the_dto() {
  let schema = read_schema("worktree-list.schema.json");
  let required = required_set(&schema, "/$defs/worktree/required");
  let properties = object_keys(&schema, "/$defs/worktree/properties");
  let serialized = serialized_keys(&sample_worktree());
  assert_field_contract("worktree-list row", &required, &serialized, &properties);
}

#[test]
fn worktree_status_schema_matches_the_dto() {
  let schema = read_schema("worktree-list.schema.json");
  let required = required_set(&schema, "/$defs/status/required");
  let properties = object_keys(&schema, "/$defs/status/properties");
  let serialized = serialized_keys(&sample_worktree().status);
  assert_field_contract("worktree status", &required, &serialized, &properties);
}

#[test]
fn workspace_repo_field_is_in_properties_but_not_required() {
  // The workspace `--workspace ... list --format=json` row flattens a
  // `JsonWorktree` and prepends `repo` (cli.rs `WorkspaceJsonWorktree`).
  // The schema must allow that extra field (it's in `properties`) without
  // demanding it for single-repo rows (it's NOT in `required`).
  let schema = read_schema("worktree-list.schema.json");
  let required = required_set(&schema, "/$defs/worktree/required");
  let properties = object_keys(&schema, "/$defs/worktree/properties");
  assert!(
    properties.contains("repo"),
    "workspace-mode `repo` must be a documented property"
  );
  assert!(
    !required.contains("repo"),
    "`repo` is workspace-only — requiring it would break single-repo rows"
  );

  // Replicate the workspace wire shape: flattened worktree + `repo`.
  let mut row = serde_json::to_value(sample_worktree()).unwrap();
  row
    .as_object_mut()
    .unwrap()
    .insert("repo".into(), Value::String("gwm-cli".into()));
  let serialized: BTreeSet<String> = row.as_object().unwrap().keys().cloned().collect();
  assert_field_contract("workspace worktree row", &required, &serialized, &properties);
}

#[test]
fn doctor_schema_matches_the_dtos() {
  let schema = read_schema("doctor.schema.json");

  let report = JsonDoctorReport {
    checks: vec![JsonCheck {
      name: "config".into(),
      status: "ok".into(),
      detail: "found .gwm.toml".into(),
      fix_hint: None,
    }],
    severity: "ok".into(),
    exit_code: 0,
  };
  let required = required_set(&schema, "/required");
  let properties = object_keys(&schema, "/properties");
  assert_field_contract("doctor report", &required, &serialized_keys(&report), &properties);

  let check_required = required_set(&schema, "/$defs/check/required");
  let check_properties = object_keys(&schema, "/$defs/check/properties");
  assert_field_contract(
    "doctor check",
    &check_required,
    &serialized_keys(&report.checks[0]),
    &check_properties,
  );
}

#[test]
fn path_schema_matches_the_dto() {
  let schema = read_schema("path.schema.json");
  let required = required_set(&schema, "/required");
  let properties = object_keys(&schema, "/properties");
  let dto = JsonPath {
    name: "feat-317".into(),
    path: "/wt/feat-317".into(),
    branch: Some("feat/#317-freeze-machine-contracts".into()),
  };
  assert_field_contract("path result", &required, &serialized_keys(&dto), &properties);
}

// --- 2. Schema version freeze ----------------------------------------------

#[test]
fn schema_version_baseline_is_one() {
  // The 1.0 baseline. Bumping this is a deliberate breaking decision; this
  // test flags the bump so it can't happen by accident.
  assert_eq!(contract::SCHEMA_VERSION, 1);
}

#[test]
fn every_schema_file_declares_the_contract_version() {
  for file in ["worktree-list.schema.json", "doctor.schema.json", "path.schema.json"] {
    let schema = read_schema(file);
    let v = schema
      .get("version")
      .unwrap_or_else(|| panic!("{file} must declare a `version`"))
      .as_u64()
      .unwrap_or_else(|| panic!("{file} `version` must be an integer"));
    assert_eq!(
      v as u32,
      contract::SCHEMA_VERSION,
      "{file} version drifted from contract::SCHEMA_VERSION"
    );
  }
}

// --- 3. Daemon protocol freeze ---------------------------------------------

#[test]
fn daemon_notification_carries_the_schema_version() {
  // The runtime drift signal for a long-lived `subscribe` client.
  let note = gwm::daemon::worktrees_changed_notification(&[]);
  assert_eq!(note["method"], Value::String("worktrees.changed".into()));
  assert_eq!(
    note["params"]["schema_version"]
      .as_u64()
      .expect("schema_version present"),
    contract::SCHEMA_VERSION as u64
  );
  // The frozen payload field is still there alongside the version.
  assert!(note["params"]["worktrees"].is_array());
}

#[test]
fn daemon_method_and_notification_names_are_frozen() {
  assert_eq!(contract::DAEMON_METHODS, &["list", "doctor", "path", "subscribe"]);
  assert_eq!(contract::DAEMON_NOTIFICATIONS, &["worktrees.changed"]);
}

#[test]
fn daemon_jsonrpc_error_codes_are_the_standard_values() {
  // Frozen to the JSON-RPC 2.0 standard codes — a client maps on these.
  assert_eq!(gwm::daemon::PARSE_ERROR, -32700);
  assert_eq!(gwm::daemon::INVALID_REQUEST, -32600);
  assert_eq!(gwm::daemon::METHOD_NOT_FOUND, -32601);
  assert_eq!(gwm::daemon::INVALID_PARAMS, -32602);
  assert_eq!(gwm::daemon::INTERNAL_ERROR, -32603);
}

// --- 4. Config schema freeze -----------------------------------------------

#[test]
fn config_top_level_sections_are_frozen() {
  // The serialized top-level keys of a default Config ARE the `.gwm.toml`
  // section set. Pinning them against `contract::CONFIG_SECTIONS` makes a
  // renamed/added/removed section a conscious edit in two places.
  let serialized = serialized_keys(&gwm::config::Config::default());
  let frozen: BTreeSet<String> = contract::CONFIG_SECTIONS.iter().map(|s| s.to_string()).collect();
  assert_eq!(
    serialized, frozen,
    "the `.gwm.toml` top-level section set drifted from contract::CONFIG_SECTIONS"
  );
}

#[test]
fn a_config_with_every_frozen_section_round_trips() {
  // Each frozen section must still parse — `deny_unknown_fields` means a
  // renamed section would make this fail, a second guard on the set above.
  let toml = "\
[worktree]
[bootstrap]
[hooks]
[doctor]
[tui]
[theme]
[git_tui]
[review]
[issue_template]
[pr_template]
[aliases]
[gitmoji]
";
  let cfg: gwm::config::Config = toml::from_str(toml).expect("frozen sections must parse");
  // Array-of-table and list sections default to empty without a block.
  let _ = serde_json::to_value(&cfg).unwrap();
}
