use gwm::config::{
  expand_placeholders, resolved_rows, review_tool_preset, BranchTypesSource, ClipboardMode, Config, ConfigRow,
  ConfigSource, MacroOpenMode, MuxTarget, SidebarOrientation, SidebarPosition, TuiLayout, TuiOpenMode, WorktreeConfig,
  CONFIG_FILE,
};
use gwm::multiplexer::SplitDirection;
use tempfile::TempDir;

/// Process-global lock guarding every test in this binary that can **observe**
/// `$HOME`, not only the ones whose template mentions `{home}` (#503).
///
/// The narrower rule this doc used to state was the bug: `expand_placeholders`
/// resolves `dirs::home_dir` unconditionally, before it looks at a single
/// token (`src/config.rs`), so *every* call is a concurrent reader whatever
/// the pattern says. `set_var` is `unsafe` precisely because it races a
/// concurrent `getenv`, and a mutex only serialises the participants that take
/// it — so the rule has to be "can this test see `$HOME`?", which a reader
/// adding a test can answer, rather than "does this string contain `{home}`?",
/// which looks answerable and is not.
///
/// The other half of that rule is which functions can see `$HOME`, and the
/// note here used to answer it wrongly (#507): it claimed a layered load reads
/// it through `Config::load_layered`. It does not. `load_layered` and
/// `resolved_rows` take the global config path as a **parameter**, so they
/// resolve nothing; `Config::load_for_repo` is the one that calls
/// `global_config_path()`, and no test in this binary calls it. So
/// `expand_placeholders` is the only ambient reader reachable from here, and
/// #503 covered it completely.
///
/// The rule is no longer only stated: `tests/env_guard_invariant_tests.rs`
/// checks it by construction, for this binary and the three others that
/// rewrite an environment variable.
fn env_lock() -> &'static std::sync::Mutex<()> {
  static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
  LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(unix)]
fn toml_absolute_path(unix: &'static str, _windows: &'static str) -> &'static str {
  unix
}

#[cfg(windows)]
fn toml_absolute_path(_unix: &'static str, windows: &'static str) -> &'static str {
  windows
}

// --- Labels section (issue #81) -----------------------------------------

#[test]
fn labels_default_is_empty() {
  // Absent `[[labels]]` block must resolve to an empty vec — never None or
  // a placeholder set. This is the "0 labels declared, nothing to push"
  // contract from the issue.
  let cfg = Config::default();
  assert!(cfg.labels.is_empty());
}

#[test]
fn labels_section_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[labels]]
name = "bug"
description = "Something isn't working"
color = "d73a4a"

[[labels]]
name = "enhancement"
description = "New feature or request"

[[labels]]
name = "good first issue"
description = "Good for newcomers"
color = "7057ff"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.labels.len(), 3);

  assert_eq!(cfg.labels[0].name, "bug");
  assert_eq!(cfg.labels[0].description.as_deref(), Some("Something isn't working"));
  assert_eq!(cfg.labels[0].color.as_deref(), Some("d73a4a"));

  assert_eq!(cfg.labels[1].name, "enhancement");
  // Omitted `color` reads as None — colour resolution happens later in the
  // labels module via deterministic hashing.
  assert_eq!(cfg.labels[1].color, None);

  // Names with whitespace must round-trip verbatim (issue mentioned
  // "good first issue" as the canary).
  assert_eq!(cfg.labels[2].name, "good first issue");
}

#[test]
fn labels_section_minimal_only_name_is_valid() {
  // `name` is the sole required field. Both `description` and `color`
  // are optional — gwm picks a deterministic colour and an empty
  // description for the latter.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[labels]]
name = "wip"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.labels.len(), 1);
  assert_eq!(cfg.labels[0].name, "wip");
  assert_eq!(cfg.labels[0].description, None);
  assert_eq!(cfg.labels[0].color, None);
}

#[test]
fn labels_load_rejects_leading_dash_name_at_load_time() {
  // Issue #100. A `[[labels]] name = "-h"` would be parsed by gh's
  // flag splitter and silently no-op the create call. The validation
  // runs in `Config::load_for_repo` so the user finds the offending
  // entry index in the error rather than discovering it through the
  // "0 created" mystery in the push report.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[worktree]
base = "/tmp/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"

[[labels]]
name = "-h"
"#,
  )
  .unwrap();

  let err = Config::load_layered(dir.path(), None).unwrap_err();
  let msg = format!("{}", err);
  assert!(
    msg.contains("labels[0]"),
    "error must surface the offending entry index; got: {}",
    msg
  );
  assert!(
    msg.contains("'-'") || msg.contains("- ") || msg.contains("\"-h\""),
    "error must explain the leading-dash refusal; got: {}",
    msg
  );
}

#[test]
fn labels_section_absent_keeps_empty_vec() {
  // Backwards-compatibility: a config defining only `[worktree]` (no
  // `[[labels]]`) must resolve to an empty list. Same contract as the
  // doctor / tui sections.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[worktree]
base = "/tmp/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(cfg.labels.is_empty());
}

// --- Milestones section (issue #82) -------------------------------------

#[test]
fn milestones_default_is_empty() {
  // Absent `[[milestones]]` block must resolve to an empty vec. Same
  // "0 milestones declared, nothing to push" contract as labels.
  let cfg = Config::default();
  assert!(cfg.milestones.is_empty());
}

#[test]
fn milestones_section_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[milestones]]
title = "v0.7.0"
description = "Configurability sprint"
due_on = "2026-07-15"
state = "open"

[[milestones]]
title = "v0.8.0"
due_on = "2026-10-01T17:00:00Z"

[[milestones]]
title = "v0.6.0"
state = "closed"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.milestones.len(), 3);

  assert_eq!(cfg.milestones[0].title, "v0.7.0");
  assert_eq!(cfg.milestones[0].description.as_deref(), Some("Configurability sprint"));
  assert_eq!(cfg.milestones[0].due_on.as_deref(), Some("2026-07-15"));
  assert_eq!(cfg.milestones[0].state.as_deref(), Some("open"));

  assert_eq!(cfg.milestones[1].title, "v0.8.0");
  assert_eq!(cfg.milestones[1].description, None);
  // RFC3339 form round-trips verbatim — normalisation is the milestones
  // module's job, not the config loader's.
  assert_eq!(cfg.milestones[1].due_on.as_deref(), Some("2026-10-01T17:00:00Z"));
  assert_eq!(cfg.milestones[1].state, None);

  assert_eq!(cfg.milestones[2].title, "v0.6.0");
  assert_eq!(cfg.milestones[2].state.as_deref(), Some("closed"));
}

#[test]
fn milestones_section_minimal_only_title_is_valid() {
  // `title` is the sole required field. `description`, `due_on`, and
  // `state` are all optional; the module defaults `state` to "open"
  // and leaves the others as None.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[milestones]]
title = "Backlog"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.milestones.len(), 1);
  assert_eq!(cfg.milestones[0].title, "Backlog");
  assert_eq!(cfg.milestones[0].description, None);
  assert_eq!(cfg.milestones[0].due_on, None);
  assert_eq!(cfg.milestones[0].state, None);
}

#[test]
fn milestones_section_absent_keeps_empty_vec() {
  // Backwards-compatibility: a config with only `[worktree]` (no
  // `[[milestones]]`) must resolve to an empty list.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[worktree]
base = "/tmp/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(cfg.milestones.is_empty());
}

#[test]
fn defaults_are_sane() {
  let cfg = Config::default();
  assert_eq!(cfg.worktree.branch_pattern, "{type}/#{issue}-{desc}");
  assert_eq!(cfg.worktree.path_pattern, "{type}-{issue}-{desc}");
  assert!(cfg.bootstrap.copy.is_empty());
  assert!(cfg.bootstrap.guard.is_empty());
  assert!(cfg.bootstrap.command.is_empty());
}

#[test]
fn doctor_section_defaults_to_dev_and_main() {
  // The hardcoded list previously living in `src/doctor.rs` is now a
  // configurable default — but the default value must stay
  // `["dev", "main"]` so existing repos see zero behaviour change.
  let cfg = Config::default();
  assert_eq!(cfg.doctor.trunks, vec!["dev".to_string(), "main".to_string()]);
}

#[test]
fn doctor_section_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[doctor]
trunks = ["master", "release-3.x", "release-4.x"]
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(
    cfg.doctor.trunks,
    vec![
      "master".to_string(),
      "release-3.x".to_string(),
      "release-4.x".to_string()
    ]
  );
}

#[test]
fn doctor_section_absent_keeps_defaults() {
  // A config that defines only `[worktree]` (no `[doctor]` section) must
  // still resolve `cfg.doctor.trunks` to the documented default — that's
  // the backwards-compatibility contract for repos predating the section.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[worktree]
base = "/tmp/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.doctor.trunks, vec!["dev".to_string(), "main".to_string()]);
}

#[test]
fn doctor_section_empty_trunks_means_no_filter() {
  // An explicit empty list is a valid config choice — it tells doctor
  // "no merge filter, flag every gwm-style branch without a worktree".
  // Distinct from "section absent" (which falls back to defaults).
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[doctor]
trunks = []
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(cfg.doctor.trunks.is_empty());
}

#[test]
fn placeholders_expand() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let out = expand_placeholders(
    "{home}/cc-worktree/{repo}/{type}-{issue}-{desc}",
    "my-repo",
    Some("feat"),
    Some("123"),
    Some("foo"),
    None,
  )
  .unwrap();
  assert!(out.ends_with("/cc-worktree/my-repo/feat-123-foo"));
  assert!(!out.contains("{home}"));
  assert!(!out.contains("{repo}"));
}

#[test]
fn placeholders_no_optional_args_leave_repo_only() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let out = expand_placeholders("{home}/{repo}", "x", None, None, None, None).unwrap();
  assert!(out.ends_with("/x"));
}

#[test]
fn placeholders_expand_repo_path_and_parent() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let repo_path = std::path::Path::new("/Users/me/Projects/Perso/gwm-cli");
  // Derive the expected prefixes from `repo_path` itself rather than
  // duplicating the literal parent string — the contract under test is
  // "{repo_parent} → the repo's parent dir, {repo_path} → the repo dir".
  let parent = repo_path.parent().unwrap().to_string_lossy();
  let full_dir = repo_path.to_string_lossy();

  let out = expand_placeholders(
    "{repo_parent}/worktrees/{repo}-{type}-{issue}",
    "gwm-cli",
    Some("feat"),
    Some("42"),
    None,
    Some(repo_path),
  )
  .unwrap();
  assert_eq!(out, format!("{parent}/worktrees/gwm-cli-feat-42"));
  assert!(!out.contains("{repo_parent}"));

  let full = expand_placeholders("{repo_path}/.worktrees", "gwm-cli", None, None, None, Some(repo_path)).unwrap();
  assert_eq!(full, format!("{full_dir}/.worktrees"));
  assert!(!full.contains("{repo_path}"));
}

#[test]
fn placeholders_repo_path_tokens_left_literal_without_path() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  // When no repo path is supplied the disk-path tokens are passed
  // through untouched rather than collapsing to an empty string.
  let out = expand_placeholders("{repo_parent}/x", "r", None, None, None, None).unwrap();
  assert_eq!(out, "{repo_parent}/x");
}

#[test]
fn load_returns_defaults_when_no_file() {
  let dir = TempDir::new().unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.worktree.branch_pattern, WorktreeConfig::default().branch_pattern);
}

#[test]
fn load_parses_repo_config() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[worktree]
base = "/tmp/wt/{repo}"
path_pattern = "{type}_{issue}_{desc}"
branch_pattern = "{type}/{issue}-{desc}"

[[bootstrap.copy]]
from = ".env"
to = ".env"
required = false
guards = ["safe-env"]

[[bootstrap.guard]]
name = "safe-env"
deny_patterns = ["secret"]
on_match = "abort"

[[bootstrap.command]]
name = "echo"
run = "echo hi"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.worktree.base, "/tmp/wt/{repo}");
  assert_eq!(cfg.bootstrap.copy.len(), 1);
  assert_eq!(cfg.bootstrap.guard.len(), 1);
  assert_eq!(cfg.bootstrap.command.len(), 1);
  assert_eq!(cfg.bootstrap.guard[0].on_match, "abort");
  assert!(cfg.guard_by_name("safe-env").is_some());
  assert!(cfg.guard_by_name("nope").is_none());
}

#[test]
fn write_default_creates_file() {
  let dir = TempDir::new().unwrap();
  let path = Config::write_default(dir.path()).unwrap();
  assert!(path.exists());
  let raw = std::fs::read_to_string(&path).unwrap();
  assert!(raw.contains("[worktree]"));
}

#[test]
fn write_default_refuses_overwrite() {
  let dir = TempDir::new().unwrap();
  Config::write_default(dir.path()).unwrap();
  assert!(Config::write_default(dir.path()).is_err());
}

#[test]
fn malformed_config_returns_error() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "not valid toml [[[").unwrap();
  let res = Config::load_layered(dir.path(), None);
  assert!(res.is_err());
}

// Issue #30: the TUI confirm overlay has a configurable safety countdown
// when `delete_branch_on_remove` is armed. Default 3s (matches the
// example in the issue body); range 0..=5 where 0 means "no countdown —
// fall back to the classic single-keystroke confirm". Values above 5 are
// clamped on read so a misconfigured repo never strands a destructive
// action behind a 60s wait.

#[test]
fn tui_section_defaults_to_three_second_countdown() {
  let cfg = Config::default();
  assert_eq!(cfg.tui.confirm_countdown_secs, 3);
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 3);
  assert_eq!(
    cfg.tui.auto_refresh_secs, 60,
    "TUI auto-refresh defaults to once per minute"
  );
}

#[test]
fn tui_section_absent_keeps_defaults() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[worktree]
base = "/tmp/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 3);
}

#[test]
fn tui_sidebar_position_defaults_to_right() {
  // Pre-#188 behaviour: sidebar on the right. The default must hold so
  // existing users see no change until they set the knob or press `H`.
  let cfg = Config::default();
  assert_eq!(cfg.tui.sidebar_position, SidebarPosition::Right);
}

#[test]
fn tui_sidebar_position_absent_keeps_right() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
confirm_countdown_secs = 2
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.sidebar_position, SidebarPosition::Right);
}

#[test]
fn tui_sidebar_position_parses_left() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
sidebar_position = "left"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.sidebar_position, SidebarPosition::Left);
  assert!(cfg.tui.sidebar_position.is_left());
}

#[test]
fn tui_sidebar_orientation_defaults_to_stacked() {
  // #217 made `stacked` the launch layout: the status pane reads best
  // under the worktrees table where it gets the full width. Making the
  // orientation configurable (#365) must not move that default — the
  // `SidebarState::orientation` doc-comment claimed `auto` for two
  // releases, so pin the real value here rather than trust prose.
  let cfg = Config::default();
  assert_eq!(cfg.tui.sidebar_orientation, SidebarOrientation::Stacked);
}

#[test]
fn tui_sidebar_orientation_absent_keeps_stacked() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
confirm_countdown_secs = 2
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.sidebar_orientation, SidebarOrientation::Stacked);
}

#[test]
fn tui_sidebar_orientation_parses_side_by_side() {
  // The hyphenated spelling is the contract: `rename_all = "lowercase"`
  // would deserialise this variant as `sidebyside`, the exact trap
  // `MacroOpenMode` hit on PR #292 (`mux_pane` → `muxpane`).
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
sidebar_orientation = "side-by-side"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.sidebar_orientation, SidebarOrientation::SideBySide);
}

#[test]
fn tui_sidebar_orientation_parses_auto() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
sidebar_orientation = "auto"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.sidebar_orientation, SidebarOrientation::Auto);
}

#[test]
fn tui_sidebar_orientation_serialises_back_to_its_label() {
  // The config panel writes the choice back as the label string, so the
  // serialised form and `label()` must agree or a panel edit would
  // produce a file that no longer loads.
  for orientation in [
    SidebarOrientation::Auto,
    SidebarOrientation::SideBySide,
    SidebarOrientation::Stacked,
  ] {
    let serialised = toml::Value::try_from(orientation).unwrap();
    assert_eq!(
      serialised.as_str(),
      Some(orientation.label()),
      "{orientation:?} must serialise as its status-bar label"
    );
  }
}

#[test]
fn tui_sidebar_orientation_invalid_value_errors_at_parse_time() {
  // Same contract as `tui.open.mode` and `sidebar_position`: an unknown
  // variant is a hard load error, never a silent fallback.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
sidebar_orientation = "diagonal"
"#,
  )
  .unwrap();
  assert!(Config::load_layered(dir.path(), None).is_err());
}

#[test]
fn tui_section_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
confirm_countdown_secs = 2
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.confirm_countdown_secs, 2);
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 2);
}

#[test]
fn tui_countdown_zero_disables_countdown() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
confirm_countdown_secs = 0
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 0);
}

#[test]
fn tui_auto_refresh_zero_disables_periodic_refresh() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
auto_refresh_secs = 0
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.auto_refresh_secs, 0);
}

#[test]
fn tui_auto_refresh_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
auto_refresh_secs = 15
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.auto_refresh_secs, 15);
}

#[test]
fn tui_layout_defaults_to_compact() {
  // #545: compact is the default layout — the density is the point, and
  // the box rules were the single largest source of wasted space. A
  // config with no `[tui]` block gets it.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[worktree]\nbase = \"~/wt\"\n").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.layout, TuiLayout::Compact, "layout must default to compact");
  assert!(cfg.tui.layout.is_compact());
}

#[test]
fn tui_layout_round_trips_through_toml() {
  // `bordered` restores the pre-1.8 boxes for users who want them back.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
layout = "bordered"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.layout, TuiLayout::Bordered);
  assert!(!cfg.tui.layout.is_compact());
}

#[test]
fn tui_layout_rejects_an_unknown_value() {
  // Same contract as the other `[tui]` enums: a typo is a hard load
  // error, not a silent fallback to the default.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
layout = "borderd"
"#,
  )
  .unwrap();
  assert!(
    Config::load_layered(dir.path(), None).is_err(),
    "an unknown layout must fail the load"
  );
}

#[test]
fn tui_layout_labels_match_their_serialised_spelling() {
  // The Settings panel offers `label()` as a choice and writes the chosen
  // string back into `.gwm.toml`, so a label that drifts from the serde
  // spelling produces a file that no longer loads. (Until #546 wired the
  // field into the TUI tab, this reasoning described a write-back that
  // did not exist — the assertion was right, its justification was not.)
  for layout in TuiLayout::ALL {
    let dir = TempDir::new().unwrap();
    std::fs::write(
      dir.path().join(CONFIG_FILE),
      format!("[tui]\nlayout = \"{}\"\n", layout.label()),
    )
    .unwrap();
    let cfg = Config::load_layered(dir.path(), None)
      .unwrap_or_else(|e| panic!("label {:?} must round-trip: {e}", layout.label()));
    assert_eq!(cfg.tui.layout, layout);
  }
}

#[test]
fn tui_status_one_line_defaults_to_true() {
  // #547: the fold is the default, so a config with no `[tui]` block gets
  // it. `#[serde(default)]` on a bool would yield `false` and silently
  // invert the knob for every user who does not spell the key out — this
  // is the assertion that catches that.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[worktree]\nbase = \"~/wt\"\n").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(cfg.tui.status_one_line, "status_one_line must default to true");
  // Including when `[tui]` exists but says nothing about it.
  std::fs::write(dir.path().join(CONFIG_FILE), "[tui]\nlayout = \"bordered\"\n").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(
    cfg.tui.status_one_line,
    "a `[tui]` block that omits the key still gets the default"
  );
  assert!(
    Config::default().tui.status_one_line,
    "`Config::default()` must agree with the serde default"
  );
}

#[test]
fn tui_status_one_line_round_trips_through_toml() {
  // `false` restores the labelled four-row block for users who want it.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
status_one_line = false
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(!cfg.tui.status_one_line);
}

#[test]
fn tui_status_one_line_is_independent_of_the_layout() {
  // The knob is not a compact-mode behaviour (#547 amends #545's
  // acceptance): bordered folds too unless the user turns it off.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
layout = "bordered"
status_one_line = true
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.layout, TuiLayout::Bordered);
  assert!(cfg.tui.status_one_line, "bordered + folded is a valid combination");
}

#[test]
fn tui_countdown_clamped_to_five_seconds() {
  // A user who types `confirm_countdown_secs = 30` in their .gwm.toml
  // wants more friction; we cap it at 5 so the destructive path is never
  // unreasonably slow. The raw field stays at the user's value (for
  // diagnostics / round-trip), only the accessor clamps.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
confirm_countdown_secs = 30
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.confirm_countdown_secs, 30);
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 5);
}

// Issue #75: configurable launchers for `l` (git_tui) and `R` (review).
// Two sibling sections share the same shape — `command` (shell line with
// placeholders) + `fullscreen` (suspend gwm TUI for TUI tools). The
// `[review]` section also takes a `tool = ...` sugar for built-in
// presets and a `skip_when_no_changes` knob (default true). Absent
// sections must keep gwm's previous behaviour: `l` → `lazygit -p {path}`
// fullscreen, `R` inert with a status-bar hint.

#[test]
fn git_tui_section_defaults_to_lazygit_preserving_legacy_behaviour() {
  // Backwards-compat contract: no `[git_tui]` in `.gwm.toml` must keep
  // the `l` keybinding pointed at `lazygit -p {path}` with fullscreen,
  // i.e. identical to the hardcoded behaviour before issue #75 landed.
  let cfg = Config::default();
  let r = cfg.git_tui.resolved();
  assert_eq!(r.command, "lazygit -p {path}");
  assert!(r.fullscreen, "lazygit is a TUI tool, gwm must suspend itself");
}

#[test]
fn git_tui_section_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[git_tui]
command = "gitui -d {path}"
fullscreen = true
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let r = cfg.git_tui.resolved();
  assert_eq!(r.command, "gitui -d {path}");
  assert!(r.fullscreen);
}

#[test]
fn git_tui_can_opt_out_of_fullscreen() {
  // A user who wires `l` to launch a non-TUI editor (`code {path}`) needs
  // to keep gwm visible — fullscreen=false skips the suspend dance.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[git_tui]
command = "code {path}"
fullscreen = false
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let r = cfg.git_tui.resolved();
  assert_eq!(r.command, "code {path}");
  assert!(!r.fullscreen);
}

#[test]
fn review_section_defaults_to_disabled_with_skip_true() {
  // No `[review]` block ⇒ `R` is inert; the TUI shows a status-bar
  // hint inviting the user to configure it. `skip_when_no_changes`
  // defaults to true so a deliberately-set-but-empty review run
  // doesn't shell out for nothing.
  let cfg = Config::default();
  assert!(
    cfg.review.resolved().is_none(),
    "default review must be inert until configured"
  );
  assert!(cfg.review.skip_when_no_changes);
  assert!(cfg.review.default_base.is_none());
  assert!(!cfg.review.has_shadowed_tool());
}

#[test]
fn review_section_explicit_command_wins() {
  // The primary contract: a free-form shell line. Placeholders are not
  // expanded here — that's the launcher module's job.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[review]
command = "my-review --base {base} --head {head}"
fullscreen = true
skip_when_no_changes = false
default_base = "trunk"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let r = cfg.review.resolved().expect("explicit command must resolve");
  assert_eq!(r.command, "my-review --base {base} --head {head}");
  assert!(r.fullscreen);
  assert!(!cfg.review.skip_when_no_changes);
  assert_eq!(cfg.review.default_base.as_deref(), Some("trunk"));
}

#[test]
fn review_section_tool_preset_lumen_resolves_to_fullscreen_diff() {
  // `tool = "lumen"` is the canonical example from the issue.
  // Resolves to `lumen diff {base}..{head}` with fullscreen=true
  // because lumen is a ratatui TUI like gwm itself.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[review]
tool = "lumen"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let r = cfg.review.resolved().expect("lumen preset must resolve");
  assert_eq!(r.command, "lumen diff {base}..{head}");
  assert!(r.fullscreen, "lumen is a TUI — gwm must suspend itself");
}

#[test]
fn review_tool_preset_table_covers_documented_set() {
  // Pin the canonical table the docs / SKILL.md reference. A regression
  // that renames or drops one of these would break the docs silently.
  assert_eq!(review_tool_preset("lumen"), Some(("lumen diff {base}..{head}", true)));
  assert_eq!(
    review_tool_preset("claude"),
    Some(("claude --print 'review the diff {base}..{head}'", false))
  );
  assert_eq!(
    review_tool_preset("codex"),
    Some(("codex review {base}..{head}", false))
  );
  assert_eq!(
    review_tool_preset("aider"),
    Some(("aider --message 'review {base}..{head}'", true))
  );
  assert_eq!(review_tool_preset("gh"), Some(("gh pr view --web", false)));
  assert_eq!(review_tool_preset("unknown"), None);
}

#[test]
fn review_unknown_tool_resolves_to_none() {
  // Unknown preset is a soft no-op — the resolved launcher is `None` so
  // the TUI can surface a status-bar hint instead of crashing.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[review]
tool = "made-up"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(
    cfg.review.resolved().is_none(),
    "unknown preset must not silently fall back to a real tool"
  );
}

#[test]
fn review_command_overrides_tool() {
  // Both `command` and `tool` set ⇒ command wins. The user can still
  // detect the shadow via `has_shadowed_tool()` for a startup warning.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[review]
tool = "lumen"
command = "my-bot --diff-file {diff}"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let r = cfg.review.resolved().unwrap();
  assert_eq!(
    r.command, "my-bot --diff-file {diff}",
    "`command` must override `tool` when both are set"
  );
  assert!(cfg.review.has_shadowed_tool());
}

#[test]
fn review_fullscreen_overrides_preset_default() {
  // The preset for `tool = "lumen"` defaults fullscreen=true. The user
  // must be able to opt out per-repo (e.g. running lumen in a tmux pane).
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[review]
tool = "lumen"
fullscreen = false
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let r = cfg.review.resolved().unwrap();
  assert!(
    !r.fullscreen,
    "explicit fullscreen=false must override the preset default"
  );
}

#[test]
fn tui_countdown_value_above_u8_max_still_clamps() {
  // Regression for Copilot review on PR #66: the documented contract is
  // "values above 5 are clamped on read". A `u8` field would cap at 255
  // *at parse time*, turning a typo like `confirm_countdown_secs = 300`
  // into a hard `Config::load_for_repo` error instead of the documented
  // clamp-to-5. The field must accept any non-negative integer so the
  // promise stays whole.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
confirm_countdown_secs = 300
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).expect("300 must parse, not error");
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 5);
}

// Issue #73: [tui.open] table. The `o` key in the TUI dispatches to one of
// three behaviours (shell-in-worktree, editor, OS file manager) decided by
// config. Default is `shell` — lazygit-style — to match the worktree-manager
// workflow ("I want to *do work* in this worktree").

#[test]
fn tui_open_section_defaults_to_shell_mode() {
  let cfg = Config::default();
  assert_eq!(cfg.tui.open.mode, TuiOpenMode::Shell);
  assert!(cfg.tui.open.shell_cmd.is_none());
  assert!(cfg.tui.open.editor_cmd.is_none());
}

#[test]
fn tui_open_section_absent_keeps_defaults() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[worktree]
base = "/tmp/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.open.mode, TuiOpenMode::Shell);
}

#[test]
fn tui_open_mode_editor_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.open]
mode = "editor"
editor_cmd = "hx"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.open.mode, TuiOpenMode::Editor);
  assert_eq!(cfg.tui.open.editor_cmd.as_deref(), Some("hx"));
}

#[test]
fn tui_open_mode_finder_preserves_legacy_behaviour() {
  // Users who liked the pre-#73 default (`open` / `xdg-open` / `explorer`)
  // can opt back in. The variant is named `Finder` after the dominant macOS
  // use case but covers all three OS openers.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.open]
mode = "finder"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.open.mode, TuiOpenMode::Finder);
}

#[test]
fn tui_open_mode_shell_with_custom_cmd_round_trips() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.open]
mode = "shell"
shell_cmd = "/usr/bin/fish"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.open.mode, TuiOpenMode::Shell);
  assert_eq!(cfg.tui.open.shell_cmd.as_deref(), Some("/usr/bin/fish"));
}

#[test]
fn tui_open_mode_invalid_value_errors_at_parse_time() {
  // Unknown enum variants are a hard config error — we can't silently fall
  // back to a default without confusing the user about which mode is active.
  // Document the supported set in the example file; bad values surface here.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.open]
mode = "neovim"
"#,
  )
  .unwrap();
  assert!(Config::load_layered(dir.path(), None).is_err());
}

// ---- [[branch_types]] (issue #80) ------------------------------------------

#[test]
fn branch_types_absent_falls_back_to_built_in_defaults() {
  // The zero-friction contract: a repo without `[[branch_types]]` in
  // its `.gwm.toml` (or with no config at all) keeps the historical
  // built-in list. The source must report `Default` so `gwm types` can
  // surface a truthful footer.
  let cfg = Config::default();
  let resolved = cfg.resolved_branch_types();
  assert_eq!(resolved.source, BranchTypesSource::Default);
  assert!(
    resolved.types.iter().any(|t| t.name == "feat"),
    "default list must include the legacy `feat` entry"
  );
  assert!(
    resolved.types.iter().any(|t| t.name == "hotfix"),
    "default list must include `hotfix` (regression guard against partial defaults)"
  );
}

#[test]
fn branch_types_parsed_from_toml_replaces_defaults() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[branch_types]]
name = "feat"
description = "New feature implementation"

[[branch_types]]
name = "fix"
description = "Bug fix"

[[branch_types]]
name = "migration"
description = "Database migration"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let resolved = cfg.resolved_branch_types();
  assert_eq!(resolved.source, BranchTypesSource::Config);
  let names: Vec<_> = resolved.types.iter().map(|t| t.name.as_str()).collect();
  assert_eq!(names, vec!["feat", "fix", "migration"]);
  // Built-in entries that aren't in the config are dropped — the user's
  // list is authoritative once they opt in.
  assert!(!names.contains(&"hotfix"));
  assert!(!names.contains(&"chore"));
}

#[test]
fn branch_types_empty_block_treated_as_absent() {
  // An empty TOML array (`branch_types = []`) is observationally
  // identical to omitting the section — neither should replace the
  // built-in defaults with an empty list (which would lock the user
  // out of `gwm create`).
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
branch_types = []
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let resolved = cfg.resolved_branch_types();
  assert_eq!(resolved.source, BranchTypesSource::Default);
  assert!(!resolved.types.is_empty());
}

#[test]
fn branch_types_source_label_is_user_facing() {
  // Footer strings rendered by `gwm types` — pin them so a label tweak
  // shows up in code review instead of slipping through.
  assert_eq!(BranchTypesSource::Default.label(), "built-in defaults");
  assert_eq!(BranchTypesSource::Config.label(), ".gwm.toml");
}

#[test]
fn branch_types_empty_name_is_rejected_at_load() {
  // An empty `name = ""` would silently produce a worktree on a branch
  // called `/#123-foo` (no prefix) which git rejects with a cryptic
  // error several layers down. Surface it as a config error early.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[branch_types]]
name = ""
description = "Whoops"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("branch_types"), "{msg}");
  assert!(msg.contains("empty"), "{msg}");
}

#[test]
fn branch_types_invalid_name_format_is_rejected_at_load() {
  // `parse_branch` (the reverse mapping used by `gwm switch`, the TUI
  // list, and every helper that recovers a `BranchSpec` from a free-
  // form branch name) requires `^[a-z]+/#…$`. Anything that doesn't
  // match the type segment regex must be rejected at load so the user
  // sees one config-time error instead of a tangle of runtime
  // failures across surfaces.
  for bad in ["Feat", "feat-1", "wip task", "fix!", "1fix", ""].iter() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
      dir.path().join(CONFIG_FILE),
      format!(
        r#"
[[branch_types]]
name = "{}"
description = "x"
"#,
        bad
      ),
    )
    .unwrap();
    assert!(
      Config::load_layered(dir.path(), None).is_err(),
      "name = {:?} must be rejected at load",
      bad
    );
  }
}

#[test]
fn branch_types_duplicate_name_is_rejected_at_load() {
  // Two entries with the same name would non-deterministically
  // override each other downstream (and the `gwm types` listing would
  // confusingly print the same row twice). Fail fast at load.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[branch_types]]
name = "feat"
description = "Feature"

[[branch_types]]
name = "feat"
description = "Different description for the same name"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("duplicate"), "{msg}");
  assert!(msg.contains("feat"), "{msg}");
}

#[test]
fn branch_types_valid_names_load_successfully() {
  // Sanity: the validator must accept the canonical built-in names
  // and a few realistic custom additions, otherwise it's too tight
  // and the loosening of the rule would have to happen in a follow-up.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[branch_types]]
name = "feat"
description = "Feature"

[[branch_types]]
name = "migration"
description = "Database migration"

[[branch_types]]
name = "wip"
description = "Work in progress"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).expect("valid config must load");
  let names: Vec<_> = cfg.branch_types.iter().map(|t| t.name.as_str()).collect();
  assert_eq!(names, vec!["feat", "migration", "wip"]);
}

// --------------------------------------------------------------------------
// Issue #94 — bootstrap path-traversal closure: load-time validation.
// --------------------------------------------------------------------------
//
// `CopyStep.to`, `Guard.example_file`, and `FallbackContent.target` are
// joined onto `ctx.worktree` / `ctx.main_repo` and passed to `fs::copy` /
// `write_no_follow`. A `..` segment or an absolute path slips past `join`
// and lands outside the worktree tree — write-anywhere primitive on
// `step.to`, read-anywhere primitive on `example_file`. Reject both at
// config-load so the violation surfaces with the TOML key in the error
// rather than mid-bootstrap.

#[test]
fn load_rejects_traversal_in_copy_to() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.copy]]
from = "Cargo.toml"
to   = "../../OWNED"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("traversal must be rejected at load");
  let msg = format!("{}", err);
  assert!(
    msg.contains("bootstrap.copy") && msg.contains("to"),
    "error must name the offending field, got: {}",
    msg
  );
  assert!(
    msg.contains("..") || msg.contains("traversal") || msg.contains("outside"),
    "error must explain WHY (../traversal/outside), got: {}",
    msg
  );
}

#[test]
fn load_rejects_absolute_path_in_copy_to() {
  let dir = TempDir::new().unwrap();
  let absolute_path = toml_absolute_path("/etc/passwd", r#"C:\\Windows\\win.ini"#);
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    format!(
      r#"
[[bootstrap.copy]]
from = ".env"
to   = "{absolute_path}"
"#,
    ),
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("absolute path must be rejected at load");
  let msg = format!("{}", err);
  assert!(
    msg.contains("bootstrap.copy") && msg.contains("to"),
    "error must name the offending field, got: {}",
    msg
  );
  assert!(
    msg.contains("absolute") || msg.contains(absolute_path),
    "error must explain absolute path rejection, got: {}",
    msg
  );
}

#[test]
fn load_rejects_traversal_in_guard_example_file() {
  // The example_file is joined onto ctx.main_repo and read to seed the
  // worktree dst when a guard trips. `../sensitive` reads files outside
  // the main repo — info-leak primitive — and must be rejected.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.guard]]
name           = "leaky"
deny_patterns  = ["amazonaws"]
on_match       = "seed-from-example"
example_file   = "../../../etc/passwd"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("traversal in example_file must be rejected");
  let msg = format!("{}", err);
  assert!(
    msg.contains("guard") && msg.contains("example_file"),
    "error must name the offending field, got: {}",
    msg
  );
  assert!(
    msg.contains("..") || msg.contains("traversal") || msg.contains("outside"),
    "error must explain WHY, got: {}",
    msg
  );
}

#[test]
fn load_rejects_absolute_path_in_guard_example_file() {
  let dir = TempDir::new().unwrap();
  let absolute_path = toml_absolute_path("/etc/shadow", r#"C:\\Windows\\System32\\config\\SAM"#);
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    format!(
      r#"
[[bootstrap.guard]]
name           = "leaky"
deny_patterns  = ["amazonaws"]
on_match       = "seed-from-example"
example_file   = "{absolute_path}"
"#,
    ),
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("absolute example_file must be rejected");
  let msg = format!("{}", err);
  assert!(
    msg.contains("guard") && msg.contains("example_file"),
    "error must name the offending field, got: {}",
    msg
  );
  assert!(
    msg.contains("absolute") || msg.contains(absolute_path),
    "error must explain absolute path rejection, got: {}",
    msg
  );
}

#[test]
fn load_rejects_traversal_in_fallback_target() {
  // FallbackContent.target is declarative today (the runtime uses the
  // joined dst from CopyStep.to instead), but a `..` segment there
  // still misrepresents intent and is rejected for consistency with
  // the other two fields.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[bootstrap.fallback.env_testing]
target  = "../../OWNED"
content = "FOO=bar"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("traversal in fallback.target must be rejected");
  let msg = format!("{}", err);
  assert!(
    msg.contains("fallback") && msg.contains("target"),
    "error must name the offending field, got: {}",
    msg
  );
}

// Issue #94 hardening surfaced by Copilot on PR #111: on Windows,
// drive-relative inputs like `C:foo` are NOT absolute per
// `Path::is_absolute()` but contain a `Component::Prefix` segment
// that makes `PathBuf::join` discard the worktree base. Reject them
// at load time. The test is gated to Windows because `Path::new` on
// Unix never synthesises a `Prefix` component from such inputs.
#[cfg(windows)]
#[test]
fn load_rejects_windows_drive_prefix_in_copy_to() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.copy]]
from = ".env"
to   = "C:foo"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("drive-prefixed path must be rejected at load");
  let msg = format!("{}", err);
  assert!(
    msg.contains("bootstrap.copy") && msg.contains("to"),
    "error must name the offending field, got: {}",
    msg
  );
  assert!(
    msg.contains("drive") || msg.contains("prefix") || msg.contains("C:foo"),
    "error must explain Windows drive prefix rejection, got: {}",
    msg
  );
}

#[test]
fn load_accepts_benign_relative_paths_in_bootstrap_fields() {
  // Positive control: a fully relative path with no `..` and no
  // leading slash must continue to load. This is the canonical shape
  // documented in `examples/gwm.toml.example`.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.copy]]
from = ".env"
to   = "config/local.env"

[[bootstrap.guard]]
name          = "no-aws"
deny_patterns = ["amazonaws\\.com"]
on_match      = "seed-from-example"
example_file  = "config/local.env.example"

[bootstrap.fallback.env_testing]
target  = "config/local.env"
content = "X=1"
"#,
  )
  .unwrap();
  Config::load_layered(dir.path(), None).expect("benign relative paths must load");
}

// --- Issue #96: guard deny_patterns must compile at load time ---------------
//
// Historically `bootstrap.rs::guard_match` wrapped `Regex::new(pat)` in
// `if let Ok(re) = …`, silently dropping invalid patterns. A guard whose
// only deny pattern failed to compile became fail-open: the file copied
// through as if no rule existed. The contract of `[[bootstrap.guard]]`
// is a refusal mechanism — a silently broken refusal is strictly worse
// than no refusal, because the user believes they are protected. Reject
// invalid patterns at `Config::load_for_repo` so bootstrap never runs
// against a partially broken guard.

#[test]
fn load_rejects_invalid_deny_pattern_in_guard() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.guard]]
name          = "no-secrets"
deny_patterns = ["[+", "AWS_SECRET_ACCESS_KEY"]
on_match      = "abort"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("invalid deny_patterns must be rejected at load");
  let msg = format!("{}", err);
  assert!(
    msg.contains("no-secrets"),
    "error must name the offending guard, got: {}",
    msg
  );
  assert!(
    msg.contains("[+"),
    "error must quote the offending pattern, got: {}",
    msg
  );
  assert!(
    msg.contains("deny_pattern") || msg.contains("regex"),
    "error must explain WHY (regex/deny_pattern), got: {}",
    msg
  );
}

#[test]
fn load_rejects_invalid_deny_pattern_when_only_pattern_in_guard() {
  // Even when the *only* pattern is invalid, the guard must fail at
  // load — never silently degrade into a "guard with no patterns",
  // which would be fail-open under any input.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.guard]]
name          = "broken"
deny_patterns = ["*foo"]
on_match      = "abort"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("invalid sole deny pattern must be rejected at load");
  let msg = format!("{}", err);
  assert!(
    msg.contains("broken") && msg.contains("*foo"),
    "error must name guard + pattern, got: {}",
    msg
  );
}

#[test]
fn load_accepts_valid_deny_patterns() {
  // Positive control: every well-formed pattern continues to load.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.guard]]
name          = "no-aws"
deny_patterns = ["amazonaws\\.com", "AKIA[0-9A-Z]{16}", "(?i)aws_secret"]
on_match      = "abort"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).expect("valid patterns must load");
  assert_eq!(cfg.bootstrap.guard.len(), 1);
  assert_eq!(cfg.bootstrap.guard[0].deny_patterns.len(), 3);
}

#[test]
fn validate_bootstrap_guards_directly_rejects_invalid_pattern_without_load_for_repo() {
  // Direct unit test on the helper, called against a `Config` value
  // built in code without ever touching `Config::load_for_repo`.
  // Locks in the helper's contract independently of the loader, so a
  // future refactor that removes the call site from `load_for_repo`
  // doesn't go unnoticed by only the integration suite.
  use gwm::config::{BootstrapConfig, Guard};
  let mut cfg = Config {
    bootstrap: BootstrapConfig {
      guard: vec![Guard {
        name: "direct-test".into(),
        deny_patterns: vec!["(unclosed-group".into()],
        on_match: "abort".into(),
        example_file: None,
      }],
      ..Default::default()
    },
    ..Default::default()
  };
  let err = cfg
    .validate_bootstrap_guards()
    .expect_err("direct call on hand-built Config with invalid pattern must Err");
  let msg = format!("{}", err);
  assert!(
    msg.contains("direct-test") && msg.contains("(unclosed-group"),
    "error must name guard + pattern, got: {}",
    msg
  );

  // Positive control: clear the bad pattern and the same helper must
  // accept the Config, even with a non-trivial pattern set.
  cfg.bootstrap.guard[0].deny_patterns = vec!["AKIA[0-9A-Z]{16}".into(), "(?i)secret".into()];
  cfg
    .validate_bootstrap_guards()
    .expect("valid patterns must pass direct validation");
}

#[test]
fn load_rejects_invalid_deny_pattern_in_second_guard() {
  // The validator must walk every guard, not just the first one. A
  // bad pattern in guard #2 must still surface at load.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.guard]]
name          = "guard-one"
deny_patterns = ["amazonaws\\.com"]
on_match      = "abort"

[[bootstrap.guard]]
name          = "guard-two"
deny_patterns = ["[unclosed"]
on_match      = "abort"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("invalid pattern in second guard must be rejected");
  let msg = format!("{}", err);
  assert!(
    msg.contains("guard-two") && msg.contains("[unclosed"),
    "error must name the offending guard + pattern, got: {}",
    msg
  );
  // The error must also surface the TOML coordinates of the failing
  // entry so the user can jump straight to `.gwm.toml` line without
  // grepping for the pattern content (mirrors `bootstrap.copy[N].to`).
  assert!(
    msg.contains("bootstrap.guard[1]") && msg.contains("deny_patterns[0]"),
    "error must locate the failing entry by index, got: {}",
    msg
  );
}

// --- PR template section (issue #84) ------------------------------------

#[test]
fn pr_template_defaults_are_empty() {
  // Without a [pr_template] block, both `default` and `by_type`
  // resolve to empty so `gwm pr` falls back to the GitHub-side
  // PULL_REQUEST_TEMPLATE.md (same as before #84).
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(cfg.pr_template.default.is_none());
  assert!(cfg.pr_template.by_type.is_empty());
}

#[test]
fn pr_template_section_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r###"
[pr_template]
default = ".github/pull_request_template.md"

[pr_template.by_type]
feat = { path = ".github/pr-templates/feat.md" }
fix = { path = ".github/pr-templates/fix.md" }

[pr_template.by_type.chore]
body = "## Summary\n{desc}\n\nCloses #{issue}\n"
"###,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(
    cfg.pr_template.default.as_deref(),
    Some(".github/pull_request_template.md")
  );

  let feat = cfg.pr_template.by_type.get("feat").expect("feat entry");
  assert_eq!(feat.path.as_deref(), Some(".github/pr-templates/feat.md"));
  assert_eq!(feat.body, None);

  let chore = cfg.pr_template.by_type.get("chore").expect("chore entry");
  assert_eq!(chore.path, None);
  assert_eq!(chore.body.as_deref(), Some("## Summary\n{desc}\n\nCloses #{issue}\n"));
}

#[test]
fn pr_template_unknown_root_field_is_rejected() {
  // `[pr_template]` mirrors `[issue_template]`'s `deny_unknown_fields`
  // contract so typos surface at load time, not as a silent no-op.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[pr_template]
default = ".github/pull_request_template.md"
mystery = "boom"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("unknown field must reject");
  let msg = format!("{}", err);
  assert!(msg.contains("mystery"), "{msg}");
}

#[test]
fn pr_template_unknown_per_type_field_is_rejected() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[pr_template.by_type.feat]
path = ".github/pr-templates/feat.md"
bogus = true
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("unknown per-type field must reject");
  let msg = format!("{}", err);
  assert!(msg.contains("bogus"), "{msg}");
}

// --- [tui.keys] section (issue #87) -------------------------------------

#[test]
fn tui_keys_default_is_empty_map() {
  // Absent `[tui.keys]` block resolves to an empty map — the keymap
  // layer then keeps every built-in default. Mirrors the `labels` /
  // `milestones` "no override declared" contract.
  let cfg = Config::default();
  assert!(cfg.tui.keys.raw.is_empty());
}

#[test]
fn tui_keys_section_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys]
down = ["j", "Ctrl+n"]
up   = ["k", "Ctrl+p"]
top  = ["g g"]
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  // The raw `[tui.keys]` table preserves the user's source verbatim
  // (issue #219 stores it as a `toml::Table` to also carry contextual
  // sub-tables); pull each chord list back out as strings.
  let chords = |slug: &str| -> Vec<String> {
    cfg
      .tui
      .keys
      .raw
      .get(slug)
      .and_then(|v| v.as_array())
      .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
      .unwrap_or_default()
  };
  assert_eq!(chords("down"), vec!["j".to_string(), "Ctrl+n".to_string()]);
  assert_eq!(chords("up"), vec!["k".to_string(), "Ctrl+p".to_string()]);
  assert_eq!(chords("top"), vec!["g g".to_string()]);
}

#[test]
fn tui_keys_rejects_unknown_action_at_load_time() {
  // `gallop` is not in `keymap::ACTIONS`. The user almost certainly
  // typo'd a real action name; surfacing the error here saves a
  // mystifying "my binding does nothing in the TUI" round trip.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys]
gallop = ["g"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("unknown action must reject");
  let msg = format!("{}", err).to_lowercase();
  assert!(
    msg.contains("gallop"),
    "expected message to name the bad action, got: {msg}"
  );
  assert!(
    msg.contains("unknown"),
    "expected message to flag it as unknown, got: {msg}"
  );
}

#[test]
fn tui_keys_rejects_invalid_key_string() {
  // `Foobar` is not a named key and is not a single character. The
  // parser surfaces it as a config error.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys]
down = ["Foobar"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("invalid key string must reject");
  let msg = format!("{}", err).to_lowercase();
  assert!(
    msg.contains("foobar"),
    "expected message to name the bad key, got: {msg}"
  );
}

#[test]
fn tui_keys_rejects_chord_that_is_strict_prefix() {
  // Binding `g` alone while the default `g g` is still in place
  // creates a chord/prefix ambiguity. Per the design note on PR #87
  // this is a hard error at load — never a runtime timeout.
  let dir = TempDir::new().unwrap();
  // `terminal_fullscreen` replaces the old `open` slug (#290).
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys]
terminal_fullscreen = ["g"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("prefix collision must reject");
  let msg = format!("{}", err).to_lowercase();
  assert!(msg.contains("prefix"), "expected prefix error, got: {msg}");
}

#[test]
fn tui_keys_rejects_chord_conflict_across_actions() {
  // `down` and `up` both rebound to `x` — same chord, two actions,
  // ambiguous dispatch. Refuse at load time.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys]
down = ["x"]
up   = ["x"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("conflict must reject");
  let msg = format!("{}", err).to_lowercase();
  assert!(msg.contains("conflict"), "expected conflict error, got: {msg}");
}

#[test]
fn tui_keys_empty_binding_list_unbinds_action() {
  // `down = []` removes every binding for the `down` action. Useful
  // for users who prefer to navigate with the arrow keys only.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys]
down = []
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let km = cfg.tui.keys.resolved_keymap().unwrap();
  use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
  let j = KeyStroke::parse_chord("j").unwrap();
  assert!(matches!(km.lookup(&j), ChordResolution::NoMatch));
  // The other defaults survive.
  let k = KeyStroke::parse_chord("k").unwrap();
  assert!(matches!(km.lookup(&k), ChordResolution::Matched(Action::Up)));
}

// --- [theme] section (issue #33) ----------------------------------------

#[test]
fn theme_default_is_pre_issue_33_scheme() {
  // No `[theme]` block → the resolved theme matches the pre-#33
  // hardcoded scheme verbatim. Pinned at the config layer so a
  // regression in the loader is caught here rather than as a
  // surprise palette in someone's screenshot.
  use ratatui::style::Color;
  let cfg = Config::default();
  let theme = cfg.theme.resolve().unwrap();
  assert_eq!(theme.focus, Color::Cyan);
  assert_eq!(theme.branch, Color::Green);
}

#[test]
fn theme_preset_is_applied_at_load() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[theme]
preset = "catppuccin"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let theme = cfg.theme.resolve().unwrap();
  // Catppuccin's focus role differs from the default `Cyan` —
  // assert the difference rather than the specific hex to keep
  // the test resilient to upstream palette tweaks.
  use gwm::tui::theme::Theme;
  let default = Theme::default();
  assert_ne!(
    theme.focus, default.focus,
    "preset must override the default focus colour"
  );
}

#[test]
fn theme_per_role_overrides_win_over_preset() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[theme]
preset = "catppuccin"
focus  = "red"
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let theme = cfg.theme.resolve().unwrap();
  use ratatui::style::Color;
  assert_eq!(theme.focus, Color::Red, "explicit override must win over the preset");
}

#[test]
fn theme_rejects_unknown_preset() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[theme]
preset = "does-not-exist"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("unknown preset must reject");
  let msg = format!("{}", err).to_lowercase();
  assert!(msg.contains("does-not-exist"), "got: {msg}");
}

#[test]
fn theme_rejects_unknown_role() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[theme]
phantom = "red"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("unknown role must reject");
  let msg = format!("{}", err).to_lowercase();
  assert!(msg.contains("phantom"), "got: {msg}");
}

#[test]
fn theme_rejects_bad_color_value() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[theme]
focus = "not_a_color"
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("invalid color must reject");
  let msg = format!("{}", err).to_lowercase();
  assert!(msg.contains("not_a_color"), "got: {msg}");
}

// --- Resolved config rows + source attribution (issue #232) -------------

fn row_for<'a>(rows: &'a [ConfigRow], key: &str) -> &'a ConfigRow {
  rows
    .iter()
    .find(|r| r.key == key)
    .unwrap_or_else(|| panic!("key {key:?} missing from resolved rows"))
}

#[test]
fn resolved_rows_attributes_each_key_to_its_winning_layer() {
  let repo = TempDir::new().unwrap();
  let global_dir = TempDir::new().unwrap();
  let global_path = global_dir.path().join("config.toml");

  // User (global) layer sets worktree.base AND tui.confirm_countdown_secs.
  std::fs::write(
    &global_path,
    "[worktree]\nbase = \"/tmp/global-wt\"\n[tui]\nconfirm_countdown_secs = 7\n",
  )
  .unwrap();
  // Repo layer overrides worktree.base only.
  std::fs::write(repo.path().join(CONFIG_FILE), "[worktree]\nbase = \"/tmp/repo-wt\"\n").unwrap();

  let rows = resolved_rows(repo.path(), Some(&global_path)).unwrap();

  // Repo wins on a key set in both layers, and the resolved value is the
  // repo's — identical formatting to `gwm config list`.
  let base = row_for(&rows, "worktree.base");
  assert_eq!(base.source, ConfigSource::Repo);
  assert_eq!(base.value, "\"/tmp/repo-wt\"");

  // The user layer provides a key the repo never mentions.
  let countdown = row_for(&rows, "tui.confirm_countdown_secs");
  assert_eq!(countdown.source, ConfigSource::User);
  assert_eq!(countdown.value, "7");

  // A key set in neither layer falls back to the built-in default.
  assert_eq!(row_for(&rows, "worktree.path_pattern").source, ConfigSource::Default);
}

#[test]
fn resolved_rows_with_no_files_marks_everything_default() {
  let repo = TempDir::new().unwrap();
  let rows = resolved_rows(repo.path(), None).unwrap();
  assert!(!rows.is_empty(), "defaults still produce rows");
  assert!(
    rows.iter().all(|r| r.source == ConfigSource::Default),
    "with no repo/global config every row is a built-in default"
  );
}

#[test]
fn resolved_rows_attributes_array_of_tables_entries() {
  // The common real-world shape is an array-of-tables (`[[labels]]`,
  // `[[branch_types]]`), where the merged value goes through serde's struct
  // serialisation and the raw layer through a bare file parse. This pins
  // that those key shapes (`labels[0].name`, …) match so a repo-declared
  // entry attributes to Repo rather than silently reading as a default.
  let repo = TempDir::new().unwrap();
  std::fs::write(
    repo.path().join(CONFIG_FILE),
    "[[labels]]\nname = \"bug\"\ncolor = \"d73a4a\"\n\n[[branch_types]]\nname = \"feat\"\ndescription = \"a feature\"\n",
  )
  .unwrap();

  let rows = resolved_rows(repo.path(), None).unwrap();

  assert_eq!(row_for(&rows, "labels[0].name").source, ConfigSource::Repo);
  assert_eq!(row_for(&rows, "labels[0].name").value, "\"bug\"");
  assert_eq!(row_for(&rows, "labels[0].color").source, ConfigSource::Repo);
  assert_eq!(row_for(&rows, "branch_types[0].name").source, ConfigSource::Repo);

  // An unset optional sub-field (`labels[0].description`) is omitted by the
  // TOML serialiser (Option::None), so it produces no phantom default row.
  assert!(
    !rows.iter().any(|r| r.key == "labels[0].description"),
    "unset optional sub-fields must not surface as default rows"
  );
}

#[test]
fn config_source_labels_are_stable() {
  assert_eq!(ConfigSource::Repo.label(), "repo");
  assert_eq!(ConfigSource::User.label(), "user");
  assert_eq!(ConfigSource::Default.label(), "default");
}

#[test]
fn macro_open_in_accepts_documented_pty_and_mux_pane_values() {
  // #290 / Codex review on PR #292: `MacroOpenMode` must deserialize the
  // documented `open_in = "mux_pane"` (snake_case) — under the earlier
  // `rename_all = "lowercase"` serde expected "muxpane" and a config
  // following the docs failed to load, taking the whole TUI down.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.macro1]
command = "make test"
open_in = "mux_pane"

[tui.macro2]
command = "codex"
open_in = "pty"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let m1 = cfg.tui.macro1.expect("macro1 must parse");
  assert_eq!(m1.command, "make test");
  assert_eq!(m1.open_in, MacroOpenMode::MuxPane, "\"mux_pane\" must map to MuxPane");
  let m2 = cfg.tui.macro2.expect("macro2 must parse");
  assert_eq!(m2.open_in, MacroOpenMode::Pty, "\"pty\" must map to Pty");
}

#[test]
fn macro_open_in_defaults_to_pty_when_omitted() {
  // `open_in` is optional and defaults to Pty (the in-overlay mode).
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.macro1]
command = "lazygit"
"#,
  )
  .unwrap();

  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let m1 = cfg.tui.macro1.expect("macro1 must parse");
  assert_eq!(m1.open_in, MacroOpenMode::Pty);
}

// --- [tui.keys.modal.<context>] contextual modal bindings (issue #219) ---

use crossterm::event::{KeyCode, KeyModifiers};
use gwm::tui::keymap::KeyStroke;
use gwm::tui::modal_keymap::{KeyContext, ModalAction};

fn ks(code: KeyCode) -> KeyStroke {
  KeyStroke::new(code, KeyModifiers::empty())
}
fn kc(c: char) -> KeyStroke {
  KeyStroke::new(KeyCode::Char(c), KeyModifiers::empty())
}

#[test]
fn modal_keys_nested_context_resolves() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.confirm]
confirm = ["o"]
cancel  = ["n", "Esc"]
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let mk = cfg.tui.keys.resolved_modal_keymap().unwrap();
  // new key fires, old default `y` is gone
  assert_eq!(
    mk.resolve(KeyContext::Confirm, &kc('o')),
    Some(ModalAction::ConfirmConfirm)
  );
  assert_eq!(mk.resolve(KeyContext::Confirm, &kc('y')), None);
}

#[test]
fn modal_keys_link_stage_uses_dotted_table() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.link.choose_target]
issue = ["x"]

[tui.keys.modal.link.input_number]
submit = ["Right"]
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let mk = cfg.tui.keys.resolved_modal_keymap().unwrap();
  assert_eq!(
    mk.resolve(KeyContext::LinkChooseTarget, &kc('x')),
    Some(ModalAction::LinkChooseIssue)
  );
  assert_eq!(
    mk.resolve(KeyContext::LinkInputNumber, &ks(KeyCode::Right)),
    Some(ModalAction::LinkInputSubmit)
  );
}

#[test]
fn modal_keys_config_edit_substage_resolves() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.config]
close = ["q"]

[tui.keys.modal.config.edit]
cancel = ["Tab"]
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  let mk = cfg.tui.keys.resolved_modal_keymap().unwrap();
  assert_eq!(mk.resolve(KeyContext::Config, &kc('q')), Some(ModalAction::ConfigClose));
  assert_eq!(
    mk.resolve(KeyContext::ConfigEdit, &ks(KeyCode::Tab)),
    Some(ModalAction::ConfigEditCancel)
  );
}

#[test]
fn modal_keys_global_and_contextual_coexist() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys]
quit = ["Q"]

[tui.keys.modal.confirm]
confirm = ["o"]
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  // global path sees the array entry
  let km = cfg.tui.keys.resolved_keymap().unwrap();
  assert_eq!(km.primary_chord(gwm::tui::keymap::Action::Quit).as_deref(), Some("Q"));
  // contextual path sees the table entry
  let mk = cfg.tui.keys.resolved_modal_keymap().unwrap();
  assert_eq!(
    mk.resolve(KeyContext::Confirm, &kc('o')),
    Some(ModalAction::ConfirmConfirm)
  );
}

#[test]
fn modal_namespace_does_not_collide_with_a_same_named_global_action() {
  // #219 review (P2): before the dedicated `[tui.keys.modal]` namespace, a
  // global `[tui.keys] create = ["c"]` and a modal `[tui.keys.create]` shared
  // the `tui.keys.create` path. The layered merge saw array-vs-table at the
  // same path, replaced the global array with the modal table, and silently
  // dropped the user's global override (`resolved_keymap` then skipped the
  // table). With the disjoint namespace the two live at different paths
  // (`tui.keys.create` vs `tui.keys.modal.create`) and both survive the merge.
  let global = TempDir::new().unwrap();
  let global_path = global.path().join("global.toml");
  std::fs::write(&global_path, "[tui.keys]\ncreate = [\"c\"]\n").unwrap();
  let repo = TempDir::new().unwrap();
  // `next_type` is the create verb bindable on a bare letter (it lives on
  // the typing-free Type field — reserved-typing rejection, #456 it. 14).
  std::fs::write(
    repo.path().join(CONFIG_FILE),
    "[tui.keys.modal.create]\nnext_type = [\"x\"]\n",
  )
  .unwrap();

  let cfg = Config::load_layered(repo.path(), Some(&global_path)).unwrap();

  // The global `create` override survives the merge (was silently lost before).
  let km = cfg.tui.keys.resolved_keymap().unwrap();
  assert_eq!(
    km.primary_chord(gwm::tui::keymap::Action::Create).as_deref(),
    Some("c"),
    "the global create override must survive a same-named modal context"
  );

  // ...and the modal `create.next_type` override resolves independently.
  let mk = cfg.tui.keys.resolved_modal_keymap().unwrap();
  assert_eq!(
    mk.resolve(KeyContext::Create, &kc('x')),
    Some(ModalAction::CreateNextType)
  );
}

#[test]
fn modal_keys_reject_unknown_context() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.confrm]
confirm = ["y"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("typo'd context must reject");
  assert!(err.to_string().contains("unknown modal context"), "{err}");
}

#[test]
fn modal_keys_reject_unknown_verb() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.confirm]
gallop = ["y"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("unknown verb must reject");
  assert!(err.to_string().contains("unknown verb"), "{err}");
}

#[test]
fn modal_keys_reject_multistroke_chord() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.confirm]
confirm = ["g g"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("modal chords must be single strokes");
  assert!(err.to_string().contains("single keystroke"), "{err}");
}

#[test]
fn modal_keys_reject_in_context_conflict() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.confirm]
confirm = ["x"]
cancel  = ["x"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("two verbs sharing a key must conflict");
  assert!(err.to_string().contains("conflict"), "{err}");
}

#[test]
fn modal_keys_reject_array_directly_under_group() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.keys.modal.link]
issue = ["i"]
"#,
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("link needs a stage");
  assert!(err.to_string().contains("context group"), "{err}");
}

// --- [exec] / [clean] profiles (issue #324) ---------------------------------

fn load_toml(body: &str) -> gwm::error::Result<Config> {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), body).unwrap();
  Config::load_layered(dir.path(), None)
}

#[test]
fn exec_and_clean_default_to_no_profiles() {
  // Absent `[exec]` / `[clean]` blocks resolve to empty profile maps — the
  // inline `gwm exec -- <cmd>` and built-in `gwm clean` surfaces are unchanged.
  let cfg = Config::default();
  assert!(cfg.exec.profiles.is_empty());
  assert!(cfg.clean.profiles.is_empty());
}

#[test]
fn exec_profiles_parse_command_as_an_argv_array() {
  let cfg = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]

[exec.profiles.fmt]
command = ["cargo", "fmt", "--all"]
"#,
  )
  .expect("exec profiles parse");
  assert_eq!(
    cfg.exec.profiles["test"].command,
    vec!["cargo".to_string(), "test".to_string()]
  );
  assert_eq!(
    cfg.exec.profiles["fmt"].command,
    vec!["cargo".to_string(), "fmt".to_string(), "--all".to_string()]
  );
}

#[test]
fn exec_jobs_parse_global_and_per_profile() {
  let cfg = load_toml(
    r#"
[exec]
jobs = 4

[exec.profiles.fmt]
command = ["cargo", "fmt"]
jobs = 2
"#,
  )
  .expect("jobs parse");
  assert_eq!(cfg.exec.jobs, Some(4), "global [exec] jobs");
  assert_eq!(cfg.exec.profiles["fmt"].jobs, Some(2), "per-profile jobs");
}

#[test]
fn exec_jobs_default_honors_global_even_without_a_repo() {
  // A bare repo passes `repo = None`, but the GLOBAL `[exec] jobs` still
  // applies (#324 review). A repo `[exec] jobs` overrides the global.
  let global_dir = TempDir::new().unwrap();
  let global = global_dir.path().join("config.toml");
  std::fs::write(&global, "[exec]\njobs = 4\n").unwrap();

  // repo = None (bare): only the global is read.
  assert_eq!(
    Config::load_exec_jobs_default_layered(Some(&global), None).unwrap(),
    Some(4),
    "bare repo still honours the global [exec] jobs"
  );

  // A repo `.gwm.toml` overrides the global default.
  let repo_dir = TempDir::new().unwrap();
  std::fs::write(repo_dir.path().join(CONFIG_FILE), "[exec]\njobs = 2\n").unwrap();
  assert_eq!(
    Config::load_exec_jobs_default_layered(Some(&global), Some(repo_dir.path())).unwrap(),
    Some(2),
    "repo [exec] jobs overrides the global"
  );

  // No global, no repo ⇒ None (sequential).
  assert_eq!(Config::load_exec_jobs_default_layered(None, None).unwrap(), None);
}

#[test]
fn exec_jobs_default_to_none() {
  let cfg = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]
"#,
  )
  .expect("parse");
  assert!(cfg.exec.jobs.is_none(), "no [exec] jobs ⇒ None (sequential)");
  assert!(cfg.exec.profiles["test"].jobs.is_none());
}

#[test]
fn clean_profiles_parse_dirs_as_a_complete_set() {
  let cfg = load_toml(
    r#"
[clean.profiles.default]
dirs = ["target", "node_modules", "dist", "build", "coverage", ".turbo"]

[clean.profiles.deep]
dirs = ["target", ".cache", ".venv"]
"#,
  )
  .expect("clean profiles parse");
  assert_eq!(
    cfg.clean.profiles["default"].dirs,
    vec!["target", "node_modules", "dist", "build", "coverage", ".turbo"]
  );
  assert_eq!(cfg.clean.profiles["deep"].dirs, vec!["target", ".cache", ".venv"]);
}

#[test]
fn exec_profile_without_command_is_a_load_error() {
  // `command` is required within a profile — an empty `[exec.profiles.x]`
  // table is a config error, surfaced at load time.
  let err = load_toml(
    r#"
[exec.profiles.broken]
"#,
  )
  .expect_err("missing command must error");
  assert!(
    err.to_string().contains("command"),
    "error should name the missing field: {err}"
  );
}

#[test]
fn clean_profile_without_dirs_is_a_load_error() {
  let err = load_toml(
    r#"
[clean.profiles.broken]
"#,
  )
  .expect_err("missing dirs must error");
  assert!(
    err.to_string().contains("dirs"),
    "error should name the missing field: {err}"
  );
}

#[test]
fn exec_profile_rejects_unknown_fields() {
  // `deny_unknown_fields` guards the frozen schema — a stray key is refused
  // rather than silently ignored.
  let err = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]
nonsense = true
"#,
  )
  .expect_err("unknown field must error");
  assert!(
    err.to_string().contains("nonsense") || err.to_string().contains("unknown"),
    "{err}"
  );
}

#[test]
fn exec_profile_with_an_empty_command_fails_validation() {
  // `command = []` parses (it's a valid array) but is semantically invalid —
  // caught at config-load time so `gwm config validate` rejects what
  // `gwm exec --profile` would.
  let err = load_toml(
    r#"
[exec.profiles.empty]
command = []
"#,
  )
  .expect_err("empty command must fail validation");
  assert!(err.to_string().contains("empty `command`"), "{err}");
}

#[test]
fn exec_profile_parses_a_container_block() {
  // Issue #421. The block rides a profile, so the inline `gwm exec -- <cmd>`
  // surface is untouched; `image` is the only required field.
  let cfg = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]

  [exec.profiles.test.container]
  image = "rust:1.90"
  runtime = "podman"
  extra_args = ["-e", "CI=1"]
"#,
  )
  .expect("container block parses");
  let c = cfg.exec.profiles["test"]
    .container
    .as_ref()
    .expect("container block present");
  assert_eq!(c.image, "rust:1.90");
  assert_eq!(c.runtime.as_deref(), Some("podman"));
  assert_eq!(c.extra_args, vec!["-e", "CI=1"]);
  assert!(
    !c.selinux_relabel,
    "relabelling is off unless asked: it writes to the host"
  );
  assert!(
    cfg.exec.profiles["test"].container.is_some() && cfg.exec.jobs.is_none(),
    "the block is nested under the profile, not a new top-level section"
  );
}

#[test]
fn exec_profile_container_defaults_to_absent() {
  let cfg = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]
"#,
  )
  .expect("profile without a container parses");
  assert!(
    cfg.exec.profiles["test"].container.is_none(),
    "no `[container]` ⇒ the command runs on the host"
  );
}

#[test]
fn exec_container_parses_the_selinux_relabel_flag() {
  let cfg = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]

  [exec.profiles.test.container]
  image = "fedora:41"
  selinux_relabel = true
"#,
  )
  .expect("selinux_relabel parses");
  assert!(cfg.exec.profiles["test"].container.as_ref().unwrap().selinux_relabel);
}

#[test]
fn exec_container_rejects_unknown_fields() {
  // The reference implementation's schema is wider (mounts, env, interactive,
  // working_dir); gwm's is deliberately not, so a copy-pasted key must be
  // refused rather than silently ignored.
  let err = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]

  [exec.profiles.test.container]
  image = "rust:1.90"
  interactive = true
"#,
  )
  .expect_err("unknown field must error");
  assert!(
    err.to_string().contains("interactive") || err.to_string().contains("unknown"),
    "{err}"
  );
}

#[test]
fn exec_container_without_an_image_is_a_load_error() {
  let err = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]

  [exec.profiles.test.container]
  runtime = "docker"
"#,
  )
  .expect_err("missing image must error");
  assert!(
    err.to_string().contains("image"),
    "error names the missing field: {err}"
  );
}

#[test]
fn exec_container_with_an_empty_image_fails_validation() {
  // `image = ""` parses but is semantically invalid — caught at config-load
  // time so `gwm config validate` / `gwm doctor` reject what
  // `gwm exec --profile` would.
  let err = load_toml(
    r#"
[exec.profiles.test]
command = ["cargo", "test"]

  [exec.profiles.test.container]
  image = ""
"#,
  )
  .expect_err("empty image must fail validation");
  assert!(err.to_string().contains("empty `image`"), "{err}");
}

#[test]
fn clean_profile_with_an_escaping_dir_fails_validation() {
  // `dirs = [".."]` parses but escapes the worktree — caught at config-load
  // time, not only later in `gwm clean`.
  let err = load_toml(
    r#"
[clean.profiles.default]
dirs = [".."]
"#,
  )
  .expect_err("escaping dir must fail validation");
  assert!(err.to_string().contains(".."), "{err}");
}

// ---- [tui] clipboard (issue #367) ------------------------------------------

#[test]
fn tui_clipboard_defaults_to_auto() {
  // `auto` is what makes the SSH fix work without configuration; a default of
  // `tools` would leave the reported bug in place for everyone who never edits
  // their config.
  assert_eq!(Config::default().tui.clipboard, ClipboardMode::Auto);
}

#[test]
fn tui_clipboard_absent_keeps_auto() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[tui]\nconfirm_countdown_secs = 2\n").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.clipboard, ClipboardMode::Auto);
}

#[test]
fn tui_clipboard_parses_every_mode() {
  for (text, expected) in [
    ("auto", ClipboardMode::Auto),
    ("osc52", ClipboardMode::Osc52),
    ("tools", ClipboardMode::Tools),
  ] {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(CONFIG_FILE), format!("[tui]\nclipboard = \"{text}\"\n")).unwrap();
    let cfg = Config::load_layered(dir.path(), None).unwrap();
    assert_eq!(cfg.tui.clipboard, expected, "{text:?} parses");
    assert_eq!(
      cfg.tui.clipboard.label(),
      text,
      "label round-trips to the TOML spelling"
    );
  }
}

#[test]
fn tui_clipboard_invalid_value_errors_at_parse_time() {
  // Same hard-error contract as the other [tui] enums — silently falling back
  // would leave the user unsure which clipboard path is live, and this one is
  // already hard to observe (OSC52 never acknowledges).
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[tui]\nclipboard = \"osc-52\"\n").unwrap();
  assert!(Config::load_layered(dir.path(), None).is_err());
}

// --- single-pass substitution (issue #494) --------------------------------

/// `expand_placeholders` chained `str::replace` calls, so each one re-scanned
/// what the previous had just written. `{home}` and `{repo}` go first, which
/// means a repo whose own name contains `{type}` made `{repo}/{desc}` write a
/// type the template never mentions.
///
/// An expansion is a **value**, not more template. Nothing downstream can
/// reason about a pattern by reading it otherwise: `naming::editable_segments`
/// derived the create form's fields from the wrong set, and
/// `BranchParser::compile` had to refuse the whole class rather than mirror a
/// formatter it could not predict.
///
/// Same property `lifecycle::expand` has had since the security patch
/// (`97e4e09`), and for the same reason.
#[test]
fn a_token_produced_by_expanding_the_repo_name_stays_literal() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let out = expand_placeholders(
    "{repo}/{desc}",
    "api-{type}",
    Some("fix"),
    Some("42"),
    Some("foo"),
    None,
  )
  .unwrap();
  assert_eq!(
    out, "api-{type}/foo",
    "the repo name is a value; the `{{type}}` inside it is not a placeholder"
  );

  // Every token, so no single ordering of the replace chain passes by luck.
  for token in ["{type}", "{issue}", "{desc}", "{repo}", "{home}"] {
    let out = expand_placeholders(
      "{repo}-x",
      &format!("api-{token}"),
      Some("fix"),
      Some("42"),
      Some("foo"),
      None,
    )
    .unwrap();
    assert_eq!(
      out,
      format!("api-{token}-x"),
      "`{}` arrived through the repo name, so it is data",
      token
    );
  }
}

/// The same for `{home}`, which is substituted first of all and is the one token
/// whose value the user does not choose directly.
///
/// **Unix only, and the gate is the point rather than a shortcut.** `dirs 6` on
/// Windows resolves the home directory through `SHGetKnownFolderPath` and never
/// reads `$HOME`, so swapping the variable there changes nothing and the
/// assertion fails on every run. Caught by the `windows-latest` job on PR #495,
/// which is the CLAUDE.md rule about pre-validating environment-dependent tests
/// being paid for rather than followed.
///
/// The *property* is covered portably by the test above: `{home}` and `{repo}`
/// are two arms of one `value_for` match, and that one exercises all five tokens
/// arriving through a value. What this adds is the guarantee that `{home}` is
/// not special-cased back into a pre-pass, which is exactly how the defect was
/// shaped, so it is worth keeping on the platform that can express it.
#[cfg(unix)]
#[test]
fn a_token_produced_by_expanding_home_stays_literal() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let previous = std::env::var("HOME").ok();
  unsafe { std::env::set_var("HOME", "/tmp/h-{issue}") };

  let out = expand_placeholders("{home}/wt/{desc}", "r", Some("fix"), Some("42"), Some("foo"), None);

  match previous {
    Some(v) => unsafe { std::env::set_var("HOME", v) },
    None => unsafe { std::env::remove_var("HOME") },
  }

  assert_eq!(out.unwrap(), "/tmp/h-{issue}/wt/foo");
}

/// A token with no value is left **literal**, not emptied.
/// `BranchSpec::branch_name` passes no `repo_path` precisely so the disk-path
/// tokens survive verbatim into the branch name, and `BranchParser::compile`
/// mirrors that. Restated here against the rewritten expander, since the old
/// one got it by not calling `replace` at all.
#[test]
fn a_token_with_no_value_survives_the_single_pass_verbatim() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  assert_eq!(
    expand_placeholders("{repo_parent}/{type}/{issue}/{desc}", "r", None, None, None, None).unwrap(),
    "{repo_parent}/{type}/{issue}/{desc}"
  );
  // And an unknown token is not a token.
  assert_eq!(
    expand_placeholders("{nope}/{repo}", "r", None, None, None, None).unwrap(),
    "{nope}/r"
  );
  // Nor is an unbalanced brace.
  assert_eq!(
    expand_placeholders("{repo}/{unclosed", "r", None, None, None, None).unwrap(),
    "r/{unclosed"
  );
}

/// The asymmetry the old nesting encoded by accident: with `repo_path` set but
/// its parent `None`, `{repo_path}` resolves and `{repo_parent}` does not. A
/// token lookup has to state that, where the nested `if let` fell into it.
#[test]
fn a_repo_path_without_a_parent_resolves_only_the_path_token() {
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let root = std::path::Path::new("/");
  assert!(root.parent().is_none(), "the fixture must actually have no parent");

  let out = expand_placeholders("{repo_path}|{repo_parent}", "r", None, None, None, Some(root)).unwrap();
  assert_eq!(out, "/|{repo_parent}");
}

/// Issue #531: the layered load must be able to work from bytes the caller
/// already read, so a delete can hash, merge and gate one single snapshot of
/// `.gwm.toml` instead of opening it once per question.
#[test]
fn the_layered_config_merges_the_repo_bytes_it_was_handed() {
  let handed = br#"
[worktree]
branch_pattern = "handed/{desc}"
"#;
  let cfg = Config::load_layered_from_bytes(Some(handed), None).unwrap();
  assert_eq!(cfg.worktree.branch_pattern, "handed/{desc}");
}

/// `None` repo bytes is "this repo has no `.gwm.toml`" — the global layer
/// alone, exactly what an absent file produces through the path-based form.
#[test]
fn no_repo_bytes_leaves_the_global_layer_alone() {
  let dir = TempDir::new().unwrap();
  let global = dir.path().join("config.toml");
  std::fs::write(&global, "[worktree]\nbranch_pattern = \"global/{desc}\"\n").unwrap();

  let cfg = Config::load_layered_from_bytes(None, Some(&global)).unwrap();
  assert_eq!(cfg.worktree.branch_pattern, "global/{desc}");
}

/// The handed bytes are still a repo layer, so they win over the global one
/// key by key — same merge rule as the path-based form (#190).
#[test]
fn handed_repo_bytes_override_the_global_layer() {
  let dir = TempDir::new().unwrap();
  let global = dir.path().join("config.toml");
  std::fs::write(
    &global,
    "[worktree]\nbranch_pattern = \"global/{desc}\"\nbase = \"develop\"\n",
  )
  .unwrap();

  let cfg =
    Config::load_layered_from_bytes(Some(b"[worktree]\nbranch_pattern = \"repo/{desc}\"\n"), Some(&global)).unwrap();
  assert_eq!(cfg.worktree.branch_pattern, "repo/{desc}");
  assert_eq!(
    cfg.worktree.base, "develop",
    "an untouched sibling key survives the merge"
  );
}

/// The validators run on the handed bytes as they do on the file, so the
/// snapshot form cannot become a way to smuggle a config the path-based form
/// rejects.
#[test]
fn handed_repo_bytes_go_through_the_same_validators() {
  let out = Config::load_layered_from_bytes(Some(b"[exec.profiles.bad]\ncommand = []\n"), None);
  let err = out.expect_err("an empty exec profile command must be rejected");
  assert!(
    format!("{err}").contains("empty `command`"),
    "the semantic validator has to run on handed bytes too, got: {err}"
  );
}

#[test]
fn tui_mux_open_in_defaults_to_a_pane() {
  // #608. `t` has always opened a pane, so the split of `mux_pane_direction`
  // into two keys must not move what the key does by default.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[worktree]\nbase = \"~/wt\"\n").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.mux_open_in, MuxTarget::Pane);
  assert_eq!(
    Config::default().tui.mux_open_in,
    MuxTarget::Pane,
    "`Config::default()` must agree with the serde default"
  );
}

#[test]
fn tui_mux_open_in_round_trips_every_documented_value() {
  for (value, expected) in [
    ("pane", MuxTarget::Pane),
    ("tab", MuxTarget::Tab),
    ("workspace", MuxTarget::Workspace),
  ] {
    let dir = TempDir::new().unwrap();
    std::fs::write(
      dir.path().join(CONFIG_FILE),
      format!("[tui]\nmux_open_in = \"{value}\"\n"),
    )
    .unwrap();
    let cfg = Config::load_layered(dir.path(), None).unwrap();
    assert_eq!(cfg.tui.mux_open_in, expected, "`{value}` must load back");
    assert_eq!(expected.label(), value, "the label is the TOML spelling");
  }
}

#[test]
fn tui_mux_open_in_rejects_a_level_no_multiplexer_has() {
  // `session` is the tempting one: tmux and zellij both have sessions, but
  // gwm runs *inside* one and none of the three can spawn a sibling from
  // there. A value the loader accepted would reach a builder with no arm.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[tui]\nmux_open_in = \"session\"\n").unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("`session` is not a target gwm can open");
  assert!(
    format!("{err}").contains("mux_open_in"),
    "the error must name the key, got: {err}"
  );
}

#[test]
fn tui_mux_pane_direction_is_a_direction_and_nothing_else() {
  // #608 took `window` out of this key: it said *what* to open, not *which
  // half*, and it now lives in `mux_open_in = "tab"`. The old spelling must
  // fail at load rather than parse as something else, so a config written
  // against the unreleased #589 shape is told where the value went.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[tui]\nmux_pane_direction = \"window\"\n").unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("`window` is not a direction");
  assert!(
    format!("{err}").contains("mux_pane_direction"),
    "the error must name the key, got: {err}"
  );
}

#[test]
fn tui_mux_pane_direction_defaults_to_right() {
  // #589. Up to 1.9 a split carried no direction and each backend answered
  // for itself: tmux stacked, zellij took the biggest free space, herdr went
  // right. `right` is the deliberate default — it is what the `--split` help
  // has promised since it shipped, and the half that is free on a wide
  // screen. `down` is the value that restores tmux's old behaviour.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[worktree]\nbase = \"~/wt\"\n").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.mux_pane_direction, SplitDirection::Right);
  assert_eq!(
    Config::default().tui.mux_pane_direction,
    SplitDirection::Right,
    "`Config::default()` must agree with the serde default"
  );
}

#[test]
fn tui_mux_pane_direction_round_trips_every_documented_value() {
  // Both values are what the Settings panel writes back, so each one has to
  // load from the file it produces.
  for (value, expected) in [
    ("right", SplitDirection::Right),
    ("down", SplitDirection::Down),
    ("left", SplitDirection::Left),
    ("up", SplitDirection::Up),
  ] {
    let dir = TempDir::new().unwrap();
    std::fs::write(
      dir.path().join(CONFIG_FILE),
      format!("[tui]\nmux_pane_direction = \"{value}\"\n"),
    )
    .unwrap();
    let cfg = Config::load_layered(dir.path(), None).unwrap();
    assert_eq!(cfg.tui.mux_pane_direction, expected, "`{value}` must load back");
    assert_eq!(expected.label(), value, "the label is the TOML spelling");
  }
}

#[test]
fn tui_mux_pane_direction_rejects_a_value_no_backend_can_honour() {
  // `left` and `up` joined the set in #611, so the rejected value has to be
  // one no backend spells. A typo must fail at load rather than silently
  // fall back to the default.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    "[tui]\nmux_pane_direction = \"sideways\"\n",
  )
  .unwrap();
  let err = Config::load_layered(dir.path(), None).expect_err("`sideways` is not a value gwm can honour");
  assert!(
    format!("{err}").contains("mux_pane_direction"),
    "the error must name the key, got: {err}"
  );
}

#[test]
fn tui_note_vim_is_on_unless_turned_off() {
  // #557: the mode ships on. A `#[serde(default)]` on a bool yields
  // `false`, so the serde default has to be spelled out — miss it and
  // every config that does not name the key silently gets the modeless
  // editor while `Config::default()` says otherwise.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join(CONFIG_FILE), "[worktree]\nbase = \"~/wt\"\n").unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(cfg.tui.note_vim, "note_vim must default to true");
  assert!(
    Config::default().tui.note_vim,
    "`Config::default()` must agree with the serde default"
  );
}

#[test]
fn tui_note_vim_round_trips_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui]
note_vim = false
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert!(!cfg.tui.note_vim, "the opt-out is the value worth round-tripping now");
}

// ---------------------------------------------------------------------------
// `[tui.agent_resume]` — issue #591
// ---------------------------------------------------------------------------
//
// The four resume incantations belong in config rather than in a `match`
// because they are four third-party CLIs on their own release cadence: the
// day `codex resume` grows a flag, a gwm release should not be what stands
// between the user and a working `o`.

#[test]
fn agent_resume_defaults_cover_every_detected_backend() {
  use gwm::agent_sessions::AgentKind;
  let cfg = Config::default();
  // Measured against the installed binaries on 2026-08-25, not assumed.
  assert_eq!(
    cfg.tui.agent_resume.template_for(AgentKind::ClaudeCode),
    "claude -r {session}"
  );
  assert_eq!(
    cfg.tui.agent_resume.template_for(AgentKind::Codex),
    "codex resume {session}"
  );
  assert_eq!(
    cfg.tui.agent_resume.template_for(AgentKind::Opencode),
    "opencode -s {session}"
  );
  assert_eq!(
    cfg.tui.agent_resume.template_for(AgentKind::Vibe),
    "vibe --resume {session}"
  );
}

#[test]
fn agent_resume_override_wins_and_an_empty_string_reads_as_unset() {
  use gwm::agent_sessions::AgentKind;
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[tui.agent_resume]
claude = "claude --resume {session} --dangerously-skip-permissions"
codex = ""
"#,
  )
  .unwrap();
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(
    cfg.tui.agent_resume.template_for(AgentKind::ClaudeCode),
    "claude --resume {session} --dangerously-skip-permissions"
  );
  // Same `""` == omitted convention `shell_cmd` / `editor_cmd` already use,
  // so a user who blanks a key gets the default back rather than a pane
  // running nothing.
  assert_eq!(
    cfg.tui.agent_resume.template_for(AgentKind::Codex),
    "codex resume {session}"
  );
  // Untouched backends keep theirs.
  assert_eq!(
    cfg.tui.agent_resume.template_for(AgentKind::Vibe),
    "vibe --resume {session}"
  );
}

#[test]
fn agent_resume_expansion_refuses_an_id_no_shell_can_be_trusted_with() {
  use gwm::config::{expand_agent_resume, is_shell_safe_session_id};
  // Copilot review on PR #610, confirmed empirically:
  //   sh -c 'echo "'"'"'$(echo PWNED)'"'"'"'   =>   PWNED
  // `shell_words::quote` is only safe for an UNQUOTED posix position, and the
  // template around the placeholder belongs to the user. `claude -r
  // "{session}"` is a natural override, and there the single quotes the
  // quoter adds are literal characters inside the double quotes, so the
  // substitution still runs. `cmd.exe` never honours them at all. So the id
  // is constrained rather than trusted to its wrapping.
  assert_eq!(expand_agent_resume("claude -r {session}", "s1; rm -rf ~"), None);
  assert_eq!(expand_agent_resume("claude -r \"{session}\"", "$(echo PWNED)"), None);
  assert_eq!(expand_agent_resume("claude -r {session}", "`id`"), None);
  assert_eq!(expand_agent_resume("claude -r {session}", ""), None);

  // Nothing real is refused: every backend's id is a UUID or a slug.
  for real in [
    "03bf26b2-705c-402d-b112-59ad34b08200",
    "019fa42e-4270-7240-8bd0-1c0d3c05bbaa",
    "ses_01abcDEF",
    "session.2026-08-26",
  ] {
    assert!(is_shell_safe_session_id(real), "{real} is a shape gwm really produces");
    assert!(expand_agent_resume("claude -r {session}", real).is_some());
  }
}

#[test]
fn agent_resume_expansion_quotes_the_session_and_runs_one_pass() {
  use gwm::config::expand_agent_resume;
  // The quoting stays as the second layer, under the charset guard.
  let ok = expand_agent_resume("claude -r {session}", "s1").unwrap();
  assert_eq!(ok, "claude -r s1");

  // An expansion is a value, not more template. An id that itself contains
  // the token comes out as ONE literal occurrence: what was written is never
  // re-examined, so nothing downstream can be fooled into a second round.
  // (`{`/`}` are outside the safe charset now, so this is asserted on the
  // expander's own contract rather than through a session id.)
  assert_eq!(
    expand_agent_resume("claude -r {session} --tag {session}", "abc"),
    Some("claude -r abc --tag abc".into())
  );

  // An unknown token is left verbatim, as every other gwm expander does.
  assert_eq!(
    expand_agent_resume("claude -r {nope}", "s1"),
    Some("claude -r {nope}".into())
  );
}

#[test]
fn terminal_browser_parses_and_an_empty_string_reads_as_unset() {
  // #590. Same `"" == omitted` convention as `[tui.open] shell_cmd`, so
  // blanking the key in the Settings panel turns the feature off rather than
  // leaving an empty command nobody can spawn.
  //
  // `load_for_repo` resolves the global config path from `$HOME`, so this
  // holds the binary's env lock across the read like every other loading
  // test here (`env_guard_invariant_tests` enforces it).
  let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let dir = tempfile::tempdir().unwrap();
  let write = |body: &str| {
    std::fs::write(dir.path().join(".gwm.toml"), body).unwrap();
    Config::load_for_repo(dir.path()).unwrap()
  };

  assert_eq!(
    write("[tui]\nterminal_browser = \"w3m {url}\"\n").tui.terminal_browser,
    Some("w3m {url}".into())
  );
  assert_eq!(write("[tui]\nterminal_browser = \"\"\n").tui.terminal_browser, None);
  assert_eq!(write("[tui]\nlayout = \"compact\"\n").tui.terminal_browser, None);

  // And the key reaches the Settings panel's read-only `All` tab, which is
  // what attributes a value to the layer that set it. `resolved_rows` derives
  // its rows from the serialised `Config` rather than from a hand-written key
  // list, so this is a check that the round trip survives the `Option`, not
  // that someone remembered to add a line.
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[tui]\nterminal_browser = \"w3m {url}\"\n",
  )
  .unwrap();
  let rows = gwm::config::resolved_rows(dir.path(), None).unwrap();
  let row = rows
    .iter()
    .find(|r| r.key == "tui.terminal_browser")
    .expect("tui.terminal_browser must be listed in the All tab");
  assert_eq!(row.value, "\"w3m {url}\"");
  assert_eq!(row.source, gwm::config::ConfigSource::Repo);
}

#[test]
fn terminal_browser_expansion_keeps_the_url_as_one_argument() {
  use gwm::config::expand_terminal_browser;
  // The whole security argument of #590, and the opposite order from
  // `expand_agent_resume`: the template is SPLIT first, so its own quoting is
  // consumed by the tokeniser and the URL lands as exactly one argv element
  // whose quoting gwm owns. `w3m {url}` and `w3m "{url}"` must therefore be
  // indistinguishable, and a URL carrying shell metacharacters must never be
  // able to become a second word.
  let hostile = "https://example.com/a;id&whoami?q=$(id)`id`";
  let bare = expand_terminal_browser("w3m {url}", hostile).unwrap();
  let quoted = expand_terminal_browser("w3m \"{url}\"", hostile).unwrap();
  assert_eq!(bare, quoted, "the template's quoting cannot change the value");
  assert_eq!(bare, vec!["w3m".to_string(), hostile.to_string()]);

  // Real URLs the TUI actually opens are not refused. `#` is in gwm's own
  // branch names and a check run's `details_url` carries a query string.
  for real in [
    "https://github.com/kbrdn1/gwm-cli/issues/590",
    "https://github.com/kbrdn1/gwm-cli/actions/runs/33156327787/job/98800986980",
    "https://github.com/kbrdn1/gwm-cli/tree/feat/%23590-terminal-browser",
    "https://gitlab.com/g/p/-/merge_requests/7?tab=diffs&view=inline",
  ] {
    let argv = expand_terminal_browser("w3m {url}", real).unwrap_or_else(|| panic!("{real} is a URL gwm builds"));
    assert_eq!(argv.last().map(String::as_str), Some(real));
  }

  // Substitution inside a token is single pass: an expansion is a value, not
  // more template (GHSA-fffq-vg6f-gxqm).
  assert_eq!(
    expand_terminal_browser("b --u={url} --alt={url}", "https://e.co/x"),
    Some(vec![
      "b".into(),
      "--u=https://e.co/x".into(),
      "--alt=https://e.co/x".into()
    ])
  );
}

#[test]
fn terminal_browser_expansion_appends_the_url_when_the_template_omits_it() {
  use gwm::config::expand_terminal_browser;
  // `terminal_browser = "w3m"` is what people write first, and w3m / lynx /
  // carbonyl / browsh all take the URL as their last argument. Refusing it
  // would document a loss for nothing.
  assert_eq!(
    expand_terminal_browser("w3m", "https://e.co/x"),
    Some(vec!["w3m".into(), "https://e.co/x".into()])
  );
  assert_eq!(
    expand_terminal_browser("lynx -accept_all_cookies", "https://e.co/x"),
    Some(vec![
      "lynx".into(),
      "-accept_all_cookies".into(),
      "https://e.co/x".into()
    ])
  );
}

#[test]
fn terminal_browser_expansion_refuses_what_is_not_an_absolute_web_url() {
  use gwm::config::{expand_terminal_browser, is_browsable_url};
  // Narrow on purpose: the quoting handles the metacharacters, this handles
  // everything that is not a URL at all. A `file://` or a bare path would be
  // gwm handing a local read to a browser it never built the link for, and a
  // newline is the one character that could survive into a second command
  // line whatever the quoting does.
  for bad in [
    "",
    "example.com/x",
    "file:///etc/passwd",
    "javascript:alert(1)",
    "https://e.co/a b",
    "https://e.co/a\nid",
  ] {
    assert!(!is_browsable_url(bad), "{bad:?} must not reach a terminal browser");
    assert_eq!(expand_terminal_browser("w3m {url}", bad), None, "{bad:?}");
  }
  // An unparseable or empty template is refused too, rather than spawning a
  // browser-less argv.
  assert_eq!(expand_terminal_browser("w3m \"unbalanced", "https://e.co/x"), None);
  assert_eq!(expand_terminal_browser("   ", "https://e.co/x"), None);
}
