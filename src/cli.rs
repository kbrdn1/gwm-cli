use crate::bootstrap::{self, BootstrapCtx, StepStatus};
use crate::config::Config;
use crate::doctor::{self, CheckStatus, DoctorCtx};
use crate::error::{GwmError, Result};
use crate::multiplexer::{
  build_tmux_command, build_zellij_command, detect_tmux, detect_zellij, Multiplexer, SpawnMode,
};
use crate::naming::{BranchSpec, BRANCH_TYPES};
use crate::worktree;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use std::io;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gwm", version, about = "git worktree manager (TUI + CLI)")]
pub struct Cli {
  #[command(subcommand)]
  pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListFormat {
  /// Human-readable table (default).
  Table,
  /// One worktree name per line — suitable for shell completion.
  Names,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InitShell {
  Bash,
  Zsh,
  Fish,
  Powershell,
}

#[derive(Debug, Subcommand)]
pub enum Command {
  /// Write a default .gwm.toml to the current repo.
  Init,
  /// List worktrees in the current repo.
  List {
    /// Output format. `names` prints one worktree name per line (for shell completion).
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    format: ListFormat,
  },
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
  ///
  /// Also available as `gwm cd <pattern>` — same semantics, framed for the
  /// cd flow. Pair with `gwm shell-init <shell>` for a one-line wrapper.
  #[command(visible_alias = "cd")]
  Path { pattern: String },
  /// Re-run bootstrap on an existing worktree.
  Bootstrap {
    /// Worktree path or name; defaults to CWD.
    target: Option<String>,
  },
  /// Prune stale worktree references (admin files without a working dir).
  Prune,
  /// Diagnose the gwm setup (config, env, worktree state).
  ///
  /// Exit code 0 if all green, 1 if any warning, 2 if any failure —
  /// suitable for CI / pre-commit hooks.
  Doctor,
  /// List the supported branch types.
  Types,
  /// Generate a shell completion script on stdout.
  ///
  /// Install (zsh):  `gwm completions zsh > $fpath[1]/_gwm`
  /// Install (bash): `gwm completions bash > /etc/bash_completion.d/gwm`
  /// Install (fish): `gwm completions fish > ~/.config/fish/completions/gwm.fish`
  Completions {
    /// Target shell.
    #[arg(value_enum)]
    shell: Shell,
  },
  /// Print a shell wrapper exposing `gcd <pattern>` (one-line cd into a worktree).
  ///
  /// Install (zsh):        `echo 'eval "$(gwm shell-init zsh)"' >> ~/.zshrc`
  /// Install (bash):       `echo 'eval "$(gwm shell-init bash)"' >> ~/.bashrc`
  /// Install (fish):       `gwm shell-init fish | source` (also add to config.fish)
  /// Install (powershell): `Invoke-Expression (& gwm shell-init powershell | Out-String)`
  ShellInit {
    /// Target shell.
    #[arg(value_enum)]
    shell: InitShell,
  },
  /// Open an interactive picker; print the chosen worktree's path on stdout.
  ///
  /// Same TUI as `gwm` itself, minus the create / delete / bootstrap actions.
  /// The fuzzy filter bar opens immediately so typing narrows the list right
  /// away. Press Enter to commit the highlighted pick; Esc / Ctrl-C / `q`
  /// quits without printing anything (exit code 1).
  ///
  /// Typically invoked via `gcd` (no arg) from the bundled `gwm shell-init`
  /// wrapper, which cd's into the picked worktree in one keystroke. The raw
  /// form is `cd "$(gwm switch)"` (or `gwm s`, the alias).
  #[command(visible_alias = "s")]
  Switch,
  /// Open the matched worktree in a new tmux window (current session).
  ///
  /// Requires `$TMUX` to be set — i.e. gwm must be invoked from inside an
  /// existing tmux session. Outside a tmux session the command exits
  /// non-zero with a clear error rather than spawning a stray server.
  /// Use `--split` to open in a horizontal split of the current pane
  /// instead of a new window.
  Tmux {
    /// Fuzzy worktree name pattern (same matcher as `gwm path / remove`).
    pattern: String,
    /// Split the current pane instead of opening a new window.
    #[arg(short = 'p', long = "split")]
    split: bool,
  },
  /// Open the matched worktree in a new zellij tab (current session).
  ///
  /// Requires `$ZELLIJ` to be set. `--cwd` on `zellij action new-tab`
  /// needs zellij ≥ 0.40. Use `--split` to open in a new pane of the
  /// current tab instead of a new tab.
  Zellij {
    /// Fuzzy worktree name pattern (same matcher as `gwm path / remove`).
    pattern: String,
    /// Split the current tab into a new pane instead of opening a new tab.
    #[arg(short = 'p', long = "split")]
    split: bool,
  },
}

pub fn run(cli: Cli) -> Result<()> {
  // Without a subcommand, we hand off to the TUI.
  let Some(cmd) = cli.command else {
    return crate::tui::run();
  };

  match cmd {
    Command::Init => cmd_init(),
    Command::List { format } => cmd_list(format),
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
    Command::Doctor => cmd_doctor(),
    Command::Types => cmd_types(),
    Command::Completions { shell } => cmd_completions(shell),
    Command::ShellInit { shell } => cmd_shell_init(shell),
    Command::Switch => cmd_switch(),
    Command::Tmux { pattern, split } => cmd_multiplexer(Multiplexer::Tmux, pattern, split),
    Command::Zellij { pattern, split } => cmd_multiplexer(Multiplexer::Zellij, pattern, split),
  }
}

fn cmd_init() -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?;
  let path = Config::write_default(workdir)?;
  println!("wrote {}", path.display());
  Ok(())
}

fn cmd_list(format: ListFormat) -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let trees = worktree::list(&repo)?;

  if format == ListFormat::Names {
    // Mirror `worktree::find_fuzzy`, which excludes the main workdir:
    // emitting its name here would suggest a completion candidate that
    // `path` / `remove` / `bootstrap` can never accept.
    for w in trees.iter().filter(|w| !w.is_main) {
      println!("{}", w.name);
    }
    return Ok(());
  }

  // Dynamic widths based on observed content.
  let name_w = trees.iter().map(|w| w.name.len()).max().unwrap_or(4).clamp(4, 40);
  let branch_w = trees
    .iter()
    .map(|w| w.branch.as_deref().unwrap_or("-").len())
    .max()
    .unwrap_or(6)
    .clamp(6, 40);
  let status_w = 14;

  println!(
    "  {:<nw$}  {:<bw$}  {:<sw$}  PATH",
    "NAME",
    "BRANCH",
    "STATUS",
    nw = name_w,
    bw = branch_w,
    sw = status_w,
  );
  for w in trees {
    let mark = if w.is_main { "*" } else { " " };
    let branch = w.branch.clone().unwrap_or_else(|| "-".into());
    let status = format_status_text(&w);
    println!(
      "{} {:<nw$}  {:<bw$}  {:<sw$}  {}",
      mark,
      w.name,
      branch,
      status,
      w.path.display(),
      nw = name_w,
      bw = branch_w,
      sw = status_w,
    );
  }
  Ok(())
}

fn format_status_text(w: &worktree::WorktreeInfo) -> String {
  if w.is_prunable {
    return "prunable".into();
  }
  if w.is_locked {
    return "locked".into();
  }
  let s = &w.status;
  if s.unknown {
    return "unknown".into();
  }
  let mut parts: Vec<String> = Vec::new();
  if s.is_dirty {
    parts.push("● dirty".into());
  }
  if s.has_upstream {
    if s.ahead > 0 {
      parts.push(format!("↑{}", s.ahead));
    }
    if s.behind > 0 {
      parts.push(format!("↓{}", s.behind));
    }
    if !s.is_dirty && s.synced() {
      parts.push("✓ synced".into());
    }
  } else if !s.is_dirty {
    parts.push("clean".into());
  }
  parts.join(" ")
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

fn cmd_doctor() -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
  let config = Config::load_for_repo(&workdir).unwrap_or_default();

  let ctx = DoctorCtx {
    repo_workdir: &workdir,
    repo: &repo,
    config: &config,
  };
  let report = doctor::run(&ctx)?;
  print_doctor_report(&report);

  let code = report.exit_code();
  if code != 0 {
    std::process::exit(code);
  }
  Ok(())
}

fn print_doctor_report(report: &doctor::DoctorReport) {
  for c in &report.checks {
    let sigil = match c.status {
      CheckStatus::Ok => "✓",
      CheckStatus::Warning => "!",
      CheckStatus::Failed => "✗",
    };
    println!("{} {}", sigil, c.name);
    if !c.detail.is_empty() {
      println!("    {}", c.detail);
    }
    if let Some(hint) = &c.fix_hint {
      println!("    → {}", hint);
    }
  }
}

fn cmd_types() -> Result<()> {
  for (t, d) in BRANCH_TYPES {
    println!("  {:<10} {}", t, d);
  }
  Ok(())
}

fn cmd_completions(shell: Shell) -> Result<()> {
  let mut cmd = Cli::command();
  let name = cmd.get_name().to_string();
  generate(shell, &mut cmd, name, &mut io::stdout());
  Ok(())
}

fn cmd_shell_init(shell: InitShell) -> Result<()> {
  print!("{}", shell_init_script(shell));
  Ok(())
}

/// `gwm switch` — open the TUI picker and emit the chosen worktree's path
/// on stdout. Returning a non-zero exit code when the user cancels lets the
/// shell wrapper (`gcd` in `shell-init`) skip the `cd` instead of cd'ing to
/// an empty argument.
///
/// The git-repo check runs before `tui::run_picker()` to keep the error
/// path identical to every other repo-bound subcommand (clean stderr,
/// no flicker into the alternate screen).
fn cmd_switch() -> Result<()> {
  // Probe the repo first; this is also what surfaces "not inside a git
  // repository" before we touch the terminal. Discarding the handle is
  // fine — `run_picker` re-discovers it via its own `App::new_picker_at`.
  let _ = worktree::discover_repo(None)?;
  match crate::tui::run_picker()? {
    Some(path) => {
      println!("{}", path.display());
      Ok(())
    }
    None => std::process::exit(1),
  }
}

/// `gwm tmux <pattern>` / `gwm zellij <pattern>` — open the matched
/// worktree in a new window/tab (or split with `--split`). The handler
/// is shared between the two multiplexers because the only difference
/// is the argv shape, already encoded in `multiplexer::build_*_command`.
///
/// Error contract (ordered, first match wins):
///   1. Not inside a git repo → `NotInGitRepo`.
///   2. Multiplexer not running → `Other("<bin> session not running …")`.
///   3. Worktree pattern doesn't match → `WorktreeNotFound`.
///   4. Spawn or non-zero exit from the multiplexer → `CommandFailed`.
///
/// Ordering #1 before #2 matches `gwm cd` / `gwm switch`: the repo gate
/// is the more fundamental problem, so we surface it first.
fn cmd_multiplexer(mux: Multiplexer, pattern: String, split: bool) -> Result<()> {
  let repo = worktree::discover_repo(None)?;

  let env_name = match mux {
    Multiplexer::Tmux => "TMUX",
    Multiplexer::Zellij => "ZELLIJ",
  };
  let env_value = std::env::var(env_name).ok();
  let running = match mux {
    Multiplexer::Tmux => detect_tmux(env_value),
    Multiplexer::Zellij => detect_zellij(env_value),
  };
  if !running {
    return Err(GwmError::Other(format!(
      "{0} session not running (\\${1} unset) — run `gwm {0} <pattern>` from inside a {0} session",
      mux.binary(),
      env_name,
    )));
  }

  let found = worktree::find_fuzzy(&repo, &pattern)?;
  let mode = if split { SpawnMode::Split } else { SpawnMode::Window };
  let argv = match mux {
    Multiplexer::Tmux => build_tmux_command(&found.name, &found.path, mode),
    Multiplexer::Zellij => build_zellij_command(&found.name, &found.path, mode),
  };
  spawn_multiplexer(mux, &argv)
}

/// Spawn the multiplexer command and surface its exit status. argv[0] is
/// the binary; argv[1..] are the args. Matches `tui::mod::run_lazygit`
/// in shape — `.status()` so the user sees the child's own stderr live
/// instead of swallowing it into a buffered `CommandFailed`.
fn spawn_multiplexer(mux: Multiplexer, argv: &[String]) -> Result<()> {
  let (bin, rest) = argv.split_first().ok_or_else(|| {
    GwmError::Other(format!(
      "empty argv for {} spawn (build_*_command returned [])",
      mux.binary()
    ))
  })?;
  // The data string already names the binary (`tmux` / `zellij`), so
  // the rendered message reads `command failed: tmux exited with
  // status Some(1)` — attributable to the verb the user typed.
  let status = std::process::Command::new(bin)
    .args(rest)
    .status()
    .map_err(|e| GwmError::CommandFailed(format!("could not spawn {}: {}", bin, e)))?;
  if !status.success() {
    return Err(GwmError::CommandFailed(format!(
      "{} exited with status {:?}",
      bin,
      status.code()
    )));
  }
  Ok(())
}

pub fn shell_init_script(shell: InitShell) -> &'static str {
  match shell {
    InitShell::Bash | InitShell::Zsh => POSIX_SHELL_INIT,
    InitShell::Fish => FISH_SHELL_INIT,
    InitShell::Powershell => POWERSHELL_SHELL_INIT,
  }
}

const POSIX_SHELL_INIT: &str = r#"# gwm shell helper — wraps `gwm cd` / `gwm switch` so the parent shell can cd.
# Install: eval "$(gwm shell-init bash)"   # or zsh
#
# Two paths:
#   gcd <pattern>        # fuzzy resolve via `gwm cd <pattern>`, then cd
#   gcd                  # no arg → opens the interactive picker via `gwm switch`, then cd
#
# Note: the `function name { ... }` form (zsh/bash-extended) is used instead
# of the parenthesised POSIX form so the parser does not error out with
# `defining function based on alias 'gcd'` when zsh already has a `gcd`
# alias (e.g. oh-my-zsh's `gcd=git checkout`). The `unalias` after the
# definition is what makes the function reachable at call time, since zsh
# still resolves the alias first when both exist.
function gcd {
  local target
  if [ "$#" -eq 0 ]; then
    # No arg → open the interactive picker. `gwm switch` exits non-zero on
    # cancel, in which case `gcd` must NOT attempt the `cd` (would land in $HOME).
    target="$(command gwm switch)" || return $?
  else
    target="$(command gwm cd "$@")" || return $?
  fi
  cd "$target" || return $?
}
unalias gcd 2>/dev/null || true
"#;

const FISH_SHELL_INIT: &str = r#"# gwm shell helper — wraps `gwm cd` / `gwm switch` so the parent shell can cd.
# Install: gwm shell-init fish | source   # then persist in ~/.config/fish/config.fish
#
# Two paths:
#   gcd <pattern>        # fuzzy resolve via `gwm cd <pattern>`, then cd
#   gcd                  # no arg → opens the interactive picker via `gwm switch`, then cd
function gcd --description 'cd into a gwm worktree (no arg = interactive picker)'
  set -l target
  if test (count $argv) -eq 0
    # No arg → open the interactive picker; cancel exits non-zero, in which
    # case we must NOT attempt the cd (would land in $HOME).
    set target (command gwm switch)
    or return $status
  else
    set target (command gwm cd $argv)
    or return $status
  end
  # `--` stops option parsing, "$target" prevents wildcard expansion on
  # paths containing `[`, `]`, or `*`.
  cd -- "$target"
end
"#;

const POWERSHELL_SHELL_INIT: &str = r#"# gwm shell helper — wraps `gwm cd` / `gwm switch` so the parent shell can cd.
# Install: Invoke-Expression (& gwm shell-init powershell | Out-String)
#
# Two paths:
#   gcd <pattern>        # fuzzy resolve via `gwm cd <pattern>`, then Set-Location
#   gcd                  # no arg → opens the interactive picker via `gwm switch`, then Set-Location
#
# Note: this clears any prior `gcd` alias so the function takes effect.
Remove-Alias -Name gcd -Force -ErrorAction SilentlyContinue
function gcd {
  param([string]$Pattern)
  if ([string]::IsNullOrEmpty($Pattern)) {
    # No arg → open the interactive picker. The binary exits non-zero on
    # cancel; bail out before attempting Set-Location so we don't land in $HOME.
    $target = & gwm switch
  } else {
    $target = & gwm cd $Pattern
  }
  if ($LASTEXITCODE -ne 0) { return }
  Set-Location $target
}
"#;

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
