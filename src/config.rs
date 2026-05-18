use crate::error::{GwmError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = ".gwm.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
  #[serde(default)]
  pub worktree: WorktreeConfig,
  #[serde(default)]
  pub bootstrap: BootstrapConfig,
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

impl Default for Config {
  fn default() -> Self {
    Self {
      worktree: WorktreeConfig::default(),
      bootstrap: BootstrapConfig::default(),
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
  let mut out = template
    .replace("{home}", &home)
    .replace("{repo}", repo);
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

