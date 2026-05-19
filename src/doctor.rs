//! Environment + worktree diagnostics. Aggregates a series of cheap checks
//! into a single report so users (and CI) can answer "is my setup sane?"
//! without running a dozen ad-hoc commands.

use crate::config::{Config, CONFIG_FILE};
use crate::error::Result;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct DoctorReport {
  pub checks: Vec<Check>,
}

impl DoctorReport {
  pub fn new() -> Self {
    Self::default()
  }

  /// Highest severity present in the report.
  pub fn severity(&self) -> Severity {
    let mut s = Severity::Ok;
    for c in &self.checks {
      match c.status {
        CheckStatus::Failed => return Severity::Failed,
        CheckStatus::Warning if s == Severity::Ok => s = Severity::Warning,
        _ => {}
      }
    }
    s
  }

  /// Process exit code derived from `severity()`:
  /// `0` = all green, `1` = at least one warning, `2` = at least one failure.
  /// Suitable for wiring into CI / pre-commit.
  pub fn exit_code(&self) -> i32 {
    match self.severity() {
      Severity::Ok => 0,
      Severity::Warning => 1,
      Severity::Failed => 2,
    }
  }
}

#[derive(Debug, Clone)]
pub struct Check {
  pub name: String,
  pub status: CheckStatus,
  pub detail: String,
  /// One-line user-facing remediation, displayed under the check when set.
  pub fix_hint: Option<String>,
}

impl Check {
  pub fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      status: CheckStatus::Ok,
      detail: detail.into(),
      fix_hint: None,
    }
  }

  pub fn warning(name: impl Into<String>, detail: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      status: CheckStatus::Warning,
      detail: detail.into(),
      fix_hint: None,
    }
  }

  pub fn failed(name: impl Into<String>, detail: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      status: CheckStatus::Failed,
      detail: detail.into(),
      fix_hint: None,
    }
  }

  pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
    self.fix_hint = Some(hint.into());
    self
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
  Ok,
  Warning,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
  Ok,
  Warning,
  Failed,
}

pub struct DoctorCtx<'a> {
  pub repo_workdir: &'a Path,
  pub repo: &'a git2::Repository,
  pub config: &'a Config,
}

pub fn run(ctx: &DoctorCtx<'_>) -> Result<DoctorReport> {
  let mut report = DoctorReport::new();
  report.checks.push(check_config_parses(ctx));
  Ok(report)
}

/// Check #1: `.gwm.toml` parses cleanly. Missing config is fine — defaults
/// are documented and identical to what `gwm init` writes out. Invalid TOML
/// is a hard failure since it would crash every other subcommand.
fn check_config_parses(ctx: &DoctorCtx<'_>) -> Check {
  let path = ctx.repo_workdir.join(CONFIG_FILE);
  let name = ".gwm.toml parses";

  if !path.exists() {
    return Check::ok(name, "no .gwm.toml present — defaults assumed");
  }

  let raw = match std::fs::read_to_string(&path) {
    Ok(s) => s,
    Err(e) => {
      return Check::failed(name, format!("could not read {}: {}", path.display(), e));
    }
  };

  match toml::from_str::<Config>(&raw) {
    Ok(_) => Check::ok(name, format!("{} parses cleanly", path.display())),
    Err(e) => Check::failed(name, format!("invalid TOML in {}: {}", path.display(), e))
      .with_hint("fix the syntax or back it up and re-run `gwm init`"),
  }
}
