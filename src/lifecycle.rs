use crate::bootstrap::{self, BootstrapReport, StepResult};
use crate::config::{Config, HookOnFail, HookStep};
use crate::error::{GwmError, Result};
use crate::github;
use crate::naming::BranchSpec;
use git2::Repository;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPhase {
  PreCreate,
  PostCreate,
  PreBootstrap,
  PostBootstrap,
  PreRemove,
  PostRemove,
}

impl HookPhase {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::PreCreate => "pre_create",
      Self::PostCreate => "post_create",
      Self::PreBootstrap => "pre_bootstrap",
      Self::PostBootstrap => "post_bootstrap",
      Self::PreRemove => "pre_remove",
      Self::PostRemove => "post_remove",
    }
  }

  fn parse(value: &str) -> Option<Self> {
    match value {
      "pre_create" => Some(Self::PreCreate),
      "post_create" => Some(Self::PostCreate),
      "pre_bootstrap" => Some(Self::PreBootstrap),
      "post_bootstrap" => Some(Self::PostBootstrap),
      "pre_remove" => Some(Self::PreRemove),
      "post_remove" => Some(Self::PostRemove),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, Default)]
pub struct HookSkips {
  phases: HashSet<HookPhase>,
}

impl HookSkips {
  pub fn parse(raw: Option<&str>) -> Result<Self> {
    let mut phases = HashSet::new();
    let Some(raw) = raw else {
      return Ok(Self { phases });
    };
    for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
      let phase = HookPhase::parse(part).ok_or_else(|| {
        GwmError::Config(format!(
          "unknown hook phase '{}' in --skip-hooks (expected one of pre_create,post_create,pre_bootstrap,post_bootstrap,pre_remove,post_remove)",
          part
        ))
      })?;
      phases.insert(phase);
    }
    Ok(Self { phases })
  }

  pub fn with(mut self, phase: HookPhase) -> Self {
    self.phases.insert(phase);
    self
  }

  fn contains(&self, phase: HookPhase) -> bool {
    self.phases.contains(&phase)
  }
}

#[derive(Debug, Clone)]
pub struct HookContext {
  pub main_repo: PathBuf,
  pub cwd: PathBuf,
  pub path: PathBuf,
  pub branch: String,
  pub branch_type: String,
  pub issue: String,
  pub desc: String,
  pub user: String,
  pub owner: String,
  pub repo: String,
}

impl HookContext {
  pub fn for_create(
    repo: &Repository,
    main_repo: &Path,
    cwd: &Path,
    path: &Path,
    branch: &str,
    spec: &BranchSpec,
  ) -> Self {
    let meta = RepoMeta::from_repo(repo);
    Self {
      main_repo: main_repo.to_path_buf(),
      cwd: cwd.to_path_buf(),
      path: path.to_path_buf(),
      branch: branch.to_string(),
      branch_type: spec.type_.clone(),
      issue: spec.issue.clone(),
      desc: spec.desc.clone(),
      user: git_user(repo),
      owner: meta.owner,
      repo: meta.repo,
    }
  }

  pub fn for_worktree(repo: &Repository, main_repo: &Path, cwd: &Path, path: &Path, branch: Option<&str>) -> Self {
    let meta = RepoMeta::from_repo(repo);
    let parsed = branch.and_then(crate::naming::parse_branch);
    Self {
      main_repo: main_repo.to_path_buf(),
      cwd: cwd.to_path_buf(),
      path: path.to_path_buf(),
      branch: branch.unwrap_or_default().to_string(),
      branch_type: parsed.as_ref().map(|s| s.type_.clone()).unwrap_or_default(),
      issue: parsed.as_ref().map(|s| s.issue.clone()).unwrap_or_default(),
      desc: parsed.as_ref().map(|s| s.desc.clone()).unwrap_or_default(),
      user: git_user(repo),
      owner: meta.owner,
      repo: meta.repo,
    }
  }

  pub fn with_cwd(&self, cwd: &Path) -> Self {
    let mut next = self.clone();
    next.cwd = cwd.to_path_buf();
    next
  }
}

#[derive(Debug)]
pub struct HookAbort {
  pub phase: HookPhase,
  pub step: String,
  pub detail: String,
}

pub fn run_phase(
  config: &Config,
  phase: HookPhase,
  ctx: &HookContext,
  skips: &HookSkips,
  include_legacy_post_create: bool,
) -> Result<BootstrapReport> {
  let mut report = BootstrapReport { steps: Vec::new() };
  if skips.contains(phase) {
    report.steps.push(StepResult::skipped(
      format!("[{}] hooks", phase.as_str()),
      "skipped by --skip-hooks",
    ));
    return Ok(report);
  }

  let mut steps = steps_for(config, phase);
  if phase == HookPhase::PostCreate && include_legacy_post_create {
    steps.extend(config.bootstrap.command.iter().cloned().map(HookStep::from));
  }

  for step in steps {
    let label = format!("[{}] {}", phase.as_str(), step.name);
    if let Some(ref guard) = step.when {
      if !bootstrap::evaluate_when(guard, &ctx.cwd) {
        report
          .steps
          .push(StepResult::skipped(label, format!("when condition '{}' false", guard)));
        continue;
      }
    }

    match run_step(&step, ctx) {
      Ok(output) => report
        .steps
        .push(StepResult::ok_with_detail(label, bootstrap::trailing_lines(&output, 3))),
      Err(detail) => match step.on_fail {
        HookOnFail::Abort => {
          report.steps.push(StepResult::failed(label, detail.clone()));
          print_report(&report);
          return Err(GwmError::CommandFailed(format!(
            "hook {} '{}' failed: {}",
            phase.as_str(),
            step.name,
            detail
          )));
        }
        HookOnFail::Warn => report.steps.push(StepResult::warning(label, detail)),
        HookOnFail::Ignore => report
          .steps
          .push(StepResult::skipped(label, format!("ignored failure: {}", detail))),
      },
    }
  }

  Ok(report)
}

pub fn print_report(report: &BootstrapReport) {
  for s in &report.steps {
    let sigil = s.status.sigil();
    println!("  {} {}", sigil, s.label);
    if !s.detail.is_empty() {
      for line in s.detail.lines() {
        println!("      {}", line);
      }
    }
  }
}

fn steps_for(config: &Config, phase: HookPhase) -> Vec<HookStep> {
  match phase {
    HookPhase::PreCreate => config.hooks.pre_create.clone(),
    HookPhase::PostCreate => config.hooks.post_create.clone(),
    HookPhase::PreBootstrap => config.hooks.pre_bootstrap.clone(),
    HookPhase::PostBootstrap => config.hooks.post_bootstrap.clone(),
    HookPhase::PreRemove => config.hooks.pre_remove.clone(),
    HookPhase::PostRemove => config.hooks.post_remove.clone(),
  }
}

fn run_step(step: &HookStep, ctx: &HookContext) -> std::result::Result<String, String> {
  let run = expand_placeholders(&step.run, ctx);
  let env = step
    .env
    .iter()
    .map(|(key, value)| (key.clone(), expand_placeholders(value, ctx)))
    .collect::<HashMap<_, _>>();
  let mut cmd = Command::new("sh");
  cmd.arg("-c").arg(run).current_dir(&ctx.cwd);
  for (key, value) in env {
    cmd.env(key, value);
  }
  let out = cmd.output().map_err(|e| format!("failed to spawn: {}", e))?;
  let stdout = String::from_utf8_lossy(&out.stdout).to_string();
  let stderr = String::from_utf8_lossy(&out.stderr).to_string();
  let detail = if stdout.is_empty() { stderr } else { stdout };
  if !out.status.success() {
    return Err(format!("exited with {}\n{}", out.status, detail).trim().to_string());
  }
  Ok(detail)
}

fn expand_placeholders(template: &str, ctx: &HookContext) -> String {
  template
    .replace("{branch}", &ctx.branch)
    .replace("{path}", &ctx.path.display().to_string())
    .replace("{type}", &ctx.branch_type)
    .replace("{issue}", &ctx.issue)
    .replace("{desc}", &ctx.desc)
    .replace("{user}", &ctx.user)
    .replace("{owner}", &ctx.owner)
    .replace("{repo}", &ctx.repo)
}

fn git_user(repo: &Repository) -> String {
  repo
    .config()
    .ok()
    .and_then(|cfg| cfg.get_string("user.name").ok())
    .filter(|value| !value.trim().is_empty())
    .or_else(|| std::env::var("USER").ok())
    .unwrap_or_default()
}

struct RepoMeta {
  owner: String,
  repo: String,
}

impl RepoMeta {
  fn from_repo(repo: &Repository) -> Self {
    let repo_name = crate::worktree::repo_name(repo);
    let Ok(slug) = github::repo_slug(repo) else {
      return Self {
        owner: String::new(),
        repo: repo_name,
      };
    };
    let Some((owner, name)) = slug.split_once('/') else {
      return Self {
        owner: String::new(),
        repo: repo_name,
      };
    };
    Self {
      owner: owner.to_string(),
      repo: name.to_string(),
    }
  }
}
