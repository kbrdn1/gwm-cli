use gwm::config::{
  expand_placeholders, review_tool_preset, BranchTypesSource, Config, TuiOpenMode, WorktreeConfig, CONFIG_FILE,
};
use tempfile::TempDir;

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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
  assert_eq!(cfg.labels.len(), 1);
  assert_eq!(cfg.labels[0].name, "wip");
  assert_eq!(cfg.labels[0].description, None);
  assert_eq!(cfg.labels[0].color, None);
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
  assert!(cfg.doctor.trunks.is_empty());
}

#[test]
fn placeholders_expand() {
  let out = expand_placeholders(
    "{home}/cc-worktree/{repo}/{type}-{issue}-{desc}",
    "my-repo",
    Some("feat"),
    Some("123"),
    Some("foo"),
  )
  .unwrap();
  assert!(out.ends_with("/cc-worktree/my-repo/feat-123-foo"));
  assert!(!out.contains("{home}"));
  assert!(!out.contains("{repo}"));
}

#[test]
fn placeholders_no_optional_args_leave_repo_only() {
  let out = expand_placeholders("{home}/{repo}", "x", None, None, None).unwrap();
  assert!(out.ends_with("/x"));
}

#[test]
fn load_returns_defaults_when_no_file() {
  let dir = TempDir::new().unwrap();
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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

  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let res = Config::load_for_repo(dir.path());
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 3);
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
  assert_eq!(cfg.tui.effective_confirm_countdown_secs(), 0);
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).expect("300 must parse, not error");
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  assert!(Config::load_for_repo(dir.path()).is_err());
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let cfg = Config::load_for_repo(dir.path()).unwrap();
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
  let err = Config::load_for_repo(dir.path()).unwrap_err();
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
      Config::load_for_repo(dir.path()).is_err(),
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
  let err = Config::load_for_repo(dir.path()).unwrap_err();
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
  let cfg = Config::load_for_repo(dir.path()).expect("valid config must load");
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
  let err = Config::load_for_repo(dir.path()).expect_err("traversal must be rejected at load");
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
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.copy]]
from = ".env"
to   = "/etc/passwd"
"#,
  )
  .unwrap();
  let err = Config::load_for_repo(dir.path()).expect_err("absolute path must be rejected at load");
  let msg = format!("{}", err);
  assert!(
    msg.contains("bootstrap.copy") && msg.contains("to"),
    "error must name the offending field, got: {}",
    msg
  );
  assert!(
    msg.contains("absolute") || msg.contains("/etc/passwd"),
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
  let err = Config::load_for_repo(dir.path()).expect_err("traversal in example_file must be rejected");
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
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[[bootstrap.guard]]
name           = "leaky"
deny_patterns  = ["amazonaws"]
on_match       = "seed-from-example"
example_file   = "/etc/shadow"
"#,
  )
  .unwrap();
  let err = Config::load_for_repo(dir.path()).expect_err("absolute example_file must be rejected");
  let msg = format!("{}", err);
  assert!(
    msg.contains("guard") && msg.contains("example_file"),
    "error must name the offending field, got: {}",
    msg
  );
  assert!(
    msg.contains("absolute") || msg.contains("/etc/shadow"),
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
  let err = Config::load_for_repo(dir.path()).expect_err("traversal in fallback.target must be rejected");
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
  let err = Config::load_for_repo(dir.path()).expect_err("drive-prefixed path must be rejected at load");
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
  Config::load_for_repo(dir.path()).expect("benign relative paths must load");
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
  let err = Config::load_for_repo(dir.path()).expect_err("invalid deny_patterns must be rejected at load");
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
  let err = Config::load_for_repo(dir.path()).expect_err("invalid sole deny pattern must be rejected at load");
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
  let cfg = Config::load_for_repo(dir.path()).expect("valid patterns must load");
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
  let mut cfg = Config::default();
  cfg.bootstrap = BootstrapConfig {
    guard: vec![Guard {
      name: "direct-test".into(),
      deny_patterns: vec!["(unclosed-group".into()],
      on_match: "abort".into(),
      example_file: None,
    }],
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
  cfg.validate_bootstrap_guards().expect("valid patterns must pass direct validation");
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
  let err = Config::load_for_repo(dir.path()).expect_err("invalid pattern in second guard must be rejected");
  let msg = format!("{}", err);
  assert!(
    msg.contains("guard-two") && msg.contains("[unclosed"),
    "error must name the offending guard + pattern, got: {}",
    msg
  );
}
