use gwm::config::{expand_placeholders, Config, WorktreeConfig, CONFIG_FILE};
use tempfile::TempDir;

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
