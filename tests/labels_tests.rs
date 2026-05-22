//! Unit tests for the `labels` module: deterministic colour hashing,
//! hex validation, and the declared-vs-remote diff. The `gh`-backed
//! fetch / push code is mocked at the JSON parsing boundary; CLI flow
//! lives in `tests/cli_binary.rs`.

use gwm::config::LabelConfig;
use gwm::labels::{
  deterministic_color, diff_labels, normalize_color, resolve_labels, validate_color, validate_label_name, LabelAction,
  LabelDiff, LabelSpec, RemoteLabel,
};

// --- Deterministic colour hashing ---------------------------------------

#[test]
fn deterministic_color_is_stable_for_same_name() {
  // Same name → same colour across runs. The whole point of the
  // deterministic mode is that a `bug` label gets the same colour in
  // every repo that declares it.
  let c1 = deterministic_color("bug");
  let c2 = deterministic_color("bug");
  assert_eq!(c1, c2);
}

#[test]
fn deterministic_color_returns_six_hex_chars() {
  // GitHub accepts 6-hex without leading `#`. Anything else is a bug.
  let c = deterministic_color("enhancement");
  assert_eq!(c.len(), 6, "expected 6 chars, got {:?}", c);
  assert!(
    c.chars().all(|ch| ch.is_ascii_hexdigit()),
    "expected hex only, got {:?}",
    c
  );
  assert!(
    c.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()),
    "expected lowercase, got {:?}",
    c
  );
}

#[test]
fn deterministic_color_distinguishes_different_names() {
  // We don't promise collision-free — but two well-known names must
  // map to different colours; otherwise the hash is broken.
  let a = deterministic_color("bug");
  let b = deterministic_color("enhancement");
  assert_ne!(a, b);
}

#[test]
fn deterministic_color_handles_whitespace_in_name() {
  // "good first issue" is the canary the issue spec called out.
  let c = deterministic_color("good first issue");
  assert_eq!(c.len(), 6);
  // And it must differ from the no-space variant — whitespace is part
  // of the name, not an irrelevant separator.
  assert_ne!(c, deterministic_color("goodfirstissue"));
}

// --- Hex colour validation -----------------------------------------------

#[test]
fn validate_color_accepts_canonical_form() {
  assert!(validate_color("d73a4a").is_ok());
  assert!(validate_color("0e8a16").is_ok());
}

#[test]
fn validate_color_accepts_uppercase_and_normalises() {
  // GitHub stores colours lowercase; we normalise on validation so the
  // diff doesn't surface a spurious "update" when the user wrote
  // `D73A4A` in their config.
  let norm = normalize_color("D73A4A").unwrap();
  assert_eq!(norm, "d73a4a");
}

#[test]
fn validate_color_strips_leading_hash() {
  // Users naturally type `#d73a4a`; we normalise it away rather than
  // reject the config and pretend the user "didn't read the docs".
  let norm = normalize_color("#d73a4a").unwrap();
  assert_eq!(norm, "d73a4a");
}

#[test]
fn validate_color_rejects_wrong_length() {
  assert!(validate_color("d73a4").is_err());
  assert!(validate_color("d73a4abc").is_err());
  assert!(validate_color("").is_err());
}

#[test]
fn validate_color_rejects_non_hex() {
  assert!(validate_color("xyzxyz").is_err());
  assert!(validate_color("zz4a4a").is_err());
}

// --- resolve_labels: declared → LabelSpec (final colour applied) --------

#[test]
fn resolve_labels_keeps_declared_colour() {
  let declared = vec![LabelConfig {
    name: "bug".into(),
    description: Some("Something broke".into()),
    color: Some("d73a4a".into()),
  }];
  let resolved = resolve_labels(&declared, false).unwrap();
  assert_eq!(resolved.len(), 1);
  assert_eq!(resolved[0].name, "bug");
  assert_eq!(resolved[0].description.as_deref(), Some("Something broke"));
  assert_eq!(resolved[0].color, "d73a4a");
}

#[test]
fn resolve_labels_normalises_hash_and_case() {
  let declared = vec![LabelConfig {
    name: "bug".into(),
    description: None,
    color: Some("#D73A4A".into()),
  }];
  let resolved = resolve_labels(&declared, false).unwrap();
  assert_eq!(resolved[0].color, "d73a4a");
}

#[test]
fn resolve_labels_falls_back_to_deterministic_when_color_absent() {
  // Two configs declaring the same name must yield the same final
  // colour — that's the "stable across repos" invariant.
  let a = resolve_labels(
    &[LabelConfig {
      name: "wip".into(),
      description: None,
      color: None,
    }],
    false,
  )
  .unwrap();
  let b = resolve_labels(
    &[LabelConfig {
      name: "wip".into(),
      description: None,
      color: None,
    }],
    false,
  )
  .unwrap();
  assert_eq!(a[0].color, b[0].color);
  assert_eq!(a[0].color.len(), 6);
}

#[test]
fn resolve_labels_random_overrides_omission() {
  // With `--random-colors`, missing colours get a random pastel; we
  // can only assert "it's a valid hex of length 6", not the value.
  // Explicitly declared colours are preserved.
  let declared = vec![
    LabelConfig {
      name: "x".into(),
      description: None,
      color: None,
    },
    LabelConfig {
      name: "y".into(),
      description: None,
      color: Some("00aabb".into()),
    },
  ];
  let resolved = resolve_labels(&declared, true).unwrap();
  assert_eq!(resolved[0].color.len(), 6);
  assert!(resolved[0].color.chars().all(|c| c.is_ascii_hexdigit()));
  // Explicit colour wins regardless of --random.
  assert_eq!(resolved[1].color, "00aabb");
}

#[test]
fn resolve_labels_surfaces_invalid_color() {
  // A bad hex in config explodes at resolve time with a clear message
  // — not silently coerced to a fallback.
  let declared = vec![LabelConfig {
    name: "bad".into(),
    description: None,
    color: Some("not-a-hex".into()),
  }];
  let err = resolve_labels(&declared, false).unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("bad"), "should mention the label name: {}", msg);
}

// --- Diff against the remote --------------------------------------------

fn spec(name: &str, color: &str, desc: Option<&str>) -> LabelSpec {
  LabelSpec {
    name: name.into(),
    description: desc.map(|s| s.into()),
    color: color.into(),
  }
}

fn rlabel(name: &str, color: &str, desc: Option<&str>) -> RemoteLabel {
  RemoteLabel {
    name: name.into(),
    description: desc.map(|s| s.into()),
    color: color.into(),
  }
}

#[test]
fn diff_empty_declared_yields_only_extras() {
  let declared = vec![];
  let remote = vec![rlabel("wontfix", "ffffff", None)];
  let diff = diff_labels(&declared, &remote);
  assert!(diff.to_create.is_empty());
  assert!(diff.to_update.is_empty());
  assert!(diff.matching.is_empty());
  assert_eq!(diff.extra_on_remote.len(), 1);
  assert_eq!(diff.extra_on_remote[0].name, "wontfix");
}

#[test]
fn diff_declared_not_on_remote_yields_create() {
  let declared = vec![spec("bug", "d73a4a", Some("broken"))];
  let remote = vec![];
  let diff = diff_labels(&declared, &remote);
  assert_eq!(diff.to_create.len(), 1);
  assert_eq!(diff.to_create[0].name, "bug");
  assert!(diff.to_update.is_empty());
  assert!(diff.matching.is_empty());
  assert!(diff.extra_on_remote.is_empty());
}

#[test]
fn diff_color_mismatch_yields_update() {
  let declared = vec![spec("good first issue", "7057ff", Some("Good for newcomers"))];
  let remote = vec![rlabel("good first issue", "008672", Some("Good for newcomers"))];
  let diff = diff_labels(&declared, &remote);
  assert!(diff.to_create.is_empty());
  assert_eq!(diff.to_update.len(), 1);
  assert_eq!(diff.to_update[0].action, LabelAction::Update);
  assert_eq!(diff.to_update[0].spec.color, "7057ff");
  // Carry the remote's old colour so `gwm labels list` can render
  // `~ ~good first issue (color #008672 → #7057ff)`.
  assert_eq!(diff.to_update[0].previous_color.as_deref(), Some("008672"));
}

#[test]
fn diff_description_mismatch_yields_update() {
  let declared = vec![spec("bug", "d73a4a", Some("broken"))];
  let remote = vec![rlabel("bug", "d73a4a", Some("old description"))];
  let diff = diff_labels(&declared, &remote);
  assert_eq!(diff.to_update.len(), 1);
  assert_eq!(diff.to_update[0].spec.description.as_deref(), Some("broken"));
}

#[test]
fn diff_full_match_yields_matching() {
  let declared = vec![spec("documentation", "0075ca", Some("doc"))];
  let remote = vec![rlabel("documentation", "0075ca", Some("doc"))];
  let diff = diff_labels(&declared, &remote);
  assert_eq!(diff.matching.len(), 1);
  assert_eq!(diff.matching[0].name, "documentation");
  assert!(diff.to_create.is_empty());
  assert!(diff.to_update.is_empty());
  assert!(diff.extra_on_remote.is_empty());
}

#[test]
fn diff_normalises_remote_color_case_before_compare() {
  // GitHub returns colours uppercase sometimes; the diff must not flag
  // these as updates. Same invariant as `normalize_color`.
  let declared = vec![spec("bug", "d73a4a", Some("broken"))];
  let remote = vec![rlabel("bug", "D73A4A", Some("broken"))];
  let diff = diff_labels(&declared, &remote);
  assert_eq!(diff.matching.len(), 1);
  assert!(diff.to_update.is_empty());
}

// --- Issue #100: label-name argv-injection guards -----------------------

#[test]
fn validate_label_name_accepts_normal_names() {
  // Spaces and unicode are GitHub-legal and common in real label sets
  // ("good first issue", "🚀 ship-it"). The validator must not be
  // overzealous and reject everything but `[a-z]+`.
  assert!(validate_label_name("bug").is_ok());
  assert!(validate_label_name("good first issue").is_ok());
  assert!(validate_label_name("priority/p1").is_ok());
  assert!(validate_label_name("🚀 ship-it").is_ok());
}

#[test]
fn validate_label_name_rejects_leading_dash() {
  // Issue #100. `gh label create -h` is parsed by gh's flag splitter
  // BEFORE the create call materialises — `-h` prints help and exits
  // 0. The push report would then claim "✓ created" for a label that
  // never existed. Same shape for `--repo`, which retargets to a
  // different repository entirely.
  let err = validate_label_name("-h").unwrap_err();
  let msg = format!("{}", err);
  assert!(
    msg.contains("'-'") || msg.contains("- "),
    "error must mention the offending leading dash; got: {}",
    msg
  );
  assert!(
    msg.contains("#100"),
    "error must cite issue #100 so the user can find context; got: {}",
    msg
  );

  // Same shape for `--repo`-style flags.
  assert!(validate_label_name("--repo").is_err());
  assert!(validate_label_name("-").is_err());
}

#[test]
fn validate_label_name_rejects_empty() {
  let err = validate_label_name("").unwrap_err();
  assert!(format!("{}", err).contains("empty"));
}

#[test]
fn validate_label_name_rejects_comma() {
  // GitHub uses `,` as the label-list separator in query strings; a
  // label whose name contains `,` would be split mid-name on filter
  // operations. Reject at the source.
  assert!(validate_label_name("foo,bar").is_err());
}

#[test]
fn validate_label_name_rejects_ascii_control_chars() {
  // A newline / tab in a label name breaks the `gh label list` JSON
  // round-trip and produces confusing downstream parse errors.
  assert!(validate_label_name("foo\nbar").is_err());
  assert!(validate_label_name("foo\tbar").is_err());
}

#[test]
fn resolve_labels_propagates_invalid_name_from_config() {
  // Defence-in-depth: `Config::validate_labels` runs at load time, but
  // `resolve_labels` is also reachable from a programmatic
  // `LabelConfig` (test fixtures, future API). The same refusal must
  // surface in both paths.
  let declared = vec![LabelConfig {
    name: "-h".into(),
    description: None,
    color: None,
  }];
  let err = resolve_labels(&declared, false).unwrap_err();
  assert!(format!("{}", err).contains("'-h'") || format!("{}", err).contains("-h"));
}

#[test]
fn diff_summary_counts_match_buckets() {
  // The CLI summary line ("would create 2, update 1, leave 3
  // untouched, ignore 1 extra-on-remote") is computed from these
  // counts; we want them stable across LabelDiff struct changes.
  let diff = LabelDiff {
    to_create: vec![spec("a", "aaaaaa", None), spec("b", "bbbbbb", None)],
    to_update: vec![gwm::labels::LabelUpdate {
      action: LabelAction::Update,
      spec: spec("c", "cccccc", None),
      previous_color: Some("dddddd".into()),
      previous_description: None,
    }],
    matching: vec![
      spec("d", "111111", None),
      spec("e", "222222", None),
      spec("f", "333333", None),
    ],
    extra_on_remote: vec![rlabel("g", "999999", None)],
  };
  let (c, u, m, x) = diff.counts();
  assert_eq!(c, 2);
  assert_eq!(u, 1);
  assert_eq!(m, 3);
  assert_eq!(x, 1);
}

// ---- Issue #106: generic summary_line helper ----------------------------
//
// `LabelDiff` and `MilestoneDiff` are structurally identical for the
// purposes of the one-line summaries printed by `gwm labels push` /
// `gwm milestones push`. The helper below pins the canonical shape so
// `cli.rs` can call a single function from two sites.

#[test]
fn diff_summary_line_renders_canonical_shape() {
  use gwm::labels::diff_summary_line;
  let s = diff_summary_line(2, 1, 3, 1);
  assert_eq!(s, "summary: 2 create · 1 update · 3 match · 1 extra-on-remote");
}

#[test]
fn diff_dry_run_line_uses_label_kind_for_grammar() {
  use gwm::labels::diff_dry_run_line;
  // Caller picks the noun ("label" or "milestone") so the helper
  // can be shared across modules without losing the singular noun.
  let s = diff_dry_run_line(2, 1, 3, 1, 0);
  assert_eq!(
    s,
    "would create 2, update 1, leave 3 untouched, prune 0, ignore 1 extra-on-remote"
  );
}
