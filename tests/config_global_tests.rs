//! Tests for the user-level global config merged under per-repo
//! `.gwm.toml` (issue #190).
//!
//! The merge is exercised through the path-injected seam
//! `Config::load_layered(repo_root, Some(global_path))` so the
//! contract is pinned without touching the runner's real `$HOME` /
//! `$XDG_CONFIG_HOME` (env-independence rule). `global_config_path_in`
//! pins the on-disk location separately.

use gwm::config::{global_config_path_in, Config};
use std::path::Path;
use tempfile::TempDir;

/// Write `contents` to `dir/.gwm.toml`.
fn write_repo(dir: &Path, contents: &str) {
  std::fs::write(dir.join(".gwm.toml"), contents).unwrap();
}

/// Write a global config file under `dir` and return its path.
fn write_global(dir: &Path, contents: &str) -> std::path::PathBuf {
  let p = dir.join("config.toml");
  std::fs::write(&p, contents).unwrap();
  p
}

#[test]
fn global_config_path_lives_under_gwm_config_toml() {
  let home = Path::new("/tmp/xdg-home");
  assert_eq!(global_config_path_in(home), home.join("gwm").join("config.toml"));
}

#[test]
fn no_files_resolve_to_default() {
  let repo = TempDir::new().unwrap();
  let missing = repo.path().join("nope.toml");
  let cfg = Config::load_layered(repo.path(), Some(&missing)).unwrap();
  // Same as a bare default — nothing on disk to layer.
  assert!(cfg.theme.preset.is_none());
  assert!(cfg.labels.is_empty());
}

#[test]
fn global_only_applies_when_repo_absent() {
  let gdir = TempDir::new().unwrap();
  let global = write_global(
    gdir.path(),
    r#"
[theme]
preset = "catppuccin"
"#,
  );
  let repo = TempDir::new().unwrap(); // no .gwm.toml
  let cfg = Config::load_layered(repo.path(), Some(&global)).unwrap();
  assert_eq!(cfg.theme.preset.as_deref(), Some("catppuccin"));
}

#[test]
fn repo_only_ignores_a_missing_global() {
  let repo = TempDir::new().unwrap();
  write_repo(
    repo.path(),
    r#"
[theme]
preset = "gruvbox"
"#,
  );
  let missing = repo.path().join("no-global.toml");
  let cfg = Config::load_layered(repo.path(), Some(&missing)).unwrap();
  assert_eq!(cfg.theme.preset.as_deref(), Some("gruvbox"));
}

#[test]
fn repo_scalar_wins_over_global() {
  // Both set the same scalar (theme.preset) → the repo value wins.
  let gdir = TempDir::new().unwrap();
  let global = write_global(
    gdir.path(),
    r#"
[theme]
preset = "catppuccin"
"#,
  );
  let repo = TempDir::new().unwrap();
  write_repo(
    repo.path(),
    r#"
[theme]
preset = "tokyo-night"
"#,
  );
  let cfg = Config::load_layered(repo.path(), Some(&global)).unwrap();
  assert_eq!(
    cfg.theme.preset.as_deref(),
    Some("tokyo-night"),
    "repo .gwm.toml must win on a conflicting scalar"
  );
}

#[test]
fn disjoint_tables_from_both_files_coexist() {
  // Global sets [theme], repo sets [worktree] → both survive the merge.
  let gdir = TempDir::new().unwrap();
  let global = write_global(
    gdir.path(),
    r#"
[theme]
preset = "catppuccin"
"#,
  );
  let repo = TempDir::new().unwrap();
  write_repo(
    repo.path(),
    r#"
[worktree]
base = "/tmp/custom-worktrees"
"#,
  );
  let cfg = Config::load_layered(repo.path(), Some(&global)).unwrap();
  assert_eq!(cfg.theme.preset.as_deref(), Some("catppuccin"), "global theme survives");
  assert_eq!(cfg.worktree.base, "/tmp/custom-worktrees", "repo worktree survives");
}

#[test]
fn nested_table_merges_key_by_key() {
  // Global [theme] sets preset + an override; repo [theme] overrides ONE
  // role. The deep merge keeps the global preset and the untouched
  // override, while the repo's role wins.
  let gdir = TempDir::new().unwrap();
  let global = write_global(
    gdir.path(),
    r##"
[theme]
preset = "catppuccin"
branch = "#111111"
"##,
  );
  let repo = TempDir::new().unwrap();
  write_repo(
    repo.path(),
    r##"
[theme]
accent = "#222222"
"##,
  );
  let cfg = Config::load_layered(repo.path(), Some(&global)).unwrap();
  assert_eq!(cfg.theme.preset.as_deref(), Some("catppuccin"), "global preset kept");
  assert_eq!(
    cfg.theme.overrides.get("branch").map(String::as_str),
    Some("#111111"),
    "untouched global override kept"
  );
  assert_eq!(
    cfg.theme.overrides.get("accent").map(String::as_str),
    Some("#222222"),
    "repo override merged in"
  );
}

#[test]
fn arrays_are_replaced_not_unioned() {
  // Global declares one label, repo declares another. Arrays replace
  // wholesale → only the repo's labels survive (no confusing union).
  let gdir = TempDir::new().unwrap();
  let global = write_global(
    gdir.path(),
    r#"
[[labels]]
name = "global-label"
"#,
  );
  let repo = TempDir::new().unwrap();
  write_repo(
    repo.path(),
    r#"
[[labels]]
name = "repo-label"
"#,
  );
  let cfg = Config::load_layered(repo.path(), Some(&global)).unwrap();
  let names: Vec<&str> = cfg.labels.iter().map(|l| l.name.as_str()).collect();
  assert_eq!(names, vec!["repo-label"], "repo array replaces global array");
}

#[test]
fn merged_result_is_validated() {
  // A bad colour in the *merged* config must fail at load, exactly as a
  // bad repo-only value would — validation runs on the merge.
  let gdir = TempDir::new().unwrap();
  let global = write_global(
    gdir.path(),
    r#"
[theme]
accent = "not-a-color"
"#,
  );
  let repo = TempDir::new().unwrap(); // no repo override
  let err = Config::load_layered(repo.path(), Some(&global)).unwrap_err();
  assert!(
    err.to_string().to_lowercase().contains("color"),
    "expected a colour validation error, got: {err}"
  );
}
