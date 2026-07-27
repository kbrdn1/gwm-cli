//! Environment + worktree diagnostics. Aggregates a series of cheap checks
//! into a single report so users (and CI) can answer "is my setup sane?"
//! without running a dozen ad-hoc commands.

use crate::config::{expand_placeholders, Config, CONFIG_FILE};
use crate::error::Result;
use crate::naming::{branch_pattern_warning, parse_branch};
use crate::worktree;
use git2::BranchType;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct DoctorReport {
  pub checks: Vec<Check>,
}

impl DoctorReport {
  pub fn new() -> Self {
    Self::default()
  }

  /// Highest severity present in the report — `Failed` wins over `Warning`
  /// wins over `Ok`. Returned as a `CheckStatus` (a previous `Severity`
  /// enum was a verbatim duplicate; collapsing into one type avoids the
  /// translation match and keeps the public surface minimal).
  pub fn severity(&self) -> CheckStatus {
    let mut s = CheckStatus::Ok;
    for c in &self.checks {
      match c.status {
        CheckStatus::Failed => return CheckStatus::Failed,
        CheckStatus::Warning if s == CheckStatus::Ok => s = CheckStatus::Warning,
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
      CheckStatus::Ok => 0,
      CheckStatus::Warning => 1,
      CheckStatus::Failed => 2,
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

/// Backwards-compatibility alias. `Severity` was a verbatim duplicate of
/// `CheckStatus` introduced before they were unified; keep the name so
/// callers from 0.3.0 keep compiling while we converge on `CheckStatus`.
pub type Severity = CheckStatus;

pub struct DoctorCtx<'a> {
  pub repo_workdir: &'a Path,
  pub repo: &'a git2::Repository,
  pub config: &'a Config,
  /// Global config layer to merge under the repo `.gwm.toml` when a check
  /// re-reads the on-disk config (the `[tui.keys]` keymap check). Threaded
  /// explicitly — rather than read ambiently via `global_config_path()` — so
  /// `doctor::run` stays deterministic for embedders and unit tests that
  /// inject an isolated context (issue #219 review). `None` skips the global
  /// layer entirely.
  pub global_config_path: Option<&'a Path>,
}

pub fn run(ctx: &DoctorCtx<'_>) -> Result<DoctorReport> {
  let mut report = DoctorReport::new();
  report.checks.push(check_config_parses(ctx));
  report.checks.push(check_guard_references(ctx));
  report.checks.push(check_when_predicates(ctx));
  report.checks.push(check_binaries_on_path(ctx));

  // The next two checks both need the worktree list. Hoist the libgit2
  // call here so it runs once per `gwm doctor` invocation and so each
  // check carries the same view of the world.
  match worktree::list(ctx.repo) {
    Ok(trees) => {
      report.checks.push(check_prunable_worktrees(&trees));
      report.checks.push(check_orphan_branches(ctx, &trees));
    }
    Err(e) => {
      let detail = format!("could not list worktrees: {}", e);
      report.checks.push(Check::failed("no prunable worktrees", &detail));
      report.checks.push(Check::failed("no orphan gwm branches", &detail));
    }
  }

  report.checks.push(check_base_dir_writable(ctx));
  report.checks.push(check_tui_keymap(ctx));
  report.checks.push(check_branch_pattern(ctx));
  Ok(report)
}

/// TUI keymap diagnostic (issue #87). Re-runs the same
/// [`crate::tui::keymap::Keymap`] resolution path the TUI itself uses
/// at startup, so any user-facing `[tui.keys]` mistake surfaces here
/// before the TUI actually fails to dispatch.
///
/// Three outcomes:
///
/// 1. **Failed** — the keymap fails to resolve (parse error, unknown
///    action slug, chord conflict, prefix collision). The detail
///    repeats the underlying [`crate::error::GwmError::Config`]
///    message verbatim so the user can paste it into a search.
/// 2. **Warning** — the keymap resolves, but `quit` has been
///    unbound entirely. The hard-coded `Ctrl+C` branch in `run_app`
///    keeps the TUI exitable; we warn anyway because losing the
///    discoverable quit key is a hostile UX choice users usually
///    don't realise they made.
/// 3. **Ok** — keymap is valid and `quit` has at least one
///    user-visible binding.
fn check_tui_keymap(ctx: &DoctorCtx<'_>) -> Check {
  let name = "[tui.keys] keymap resolves";

  // Re-derive `[tui.keys]` from the on-disk config (#219 review): the lenient
  // `repo_context_lenient` returns `Config::default()` when `load_for_repo`
  // rejects the user's file, so validating `ctx.config` here would silently
  // OK a config that actually refuses to start the TUI. The global layer is
  // the one threaded into the context (not an ambient `global_config_path()`
  // read) so the check stays deterministic for injected contexts. Fall back to
  // `ctx.config` only when the *merge* fails — a parse / shape error already
  // surfaced by `check_config_parses`.
  let keys = match Config::merge_layered(ctx.repo_workdir, ctx.global_config_path) {
    Ok(cfg) => cfg.tui.keys,
    Err(_) => ctx.config.tui.keys.clone(),
  };

  let keymap = match keys.resolved_keymap() {
    Ok(km) => km,
    Err(e) => {
      return Check::failed(name, format!("{}", e))
        .with_hint("fix the `[tui.keys]` entry called out above; the full list of action slugs is `gwm tui keys`");
    }
  };

  // Issue #219: the contextual modal keymap (`[tui.keys.modal.<context>]`) is
  // validated the same way — an unknown context / verb, a multi-stroke
  // chord, or a per-context conflict surfaces here with the offending
  // coordinate so `gwm doctor` flags it before the user hits it live.
  // #219 review (P2): resolve it *before* the quit warning so a hard modal
  // error (Failed) is never downgraded to the `quit` Warning when a config
  // carries both an unbound `quit` and an invalid modal binding.
  let modal = match keys.resolved_modal_keymap() {
    Ok(mk) => mk,
    Err(e) => {
      return Check::failed(name, format!("{}", e)).with_hint(
        "fix the `[tui.keys.modal.<context>]` entry called out above; `gwm tui keys` lists every context and verb",
      );
    }
  };

  // Snapshot once. The pre-review version called `keymap.list()`
  // twice — both `quit_has_user_binding` and the success count
  // cloned the bindings vector. One snapshot reused below.
  let bindings = keymap.list();

  // Quit is special: the only hard-coded escape hatch is `Ctrl+C` in
  // `run_app`. We don't refuse an empty `quit` binding (per the design
  // note in `src/tui/keymap.rs`), but we do flag it so the user knows
  // the discoverable key is gone.
  let quit_has_user_binding = bindings
    .iter()
    .any(|b| b.action == crate::tui::keymap::Action::Quit && !b.chords.is_empty());
  if !quit_has_user_binding {
    return Check::warning(
      name,
      "`quit` has no binding — Ctrl+C still exits the TUI as a hard-coded fallback, but no discoverable key remains",
    )
    .with_hint("add `quit = [\"q\", \"Esc\"]` (or any other key) to `[tui.keys]`");
  }

  // Count only actions with at least one chord. The pre-review
  // version used `bindings.len()` which includes unbound entries
  // (`action = []` in `[tui.keys]` leaves the action in the list
  // with an empty chord vec), inflating the count visible in
  // `gwm doctor` output and misleading the user about how many
  // actions are actually reachable.
  let bound_count = bindings.iter().filter(|b| !b.chords.is_empty()).count();
  let modal_bound = modal.list().iter().filter(|b| !b.keys.is_empty()).count();
  Check::ok(
    name,
    format!("{} global + {} modal binding(s) bound", bound_count, modal_bound),
  )
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

  let cfg = match toml::from_str::<Config>(&raw) {
    Ok(cfg) => cfg,
    Err(e) => {
      return Check::failed(name, format!("invalid TOML in {}: {}", path.display(), e))
        .with_hint("fix the syntax or back it up and re-run `gwm init`");
    }
  };
  // `[exec.profiles]` / `[clean.profiles]` semantics (non-empty command, a
  // worktree-relative single-name `dirs`) parse cleanly, so check them here
  // too — otherwise doctor reports green on a profile the loader and the
  // `gwm exec`/`gwm clean` commands reject (issue #324 review).
  match cfg.validate_profiles() {
    Ok(()) => Check::ok(name, format!("{} parses cleanly", path.display())),
    Err(e) => Check::failed(name, format!("invalid profile in {}: {}", path.display(), e))
      .with_hint("fix the `[exec.profiles]` / `[clean.profiles]` entry it names"),
  }
}

/// Check #2: every `[[bootstrap.copy]].guards = [...]` entry references a
/// `[[bootstrap.guard]].name` that actually exists. Dangling references are
/// silent footguns — the copy step would proceed unchecked and the guard
/// would never trip.
fn check_guard_references(ctx: &DoctorCtx<'_>) -> Check {
  let name = "guard references resolve";
  let bs = &ctx.config.bootstrap;

  let mut dangling: Vec<String> = Vec::new();
  for copy in &bs.copy {
    for guard_name in &copy.guards {
      if ctx.config.guard_by_name(guard_name).is_none() {
        dangling.push(format!(
          "{} (referenced from copy {} -> {})",
          guard_name, copy.from, copy.to
        ));
      }
    }
  }

  if dangling.is_empty() {
    let count: usize = bs.copy.iter().map(|c| c.guards.len()).sum();
    return Check::ok(name, format!("{} guard reference(s) resolve", count));
  }

  Check::failed(name, format!("dangling guard reference(s): {}", dangling.join("; ")))
    .with_hint("declare the missing `[[bootstrap.guard]]` block(s) or drop the reference")
}

/// Recognised `when:` predicate keywords. Update this list when a new
/// keyword lands in `bootstrap.rs::evaluate_when`.
const SUPPORTED_WHEN_PREFIXES: &[&str] = &["file_exists:", "cmd_exists:", "env_set:", "env_eq:", "glob_exists:"];

/// Check #3: every `[[bootstrap.command]].when` predicate uses one of the
/// supported keywords. Unknown predicates default to `true` in
/// `bootstrap::evaluate_when`, so the command runs anyway and the user's
/// intended gating condition is silently ignored — that's still a footgun
/// worth flagging, just not "command never runs".
///
/// Walks every atom in the expression (via `bootstrap::when_atoms`) so
/// negated atoms (`!env_set:CI`) and compound expressions
/// (`file_exists:a && bogus:1`) are validated as a whole instead of
/// being green-lit by their first keyword.
fn check_when_predicates(ctx: &DoctorCtx<'_>) -> Check {
  let name = "`when` predicates supported";
  let bs = &ctx.config.bootstrap;

  let mut unknown: Vec<String> = Vec::new();
  let mut recognised: usize = 0;
  for cmd in &bs.command {
    let Some(w) = &cmd.when else { continue };
    // Walk every atom in the expression (via `bootstrap::when_atoms`) so
    // negated atoms (`!env_set:CI`) and compound expressions (`file_exists:a
    // && bogus:1`) are validated as a whole rather than green-lit by their
    // first keyword. A command is `recognised` only when all its atoms
    // pass — a single unknown atom kicks it into `unknown`.
    let mut had_unknown = false;
    for atom in crate::bootstrap::when_atoms(w) {
      if !SUPPORTED_WHEN_PREFIXES.iter().any(|p| atom.starts_with(p)) {
        unknown.push(format!("{} (on command `{}`)", atom, cmd.name));
        had_unknown = true;
      }
    }
    if !had_unknown {
      recognised += 1;
    }
  }

  if unknown.is_empty() {
    let detail = if recognised == 0 {
      "no `when:` predicates configured".to_string()
    } else {
      format!("{} predicate(s) recognised", recognised)
    };
    return Check::ok(name, detail);
  }

  Check::failed(name, format!("unknown `when` predicate(s): {}", unknown.join("; ")))
    .with_hint(format!("supported keywords: {}", SUPPORTED_WHEN_PREFIXES.join(", ")))
}

/// Common shell wrappers that introduce the real binary after their
/// own switches / env assignments. Caught by Copilot's review on
/// PR #76: pre-fix, `env FOO=bar lumen diff` made the doctor check
/// `env` against `$PATH` (which is always present) and miss the real
/// launcher `lumen`. Keep this list narrow on purpose — exotic
/// wrappers (`nice`, `time`, `nohup`) take positional args, which we
/// would risk consuming and ending up with the wrong binary.
const COMMAND_WRAPPERS: &[&str] = &["env", "command"];

/// Extract the executable name from a shell command string. Tokenises
/// via `shell_words` so quoted args (`"my tool" --flag`) and escaped
/// whitespace are handled the way the shell would, then skips leading
/// `FOO=bar` env assignments and recognised `env`/`command` wrappers
/// (and the wrapper's own `KEY=VAL` / `-flag` tokens) before returning
/// the first token that looks like a real binary name. Returns `None`
/// for empty strings or strings that fail to parse (unbalanced quotes
/// — better to surface nothing than a garbage binary name that would
/// produce a confusing PATH warning).
fn extract_binary(run: &str) -> Option<String> {
  let tokens = shell_words::split(run).ok()?;
  let mut iter = tokens.into_iter().peekable();

  // Skip leading `KEY=VAL` env assignments (POSIX `FOO=bar tool` form).
  while iter.peek().is_some_and(|t| !t.starts_with('=') && t.contains('=')) {
    iter.next();
  }

  // Recognise a wrapper (`env`, `command`) and skip its own `-flag` /
  // `KEY=VAL` arguments before reaching the real binary. Stops on the
  // first positional non-flag, non-assignment token.
  if iter.peek().is_some_and(|t| COMMAND_WRAPPERS.contains(&t.as_str())) {
    iter.next(); // consume the wrapper itself
    while let Some(t) = iter.peek() {
      if t.starts_with('-') || (!t.starts_with('=') && t.contains('=')) {
        iter.next();
      } else {
        break;
      }
    }
  }

  iter.next()
}

/// Same as [`extract_binary`] but pre-strips the launcher placeholders
/// so a template like `lumen diff {base}..{head}` reduces to `lumen`
/// before tokenisation. Used for the issue #75 [`crate::config::GitTuiConfig`] /
/// [`crate::config::ReviewConfig`] entries so the doctor warning
/// names the actual binary, not a placeholder fragment.
fn extract_launcher_binary(command: &str) -> Option<String> {
  let cleaned = command
    .replace("{base}", "BASE")
    .replace("{head}", "HEAD")
    .replace("{path}", "PATH")
    .replace("{diff}", "/tmp/diff");
  extract_binary(&cleaned)
}

/// Check #4: every binary referenced by the bootstrap commands resolves on
/// `$PATH`. `lazygit` (the TUI's `l` keybinding's default) and `direnv`
/// (only if the repo has an `.envrc`) are also checked because they're
/// the two "ambient" dependencies whose absence routinely confuses new
/// users. Configured launchers ([git_tui], [review] — issue #75) are
/// added to the same set so the user gets one consolidated warning.
///
/// Issue #415: `worktree.branch_pattern` is honoured when a branch name is
/// *written* and ignored when one is *read back*, so a pattern the parser
/// cannot follow quietly turns off issue/PR auto-linking, gitmoji selection
/// and the branch-convention check above. [`branch_pattern_warning`] probes
/// the round-trip and names whichever segments actually break — a custom
/// pattern is not automatically a broken one.
///
/// Warning rather than Failed: the config is valid and the worktrees it
/// produces are perfectly usable — only the structured extras go silent.
/// This check does not fix the divergence, it states it; the parser is
/// derived from the pattern in #417.
fn check_branch_pattern(ctx: &DoctorCtx<'_>) -> Check {
  let name = "worktree.branch_pattern round-trips through the parser";

  // Re-derive from disk for the same reason `check_tui_keymap` does:
  // `repo_context_lenient` substitutes `Config::default()` when the user's
  // file fails to load for an unrelated semantic reason, and reading
  // `ctx.config` there would report the default pattern as fine while the
  // file on disk carries a broken one — a false `✓` from the one check
  // whose whole job is catching a silent failure. Fall back to `ctx.config`
  // only when the *merge* fails, which `check_config_parses` already flags.
  let pattern = match Config::merge_layered(ctx.repo_workdir, ctx.global_config_path) {
    Ok(cfg) => cfg.worktree.branch_pattern,
    Err(_) => ctx.config.worktree.branch_pattern.clone(),
  };

  match branch_pattern_warning(&pattern) {
    // The hint stays neutral on purpose: which workaround applies depends
    // on which segment broke, and the detail above already names it.
    // Recommending `gwm link` unconditionally was wrong for a pattern
    // whose `issue` survives — auto-linking works there, and `gwm link`
    // fixes neither the hook placeholders nor the TUI rename.
    Some(detail) => Check::warning(name, detail).with_hint(
      "restore the default `{type}/#{issue}-{desc}`, or keep the pattern and accept exactly the loss named above",
    ),
    None => Check::ok(name, "`parse_branch` reads back the segments `branch_name` writes"),
  }
}

/// Missing binaries are surfaced as Warning, not Failed — the user may not
/// rely on that step at all, but the visibility matters.
fn check_binaries_on_path(ctx: &DoctorCtx<'_>) -> Check {
  let name = "external binaries on PATH";
  let mut needed: BTreeSet<String> = BTreeSet::new();

  // Ambient deps the rest of the CLI uses. `[git_tui]` may override the
  // lazygit default; we extract whatever binary the resolved launcher
  // names so a `gitui` / `tig` user gets the right warning.
  let git_tui = ctx.config.git_tui.resolved();
  if let Some(bin) = extract_launcher_binary(&git_tui.command) {
    needed.insert(bin);
  }
  if ctx.repo_workdir.join(".envrc").exists() {
    needed.insert("direnv".into());
  }
  // The forge CLI (`gh` / `glab`) is probed only when the user opted in
  // (issue #419). An opt-in makes the warning actionable; probing
  // unconditionally would fire a new warning at every user who never
  // touches issue/PR linking and has no `gh` installed, which is not a
  // regression worth shipping for a feature they don't use.
  //
  // Two forms say it, and both count. `forge` names the backend. A
  // `[forge_hosts]` entry matching this repo's own origin names it *and*
  // the host — a stronger signal, not a weaker one — so probing on `forge`
  // alone left a user who authorises purely through the global table with
  // a clean report and no CLI installed. An entry for some *other* host is
  // not an opt-in here, which is what keeps one global entry from warning
  // in every unrelated repo.
  let opted_in = ctx.config.forge.or_else(|| {
    let host = crate::forge::origin_ref(ctx.repo).ok()?.host;
    Config::forge_host_in(ctx.global_config_path?, &host)
  });
  if let Some(kind) = opted_in {
    // The RESOLVED program, not the bare name: `$GWM_GH` / `$GWM_GLAB`
    // may point at an alternative binary, and probing `gh` regardless
    // warned about a setup that works and pushed the exit code to 1
    // (Codex review #458). `which` resolves an explicit path too, so the
    // same lookup covers both forms.
    let program = match kind {
      crate::forge::ForgeKind::GitHub => crate::github::gh_program(),
      crate::forge::ForgeKind::GitLab => crate::gitlab::glab_program(),
    };
    needed.insert(program.to_string_lossy().into_owned());
  }
  // Review launcher is opt-in; only probe when the user actually
  // configured one (`command` or `tool`).
  if let Some(review) = ctx.config.review.resolved() {
    if let Some(bin) = extract_launcher_binary(&review.command) {
      needed.insert(bin);
    }
  }

  // Whatever the user's own bootstrap commands invoke.
  for cmd in &ctx.config.bootstrap.command {
    if let Some(bin) = extract_binary(&cmd.run) {
      needed.insert(bin);
    }
  }

  let mut missing: Vec<String> = Vec::new();
  let mut found: usize = 0;
  for bin in &needed {
    if which::which(bin).is_ok() {
      found += 1;
    } else {
      missing.push(bin.clone());
    }
  }

  if missing.is_empty() {
    return Check::ok(name, format!("{}/{} binaries found", found, needed.len()));
  }

  Check::warning(name, format!("not on PATH: {}", missing.join(", ")))
    .with_hint("install the missing binaries or remove the steps that need them")
}

/// Check #7: the configured worktree `base` directory exists and is
/// writable. Absence is fine when the parent is writable (gwm creates the
/// base lazily on `gwm create`); a non-writable base is a Failed because
/// every future `create` would error out.
fn check_base_dir_writable(ctx: &DoctorCtx<'_>) -> Check {
  let name = "base directory writable";
  let repo_name = worktree::repo_name(ctx.repo);
  let repo_path = ctx.repo.workdir();
  let base_expanded = match expand_placeholders(&ctx.config.worktree.base, &repo_name, None, None, None, repo_path) {
    Ok(s) => s,
    Err(e) => return Check::failed(name, format!("could not expand base placeholders: {}", e)),
  };
  let base = Path::new(&base_expanded);

  if base.exists() {
    return if is_writable_dir(base) {
      Check::ok(name, format!("{} is writable", base.display()))
    } else {
      Check::failed(name, format!("{} exists but is not writable", base.display()))
        .with_hint("fix the permissions, or set `[worktree].base` to a writable path")
    };
  }

  // Base doesn't exist yet — gwm will create it. Check the parent instead.
  let parent = match base.parent() {
    Some(p) if !p.as_os_str().is_empty() => p,
    _ => {
      return Check::ok(
        name,
        format!("{} will be created on first `gwm create`", base.display()),
      )
    }
  };
  if !parent.exists() {
    return Check::warning(
      name,
      format!(
        "neither {} nor its parent {} exists yet",
        base.display(),
        parent.display()
      ),
    )
    .with_hint("create the parent directory, or pick a different `[worktree].base`");
  }
  if is_writable_dir(parent) {
    Check::ok(
      name,
      format!(
        "{} will be created on first `gwm create` (parent writable)",
        base.display()
      ),
    )
  } else {
    Check::failed(name, format!("parent {} is not writable", parent.display()))
      .with_hint("fix the permissions, or set `[worktree].base` to a writable path")
  }
}

/// Check #5: no prunable worktree entries left in `.git/worktrees/`. These
/// happen when a worktree's working directory is deleted manually without
/// going through `gwm remove` — the admin record stays and confuses future
/// `gwm list` invocations.
fn check_prunable_worktrees(trees: &[worktree::WorktreeInfo]) -> Check {
  let name = "no prunable worktrees";

  let prunable: Vec<String> = trees.iter().filter(|w| w.is_prunable).map(|w| w.name.clone()).collect();
  if prunable.is_empty() {
    return Check::ok(name, format!("{} worktree(s) tracked, none prunable", trees.len()));
  }

  let noun = if prunable.len() == 1 { "entry" } else { "entries" };
  Check::warning(
    name,
    format!("{} prunable {}: {}", prunable.len(), noun, prunable.join(", ")),
  )
  .with_hint("run `gwm prune` to clear them")
}

/// Check #6: every local branch matching the `<type>/#<issue>-<desc>`
/// shape has a worktree pointing at it. A branch without a worktree was
/// likely created by `gwm create` and lost its worktree without a
/// `--delete-branch` — purely cosmetic dead weight, hence Warning not Failed.
///
/// Branches already fully merged into one of the trunk branches
/// (configured via `[doctor].trunks`, default `["dev", "main"]`) are
/// filtered out: keeping them is the project convention, and surfacing
/// them would make the check produce N false positives on every
/// successful release. Repos with non-standard trunk names (`master`,
/// release-trains like `release-3.x`, …) opt in by overriding the list
/// in `.gwm.toml`. An empty list disables the filter entirely.
fn check_orphan_branches(ctx: &DoctorCtx<'_>, trees: &[worktree::WorktreeInfo]) -> Check {
  let name = "no orphan gwm branches";

  let claimed: BTreeSet<String> = trees.iter().filter_map(|w| w.branch.clone()).collect();

  // Resolve the trunk OIDs once. Missing trunks (e.g. a repo without `dev`,
  // or a `[doctor].trunks` entry that doesn't exist locally) are silently
  // skipped — we only check against what exists.
  let trunk_oids: Vec<git2::Oid> = ctx
    .config
    .doctor
    .trunks
    .iter()
    .filter_map(|t| {
      ctx
        .repo
        .find_branch(t, BranchType::Local)
        .ok()
        .and_then(|b| b.get().target())
    })
    .collect();

  let branches = match ctx.repo.branches(Some(BranchType::Local)) {
    Ok(b) => b,
    Err(e) => return Check::failed(name, format!("could not list local branches: {}", e)),
  };

  let mut orphans: Vec<String> = Vec::new();
  let mut merged_count: usize = 0;
  for entry in branches.flatten() {
    let (branch, _) = entry;
    let Ok(Some(branch_name)) = branch.name() else { continue };
    if parse_branch(branch_name).is_none() {
      continue; // user-managed branch, leave it alone
    }
    if claimed.contains(branch_name) {
      continue; // has a worktree — not orphan in any sense
    }
    let Some(branch_oid) = branch.get().target() else {
      continue;
    };
    match is_merged_into_any(ctx.repo, branch_oid, &trunk_oids) {
      Ok(true) => {
        merged_count += 1;
        continue; // preserved on purpose per CONTRIBUTING — not flagged
      }
      Ok(false) => {
        // Real orphan — fall through.
      }
      Err(e) => {
        // libgit2 couldn't walk the graph (missing objects, shallow
        // clone, repo corruption). Surface this loudly: silently
        // assuming "not merged" and recommending `git branch -d` would
        // be actively dangerous.
        return Check::failed(
          name,
          format!("could not determine merge status for {}: {}", branch_name, e),
        )
        .with_hint("check the repository integrity (`git fsck`) or re-fetch missing objects");
      }
    }
    orphans.push(branch_name.to_string());
  }

  if orphans.is_empty() {
    let detail = if merged_count == 0 {
      "every gwm-style branch has a matching worktree".to_string()
    } else {
      format!(
        "{} merged gwm-style branch(es) preserved per CONTRIBUTING, no unmerged orphans",
        merged_count
      )
    };
    return Check::ok(name, detail);
  }

  let suggestions: Vec<String> = orphans.iter().map(|b| format!("git branch -d {}", b)).collect();
  Check::warning(
    name,
    format!("{} unmerged orphan branch(es): {}", orphans.len(), orphans.join(", ")),
  )
  .with_hint(suggestions.join(" && "))
}

/// Returns `Ok(true)` iff `branch_oid` is fully reachable from at least
/// one of `trunks` — i.e. the branch is merged into one of the trunks
/// (or is equal to it). Implemented via libgit2's descendant check:
/// trunk is a descendant of the branch iff the branch is reachable
/// from trunk. Propagates `git2::Error` so callers can distinguish
/// "definitively unmerged" from "could not tell" — silently swallowing
/// the error would let a misclassification lead to a destructive
/// `git branch -d` suggestion.
fn is_merged_into_any(
  repo: &git2::Repository,
  branch_oid: git2::Oid,
  trunks: &[git2::Oid],
) -> std::result::Result<bool, git2::Error> {
  for trunk_oid in trunks {
    if *trunk_oid == branch_oid {
      return Ok(true);
    }
    if repo.graph_descendant_of(*trunk_oid, branch_oid)? {
      return Ok(true);
    }
  }
  Ok(false)
}

/// Probe a directory for write access by creating and deleting a unique
/// sentinel file. More reliable across platforms than parsing Unix mode
/// bits. Uses `tempfile::Builder` so concurrent `gwm doctor` runs don't
/// collide on a fixed filename, and so a SIGKILL mid-probe doesn't leak
/// a stray sentinel into the user's worktree base — `NamedTempFile`
/// RAII-cleans on drop.
fn is_writable_dir(dir: &Path) -> bool {
  tempfile::Builder::new()
    .prefix(".gwm-doctor-probe-")
    .rand_bytes(8)
    .tempfile_in(dir)
    .is_ok()
}
