use crate::config::{BootstrapConfig, CommandStep, Config, CopyStep, Guard, NoSymlink};
use crate::error::{GwmError, Result};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct BootstrapReport {
  pub steps: Vec<StepResult>,
}

#[derive(Debug, Clone)]
pub struct StepResult {
  pub label: String,
  pub status: StepStatus,
  pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
  Ok,
  Skipped,
  Warning,
  Failed,
}

pub struct BootstrapCtx<'a> {
  pub main_repo: &'a Path,
  pub worktree: &'a Path,
  pub config: &'a Config,
}

pub fn run(ctx: &BootstrapCtx<'_>) -> Result<BootstrapReport> {
  let mut report = BootstrapReport { steps: Vec::new() };
  let bs = &ctx.config.bootstrap;

  run_copies(ctx, bs, &mut report);
  run_no_symlinks(ctx, bs, &mut report);
  run_commands(ctx, bs, &mut report);

  Ok(report)
}

fn run_copies(ctx: &BootstrapCtx<'_>, bs: &BootstrapConfig, report: &mut BootstrapReport) {
  for step in &bs.copy {
    let label = format!("copy {} -> {}", step.from, step.to);
    let src = ctx.main_repo.join(&step.from);
    let dst = ctx.worktree.join(&step.to);

    if dst.exists() {
      report.steps.push(StepResult {
        label,
        status: StepStatus::Skipped,
        detail: "destination already exists, leaving it alone".into(),
      });
      continue;
    }

    if !src.exists() {
      match resolve_missing(step, bs, &dst) {
        Some(res) => report.steps.push(StepResult { label, ..res }),
        None => {
          let status = if step.required {
            StepStatus::Failed
          } else {
            StepStatus::Skipped
          };
          let detail = if step.required {
            "required source missing".into()
          } else {
            "optional source missing".into()
          };
          report.steps.push(StepResult { label, status, detail });
        }
      }
      continue;
    }

    // Run guards before copying.
    if let Some(g) = guard_match(step, bs, &src) {
      handle_guard_match(&g, &src, &dst, ctx, report, &label);
      continue;
    }

    match std::fs::copy(&src, &dst) {
      Ok(_) => report.steps.push(StepResult {
        label,
        status: StepStatus::Ok,
        detail: format!("copied from {}", src.display()),
      }),
      Err(e) => report.steps.push(StepResult {
        label,
        status: StepStatus::Failed,
        detail: format!("copy failed: {}", e),
      }),
    }
  }
}

fn resolve_missing(step: &CopyStep, bs: &BootstrapConfig, dst: &Path) -> Option<StepResult> {
  let mode = step.fallback.as_deref().unwrap_or("skip");
  match mode {
    "inline" => {
      // Find a fallback content keyed by the `to` file basename or step.fallback alias.
      let key = key_from_to(&step.to);
      let fb = bs.fallback.get(&key)?;
      match std::fs::write(dst, &fb.content) {
        Ok(_) => Some(StepResult {
          label: String::new(),
          status: StepStatus::Warning,
          detail: format!("source missing — wrote inline fallback to {}", dst.display()),
        }),
        Err(e) => Some(StepResult {
          label: String::new(),
          status: StepStatus::Failed,
          detail: format!("inline fallback write failed: {}", e),
        }),
      }
    }
    "abort" => Some(StepResult {
      label: String::new(),
      status: StepStatus::Failed,
      detail: "source missing and fallback=abort".into(),
    }),
    _ => None,
  }
}

fn key_from_to(to: &str) -> String {
  // ".env.testing" → "env_testing"
  to.trim_start_matches('.').replace(['.', '-'], "_")
}

fn guard_match(step: &CopyStep, bs: &BootstrapConfig, src: &Path) -> Option<Guard> {
  if step.guards.is_empty() {
    return None;
  }
  let content = std::fs::read_to_string(src).ok()?;
  for guard_name in &step.guards {
    let guard = bs.guard.iter().find(|g| &g.name == guard_name)?;
    for pat in &guard.deny_patterns {
      if let Ok(re) = Regex::new(pat) {
        if re.is_match(&content) {
          return Some(guard.clone());
        }
      }
    }
  }
  None
}

fn handle_guard_match(
  guard: &Guard,
  src: &Path,
  dst: &Path,
  ctx: &BootstrapCtx<'_>,
  report: &mut BootstrapReport,
  label: &str,
) {
  match guard.on_match.as_str() {
    "seed-from-example" => {
      let example_rel = guard.example_file.as_deref().unwrap_or(".env.example");
      let example_src = ctx.main_repo.join(example_rel);
      if example_src.exists() {
        match std::fs::copy(&example_src, dst) {
          Ok(_) => report.steps.push(StepResult {
            label: label.into(),
            status: StepStatus::Warning,
            detail: format!(
              "guard '{}' tripped on {} — seeded {} from {} (edit before use)",
              guard.name,
              src.display(),
              dst.display(),
              example_src.display()
            ),
          }),
          Err(e) => report.steps.push(StepResult {
            label: label.into(),
            status: StepStatus::Failed,
            detail: format!("guard '{}' seed-from-example failed: {}", guard.name, e),
          }),
        }
      } else {
        report.steps.push(StepResult {
          label: label.into(),
          status: StepStatus::Failed,
          detail: format!(
            "guard '{}' tripped and no example_file {} available",
            guard.name,
            example_src.display()
          ),
        });
      }
    }
    _ => {
      // abort
      report.steps.push(StepResult {
        label: label.into(),
        status: StepStatus::Failed,
        detail: format!("guard '{}' tripped on {} — abort", guard.name, src.display()),
      });
    }
  }
}

fn run_no_symlinks(ctx: &BootstrapCtx<'_>, bs: &BootstrapConfig, report: &mut BootstrapReport) {
  for ns in &bs.no_symlink {
    let label = format!("no-symlink {}", ns.path);
    let target: PathBuf = ctx.worktree.join(&ns.path);
    handle_no_symlink(&label, &target, report);
  }
  // Also enforce common defaults if not declared explicitly.
  for default in ["vendor", "node_modules"] {
    if bs.no_symlink.iter().any(|n: &NoSymlink| n.path == default) {
      continue;
    }
    let target = ctx.worktree.join(default);
    if target.is_symlink() {
      handle_no_symlink(&format!("no-symlink {} (auto)", default), &target, report);
    }
  }
}

fn handle_no_symlink(label: &str, target: &Path, report: &mut BootstrapReport) {
  if !target.exists() && !target.is_symlink() {
    report.steps.push(StepResult {
      label: label.into(),
      status: StepStatus::Skipped,
      detail: "not present".into(),
    });
    return;
  }
  if target.is_symlink() {
    match std::fs::remove_file(target) {
      Ok(_) => report.steps.push(StepResult {
        label: label.into(),
        status: StepStatus::Warning,
        detail: format!("removed symlink {}", target.display()),
      }),
      Err(e) => report.steps.push(StepResult {
        label: label.into(),
        status: StepStatus::Failed,
        detail: format!("failed to remove symlink {}: {}", target.display(), e),
      }),
    }
  } else {
    report.steps.push(StepResult {
      label: label.into(),
      status: StepStatus::Ok,
      detail: "real directory, ok".to_string(),
    });
  }
}

fn run_commands(ctx: &BootstrapCtx<'_>, bs: &BootstrapConfig, report: &mut BootstrapReport) {
  for step in &bs.command {
    let label = format!("run {}", step.name);
    if let Some(ref guard) = step.when {
      if !evaluate_when(guard, ctx.worktree) {
        report.steps.push(StepResult {
          label,
          status: StepStatus::Skipped,
          detail: format!("when condition '{}' false", guard),
        });
        continue;
      }
    }
    match exec_shell(step, ctx.worktree) {
      Ok(output) => report.steps.push(StepResult {
        label,
        status: StepStatus::Ok,
        detail: trailing_lines(&output, 3),
      }),
      Err(e) => report.steps.push(StepResult {
        label,
        status: StepStatus::Failed,
        detail: e.to_string(),
      }),
    }
  }
}

/// Evaluate a `[[bootstrap.command]].when` expression against the given
/// worktree. Unknown predicates default to `true` so configs we don't
/// understand still run (the doctor surfaces them as a warning).
pub fn evaluate_when(expr: &str, cwd: &Path) -> bool {
  if let Some(rest) = expr.strip_prefix("file_exists:") {
    return cwd.join(rest.trim()).exists();
  }
  true
}

fn exec_shell(step: &CommandStep, cwd: &Path) -> Result<String> {
  let mut cmd = Command::new("sh");
  cmd.arg("-c").arg(&step.run).current_dir(cwd);
  for (k, v) in &step.env {
    cmd.env(k, v);
  }
  let out = cmd
    .output()
    .map_err(|e| GwmError::CommandFailed(format!("{}: {}", step.name, e)))?;
  let stdout = String::from_utf8_lossy(&out.stdout).to_string();
  let stderr = String::from_utf8_lossy(&out.stderr).to_string();
  if !out.status.success() {
    return Err(GwmError::CommandFailed(format!(
      "{} exited with {}\n{}",
      step.name,
      out.status,
      if stderr.is_empty() { stdout } else { stderr }
    )));
  }
  Ok(if stdout.is_empty() { stderr } else { stdout })
}

fn trailing_lines(s: &str, n: usize) -> String {
  let lines: Vec<&str> = s.lines().collect();
  let start = lines.len().saturating_sub(n);
  lines[start..].join("\n")
}
