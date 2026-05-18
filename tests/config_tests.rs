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
