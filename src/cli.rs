use crate::bootstrap::{self, BootstrapCtx, StepStatus};
use crate::config::Config;
use crate::error::{GwmError, Result};
use crate::naming::{BranchSpec, BRANCH_TYPES};
use crate::worktree;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gwm", version, about = "git worktree manager (TUI + CLI)")]
pub struct Cli {
  #[command(subcommand)]
  pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
  /// Write a default .gwm.toml to the current repo.
  Init,
  /// List worktrees in the current repo.
  List,
  /// Create a new worktree (and matching branch).
  Create {
    /// Branch type (feat, fix, hotfix, docs, test, refactor, chore, perf, ci, build).
    #[arg()]
    branch_type: String,
    /// Issue number (digits only).
    #[arg()]
    issue: String,
    /// Short description (kebab-case, will be normalized).
    #[arg()]
    desc: String,
    /// Skip bootstrap after creation.
    #[arg(long)]
    no_bootstrap: bool,
  },
  /// Remove a worktree by fuzzy name match.
  Remove {
    pattern: String,
    /// Also delete the branch.
    #[arg(long)]
    delete_branch: bool,
  },
  /// Print the on-disk path of a worktree (use `$(gwm path …)` to cd into it).
  Path { pattern: String },
  /// Re-run bootstrap on an existing worktree.
  Bootstrap {
    /// Worktree path or name; defaults to CWD.
    target: Option<String>,
  },
  /// Prune stale worktree references (admin files without a working dir).
  Prune,
  /// List the supported branch types.
  Types,
}

pub fn run(cli: Cli) -> Result<()> {
  // Without a subcommand, we hand off to the TUI.
  let Some(cmd) = cli.command else {
    return crate::tui::run();
  };

  match cmd {
    Command::Init => cmd_init(),
    Command::List => cmd_list(),
    Command::Create {
      branch_type,
      issue,
      desc,
      no_bootstrap,
    } => cmd_create(branch_type, issue, desc, no_bootstrap),
    Command::Remove { pattern, delete_branch } => cmd_remove(pattern, delete_branch),
    Command::Path { pattern } => cmd_path(pattern),
    Command::Bootstrap { target } => cmd_bootstrap(target),
    Command::Prune => cmd_prune(),
    Command::Types => cmd_types(),
  }
}

fn cmd_init() -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?;
  let path = Config::write_default(workdir)?;
  println!("wrote {}", path.display());
  Ok(())
}

fn cmd_list() -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let trees = worktree::list(&repo)?;
  println!("{:<32} {:<40} PATH", "NAME", "BRANCH");
  for w in trees {
    let mark = if w.is_main { "*" } else { " " };
    let branch = w.branch.clone().unwrap_or_else(|| "-".into());
    let locked = if w.is_locked { " [locked]" } else { "" };
    let prunable = if w.is_prunable { " [prunable]" } else { "" };
    println!(
      "{}{:<31} {:<40} {}{}{}",
      mark,
      w.name,
      branch,
      w.path.display(),
      locked,
      prunable
    );
  }
  Ok(())
}

fn cmd_create(branch_type: String, issue: String, desc: String, no_bootstrap: bool) -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
  let repo_name = worktree::repo_name(&repo);

  let config = Config::load_for_repo(&workdir)?;
  let spec = BranchSpec::new(branch_type, issue, desc)?;
  let branch = spec.branch_name(&config.worktree, &repo_name)?;
  let dirname = spec.worktree_dirname(&config.worktree, &repo_name)?;
  let target = spec.worktree_path(&config.worktree, &repo_name)?;

  println!("creating worktree:");
  println!("  branch : {}", branch);
  println!("  dir    : {}", dirname);
  println!("  path   : {}", target.display());

  let created = worktree::add(&repo, &dirname, &target, &branch)?;
  println!("✓ worktree created at {}", created.display());

  if no_bootstrap {
    println!("(skipped bootstrap)");
    return Ok(());
  }

  let ctx = BootstrapCtx {
    main_repo: &workdir,
    worktree: &created,
    config: &config,
  };
  let report = bootstrap::run(&ctx)?;
  print_report(&report);
  Ok(())
}

fn cmd_remove(pattern: String, delete_branch: bool) -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let found = worktree::find_fuzzy(&repo, &pattern)?;
  worktree::remove(&repo, &found.name, delete_branch)?;
  println!("✓ removed {} ({})", found.name, found.path.display());
  if delete_branch {
    if let Some(b) = &found.branch {
      println!("  branch {} deleted", b);
    }
  }
  Ok(())
}

fn cmd_path(pattern: String) -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let found = worktree::find_fuzzy(&repo, &pattern)?;
  println!("{}", found.path.display());
  Ok(())
}

fn cmd_bootstrap(target: Option<String>) -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
  let config = Config::load_for_repo(&workdir)?;

  let worktree_path: PathBuf = match target {
    Some(t) => {
      let p = PathBuf::from(&t);
      if p.is_dir() {
        p
      } else {
        worktree::find_fuzzy(&repo, &t)?.path
      }
    }
    None => std::env::current_dir()?,
  };

  let ctx = BootstrapCtx {
    main_repo: &workdir,
    worktree: &worktree_path,
    config: &config,
  };
  let report = bootstrap::run(&ctx)?;
  print_report(&report);
  Ok(())
}

fn cmd_prune() -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let n = worktree::prune(&repo)?;
  println!("pruned {} stale worktree(s)", n);
  Ok(())
}

fn cmd_types() -> Result<()> {
  for (t, d) in BRANCH_TYPES {
    println!("  {:<10} {}", t, d);
  }
  Ok(())
}

fn print_report(report: &bootstrap::BootstrapReport) {
  println!();
  println!("bootstrap report:");
  for s in &report.steps {
    let sigil = match s.status {
      StepStatus::Ok => "✓",
      StepStatus::Skipped => "·",
      StepStatus::Warning => "!",
      StepStatus::Failed => "✗",
    };
    println!("  {} {}", sigil, s.label);
    if !s.detail.is_empty() {
      for line in s.detail.lines() {
        println!("      {}", line);
      }
    }
  }
}
