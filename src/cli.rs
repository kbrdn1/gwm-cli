use crate::bootstrap::{self, BootstrapCtx, StepStatus};
use crate::config::Config;
use crate::doctor::{self, CheckStatus, DoctorCtx};
use crate::error::{GwmError, LinkKind, Result};
use crate::github::{self, BranchLink, IssueState, IssueStatus, LinkSource, PrState, PrStatus};
use crate::labels::{self, LabelDiff};
use crate::milestones::{self, MilestoneDiff};
use crate::multiplexer::{
  build_tmux_command, build_zellij_command, detect_tmux, detect_zellij, Multiplexer, SpawnMode,
};
use crate::naming::BranchSpec;
use crate::trust::{self, TrustLedger, TrustMode, TrustOutcome};
use crate::worktree;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use git2::Repository;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "gwm", version, about = "git worktree manager (TUI + CLI)")]
pub struct Cli {
  /// Skip the TOFU trust prompt on `.gwm.toml` (issue #95).
  ///
  /// Equivalent to `GWM_ALLOW_BOOTSTRAP=1`. Use in non-interactive
  /// environments (CI runners, scripted workflows) where there is no
  /// human to answer the prompt. Off by default — the threat model is
  /// arbitrary RCE via `[[bootstrap.command]]` lines from an untrusted
  /// remote, so the safe default is "prompt".
  #[arg(long, global = true)]
  pub allow_bootstrap: bool,

  /// Refuse to run `.gwm.toml` bootstrap regardless of trust state
  /// (issue #95). Useful for forensic inspection of an unfamiliar
  /// repo: `gwm bootstrap --deny-bootstrap` short-circuits the
  /// execution path even if the ledger says trusted.
  #[arg(long, global = true, conflicts_with = "allow_bootstrap")]
  pub deny_bootstrap: bool,

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

/// Target of `gwm link / unlink / open` — issue or pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LinkTarget {
  /// GitHub issue.
  Issue,
  /// GitHub pull request.
  Pr,
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
    /// Attach the new worktree to an already-existing local branch of the
    /// same name instead of refusing (issue #99). Off by default — a
    /// pre-existing branch ends `gwm create` with an error naming the
    /// stale tip so the user can audit it.
    #[arg(long)]
    reuse_branch: bool,
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
  /// Link the current (or named) worktree to a GitHub issue or pull request.
  ///
  /// The link is stored in `git config branch.<name>.gwm-issue` (or
  /// `gwm-pr`) — local, per-branch, survives worktree moves. Issue
  /// numbers are auto-detected from the `<type>/#<N>-<slug>` convention
  /// when no explicit override is set; `gwm link issue <N>` overrides
  /// that. PR numbers are not auto-detected; link them explicitly with
  /// `gwm link pr <N>`.
  Link {
    /// What to link: `issue` or `pr`.
    #[arg(value_enum)]
    target: LinkTarget,
    /// Number to link (digits only).
    number: u64,
    /// Optional worktree pattern; defaults to the current worktree (CWD).
    #[arg(long)]
    worktree: Option<String>,
  },
  /// Remove the explicit issue / PR link on the current (or named) worktree.
  ///
  /// After `gwm unlink issue`, auto-detection from the branch name
  /// resurfaces if the branch follows `<type>/#<N>-<slug>`. Idempotent —
  /// safe to run when nothing is linked.
  Unlink {
    /// What to unlink: `issue` or `pr`.
    #[arg(value_enum)]
    target: LinkTarget,
    /// Optional worktree pattern; defaults to the current worktree (CWD).
    #[arg(long)]
    worktree: Option<String>,
  },
  /// Open the linked issue or PR in the browser.
  ///
  /// Uses the OS opener (`open` on macOS, `xdg-open` on Linux,
  /// `explorer` on Windows). Pass `--print-url` to emit the URL on
  /// stdout instead — useful for piping, testing, and headless shells.
  Open {
    /// What to open: `issue` or `pr`.
    #[arg(value_enum)]
    target: LinkTarget,
    /// Optional worktree pattern; defaults to the current worktree (CWD).
    #[arg(long)]
    worktree: Option<String>,
    /// Print the URL on stdout instead of spawning the browser.
    #[arg(long)]
    print_url: bool,
  },
  /// Show the issue / PR link and (when `gh` is available) live GitHub status.
  ///
  /// Shells out to `gh issue view` and `gh pr view` to fetch state, title,
  /// labels, and CI rollup. Without `gh` (or outside a GitHub repo), prints
  /// only the local link. `--json` emits a stable schema for scripting.
  Status {
    /// Optional worktree pattern; defaults to the current worktree (CWD).
    #[arg(long)]
    worktree: Option<String>,
    /// Emit JSON instead of the human-readable summary.
    #[arg(long)]
    json: bool,
  },
  /// Manage the declarative GitHub label set from `.gwm.toml` (issue #81).
  ///
  /// Declares the desired label set under `[[labels]]` in `.gwm.toml`,
  /// then pushes it to the upstream `origin` remote via `gh label
  /// create --force`. Without a `[[labels]]` block, both subcommands
  /// are no-ops (`0 labels declared, nothing to push`).
  Labels {
    #[command(subcommand)]
    action: LabelsAction,
  },
  /// Manage the declarative GitHub milestone set from `.gwm.toml` (issue #82).
  ///
  /// Declares the desired milestone set under `[[milestones]]` in
  /// `.gwm.toml`, then pushes it to the upstream `origin` remote via
  /// `gh api repos/:owner/:repo/milestones` (no native `gh milestone`
  /// subcommand exists). Without a `[[milestones]]` block, both
  /// subcommands are no-ops (`0 milestones declared, nothing to push`).
  Milestones {
    #[command(subcommand)]
    action: MilestonesAction,
  },
  /// Manage the TOFU trust ledger for `.gwm.toml` files (issue #95).
  ///
  /// `gwm` runs `[[bootstrap.command]]` lines from `.gwm.toml` under
  /// the user's privileges — equivalent to `curl … | sh` against the
  /// repo author. The trust ledger at `~/.config/gwm/trust.toml`
  /// (override via `$GWM_TRUST_LEDGER`) records the `(origin URL,
  /// sha256 of .gwm.toml)` tuples the user has approved, so
  /// subsequent runs skip the prompt. Hash drift (any byte changes
  /// in `.gwm.toml`) re-prompts — see the module-level comment in
  /// `src/trust.rs` for the threat model.
  Trust {
    #[command(subcommand)]
    action: TrustAction,
  },
}

/// Subcommands of `gwm labels`. The split is intentional: `list` is
/// read-only and safe to run in CI; `push` mutates the remote and
/// therefore gets `--dry-run` / `--prune` flags of its own.
#[derive(Debug, Subcommand)]
pub enum LabelsAction {
  /// Print the declared label set plus the diff against the upstream remote.
  ///
  /// Each line is one of: `+ create`, `~ update (color/desc change)`,
  /// `= match`, `- extra-on-remote`. Without a `[[labels]]` block in
  /// `.gwm.toml`, prints `0 labels declared` and exits 0 without
  /// shelling out to `gh`.
  List,
  /// Apply the diff: create new labels and update mismatched ones on
  /// the upstream remote.
  ///
  /// `--dry-run` prints the plan without mutating the remote (it
  /// still reads the remote via `gh label list` to compute the
  /// diff; only create / update / delete calls are skipped).
  /// `--prune` opt-in deletes labels on remote that aren't declared in
  /// config (off by default — destructive). `--random-colors` picks a
  /// random pastel for labels with no `color` field instead of the
  /// default deterministic hash.
  Push {
    /// Print the plan without mutating the remote. Still reads remote
    /// labels via `gh label list` to compute the diff — only the
    /// create / update / delete calls are skipped.
    #[arg(long)]
    dry_run: bool,
    /// Delete remote labels that aren't declared in `.gwm.toml`.
    /// Destructive — off by default.
    #[arg(long)]
    prune: bool,
    /// Generate a random pastel for labels with no `color` field
    /// (overrides the default deterministic-hash colour).
    #[arg(long)]
    random_colors: bool,
  },
}

/// Subcommands of `gwm trust` (issue #95). All three are read-only or
/// purely local — no network, no git mutation — so they're safe to
/// surface in CI as inspection helpers.
#[derive(Debug, Subcommand)]
pub enum TrustAction {
  /// List every recorded `(origin, hash)` pair in the active ledger.
  ///
  /// Empty ledger prints a single line and exits 0 — the no-op fast
  /// path for fresh installs. The `trusted_at` timestamp is the
  /// audit anchor; revoke entries whose age looks suspicious with
  /// `gwm trust revoke <origin>`.
  List,
  /// Remove every entry whose `origin` matches verbatim. After revoke,
  /// the next `gwm create` / `gwm bootstrap` against that repo
  /// re-prompts — use this when you change machines, rotate
  /// credentials, or no longer trust a previously approved repo.
  Revoke {
    /// Origin URL to revoke (must match the recorded form verbatim —
    /// SSH and HTTPS flavours of the same GitHub repo are recorded as
    /// distinct entries because they ARE distinct trust paths).
    origin: String,
  },
  /// Print the active ledger path and its raw TOML contents.
  ///
  /// Honours `$GWM_TRUST_LEDGER` if set, falls back to
  /// `$XDG_CONFIG_HOME/gwm/trust.toml` (or the platform-specific
  /// equivalent). Useful when triaging "why is gwm re-prompting?"
  /// situations — eyeball the recorded hash vs. what `sha256sum
  /// .gwm.toml` produces.
  Show,
}

/// Subcommands of `gwm milestones`. Mirrors `LabelsAction`: `list` is
/// read-only and safe to run in CI; `push` mutates the remote and
/// therefore gets `--dry-run` / `--prune` flags of its own.
#[derive(Debug, Subcommand)]
pub enum MilestonesAction {
  /// Print the declared milestone set plus the diff against the upstream remote.
  ///
  /// Each line is one of: `+ create`, `~ update (due/desc/state
  /// change)`, `= match`, `- extra-on-remote`. Without a
  /// `[[milestones]]` block in `.gwm.toml`, prints `0 milestones
  /// declared` and exits 0 without shelling out to `gh`.
  List,
  /// Apply the diff: create new milestones and update mismatched ones
  /// on the upstream remote.
  ///
  /// `--dry-run` prints the plan without mutating the remote (it
  /// still reads the remote via `gh api …/milestones` to compute the
  /// diff; only create / update / delete calls are skipped).
  /// `--prune` opt-in deletes milestones on remote that aren't
  /// declared in config (off by default — destructive).
  Push {
    /// Print the plan without mutating the remote. Still reads remote
    /// milestones via `gh api` to compute the diff — only the
    /// create / update / delete calls are skipped.
    #[arg(long)]
    dry_run: bool,
    /// Delete remote milestones that aren't declared in `.gwm.toml`.
    /// Destructive — off by default.
    #[arg(long)]
    prune: bool,
  },
}

pub fn run(cli: Cli) -> Result<()> {
  // Resolve the trust mode once at dispatch time so every handler
  // that gates bootstrap sees the same value — CLI subcommands AND
  // the TUI alike, both honour the same flags. `--deny-bootstrap`
  // wins over `--allow-bootstrap` if both are passed (clap's
  // `conflicts_with` already rejects this combination at parse time
  // — the explicit ordering inside `trust::resolve_mode` is defence
  // in depth).
  let mode = trust::resolve_mode(cli.allow_bootstrap, cli.deny_bootstrap);

  // Without a subcommand, we hand off to the TUI — but with the
  // resolved mode threaded through so the TUI's bootstrap call
  // sites (`submit_create`, `bootstrap_selected`) take the same
  // trust decision as `gwm create` / `gwm bootstrap`.
  let Some(cmd) = cli.command else {
    return crate::tui::run(mode);
  };

  match cmd {
    Command::Init => cmd_init(),
    Command::List { format } => cmd_list(format),
    Command::Create {
      branch_type,
      issue,
      desc,
      no_bootstrap,
      reuse_branch,
    } => cmd_create(branch_type, issue, desc, no_bootstrap, reuse_branch, mode),
    Command::Remove { pattern, delete_branch } => cmd_remove(pattern, delete_branch),
    Command::Path { pattern } => cmd_path(pattern),
    Command::Bootstrap { target } => cmd_bootstrap(target, mode),
    Command::Prune => cmd_prune(),
    Command::Doctor => cmd_doctor(),
    Command::Types => cmd_types(),
    Command::Completions { shell } => cmd_completions(shell),
    Command::ShellInit { shell } => cmd_shell_init(shell),
    Command::Switch => cmd_switch(),
    Command::Tmux { pattern, split } => cmd_multiplexer(Multiplexer::Tmux, pattern, split),
    Command::Zellij { pattern, split } => cmd_multiplexer(Multiplexer::Zellij, pattern, split),
    Command::Link {
      target,
      number,
      worktree,
    } => cmd_link(target, number, worktree),
    Command::Unlink { target, worktree } => cmd_unlink(target, worktree),
    Command::Open {
      target,
      worktree,
      print_url,
    } => cmd_open(target, worktree, print_url),
    Command::Status { worktree, json } => cmd_status(worktree, json),
    Command::Labels { action } => cmd_labels(action),
    Command::Milestones { action } => cmd_milestones(action),
    Command::Trust { action } => cmd_trust(action),
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

fn cmd_create(
  branch_type: String,
  issue: String,
  desc: String,
  no_bootstrap: bool,
  reuse_branch: bool,
  trust_mode: TrustMode,
) -> Result<()> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
  let repo_name = worktree::repo_name(&repo);

  let config = Config::load_for_repo(&workdir)?;
  let resolved_types = config.resolved_branch_types();
  let spec = BranchSpec::new_with_types(branch_type, issue, desc, &resolved_types.types)?;
  let branch = spec.branch_name(&config.worktree, &repo_name)?;
  let dirname = spec.worktree_dirname(&config.worktree, &repo_name)?;
  let target = spec.worktree_path(&config.worktree, &repo_name)?;

  // Gate the bootstrap RCE primitive on the TOFU ledger BEFORE
  // creating the worktree — a deny / abort here leaves the user's
  // disk state untouched (no orphaned worktree to clean up).
  if !no_bootstrap {
    trust_or_prompt(&workdir, Some(&repo), trust_mode)?;
  }

  println!("creating worktree:");
  println!("  branch : {}", branch);
  println!("  dir    : {}", dirname);
  println!("  path   : {}", target.display());

  let created = worktree::add(&repo, &dirname, &target, &branch, reuse_branch)?;
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

fn cmd_bootstrap(target: Option<String>, trust_mode: TrustMode) -> Result<()> {
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

  trust_or_prompt(&workdir, Some(&repo), trust_mode)?;

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
  // Resolve the active branch-type list. When invoked inside a repo
  // with a workdir we honour any `[[branch_types]]` override in
  // `.gwm.toml`; outside of one — or inside a bare repo where
  // `repo.workdir()` is `None` and there's no place to look for
  // `.gwm.toml` — we silently fall back to the built-in defaults so
  // `gwm types` remains useful as a discovery command (used by `gwm`
  // newcomers before they've initialised a config, and from CI inspect
  // commands that point at bare clones).
  let resolved = match worktree::discover_repo(None) {
    Ok(repo) => match repo.workdir().map(|w| w.to_path_buf()) {
      Some(workdir) => Config::load_for_repo(&workdir)?.resolved_branch_types(),
      None => Config::default().resolved_branch_types(),
    },
    Err(_) => Config::default().resolved_branch_types(),
  };

  // Align the description column on the longest name so a custom list
  // with a long entry (e.g. `migration`) still renders cleanly.
  let width = resolved.types.iter().map(|t| t.name.len()).max().unwrap_or(0).max(8);
  for t in &resolved.types {
    println!("  {:<width$}  {}", t.name, t.description, width = width);
  }
  println!();
  println!("(source: {})", resolved.source.label());
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
    // `${env_name}` renders bare in stderr (not shell source, so no
    // backslash escaping). Pre-fix this read `\\${env_name}` and
    // surfaced `\$TMUX` to the user — caught at PR #65 review.
    return Err(GwmError::Other(format!(
      "{0} session not running (${1} unset) — run `gwm {0} <pattern>` from inside a {0} session",
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

// ---- Issue / PR link commands (issue #67) -------------------------------

/// Resolve the repo + branch + repo-relative path to operate on.
///
/// `--worktree <pattern>` overrides; otherwise we use the current directory.
/// The returned Repository is opened *at the target worktree*, so reading
/// HEAD gives the branch the user expects, but git config writes still land
/// on the main repo's config (git2 propagates branch.* config up).
fn resolve_target_repo(worktree: Option<String>) -> Result<(Repository, String, PathBuf)> {
  let path: PathBuf = match worktree {
    Some(pat) => {
      // Allow either a fuzzy worktree pattern or a direct path.
      let p = PathBuf::from(&pat);
      if p.is_dir() {
        p
      } else {
        let main = worktree::discover_repo(None)?;
        worktree::find_fuzzy(&main, &pat)?.path
      }
    }
    None => std::env::current_dir()?,
  };
  let repo = Repository::discover(&path).map_err(|_| GwmError::NotInGitRepo)?;
  let branch = current_branch(&repo)?;
  Ok((repo, branch, path))
}

fn current_branch(repo: &Repository) -> Result<String> {
  let head = repo.head().map_err(|_| GwmError::UnbornHead {
    reason: "HEAD is unborn or detached".into(),
  })?;
  head
    .shorthand()
    .map(|s| s.to_string())
    .ok_or_else(|| GwmError::UnbornHead {
      reason: "HEAD has no shorthand (detached?)".into(),
    })
}

fn cmd_link(target: LinkTarget, number: u64, worktree: Option<String>) -> Result<()> {
  let (repo, branch, _path) = resolve_target_repo(worktree)?;
  match target {
    LinkTarget::Issue => {
      github::link_issue(&repo, &branch, number)?;
      println!("✓ linked issue #{} to branch {}", number, branch);
    }
    LinkTarget::Pr => {
      github::link_pr(&repo, &branch, number)?;
      println!("✓ linked PR #{} to branch {}", number, branch);
    }
  }
  Ok(())
}

fn cmd_unlink(target: LinkTarget, worktree: Option<String>) -> Result<()> {
  let (repo, branch, _path) = resolve_target_repo(worktree)?;
  match target {
    LinkTarget::Issue => {
      github::unlink_issue(&repo, &branch)?;
      println!("✓ unlinked issue on branch {}", branch);
    }
    LinkTarget::Pr => {
      github::unlink_pr(&repo, &branch)?;
      println!("✓ unlinked PR on branch {}", branch);
    }
  }
  Ok(())
}

fn cmd_open(target: LinkTarget, worktree: Option<String>, print_url: bool) -> Result<()> {
  let (repo, branch, _path) = resolve_target_repo(worktree)?;
  let link = github::read_link(&repo, &branch)?;
  let slug = github::repo_slug(&repo)?;

  let url = match target {
    LinkTarget::Issue => {
      let n = link.issue.ok_or_else(|| GwmError::LinkMissing {
        kind: LinkKind::Issue,
        branch: branch.clone(),
      })?;
      github::issue_url(&slug, n)
    }
    LinkTarget::Pr => {
      let n = link.pr.ok_or_else(|| GwmError::LinkMissing {
        kind: LinkKind::Pr,
        branch: branch.clone(),
      })?;
      github::pr_url(&slug, n)
    }
  };

  if print_url {
    println!("{}", url);
    return Ok(());
  }
  spawn_opener(&url)
}

fn spawn_opener(url: &str) -> Result<()> {
  let opener = if cfg!(target_os = "macos") {
    "open"
  } else if cfg!(target_os = "windows") {
    "explorer"
  } else {
    "xdg-open"
  };
  let status = std::process::Command::new(opener)
    .arg(url)
    .status()
    .map_err(|e| GwmError::CommandFailed(format!("could not spawn {}: {}", opener, e)))?;
  if !status.success() {
    return Err(GwmError::CommandFailed(format!(
      "{} exited with status {:?}",
      opener,
      status.code()
    )));
  }
  Ok(())
}

fn cmd_status(worktree: Option<String>, json: bool) -> Result<()> {
  let (repo, branch, _path) = resolve_target_repo(worktree)?;
  let link = github::read_link(&repo, &branch)?;

  // Slug + fetched status are best-effort: if there's no GitHub remote
  // or `gh` isn't installed, we still print the local link.
  let slug = github::repo_slug(&repo).ok();
  let (issue_status, pr_status) = fetch_link_status(&link, slug.as_deref());

  if json {
    print_status_json(&branch, slug.as_deref(), &link, &issue_status, &pr_status);
  } else {
    print_status_human(&branch, slug.as_deref(), &link, &issue_status, &pr_status);
  }
  Ok(())
}

fn fetch_link_status(link: &BranchLink, slug: Option<&str>) -> (Option<IssueStatus>, Option<PrStatus>) {
  let Some(slug) = slug else {
    return (None, None);
  };
  // `gh` is optional — if either call fails we degrade gracefully.
  let issue = link.issue.and_then(|n| github::fetch_issue(slug, n).ok());
  let pr = link.pr.and_then(|n| github::fetch_pr(slug, n).ok());
  (issue, pr)
}

fn issue_state_str(s: IssueState) -> &'static str {
  match s {
    IssueState::Open => "open",
    IssueState::Closed => "closed",
  }
}

fn pr_state_str(s: PrState) -> &'static str {
  match s {
    PrState::Open => "open",
    PrState::Draft => "draft",
    PrState::Closed => "closed",
    PrState::Merged => "merged",
  }
}

fn link_source_str(s: LinkSource) -> &'static str {
  match s {
    LinkSource::None => "none",
    LinkSource::BranchName => "branch-name",
    LinkSource::Explicit => "explicit",
  }
}

fn print_status_human(
  branch: &str,
  slug: Option<&str>,
  link: &BranchLink,
  issue: &Option<IssueStatus>,
  pr: &Option<PrStatus>,
) {
  println!("branch: {}", branch);
  if let Some(s) = slug {
    println!("repo:   {}", s);
  }
  println!("link:   {}", link.summary());

  if let Some(n) = link.issue {
    print!("issue:  #{}", n);
    match issue {
      Some(s) => println!(" [{}] {}", issue_state_str(s.state), s.title),
      None => println!(" (status unavailable)"),
    }
  }
  if let Some(n) = link.pr {
    print!("pr:     #{}", n);
    match pr {
      Some(s) => {
        let checks = if s.checks_total > 0 {
          format!(" · checks {}/{}", s.checks_passed, s.checks_total)
        } else {
          String::new()
        };
        println!(" [{}]{} {}", pr_state_str(s.state), checks, s.title);
      }
      None => println!(" (status unavailable)"),
    }
  }
}

fn print_status_json(
  branch: &str,
  slug: Option<&str>,
  link: &BranchLink,
  issue: &Option<IssueStatus>,
  pr: &Option<PrStatus>,
) {
  let mut obj = serde_json::Map::new();
  obj.insert("branch".into(), serde_json::Value::String(branch.into()));
  if let Some(s) = slug {
    obj.insert("repo".into(), serde_json::Value::String(s.into()));
  }
  obj.insert(
    "issue".into(),
    match link.issue {
      Some(n) => {
        let mut o = serde_json::Map::new();
        o.insert("number".into(), serde_json::Value::Number(n.into()));
        o.insert(
          "source".into(),
          serde_json::Value::String(link_source_str(link.issue_source).into()),
        );
        if let Some(s) = issue {
          o.insert(
            "state".into(),
            serde_json::Value::String(issue_state_str(s.state).into()),
          );
          o.insert("title".into(), serde_json::Value::String(s.title.clone()));
          o.insert(
            "labels".into(),
            serde_json::Value::Array(s.labels.iter().map(|l| serde_json::Value::String(l.clone())).collect()),
          );
          o.insert("url".into(), serde_json::Value::String(s.url.clone()));
        }
        serde_json::Value::Object(o)
      }
      None => serde_json::Value::Null,
    },
  );
  obj.insert(
    "pr".into(),
    match link.pr {
      Some(n) => {
        let mut o = serde_json::Map::new();
        o.insert("number".into(), serde_json::Value::Number(n.into()));
        o.insert(
          "source".into(),
          serde_json::Value::String(link_source_str(link.pr_source).into()),
        );
        if let Some(s) = pr {
          o.insert("state".into(), serde_json::Value::String(pr_state_str(s.state).into()));
          o.insert("title".into(), serde_json::Value::String(s.title.clone()));
          o.insert(
            "checks_passed".into(),
            serde_json::Value::Number(s.checks_passed.into()),
          );
          o.insert("checks_total".into(), serde_json::Value::Number(s.checks_total.into()));
          o.insert("url".into(), serde_json::Value::String(s.url.clone()));
        }
        serde_json::Value::Object(o)
      }
      None => serde_json::Value::Null,
    },
  );
  println!("{}", serde_json::Value::Object(obj));
}

// ---- Labels commands (issue #81) ----------------------------------------

fn cmd_labels(action: LabelsAction) -> Result<()> {
  match action {
    LabelsAction::List => cmd_labels_list(),
    LabelsAction::Push {
      dry_run,
      prune,
      random_colors,
    } => cmd_labels_push(dry_run, prune, random_colors),
  }
}

fn cmd_labels_list() -> Result<()> {
  let config = load_labels_config()?;
  if config.labels.is_empty() {
    println!("0 labels declared in .gwm.toml — nothing to push.");
    return Ok(());
  }
  // Resolve (and validate colours) before touching the network, so a
  // typo in `.gwm.toml` surfaces "label 'bug' has invalid color: …"
  // rather than the unrelated "no origin remote" error.
  let declared = labels::resolve_labels(&config.labels, false)?;
  let slug = labels_slug()?;
  let remote = github::fetch_remote_labels(&slug)?;
  let diff = labels::diff_labels(&declared, &remote);
  print_labels_diff(&slug, &declared, &diff);
  Ok(())
}

fn cmd_labels_push(dry_run: bool, prune: bool, random_colors: bool) -> Result<()> {
  let config = load_labels_config()?;
  if config.labels.is_empty() {
    println!("0 labels declared in .gwm.toml — nothing to push.");
    return Ok(());
  }
  let declared = labels::resolve_labels(&config.labels, random_colors)?;
  let slug = labels_slug()?;
  let remote = github::fetch_remote_labels(&slug)?;
  let diff = labels::diff_labels(&declared, &remote);
  let (n_create, n_update, n_match, n_extra) = diff.counts();

  if dry_run {
    print_labels_diff(&slug, &declared, &diff);
    let pruned = if prune { n_extra } else { 0 };
    println!(
      "would create {}, update {}, leave {} untouched, prune {}, ignore {} extra-on-remote",
      n_create,
      n_update,
      n_match,
      pruned,
      n_extra.saturating_sub(pruned),
    );
    return Ok(());
  }

  for spec in &diff.to_create {
    github::push_label(&slug, spec)?;
    println!("✓ created {}", spec.name);
  }
  for upd in &diff.to_update {
    github::push_label(&slug, &upd.spec)?;
    println!("✓ updated {}", upd.spec.name);
  }
  if prune {
    for remote_label in &diff.extra_on_remote {
      github::delete_label(&slug, &remote_label.name)?;
      println!("✗ pruned {}", remote_label.name);
    }
  } else if !diff.extra_on_remote.is_empty() {
    println!(
      "{} label(s) on remote not in config — pass --prune to delete",
      diff.extra_on_remote.len()
    );
  }
  println!("{} label(s) untouched", n_match);
  Ok(())
}

/// Open the repo and parse `.gwm.toml`. Shared by `labels list /
/// push`; both surface a uniform "not inside a git repository" error
/// before they touch network or config-resolve logic.
fn load_labels_config() -> Result<Config> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
  Config::load_for_repo(&workdir)
}

/// Resolve the `origin` remote slug. Called *after* `resolve_labels`
/// in both subcommands so a config typo (bad colour) surfaces with
/// the offending label name rather than the unrelated "no origin
/// remote" error.
fn labels_slug() -> Result<String> {
  let repo = worktree::discover_repo(None)?;
  github::repo_slug(&repo)
}

fn print_labels_diff(slug: &str, declared: &[labels::LabelSpec], diff: &LabelDiff) {
  let (n_create, n_update, n_match, n_extra) = diff.counts();
  println!(
    "declared in .gwm.toml: {} labels — diff against {}:",
    declared.len(),
    slug
  );
  for spec in &diff.to_create {
    println!("  + {:<20} (will create — color #{})", spec.name, spec.color);
  }
  for upd in &diff.to_update {
    let detail = match (&upd.previous_color, &upd.previous_description) {
      (Some(old), _) => format!("color #{} → #{}", old, upd.spec.color),
      (None, Some(_)) => "description changed".into(),
      _ => "diff".into(),
    };
    println!("  ~ {:<20} ({})", upd.spec.name, detail);
  }
  for spec in &diff.matching {
    println!("  = {:<20} (match)", spec.name);
  }
  for remote in &diff.extra_on_remote {
    println!("  - {:<20} (on remote, not in config)", remote.name);
  }
  println!(
    "summary: {} create · {} update · {} match · {} extra-on-remote",
    n_create, n_update, n_match, n_extra
  );
}

// ---- Milestones commands (issue #82) ------------------------------------

fn cmd_milestones(action: MilestonesAction) -> Result<()> {
  match action {
    MilestonesAction::List => cmd_milestones_list(),
    MilestonesAction::Push { dry_run, prune } => cmd_milestones_push(dry_run, prune),
  }
}

fn cmd_milestones_list() -> Result<()> {
  let config = load_milestones_config()?;
  if config.milestones.is_empty() {
    println!("0 milestones declared in .gwm.toml — nothing to push.");
    return Ok(());
  }
  // Resolve (and validate due_on / state) before touching the network,
  // so a typo in `.gwm.toml` surfaces "milestone 'v0.7.0' has invalid
  // …" rather than the unrelated "no origin remote" error.
  let declared = milestones::resolve_milestones(&config.milestones)?;
  let slug = milestones_slug()?;
  let remote = github::fetch_remote_milestones(&slug)?;
  let diff = milestones::diff_milestones(&declared, &remote);
  print_milestones_diff(&slug, &declared, &diff);
  Ok(())
}

fn cmd_milestones_push(dry_run: bool, prune: bool) -> Result<()> {
  let config = load_milestones_config()?;
  if config.milestones.is_empty() {
    println!("0 milestones declared in .gwm.toml — nothing to push.");
    return Ok(());
  }
  let declared = milestones::resolve_milestones(&config.milestones)?;
  let slug = milestones_slug()?;
  let remote = github::fetch_remote_milestones(&slug)?;
  let diff = milestones::diff_milestones(&declared, &remote);
  let (n_create, n_update, n_match, n_extra) = diff.counts();

  if dry_run {
    print_milestones_diff(&slug, &declared, &diff);
    let pruned = if prune { n_extra } else { 0 };
    println!(
      "would create {}, update {}, leave {} untouched, prune {}, ignore {} extra-on-remote",
      n_create,
      n_update,
      n_match,
      pruned,
      n_extra.saturating_sub(pruned),
    );
    return Ok(());
  }

  for spec in &diff.to_create {
    github::create_milestone(&slug, spec)?;
    println!("✓ created {}", spec.title);
  }
  for upd in &diff.to_update {
    github::update_milestone(&slug, upd.number, &upd.spec)?;
    println!("✓ updated {}", upd.spec.title);
  }
  if prune {
    for remote_milestone in &diff.extra_on_remote {
      github::delete_milestone(&slug, remote_milestone.number)?;
      println!("✗ pruned {}", remote_milestone.title);
    }
  } else if !diff.extra_on_remote.is_empty() {
    println!(
      "{} milestone(s) on remote not in config — pass --prune to delete",
      diff.extra_on_remote.len()
    );
  }
  println!("{} milestone(s) untouched", n_match);
  Ok(())
}

/// Open the repo and parse `.gwm.toml`. Shared by `milestones list /
/// push`; both surface a uniform "not inside a git repository" error
/// before they touch network or config-resolve logic.
fn load_milestones_config() -> Result<Config> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
  Config::load_for_repo(&workdir)
}

/// Resolve the `origin` remote slug. Called *after* `resolve_milestones`
/// in both subcommands so a config typo (bad due_on / state) surfaces
/// with the offending milestone title rather than the unrelated "no
/// origin remote" error.
fn milestones_slug() -> Result<String> {
  let repo = worktree::discover_repo(None)?;
  github::repo_slug(&repo)
}

fn print_milestones_diff(slug: &str, declared: &[milestones::MilestoneSpec], diff: &MilestoneDiff) {
  let (n_create, n_update, n_match, n_extra) = diff.counts();
  println!(
    "declared in .gwm.toml: {} milestones — diff against {}:",
    declared.len(),
    slug
  );
  for spec in &diff.to_create {
    let due = spec.due_on.as_deref().unwrap_or("no due date");
    println!(
      "  + {:<20} (will create — state {}, due {})",
      spec.title,
      spec.state.as_str(),
      due
    );
  }
  for upd in &diff.to_update {
    let detail = match (&upd.previous_due_on, &upd.previous_state, &upd.previous_description) {
      (Some(old_due), _, _) => format!("due {} → {}", old_due, upd.spec.due_on.as_deref().unwrap_or("cleared")),
      (None, Some(old_state), _) => format!("state {} → {}", old_state.as_str(), upd.spec.state.as_str()),
      (None, None, Some(_)) => "description changed".into(),
      _ => "diff".into(),
    };
    println!("  ~ {:<20} ({})", upd.spec.title, detail);
  }
  for spec in &diff.matching {
    println!("  = {:<20} (match)", spec.title);
  }
  for remote in &diff.extra_on_remote {
    println!("  - {:<20} (#{} on remote, not in config)", remote.title, remote.number);
  }
  println!(
    "summary: {} create · {} update · {} match · {} extra-on-remote",
    n_create, n_update, n_match, n_extra
  );
}

// ---- Trust ledger commands (issue #95) ----------------------------------

fn cmd_trust(action: TrustAction) -> Result<()> {
  match action {
    TrustAction::List => cmd_trust_list(),
    TrustAction::Revoke { origin } => cmd_trust_revoke(origin),
    TrustAction::Show => cmd_trust_show(),
  }
}

fn cmd_trust_list() -> Result<()> {
  let path = trust::default_ledger_path()?;
  let ledger = TrustLedger::load(&path)?;
  if ledger.entries.is_empty() {
    println!("0 entries in trust ledger ({}).", path.display());
    return Ok(());
  }
  println!("trust ledger: {}", path.display());
  println!(
    "  {} entr{} recorded:",
    ledger.entries.len(),
    if ledger.entries.len() == 1 { "y" } else { "ies" }
  );
  let origin_w = ledger
    .entries
    .iter()
    .map(|e| e.origin.len())
    .max()
    .unwrap_or(6)
    .clamp(6, 60);
  for e in &ledger.entries {
    // First 12 chars of the sha256 is plenty for a visual diff; the
    // full digest still ships in the toml file for forensic use.
    // Truncate by chars (not bytes) so a hand-edited ledger with a
    // multi-byte `config_sha` (corrupt but parseable TOML) renders
    // instead of panicking on a UTF-8 boundary.
    let short_sha: String = e.config_sha.chars().take(12).collect();
    println!(
      "  {:<ow$}  {}  trusted_at {}  by {}",
      e.origin,
      short_sha,
      e.trusted_at.to_rfc3339(),
      e.trusted_by,
      ow = origin_w,
    );
  }
  Ok(())
}

fn cmd_trust_revoke(origin: String) -> Result<()> {
  let path = trust::default_ledger_path()?;
  let mut ledger = TrustLedger::load(&path)?;
  let removed = ledger.revoke(&origin);
  if removed == 0 {
    println!("0 entries matched origin {} (nothing to revoke).", origin);
    return Ok(());
  }
  ledger.save(&path)?;
  println!(
    "✓ revoked {} entr{} for {}",
    removed,
    if removed == 1 { "y" } else { "ies" },
    origin
  );
  Ok(())
}

fn cmd_trust_show() -> Result<()> {
  let path = trust::default_ledger_path()?;
  println!("ledger path: {}", path.display());
  match std::fs::read_to_string(&path) {
    Ok(body) => {
      println!("---");
      print!("{}", body);
      if !body.ends_with('\n') {
        println!();
      }
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
      println!("(file does not exist yet — nothing has been trusted on this machine)");
    }
    Err(e) => return Err(e.into()),
  }
  Ok(())
}

/// TOFU gate called by `cmd_create` and `cmd_bootstrap` before any
/// `bootstrap::run` invocation. The contract:
///
///   * Returns `Ok(())` when the caller is cleared to proceed.
///   * Returns `Err(GwmError::Other(..))` when the user declined,
///     `--deny-bootstrap` was passed, or stdin isn't interactive and
///     no `--allow-bootstrap` bypass was provided.
///   * No-ops silently when there is no `.gwm.toml` in the workdir
///     (nothing for bootstrap to execute — no trust decision needed).
///
/// The `repo` is passed in so we can read `origin` from the existing
/// `Repository` handle (already opened by every caller) without
/// re-discovering it. Falls back to the canonical workdir path when
/// there is no origin remote — local-only repos still benefit from
/// the drift-detection half of the feature even when the threat model
/// is weaker.
fn trust_or_prompt(workdir: &Path, repo: Option<&Repository>, mode: TrustMode) -> Result<()> {
  let origin = origin_url_for_repo(repo);
  let origin_key = trust::resolve_origin_key(origin.as_deref(), workdir);

  match trust::evaluate(workdir, &origin_key, mode)? {
    TrustOutcome::Proceed => Ok(()),
    TrustOutcome::Refuse { message } => Err(GwmError::Other(message)),
    TrustOutcome::Prompt {
      cfg_path,
      body,
      sha,
      origin,
      mut ledger,
      ledger_path,
    } => {
      // Refuse cleanly if stdin isn't a tty rather than hanging on a
      // read that will never see input — this is the case that makes
      // `--allow-bootstrap` actually load-bearing in CI.
      use std::io::IsTerminal;
      if !std::io::stdin().is_terminal() {
        return Err(GwmError::Other(format!(
          ".gwm.toml at {} is not in the trust ledger and stdin is not interactive — \
           pass --allow-bootstrap (or set GWM_ALLOW_BOOTSTRAP=1) to bypass, \
           or run interactively to approve",
          cfg_path.display()
        )));
      }

      let granted = prompt_user(&cfg_path, &body, &origin, &sha)?;
      if !granted {
        return Err(GwmError::Other(format!(
          "trust prompt declined for {} — aborting bootstrap",
          cfg_path.display()
        )));
      }

      ledger.record(&origin, &sha, &trust::current_actor());
      ledger.save(&ledger_path)?;
      println!("✓ recorded trust for {} in {}", origin, ledger_path.display());
      Ok(())
    }
  }
}

/// Pull the `origin` remote URL out of a Repository handle, if there
/// is one. Returns `None` for repos with no `origin` remote — caller
/// (or `trust::resolve_origin_key`) falls back to the canonical
/// workdir path in that case.
fn origin_url_for_repo(repo: Option<&Repository>) -> Option<String> {
  let r = repo?;
  let remote = r.find_remote("origin").ok()?;
  remote.url().map(|s| s.to_string())
}

/// Interactive y/N/show loop. Prints a one-shot summary of the
/// bootstrap surface (copy targets, guards, command lines, no-symlink
/// declarations) so the user has the relevant signal before answering.
/// `show` re-prints the raw `.gwm.toml`.
fn prompt_user(cfg_path: &Path, bytes: &[u8], origin: &str, sha: &str) -> Result<bool> {
  use std::io::{BufRead, Write};

  let body = String::from_utf8_lossy(bytes);
  let parsed: Option<Config> = toml::from_str(&body).ok();
  let stdin = std::io::stdin();
  let mut stdout = std::io::stdout();

  println!();
  println!("gwm: this repo's .gwm.toml has not been trusted yet.");
  println!("     path   : {}", cfg_path.display());
  println!("     origin : {}", origin);
  println!("     hash   : {}", sha);
  if let Some(cfg) = parsed.as_ref() {
    print_bootstrap_summary(cfg);
  } else {
    println!("     (could not parse .gwm.toml for summary — see raw via `show` below)");
  }
  println!();

  loop {
    print!("Trust this .gwm.toml? [y/N/show]: ");
    stdout.flush().ok();
    let mut line = String::new();
    let n = stdin.lock().read_line(&mut line)?;
    if n == 0 {
      // EOF without an answer — same conservative default as `N`.
      return Ok(false);
    }
    match line.trim().to_ascii_lowercase().as_str() {
      "y" | "yes" => return Ok(true),
      "n" | "no" | "" => return Ok(false),
      "show" | "s" => {
        println!("---");
        print!("{}", body);
        if !body.ends_with('\n') {
          println!();
        }
        println!("---");
      }
      other => {
        println!("unrecognised answer '{}' — answer y, N, or show", other);
      }
    }
  }
}

fn print_bootstrap_summary(cfg: &Config) {
  let bs = &cfg.bootstrap;
  if bs.copy.is_empty() && bs.command.is_empty() && bs.guard.is_empty() && bs.no_symlink.is_empty() {
    println!("     bootstrap surface: (empty — no copies/commands/guards/no_symlinks declared)");
    return;
  }
  println!("     bootstrap surface:");
  for c in &bs.copy {
    println!("       - copy   {} → {}", c.from, c.to);
  }
  for g in &bs.guard {
    println!(
      "       - guard  {} (on_match={}, deny={} pattern(s))",
      g.name,
      g.on_match,
      g.deny_patterns.len()
    );
  }
  for ns in &bs.no_symlink {
    println!("       - no-symlink {}", ns.path);
  }
  for c in &bs.command {
    println!("       - run    {} ({})", c.name, c.run);
  }
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
