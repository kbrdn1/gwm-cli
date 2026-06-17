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

// --- 5. Frozen field + type baseline ---------------------------------------
//
// The parity tests above compare the live DTO to the live schema file, so
// they catch a DTO field renamed *without* the schema following. They do NOT
// catch two cases the 1.0 pledge cares about (raised in #317 review):
//
//   1. a coordinated rename — DTO field AND its schema entry renamed in the
//      same commit — leaves `required ⊆ serialized ⊆ properties` still true;
//   2. a *type* change with the same name (e.g. `head: String` -> `head: u64`)
//      is invisible to a name-only set comparison.
//
// This baseline is a third, independent anchor: a hardcoded `(field, type)`
// map per surface. A rename now needs THREE coordinated edits (DTO, schema,
// baseline) and a type change is caught outright — turning an accidental
// break into a deliberate one that must also bump `SCHEMA_VERSION`. Editing
// the maps below is the conscious act the freeze exists to force.

/// Coarse JSON type tag of a value, with integers split out from floats so a
/// `u64 -> f64` drift is caught. A `null` reports `"null"`, but the baseline
/// is asserted against samples whose nullable fields are populated, so the
/// *non-null* type is what gets pinned (null is always permitted on a
/// nullable field).
fn json_type(v: &Value) -> &'static str {
  match v {
    Value::Null => "null",
    Value::Bool(_) => "bool",
    Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
    Value::Number(_) => "number",
    Value::String(_) => "string",
    Value::Array(_) => "array",
    Value::Object(_) => "object",
  }
}

/// Assert every `(field, expected_type)` in `baseline` is present in the
/// serialized object with the matching JSON type. `what` labels failures.
fn assert_type_baseline<T: serde::Serialize>(what: &str, value: &T, baseline: &[(&str, &str)]) {
  let v = serde_json::to_value(value).expect("serialize");
  let obj = v.as_object().expect("DTO must serialize to an object");
  for (field, expected) in baseline {
    let actual = obj
      .get(*field)
      .unwrap_or_else(|| panic!("{what}: stable field `{field}` is gone from the serialized output (rename/removal?)"));
    assert_eq!(
      json_type(actual),
      *expected,
      "{what}: stable field `{field}` changed type — was `{expected}`, now `{}`",
      json_type(actual)
    );
  }
}

#[test]
fn worktree_field_types_are_frozen() {
  assert_type_baseline(
    "worktree-list row",
    &sample_worktree(),
    &[
      ("name", "string"),
      ("id", "string"),
      ("path", "string"),
      ("branch", "string"),
      ("head", "string"),
      ("is_main", "bool"),
      ("is_locked", "bool"),
      ("is_prunable", "bool"),
      ("status", "object"),
      ("age_seconds", "integer"),
      ("issue", "integer"),
      ("pr", "integer"),
    ],
  );
  assert_type_baseline(
    "worktree status",
    &sample_worktree().status,
    &[
      ("is_dirty", "bool"),
      ("has_upstream", "bool"),
      ("ahead", "integer"),
      ("behind", "integer"),
      ("unknown", "bool"),
    ],
  );
}

#[test]
fn path_field_types_are_frozen() {
  let dto = JsonPath {
    name: "feat-317".into(),
    path: "/wt/feat-317".into(),
    branch: Some("feat/#317-freeze-machine-contracts".into()),
  };
  assert_type_baseline(
    "path result",
    &dto,
    &[("name", "string"), ("path", "string"), ("branch", "string")],
  );
}

#[test]
fn doctor_field_types_are_frozen() {
  let report = JsonDoctorReport {
    checks: vec![JsonCheck {
      name: "config".into(),
      status: "ok".into(),
      detail: "found .gwm.toml".into(),
      fix_hint: Some("run gwm init".into()),
    }],
    severity: "ok".into(),
    exit_code: 0,
  };
  assert_type_baseline(
    "doctor report",
    &report,
    &[("checks", "array"), ("severity", "string"), ("exit_code", "integer")],
  );
  assert_type_baseline(
    "doctor check",
    &report.checks[0],
    &[
      ("name", "string"),
      ("status", "string"),
      ("detail", "string"),
      ("fix_hint", "string"),
    ],
  );
}

// --- 6. Frozen *schema-side* baseline --------------------------------------
//
// Section 5 freezes the DTO side (what gwm emits). The committed schema
// files are themselves a published artifact external validators consume, so
// they need the symmetric freeze (raised in #317 review):
//
//   1. a schema edit that DROPS a stable field from `required` (makes it
//      optional) while the DTO still emits it leaves the parity check green
//      (`required` only shrinks, `serialized ⊆ properties` still holds) —
//      silently weakening the published contract;
//   2. a schema-only TYPE drift (e.g. `head`'s declared type flipped to
//      `integer` while the output stays a string) is invisible to a baseline
//      that only inspects the serialized DTO.
//
// So pin the schema's own `required` set and per-field declared `type`/`$ref`
// against an explicit frozen baseline, independent of the DTO. Experimental
// fields (`repo`) are deliberately excluded — they may move without a bump.

/// Normalized descriptor of a schema property's declared type: the `$ref`
/// target verbatim (prefixed), or the `type` keyword with a multi-type
/// array sorted+joined so `["string","null"]` is stable regardless of order.
fn schema_type_descriptor(prop: &Value) -> String {
  if let Some(r) = prop.get("$ref").and_then(|v| v.as_str()) {
    return format!("$ref:{r}");
  }
  match prop.get("type") {
    Some(Value::String(s)) => s.clone(),
    Some(Value::Array(a)) => {
      let mut parts: Vec<String> = a.iter().filter_map(|x| x.as_str().map(String::from)).collect();
      parts.sort();
      parts.join("|")
    }
    _ => panic!("schema property has neither `$ref` nor `type`"),
  }
}

/// Assert the schema object at `base_ptr` has exactly `frozen_required` as
/// its stable required set (experimental fields excluded by construction),
/// and that each `(field, descriptor)` in `frozen_types` matches the
/// property's declared `type`/`$ref`.
fn assert_schema_baseline(
  what: &str,
  schema: &Value,
  base_ptr: &str,
  frozen_required: &[&str],
  frozen_types: &[(&str, &str)],
) {
  let required = required_set(schema, &format!("{base_ptr}/required"));
  let frozen: BTreeSet<String> = frozen_required.iter().map(|s| s.to_string()).collect();
  assert_eq!(
    required, frozen,
    "{what}: schema `required` set drifted from the frozen stable baseline — a dropped/added required field is a contract change"
  );
  let props = schema
    .pointer(&format!("{base_ptr}/properties"))
    .unwrap_or_else(|| panic!("{what}: no properties at {base_ptr}"));
  for (field, expected) in frozen_types {
    let prop = props
      .get(*field)
      .unwrap_or_else(|| panic!("{what}: stable field `{field}` missing from schema properties"));
    assert_eq!(
      &schema_type_descriptor(prop),
      expected,
      "{what}: schema-declared type of `{field}` drifted from the frozen contract"
    );
  }
}

#[test]
fn worktree_schema_required_and_types_are_frozen() {
  let schema = read_schema("worktree-list.schema.json");
  assert_schema_baseline(
    "worktree-list row",
    &schema,
    "/$defs/worktree",
    // `repo` is experimental — intentionally NOT in the frozen required set.
    &[
      "name",
      "id",
      "path",
      "branch",
      "head",
      "is_main",
      "is_locked",
      "is_prunable",
      "status",
      "age_seconds",
      "issue",
      "pr",
    ],
    &[
      ("name", "string"),
      ("id", "string"),
      ("path", "string"),
      ("branch", "null|string"),
      ("head", "null|string"),
      ("is_main", "boolean"),
      ("is_locked", "boolean"),
      ("is_prunable", "boolean"),
      ("status", "$ref:#/$defs/status"),
      ("age_seconds", "integer|null"),
      ("issue", "integer|null"),
      ("pr", "integer|null"),
    ],
  );
  assert_schema_baseline(
    "worktree status",
    &schema,
    "/$defs/status",
    &["is_dirty", "has_upstream", "ahead", "behind", "unknown"],
    &[
      ("is_dirty", "boolean"),
      ("has_upstream", "boolean"),
      ("ahead", "integer"),
      ("behind", "integer"),
      ("unknown", "boolean"),
    ],
  );
}

#[test]
fn doctor_schema_required_and_types_are_frozen() {
  let schema = read_schema("doctor.schema.json");
  assert_schema_baseline(
    "doctor report",
    &schema,
    "",
    &["checks", "severity", "exit_code"],
    &[
      ("checks", "array"),
      ("severity", "$ref:#/$defs/status"),
      ("exit_code", "integer"),
    ],
  );
  assert_schema_baseline(
    "doctor check",
    &schema,
    "/$defs/check",
    &["name", "status", "detail", "fix_hint"],
    &[
      ("name", "string"),
      ("status", "$ref:#/$defs/status"),
      ("detail", "string"),
      ("fix_hint", "null|string"),
    ],
  );
}

#[test]
fn path_schema_required_and_types_are_frozen() {
  let schema = read_schema("path.schema.json");
  assert_schema_baseline(
    "path result",
    &schema,
    "",
    &["name", "path", "branch"],
    &[("name", "string"), ("path", "string"), ("branch", "null|string")],
  );
}

#[test]
fn output_schemas_tolerate_additive_fields() {
  // The versioning policy promises additive fields are backward-compatible
  // and consumers ignore unknowns (#317 review). For a schema-validating
  // consumer that's only true if the object schemas don't set
  // `additionalProperties: false` — otherwise a v1 validator would reject
  // an output carrying a new optional field. Freeze that: nobody may
  // re-tighten these without consciously breaking the documented policy.
  // (The producer-side guard `serialized ⊆ properties` still catches an
  // accidental leaked field at our own CI.)
  let cases: &[(&str, &[&str])] = &[
    ("worktree-list.schema.json", &["/$defs/worktree", "/$defs/status"]),
    ("doctor.schema.json", &["", "/$defs/check"]),
    ("path.schema.json", &[""]),
  ];
  for (file, pointers) in cases {
    let schema = read_schema(file);
    for ptr in *pointers {
      let obj = schema
        .pointer(ptr)
        .unwrap_or_else(|| panic!("{file}: no object at '{ptr}'"));
      assert_ne!(
        obj.get("additionalProperties"),
        Some(&Value::Bool(false)),
        "{file} at '{ptr}': additionalProperties:false breaks the additive-compatibility policy"
      );
    }
  }
}
