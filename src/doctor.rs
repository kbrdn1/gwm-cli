//! Environment + worktree diagnostics. Aggregates a series of cheap checks
//! into a single report so users (and CI) can answer "is my setup sane?"
//! without running a dozen ad-hoc commands.

use crate::config::{expand_placeholders, Config, CONFIG_FILE};
use crate::error::Result;
use crate::naming::branch_pattern_warning;
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

  report.checks.push(check_orphan_notes(ctx));
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
      "`quit` has no binding: Ctrl+C still exits the TUI as a hard-coded fallback, but no discoverable key remains",
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
    return Check::ok(name, "no .gwm.toml present, defaults assumed");
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

/// Check #3: every `when` predicate uses one of the supported keywords.
/// Unknown predicates default to `true` in `bootstrap::evaluate_when`, so
/// the command runs anyway and the user's intended gating condition is
/// silently ignored — that's still a footgun worth flagging, just not
/// "command never runs".
///
/// Both surfaces that carry a predicate are walked: `[[bootstrap.command]]`
/// and the six `[hooks.*]` phases. Reading the first only made this check
/// pass vacuously on a config built out of hooks, reporting "no `when:`
/// predicates configured" about a file with plenty of them.
///
/// Walks every atom in the expression (via `bootstrap::when_atoms`) so
/// negated atoms (`!env_set:CI`) and compound expressions
/// (`file_exists:a && bogus:1`) are validated as a whole instead of
/// being green-lit by their first keyword.
fn check_when_predicates(ctx: &DoctorCtx<'_>) -> Check {
  let name = "`when` predicates supported";

  let commands = ctx
    .config
    .bootstrap
    .command
    .iter()
    .map(|cmd| (format!("command `{}`", cmd.name), cmd.when.as_ref()));
  let hooks = ctx
    .config
    .hooks
    .all_steps()
    .map(|(phase, step)| (format!("hook {} `{}`", phase, step.name), step.when.as_ref()));

  let mut unknown: Vec<String> = Vec::new();
  let mut recognised: usize = 0;
  for (label, when) in commands.chain(hooks) {
    let Some(w) = when else { continue };
    // Walk every atom in the expression (via `bootstrap::when_atoms`) so
    // negated atoms (`!env_set:CI`) and compound expressions (`file_exists:a
    // && bogus:1`) are validated as a whole rather than green-lit by their
    // first keyword. A step is `recognised` only when all its atoms
    // pass — a single unknown atom kicks it into `unknown`.
    let mut had_unknown = false;
    for atom in crate::bootstrap::when_atoms(w) {
      if !SUPPORTED_WHEN_PREFIXES.iter().any(|p| atom.starts_with(p)) {
        unknown.push(format!("{} (on {})", atom, label));
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
/// Words that stand in front of the real binary rather than being one.
/// `exec composer install` runs composer, so stopping at `exec` would drop
/// the step and let a missing composer go unreported.
const COMMAND_WRAPPERS: &[&str] = &["env", "command", "exec"];

/// Shell keywords and builtins that can legitimately open a `run` script.
///
/// A step's `run` is handed whole to `sh -c`, so it is a script, not an argv,
/// and its first token is a shell word at least as often as it is a program
/// name: `cd sub && ./setup.sh`, `set -e; …`, `if [ -f composer.json ]; then …`.
/// A few of these do ship an external binary on some systems (`cd` has a
/// `/usr/bin/cd` on macOS), but the shell resolves the builtin first, so
/// probing `$PATH` for any of them answers a question the shell never asks
/// and warns about a step that works.
///
/// Names with a real external binary everywhere (`echo`, `test`, `printf`,
/// `true`) are deliberately absent: probing them costs nothing because they
/// resolve, and leaving them out keeps this list to the words that would
/// produce a false warning.
///
/// So is `source`, and for a sharper reason: it is a bashism, and where
/// `/bin/sh` is dash, which is most Linux distributions, it is neither a
/// keyword nor a builtin, so the step dies with "source: not found". Probing
/// it produces exactly the warning worth having, whose fix is the portable
/// `.` form. `exec` is absent too, for the opposite reason: it stands in
/// front of the real binary, so it belongs in [`COMMAND_WRAPPERS`].
const SHELL_KEYWORDS: &[&str] = &[
  // Reserved words.
  "!", "case", "do", "done", "elif", "else", "esac", "fi", "for", "if", "in", "then", "until", "while", "{", "}",
  // Special builtins, plus the regular ones that commonly open a script.
  ".", ":", "alias", "bg", "break", "cd", "continue", "eval", "exit", "export", "fg", "getopts", "hash", "jobs",
  "local", "read", "readonly", "return", "set", "shift", "times", "trap", "type", "ulimit", "umask", "unalias",
  "unset", "wait",
];

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
  executable_in(&shell_words::split(run).ok()?)
}

/// [`extract_binary`] for a caller that already holds an argv, which is the
/// half `[tui] terminal_browser` needs (issue #590): it expands to an argv
/// before anything probes it, and re-joining it only to re-split here would
/// be a round trip through a quoting layer for nothing.
///
/// Same skipping rules, which is the point of sharing it: a `terminal_browser
/// = "env -u NO_COLOR w3m {url}"` must probe `w3m`, not `env`, or the check
/// passes on a browser that is not installed and the pane the plan opens dies
/// on the spot (Codex review on PR #615). The `env -u NAME` operand rule this
/// walks was itself a fix (`env -u NODE_OPTIONS npm ci` resolved to
/// `NODE_OPTIONS`), which is exactly why there should be one copy of it.
/// Is `token` one of [`COMMAND_WRAPPERS`], however it was spelled?
///
/// Compared by basename because writing the wrapper by its path is ordinary
/// (a pinned coreutils, a nix store path, plain habit), and matching the exact
/// token missed every one of them (Codex review on PR #615). `/usr/bin/env -u
/// NO_COLOR w3m` then resolved to `/usr/bin/env`, hiding the binary the walk
/// exists to find.
fn is_command_wrapper(token: &str) -> bool {
  let base = std::path::Path::new(token)
    .file_name()
    .and_then(|n| n.to_str())
    .unwrap_or(token);
  COMMAND_WRAPPERS.contains(&base)
}

pub(crate) fn executable_in(tokens: &[String]) -> Option<String> {
  let mut iter = tokens.iter().cloned().peekable();

  // Skip leading `KEY=VAL` env assignments (POSIX `FOO=bar tool` form).
  while iter.peek().is_some_and(|t| !t.starts_with('=') && t.contains('=')) {
    iter.next();
  }

  // Recognise a wrapper (`env`, `command`) and skip its own `-flag` /
  // `KEY=VAL` arguments before reaching the real binary. Stops on the
  // first positional non-flag, non-assignment token.
  if iter.peek().is_some_and(|t| is_command_wrapper(t)) {
    iter.next(); // consume the wrapper itself
    while let Some(t) = iter.peek() {
      if t.starts_with('-') {
        // Some of `env`'s options take a separate operand, and skipping the
        // flag without it leaves the operand looking like the executable:
        // `env -u NODE_OPTIONS npm ci` used to resolve to `NODE_OPTIONS`. The
        // attached forms (`--unset=NAME`) carry their own operand, so only
        // the detached spellings consume the next token.
        let takes_operand = matches!(
          t.as_str(),
          "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string"
        );
        iter.next();
        if takes_operand {
          iter.next();
        }
      } else if !t.starts_with('=') && t.contains('=') {
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
/// users. Configured launchers ([git_tui] and [review] from issue #75, plus
/// [tui] terminal_browser from #590) are added to the same set so the user
/// gets one consolidated warning.
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
  let effective = match Config::merge_layered(ctx.repo_workdir, ctx.global_config_path) {
    Ok(cfg) => cfg,
    Err(_) => ctx.config.clone(),
  };
  let types = effective.resolved_branch_types().types;

  match branch_pattern_warning(
    &effective.worktree.branch_pattern,
    &worktree::repo_name(ctx.repo),
    &types,
  ) {
    // The hint stays neutral on purpose: which workaround applies depends
    // on which segment broke, and the detail above already names it.
    // Recommending `gwm link` unconditionally was wrong for a pattern
    // whose `issue` survives — auto-linking works there, and `gwm link`
    // fixes neither the hook placeholders nor the TUI rename.
    Some(detail) => Check::warning(name, detail).with_hint(
      "restore the default `{type}/#{issue}-{desc}`, or keep the pattern and accept exactly the loss named above",
    ),
    None => Check::ok(
      name,
      "the parser compiled from this pattern reads back the segments it writes",
    ),
  }
}

/// Missing binaries are surfaced as Warning, not Failed — the user may not
/// rely on that step at all, but the visibility matters.
/// Whether `gwm doctor` is willing to evaluate a `when` expression it read
/// out of a `.gwm.toml`.
///
/// Two shapes qualify, and the rule behind both is that evaluating them can
/// tell an unvetted repo nothing it does not already know.
///
/// `cmd_exists:` on a bare binary name is a `$PATH` lookup, and `$PATH` is
/// the very set this probe reports on, so it answers the probe's own question
/// with the probe's own data. `file_exists:` on a single path component that
/// is not itself a symlink is a `stat` on one entry of the repo root, and the
/// repo root is what the `.gwm.toml` author committed.
///
/// Everything else is declined, because this evaluation runs on a file that
/// never went through the trust gate (issue #473), including in the advisory
/// CI job on every pull request. Each excluded shape is a channel out of the
/// repo and each was a separate finding on this PR: `glob_exists:` picks its
/// own root and walks it; a `file_exists:` path with more than one component
/// escapes through a committed symlink (`outside/etc/passwd` with
/// `outside -> /`), which no lexical check catches, and so does a single
/// component that is a symlink itself, because `exists()` follows it;
/// `env_set:` / `env_eq:` read the process environment and report the answer
/// through which binaries got probed; and a `cmd_exists:` argument carrying a
/// path separator is `file_exists:` wearing a different keyword.
///
/// A single declined atom taints the whole expression, since the boolean
/// would depend on it either way. Declining costs nothing: the step stays
/// probed, which is exactly what this check did before it evaluated any
/// predicate at all, so it can never silence a warning. What the allowance
/// buys is the `node` preset, whose install hooks are
/// `file_exists:package.json && cmd_exists:bun` and the same with
/// `!cmd_exists:bun`, and that pair was the entire reason to evaluate
/// anything here.
fn predicate_is_safe_to_evaluate(expr: &str, repo_workdir: &Path) -> bool {
  crate::bootstrap::when_atoms(expr).iter().all(|atom| {
    if let Some(rel) = atom.strip_prefix("file_exists:") {
      let rel = rel.trim();
      // One plain component, so the OS resolves nothing on the way in, and
      // not a symlink, so it resolves nothing at the end either. Together
      // that is what keeps `exists()` inside the repo.
      return is_plain_name(rel) && !repo_workdir.join(rel).is_symlink();
    }
    match atom.strip_prefix("cmd_exists:") {
      Some(name) => is_plain_name(name.trim()),
      None => false,
    }
  })
}

/// One ordinary path component and nothing else: no separator, no `.` or
/// `..`, and no Windows drive prefix.
///
/// Spelled through `Components` rather than as a scan for `/` and `\`,
/// because `C:secret` contains neither and is still drive-relative on
/// Windows, where `join` drops the base it was given and `which` resolves it
/// against another directory entirely. On Windows that string parses as
/// `Prefix` + `Normal`, so counting components catches it; on Unix it is an
/// ordinary filename and stays allowed, which is correct there.
fn is_plain_name(value: &str) -> bool {
  let mut components = Path::new(value).components();
  matches!(components.next(), Some(std::path::Component::Normal(_))) && components.next().is_none()
}

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
  // `[tui] terminal_browser` is a third configurable binary (issue #590),
  // opt-in like the review launcher. The TUI probes it too and falls back to
  // the system browser, so a missing one is never fatal; surfacing it here is
  // what stops a user from setting the key, seeing links keep opening
  // externally, and having nothing tell them why.
  if let Some(bin) = ctx
    .config
    .tui
    .terminal_browser
    .as_deref()
    .and_then(extract_launcher_binary)
  {
    needed.insert(bin);
  }

  // Whatever the user's own bootstrap commands and lifecycle hooks invoke.
  // Both surfaces, not just the first: a `[hooks.post_create]` step naming a
  // binary that is not installed used to produce a clean report right up to
  // the moment `gwm create` ran it and failed.
  //
  // A step its `when` switches off is not probed, because it is not going to
  // run. The `node` preset is the case that makes this load-bearing: it ships
  // `bun install` under `cmd_exists:bun` and `npm ci` under `!cmd_exists:bun`,
  // so probing both regardless warns about whichever one the predicate has
  // just switched off, and a Warning takes the exit code to 1. The predicate
  // is evaluated against the main checkout rather than the future worktree,
  // the same approximation the `.envrc` probe above already makes: the
  // worktree gets the same tracked files. An unknown keyword evaluates to
  // `true` in `evaluate_when`, so it stays probed, which matches the step
  // still running at bootstrap time.
  let steps = ctx
    .config
    .bootstrap
    .command
    .iter()
    .map(|cmd| (&cmd.run, cmd.when.as_deref(), &cmd.env))
    .chain(
      ctx
        .config
        .hooks
        .all_steps()
        .map(|(_, step)| (&step.run, step.when.as_deref(), &step.env)),
    );
  for (run, when, env) in steps {
    let gated_off = when.is_some_and(|w| {
      predicate_is_safe_to_evaluate(w, ctx.repo_workdir) && !crate::bootstrap::evaluate_when(w, ctx.repo_workdir)
    });
    if gated_off {
      continue;
    }
    // A step that carries its own `PATH` resolves against that one, since
    // both runners hand `env` to `Command::env`, so probing it against the
    // doctor's ambient `$PATH` answers a question nobody asked. Matched
    // case-insensitively because Windows environment names are, so
    // `Path = "C:\\project\\bin"` really does replace the child's PATH there.
    // Applied on every platform rather than behind `cfg(windows)`: the only
    // cost on Unix is not probing a step that named its variable `Path`, and
    // one behaviour is one thing to test.
    if env.keys().any(|k| k.eq_ignore_ascii_case("PATH")) {
      continue;
    }
    let Some(bin) = extract_binary(run) else { continue };
    // `lifecycle::run_step` expands `{path}` / `{repo}` in `run` before
    // spawning, so a hook reading `{path}/scripts/setup` would be probed as
    // that literal string and always come back missing. Deliberately applied
    // to `[[bootstrap.command]]` too, which does *not* expand its `run`: a
    // first token carrying a `{…}` is not a binary name this check can
    // resolve either way, and staying quiet beats a warning we know is wrong.
    if bin.contains('{') {
      continue;
    }
    // A script that opens on a shell word names no binary to probe.
    if SHELL_KEYWORDS.contains(&bin.as_str()) {
      continue;
    }
    // A path rather than a name: `which` would resolve `./scripts/setup`
    // against the doctor's own working directory, while the step runs from
    // the worktree root. Same rule as the placeholder above, one more way the
    // string we hold is not the file that will be executed.
    if bin.contains(['/', '\\']) {
      continue;
    }
    needed.insert(bin);
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

/// Every stored note still has the branch it is keyed on (issue #515).
///
/// The note's lifecycle rule is "it lives as long as the branch", and this
/// is where that rule is enforced — advisory, like the rest of `doctor`.
/// Deliberately **not** wired into `gwm clean`: that command reclaims
/// regenerable build artefacts, its safety property is that `--yes` only
/// removes directories git already ignores, and its surface has been frozen
/// since #319. Deleting non-regenerable user prose under it would
/// contradict all three. Nor is it wired into `gwm remove`: surviving a
/// removed worktree until the work actually lands is the reason the note
/// lives in the main checkout's git dir rather than inside the worktree.
///
/// Warning, never Failed: an orphan note costs a few hundred bytes and may
/// well be deliberate (the worktree is gone, the PR is not merged yet).
fn check_orphan_notes(ctx: &DoctorCtx<'_>) -> Check {
  let name = "no orphan worktree notes";

  let noted = crate::notes::branches_with_notes(ctx.repo);
  if noted.is_empty() {
    return Check::ok(name, "no notes stored");
  }

  let live: BTreeSet<String> = match ctx.repo.branches(Some(BranchType::Local)) {
    Ok(branches) => branches
      .flatten()
      .filter_map(|(b, _)| b.name().ok().flatten().map(|n| n.to_string()))
      .collect(),
    Err(e) => return Check::failed(name, format!("could not list local branches: {}", e)),
  };

  let orphans: Vec<String> = noted.iter().filter(|b| !live.contains(*b)).cloned().collect();
  if orphans.is_empty() {
    return Check::ok(
      name,
      format!("{} note(s) stored, every branch still exists", noted.len()),
    );
  }

  let noun = if orphans.len() == 1 { "note" } else { "notes" };
  Check::warning(
    name,
    format!(
      "{} orphan {}: {}",
      orphans.len(),
      noun,
      orphans
        .iter()
        .map(|b| crate::naming::sanitise_for_terminal(b))
        .collect::<Vec<_>>()
        .join(", ")
    ),
  )
  .with_hint(format!(
    "the branch is gone: delete the file under {} when the work has landed",
    crate::notes::notes_dir(ctx.repo).display()
  ))
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

  // Issue #417: "did gwm create this branch?" is a question about this repo's
  // `worktree.branch_pattern`, so it is asked with a parser compiled from it.
  // Against the built-in shape, every branch in a repo with a custom pattern
  // read as user-managed and no orphan was ever reported.
  let parser = crate::naming::BranchParser::from_config(ctx.config, &worktree::repo_name(ctx.repo));

  let mut orphans: Vec<String> = Vec::new();
  let mut merged_count: usize = 0;
  for entry in branches.flatten() {
    let (branch, _) = entry;
    let Ok(Some(branch_name)) = branch.name() else { continue };
    if parser.parse(branch_name).is_none() {
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
