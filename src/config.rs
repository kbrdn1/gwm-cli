use crate::error::{GwmError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = ".gwm.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
  #[serde(default)]
  pub worktree: WorktreeConfig,
  #[serde(default)]
  pub bootstrap: BootstrapConfig,
  #[serde(default)]
  pub doctor: DoctorConfig,
  #[serde(default)]
  pub tui: TuiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
  pub base: String,
  pub path_pattern: String,
  pub branch_pattern: String,
}

impl Default for WorktreeConfig {
  fn default() -> Self {
    Self {
      base: "{home}/cc-worktree/{repo}".into(),
      path_pattern: "{type}-{issue}-{desc}".into(),
      branch_pattern: "{type}/#{issue}-{desc}".into(),
    }
  }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BootstrapConfig {
  #[serde(default)]
  pub copy: Vec<CopyStep>,
  #[serde(default)]
  pub guard: Vec<Guard>,
  #[serde(default)]
  pub no_symlink: Vec<NoSymlink>,
  #[serde(default)]
  pub command: Vec<CommandStep>,
  #[serde(default)]
  pub fallback: HashMap<String, FallbackContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyStep {
  pub from: String,
  pub to: String,
  #[serde(default)]
  pub required: bool,
  #[serde(default)]
  pub guards: Vec<String>,
  /// "inline" → use [bootstrap.fallback.<key>] content if source missing.
  /// "skip" → silently skip (default for non-required).
  /// "abort" → fail bootstrap.
  #[serde(default)]
  pub fallback: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guard {
  pub name: String,
  #[serde(default)]
  pub deny_patterns: Vec<String>,
  /// "abort" (default) | "seed-from-example"
  #[serde(default = "default_on_match")]
  pub on_match: String,
  #[serde(default)]
  pub example_file: Option<String>,
}

fn default_on_match() -> String {
  "abort".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoSymlink {
  pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandStep {
  pub name: String,
  pub run: String,
  /// `file_exists:<path>` only for now.
  #[serde(default)]
  pub when: Option<String>,
  #[serde(default)]
  pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackContent {
  pub target: String,
  pub content: String,
}

/// `[doctor]` table — knobs for `gwm doctor`. Currently exposes the trunk
/// list used by the orphan-branch check; previously this was hardcoded to
/// `["dev", "main"]` in `doctor.rs`, which silently no-op'd the filter on
/// any repo using a different trunk convention (`master`, `trunk`,
/// `release-1.x`, …). Default preserves the previous behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorConfig {
  /// Trunk branches the orphan-branch check treats as "merge destinations".
  /// A gwm-style branch fully reachable from one of these is preserved per
  /// CONTRIBUTING.md ("never delete the source branch after merge") and is
  /// therefore not flagged as orphan. An empty list disables the filter
  /// entirely (every unclaimed gwm-style branch becomes orphan).
  #[serde(default = "default_trunks")]
  pub trunks: Vec<String>,
}

impl Default for DoctorConfig {
  fn default() -> Self {
    Self {
      trunks: default_trunks(),
    }
  }
}

fn default_trunks() -> Vec<String> {
  vec!["dev".into(), "main".into()]
}

/// `[tui]` table — runtime knobs for the worktree TUI. Currently exposes
/// the safety countdown on the delete-confirm overlay (issue #30): when
/// `delete_branch_on_remove` has been toggled ON, the modal forces the
/// user to wait N seconds (visualised by a progress bar) before the
/// destructive action actually fires. `0` disables the countdown and
/// falls back to the classic single-keystroke confirm even when delete-
/// branch is armed; the value is clamped to `5` at read time so a typo
/// like `confirm_countdown_secs = 300` can never strand a destructive
/// path behind a 300-second wait.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
  /// Safety countdown (in seconds) applied to the confirm overlay when
  /// `delete_branch_on_remove` is ON. Accepts any non-negative integer;
  /// values above [`Self::MAX_CONFIRM_COUNTDOWN_SECS`] are clamped on
  /// read via [`Self::effective_confirm_countdown_secs`]. The field is
  /// `u32` (rather than `u8`) so a typo like `confirm_countdown_secs = 300`
  /// still round-trips through TOML deserialization and reaches the
  /// clamp instead of erroring out at parse time.
  #[serde(default = "default_confirm_countdown_secs")]
  pub confirm_countdown_secs: u32,

  /// `[tui.open]` sub-table — drives the dispatch of the `o` key in the
  /// list view. Default mode is `shell` (lazygit-like worktree-manager
  /// workflow); pre-#73 behaviour (`open` / `xdg-open` / `explorer`) is
  /// kept available under `mode = "finder"`.
  #[serde(default)]
  pub open: TuiOpenConfig,
}

impl Default for TuiConfig {
  fn default() -> Self {
    Self {
      confirm_countdown_secs: default_confirm_countdown_secs(),
      open: TuiOpenConfig::default(),
    }
  }
}

/// `[tui.open]` — how the `o` key resolves the action on the selected
/// worktree. Adds a configurable hook on top of the historical "reveal
/// in OS file manager" so users with a worktree-heavy workflow can land
/// in a shell or `$EDITOR` directly, sharing the spawn-and-restore
/// lifecycle that `l: lazygit` already uses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TuiOpenConfig {
  #[serde(default)]
  pub mode: TuiOpenMode,
  /// Override `$SHELL` when `mode = "shell"`. Falls back to `$SHELL`,
  /// then `/bin/sh`. Empty TOML string reads as `None` so
  /// `shell_cmd = ""` and an omitted key are observationally identical.
  #[serde(default, deserialize_with = "deserialize_optional_non_empty")]
  pub shell_cmd: Option<String>,
  /// Override `$EDITOR` when `mode = "editor"`. Falls back to `$EDITOR`,
  /// then `vi`. Same empty-string-as-unset convention as `shell_cmd`.
  #[serde(default, deserialize_with = "deserialize_optional_non_empty")]
  pub editor_cmd: Option<String>,
}

/// The three documented behaviours of the `o` key. Serialised in
/// lowercase so `.gwm.toml` keys stay idiomatic (`mode = "shell"`); an
/// unknown value is a hard config error surfaced by
/// `Config::load_for_repo`, never silently coerced to a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TuiOpenMode {
  /// Spawn an interactive shell with `cwd` set to the worktree
  /// (lazygit-style suspend / spawn / restore). Default — matches the
  /// "I want to do work in this worktree" intent of a worktree manager.
  #[default]
  Shell,
  /// Spawn `$EDITOR <worktree-path>` and wait for it to exit. Useful
  /// for drive-by edits without dropping into a full shell session.
  Editor,
  /// Pre-#73 behaviour: ask the OS to reveal the worktree directory
  /// (`open` on macOS, `xdg-open` on Linux, `explorer` on Windows).
  Finder,
}

/// Serde helper: treat an empty TOML string as `None`. Keeps
/// `shell_cmd = ""` and an omitted key identical at the call site so
/// the TUI never has to special-case the empty command.
fn deserialize_optional_non_empty<'de, D>(d: D) -> std::result::Result<Option<String>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let opt = Option::<String>::deserialize(d)?;
  Ok(opt.filter(|s| !s.is_empty()))
}

impl TuiConfig {
  /// Documented range cap. Centralised so the TUI and the doctor share
  /// the same clamp logic.
  pub const MAX_CONFIRM_COUNTDOWN_SECS: u32 = 5;

  /// Effective countdown value used by the TUI, clamped to
  /// `[0, MAX_CONFIRM_COUNTDOWN_SECS]`. The raw field stays at the
  /// user's value so a future doctor check can surface "your config
  /// asked for X but we capped at 5".
  pub fn effective_confirm_countdown_secs(&self) -> u32 {
    self.confirm_countdown_secs.min(Self::MAX_CONFIRM_COUNTDOWN_SECS)
  }
}

fn default_confirm_countdown_secs() -> u32 {
  3
}

impl Config {
  /// Look for `.gwm.toml` in the given repo root.
  /// Falls back to defaults when missing.
  pub fn load_for_repo(repo_root: &Path) -> Result<Self> {
    let path = repo_root.join(CONFIG_FILE);
    if !path.exists() {
      return Ok(Self::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    let cfg: Config = toml::from_str(&raw)?;
    Ok(cfg)
  }

  /// Write a default config to the given repo root.
  pub fn write_default(repo_root: &Path) -> Result<PathBuf> {
    let target = repo_root.join(CONFIG_FILE);
    if target.exists() {
      return Err(GwmError::Config(format!("{} already exists", target.display())));
    }
    let body = include_str!("../examples/gwm.toml.example");
    std::fs::write(&target, body)?;
    Ok(target)
  }

  pub fn guard_by_name(&self, name: &str) -> Option<&Guard> {
    self.bootstrap.guard.iter().find(|g| g.name == name)
  }
}

/// Expand `{home}`, `{repo}`, `{type}`, `{issue}`, `{desc}` in a template string.
pub fn expand_placeholders(
  template: &str,
  repo: &str,
  type_: Option<&str>,
  issue: Option<&str>,
  desc: Option<&str>,
) -> Result<String> {
  let home = dirs::home_dir()
    .ok_or_else(|| GwmError::Config("cannot resolve $HOME".into()))?
    .to_string_lossy()
    .to_string();
  let mut out = template.replace("{home}", &home).replace("{repo}", repo);
  if let Some(t) = type_ {
    out = out.replace("{type}", t);
  }
  if let Some(i) = issue {
    out = out.replace("{issue}", i);
  }
  if let Some(d) = desc {
    out = out.replace("{desc}", d);
  }
  // Tilde expansion in case the template starts with ~/...
  let expanded = shellexpand::tilde(&out).to_string();
  Ok(expanded)
}
