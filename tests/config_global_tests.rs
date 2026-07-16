//! Tests for the user-level global config merged under per-repo
//! `.gwm.toml` (issue #190).
//!
//! The merge is exercised through the path-injected seam
//! `Config::load_layered(repo_root, Some(global_path))` so the
//! contract is pinned without touching the runner's real `$HOME` /
//! `$XDG_CONFIG_HOME` (env-independence rule). `global_config_path_in`
//! pins the on-disk location separately.

use gwm::config::{global_config_path, global_config_path_in, resolve_global_config_path, Config};
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

/// Process-wide guard for the env-mutating test below: `set_var` /
/// `remove_var` touch the shared libc env table, so a parallel
/// env-mutating test would race. No other test in this binary mutates
/// process env (they all drive the injected `load_layered` seam), but
/// the lock keeps the idiom consistent with `history_tests.rs`.
fn env_lock() -> &'static Mutex<()> {
  static LOCK: Mutex<()> = Mutex::new(());
  &LOCK
}

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

// --- issue #372: the documented `~/.config` path is honoured across
// platforms, not just where `dirs::config_dir()` happens to be `~/.config`.
// These drive the pure resolver seam (existence injected) so the contract
// holds identically on every runner OS, per the env-independence rule.

/// Build a `.../gwm/config.toml` under `home/.config` and `touch` it.
fn touch_dotconfig(home: &Path) -> std::path::PathBuf {
  let dir = home.join(".config").join("gwm");
  std::fs::create_dir_all(&dir).unwrap();
  let cfg = dir.join("config.toml");
  std::fs::write(&cfg, "").unwrap();
  cfg
}

#[test]
fn resolver_prefers_dotconfig_when_it_exists() {
  // A macOS user who followed the docs and put their config at
  // ~/.config/gwm/config.toml must be honoured even though the platform
  // config dir (Application Support) differs from ~/.config (issue #372).
  let home = TempDir::new().unwrap();
  let platform = TempDir::new().unwrap(); // stand-in for Application Support
  let cfg = touch_dotconfig(home.path());

  let resolved = resolve_global_config_path(None, Some(home.path()), Some(platform.path()), |p| p.exists());
  assert_eq!(
    resolved,
    Some(cfg),
    "the documented ~/.config path must win when present"
  );
}

#[test]
fn resolver_falls_back_to_platform_dir_when_only_it_exists() {
  // Back-compat: a config already living at the platform dir keeps working
  // when ~/.config has none.
  let home = TempDir::new().unwrap();
  let platform = TempDir::new().unwrap();
  let cfg = global_config_path_in(platform.path());
  std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
  std::fs::write(&cfg, "").unwrap();

  let resolved = resolve_global_config_path(None, Some(home.path()), Some(platform.path()), |p| p.exists());
  assert_eq!(
    resolved,
    Some(cfg),
    "an existing platform-dir config must still resolve"
  );
}

#[test]
fn resolver_returns_canonical_dotconfig_when_nothing_exists() {
  // Neither present → point doctor / callers at the documented location.
  let home = TempDir::new().unwrap();
  let platform = TempDir::new().unwrap();

  let resolved = resolve_global_config_path(None, Some(home.path()), Some(platform.path()), |_| false);
  assert_eq!(
    resolved,
    Some(global_config_path_in(&home.path().join(".config"))),
    "with nothing on disk the canonical ~/.config path is reported"
  );
}

#[test]
fn resolver_honours_xdg_outright_even_when_dotconfig_exists() {
  // An explicit $XDG_CONFIG_HOME wins over both candidates and is returned
  // whether or not the file exists — the pre-#372 contract.
  let xdg = TempDir::new().unwrap();
  let home = TempDir::new().unwrap();
  let platform = TempDir::new().unwrap();
  touch_dotconfig(home.path()); // present, yet XDG must still win

  let resolved = resolve_global_config_path(Some(xdg.path()), Some(home.path()), Some(platform.path()), |p| {
    p.exists()
  });
  assert_eq!(resolved, Some(global_config_path_in(xdg.path())));
}

#[test]
fn resolver_linux_dotconfig_equals_platform_dir_is_unchanged() {
  // On Linux dirs::config_dir() == ~/.config, so both candidates coincide
  // and resolution is byte-for-byte pre-#372 whether or not the file exists.
  let home = TempDir::new().unwrap();
  let platform = home.path().join(".config"); // simulate the Linux equality
  let resolved = resolve_global_config_path(None, Some(home.path()), Some(&platform), |_| false);
  assert_eq!(resolved, Some(global_config_path_in(&platform)));
}

#[test]
fn gwm_no_global_config_env_forces_repo_only() {
  // `GWM_NO_GLOBAL_CONFIG=1` makes `global_config_path()` report no
  // path, so `load_for_repo` degrades to repo-only — the opt-out that
  // keeps tests/CI deterministic on a machine with a real global
  // config (PR #191 review).
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prev_flag = std::env::var("GWM_NO_GLOBAL_CONFIG").ok();
  let prev_xdg = std::env::var("XDG_CONFIG_HOME").ok();

  // Pin XDG_CONFIG_HOME so the non-opt-out branch resolves to a known
  // path regardless of the runner's home dir (env-independence rule).
  let xdg = TempDir::new().unwrap();

  // SAFETY: env mutation is serialised by `env_lock()`; we restore the
  // prior values before dropping the guard.
  unsafe {
    std::env::set_var("XDG_CONFIG_HOME", xdg.path());
    std::env::set_var("GWM_NO_GLOBAL_CONFIG", "1");
  }
  assert_eq!(global_config_path(), None, "opt-out must suppress the global path");

  unsafe {
    std::env::set_var("GWM_NO_GLOBAL_CONFIG", "0");
  }
  assert_eq!(
    global_config_path(),
    Some(xdg.path().join("gwm").join("config.toml")),
    "a falsey value must not opt out (path resolves under XDG)"
  );

  // SAFETY: restoration paired with the mutations above, still guarded.
  unsafe {
    match prev_flag {
      Some(v) => std::env::set_var("GWM_NO_GLOBAL_CONFIG", v),
      None => std::env::remove_var("GWM_NO_GLOBAL_CONFIG"),
    }
    match prev_xdg {
      Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
      None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
  }
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
