//! CLI aliases — `[aliases]` in `.gwm.toml` plus a user-level fallback
//! at `~/.config/gwm/aliases.toml` (issue #86).
//!
//! `git config` ships with `[alias]`; `gwm` mirrors the shape. Aliases
//! are string-substitution: `gwm <alias>` is expanded to argv tokens
//! before clap parses the command. Three resolution levels coexist:
//!
//!   1. **Built-in** — every `visible_alias` declared on a clap
//!      subcommand (`cd → path` from issue #67, `s → switch` from
//!      issue #43). Always wins, can never be shadowed by user config.
//!   2. **Repo (`.gwm.toml`)** — declared under `[aliases]`. Follows
//!      the repo across machines.
//!   3. **User (`~/.config/gwm/aliases.toml`)** — same `[aliases]`
//!      block; survives a machine reinstall but is invisible to the
//!      rest of the team. Repo aliases win on name collision.
//!
//! ## Why expansion happens before clap parses
//!
//! Aliases must turn into argv tokens BEFORE clap reaches the
//! subcommand slot — otherwise clap rejects an unknown subcommand
//! before we get a chance to substitute it. The flow is:
//!
//! ```text
//!   main() → aliases::load() → aliases::expand_argv() → Cli::parse(expanded)
//! ```
//!
//! This shape mirrors what `git` does with `[alias]` — the dispatcher
//! sees the expanded form, never the alias name.
//!
//! ## What aliases CAN'T do
//!
//! - **No shell pipelines** — `wip = "create feat 0 wip && lazygit"`
//!   is rejected at load. Shell metachars (`&&`, `||`, `|`, `;`,
//!   backticks) cannot be honoured by an argv-substitution path that
//!   hands off to clap. Use a shell alias if that's what you need.
//! - **No recursion** — `wip = "ll"` followed by `ll = "list --
//!   format names"` expands once, then dispatches. Matches git's
//!   behaviour and keeps the resolution loop linear.
//! - **No shadowing of built-in subcommands** — `[aliases] list =
//!   "create feat 0 wip"` is a hard config error. The check uses the
//!   compile-time clap CommandFactory, so adding a new subcommand
//!   automatically extends the shadow gate.

use crate::error::{GwmError, Result};
use clap::CommandFactory;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One entry in the built-in alias snapshot. `name` is the clap
/// `visible_alias` (e.g. `cd`); `expansion` is the canonical
/// subcommand it points at (e.g. `path`). Static `&'static str` so the
/// snapshot lives in `BUILT_IN_ALIASES` as a `const` slice (no heap
/// allocation, no lazy init).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasEntry {
  pub name: &'static str,
  pub expansion: &'static str,
}

/// Built-in aliases — every `#[command(visible_alias = "…")]` declared
/// on the clap `Command` enum. Must stay in lockstep with `src/cli.rs`;
/// a regression test in `tests/aliases_tests.rs` pins the contract.
///
/// The list is short by design — clap visible aliases are the only
/// "built-ins" gwm exposes. They are reachable as bare argv tokens
/// (`gwm cd foo`, `gwm s`) so the shadow check has to know about them
/// to refuse user aliases of the same name.
pub const BUILT_IN_ALIASES: &[AliasEntry] = &[
  AliasEntry {
    name: "cd",
    expansion: "path",
  },
  AliasEntry {
    name: "s",
    expansion: "switch",
  },
];

/// Resolved alias chain — built-in + repo + user, in lookup-priority
/// order. Built by [`load`] and consumed by [`expand_argv`] (for the
/// pre-clap expansion) and by `gwm aliases list` (for the user-facing
/// summary).
#[derive(Debug, Clone)]
pub struct ResolvedAliases {
  /// Built-in clap `visible_alias` set — always wins over user
  /// declarations. Static slice cloned into a `Vec` so callers can
  /// extend it cheaply (e.g. for tests).
  pub built_in: Vec<AliasEntry>,
  /// Repo-level aliases from `.gwm.toml`. Wins over `user` on name
  /// collision. `BTreeMap` so iteration order is deterministic
  /// (alphabetical), matching the output of `gwm aliases list`.
  pub repo: BTreeMap<String, String>,
  /// User-level aliases from `~/.config/gwm/aliases.toml`. Lowest
  /// precedence — overridden by both `built_in` and `repo`.
  pub user: BTreeMap<String, String>,
}

impl ResolvedAliases {
  /// Look up `name` honouring the resolution chain: built-in first,
  /// then repo, then user. Returns the raw expansion string (to be
  /// tokenised by `shell_words::split` at call time).
  ///
  /// Built-in entries are checked first because they must be
  /// impossible to shadow — even if a malformed `ResolvedAliases` is
  /// constructed by hand (tests, future APIs) with a `list` entry in
  /// `user`, `expand_argv` MUST treat `list` as the built-in
  /// subcommand and skip the substitution.
  fn lookup(&self, name: &str) -> Option<String> {
    if BUILT_IN_SUBCOMMANDS.contains(&name) {
      // Built-in subcommand names ARE the strongest binding — never
      // expand them, regardless of what `repo`/`user` say. This is
      // the defence-in-depth complement to the shadow check in
      // `load`.
      return None;
    }
    if let Some(e) = self.built_in.iter().find(|e| e.name == name) {
      return Some(e.expansion.to_string());
    }
    if let Some(v) = self.repo.get(name) {
      return Some(v.clone());
    }
    if let Some(v) = self.user.get(name) {
      return Some(v.clone());
    }
    None
  }
}

/// Load and validate the alias chain. `repo_root` is the repo root
/// (where `.gwm.toml` lives) — `None` skips the repo step entirely
/// (used by `aliases load` outside a git repo). `user_path` is the
/// user-level file path — `None` falls back to the default
/// `~/.config/gwm/aliases.toml`; an explicit path is honoured even if
/// it doesn't exist (returns empty user map).
///
/// Errors:
///   - `GwmError::Config` when a TOML parse fails, an alias shadows a
///     built-in subcommand, an alias value contains shell pipeline
///     metachars (`&&`, `||`, `|`, `;`, backticks), or an alias value
///     is empty.
///   - `GwmError::Io` if the file exists but can't be read.
///   - `GwmError::TomlParse` propagates the underlying serde error.
pub fn load(repo_root: Option<&Path>, user_path: Option<&Path>) -> Result<ResolvedAliases> {
  let built_in = BUILT_IN_ALIASES.to_vec();

  // Repo: read `.gwm.toml`'s `[aliases]` block via `Config::load_for_repo`,
  // which already validates the same shadow / shell-pipeline rules via
  // its dedicated `validate_aliases` method (called below).
  let repo = match repo_root {
    Some(root) => {
      let cfg = crate::config::Config::load_for_repo(root)?;
      cfg.aliases
    }
    None => BTreeMap::new(),
  };

  // User: same shape, validated through the standalone helper so the
  // file path appears in the error message.
  let resolved_user_path = user_path.map(PathBuf::from).or_else(default_user_path);
  let user = match resolved_user_path {
    Some(path) if path.exists() => {
      let raw = std::fs::read_to_string(&path)?;
      let file: AliasesFile = toml::from_str(&raw)?;
      let map = file.aliases;
      validate_aliases(&map, &format!("{} `[aliases]`", path.display()))?;
      map
    }
    _ => BTreeMap::new(),
  };

  Ok(ResolvedAliases { built_in, repo, user })
}

/// Expand `argv` in place: replace the first non-flag token in
/// `argv[1..]` with its alias expansion (if any). Single-pass — never
/// recurses, never expands a token that maps to a built-in subcommand
/// (defence-in-depth on top of `load`'s shadow check).
///
/// `argv[0]` (the binary name) is preserved unchanged. Trailing
/// arguments after the alias slot are appended after the expansion —
/// `gwm wip --no-bootstrap` with `wip = "create feat 0 wip"` becomes
/// `gwm create feat 0 wip --no-bootstrap`.
///
/// Tokenisation uses `shell_words::split` (POSIX shell quoting). A
/// malformed value (unbalanced quotes) returns the original argv
/// unchanged — the load-time validation should already have caught
/// shell metachars, so reaching this branch means the user
/// hand-edited the config to something pathological. We refuse to
/// dispatch a partial substitution and let clap report the unknown
/// subcommand verbatim.
pub fn expand_argv(argv: Vec<String>, aliases: &ResolvedAliases) -> Vec<String> {
  if argv.len() < 2 {
    return argv;
  }
  // Find the first non-flag token starting at index 1. Anything
  // starting with `-` is a global flag (clap parses `--allow-bootstrap`
  // anywhere); we look past it to land on the subcommand slot.
  let Some(alias_idx) = argv
    .iter()
    .enumerate()
    .skip(1)
    .find_map(|(i, tok)| if tok.starts_with('-') { None } else { Some(i) })
  else {
    return argv;
  };

  let alias_name = &argv[alias_idx];
  let Some(expansion) = aliases.lookup(alias_name) else {
    return argv;
  };

  let Ok(expanded_tokens) = shell_words::split(&expansion) else {
    // Pathological case: load() should have rejected this. Refuse
    // partial substitution and let clap surface the unknown subcommand
    // verbatim — the user sees "error: unrecognized subcommand" with
    // the original alias name, not a half-expanded mess.
    return argv;
  };

  let mut out = Vec::with_capacity(argv.len() + expanded_tokens.len());
  out.extend_from_slice(&argv[..alias_idx]);
  out.extend(expanded_tokens);
  out.extend_from_slice(&argv[alias_idx + 1..]);
  out
}

/// `OsString` counterpart of [`expand_argv`] — accepts the raw
/// `std::env::args_os()` slice without forcing a UTF-8 round-trip on
/// every token.
///
/// Why this matters: `std::env::args()` panics on the first non-UTF-8
/// argv entry (Linux/macOS allow arbitrary bytes in argv). Clap parses
/// `OsString` natively via `args_os`, and the panic in `main` was a
/// regression vs. that default. We mirror the `expand_argv` logic on
/// `OsString` and only attempt UTF-8 conversion on the alias-slot
/// token — if it is not valid UTF-8 it cannot match an alias name
/// (alias keys are `String` by construction in `ResolvedAliases`), so
/// the argv is returned unchanged and clap surfaces the unknown
/// subcommand verbatim.
///
/// Flag detection is byte-level: a leading `b'-'` is unambiguous in
/// every valid argv encoding (the byte is ASCII, so it cannot appear
/// mid-UTF-8-sequence), which means we can scan past flags without
/// decoding them.
pub fn expand_argv_os(argv: Vec<std::ffi::OsString>, aliases: &ResolvedAliases) -> Vec<std::ffi::OsString> {
  if argv.len() < 2 {
    return argv;
  }
  // First non-flag token from index 1. Use the underlying bytes for
  // the leading-dash check so we don't reject argv with a non-UTF-8
  // tail; on Unix this is `as_bytes`, on Windows the encoded form is
  // WTF-8 and the same ASCII-byte invariant holds for the leading
  // dash check we do here.
  let alias_idx = argv.iter().enumerate().skip(1).find_map(|(i, tok)| {
    let is_flag = first_byte_is_dash(tok);
    if is_flag {
      None
    } else {
      Some(i)
    }
  });
  let Some(alias_idx) = alias_idx else {
    return argv;
  };

  // Only valid UTF-8 can match an alias key. Non-UTF-8 tokens cannot
  // be alias names (the key type is `String`), so we return the argv
  // unchanged and let clap surface the unknown subcommand verbatim.
  let Some(alias_name) = argv[alias_idx].to_str() else {
    return argv;
  };

  let Some(expansion) = aliases.lookup(alias_name) else {
    return argv;
  };

  let Ok(expanded_tokens) = shell_words::split(&expansion) else {
    // Pathological case: load() should have rejected this. See
    // `expand_argv` for the rationale — refuse partial substitution.
    return argv;
  };

  let mut out = Vec::with_capacity(argv.len() + expanded_tokens.len());
  out.extend_from_slice(&argv[..alias_idx]);
  out.extend(expanded_tokens.into_iter().map(std::ffi::OsString::from));
  out.extend_from_slice(&argv[alias_idx + 1..]);
  out
}

/// Inspect the first byte of an `OsStr` to decide whether the token
/// is a flag (leading `-`). The byte is examined in the platform's
/// native argv encoding — ASCII bytes survive both UTF-8 (Unix) and
/// WTF-8 (Windows) round-trips intact, so a simple `as_encoded_bytes`
/// check is correct on both targets.
fn first_byte_is_dash(token: &std::ffi::OsStr) -> bool {
  // `OsStr::as_encoded_bytes` is stable since 1.74 and exposes the
  // platform-native encoding. We only check the first byte against
  // ASCII `-` which is identical in UTF-8 and WTF-8, so this is safe
  // without ever decoding the rest of the token.
  token.as_encoded_bytes().first() == Some(&b'-')
}

/// Default location of the user-level alias file. Mirrors the
/// trust-ledger pattern (`~/.config/gwm/trust.toml`): honour
/// `$XDG_CONFIG_HOME` first, fall back to `dirs::config_dir()` —
/// returns `None` on systems where neither resolves (sandboxed CI,
/// containers without `$HOME`).
fn default_user_path() -> Option<PathBuf> {
  if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
    if !xdg.is_empty() {
      return Some(PathBuf::from(xdg).join("gwm").join("aliases.toml"));
    }
  }
  dirs::config_dir().map(|p| p.join("gwm").join("aliases.toml"))
}

/// Internal shape of the user-level alias file. Mirrors the
/// `[aliases]` block in `.gwm.toml` so the user can copy-paste
/// between the two without remembering whether the key prefix
/// differs.
#[derive(Debug, Default, serde::Deserialize)]
struct AliasesFile {
  #[serde(default)]
  aliases: BTreeMap<String, String>,
}

/// Built-in subcommand names. Resolved from the clap `Command` factory
/// at call time — adding a new subcommand to `cli::Command` extends
/// this set automatically. Memoised inside `validate_aliases` per
/// call; the slice form here is just for the const-time lookup in
/// `ResolvedAliases::lookup` (subcommands hard-coded so the lookup
/// path doesn't need to allocate). Adding a new subcommand requires
/// adding its name here AND `tests/aliases_tests.rs` will catch a
/// miss via the canary test.
const BUILT_IN_SUBCOMMANDS: &[&str] = &[
  "init",
  "list",
  "create",
  "remove",
  "path",
  "bootstrap",
  "prune",
  "doctor",
  "types",
  "completions",
  "shell-init",
  "switch",
  "tmux",
  "zellij",
  "link",
  "unlink",
  "open",
  "status",
  "labels",
  "milestones",
  "trust",
  "aliases",
  "help",
];

/// Validate a user-supplied alias map. Used by both
/// [`crate::config::Config::validate_aliases`] (repo-level) and the
/// user-level loader, so the rules stay symmetric.
///
/// `source_label` is woven into the error message ("`.gwm.toml`
/// `[aliases]`" vs `"/home/x/.config/gwm/aliases.toml [aliases]"`)
/// so the user knows which file to edit.
///
/// Rules enforced (matching the issue contract):
///
///   1. Alias name must NOT shadow a built-in subcommand or a
///      built-in visible alias. The check uses the runtime clap
///      `CommandFactory` so it's always in sync with `src/cli.rs`.
///   2. Alias value must NOT be empty after trimming.
///   3. Alias value must NOT contain shell pipeline metachars:
///      `&&`, `||`, `|`, `;`, backticks. These would silently lose
///      semantics under argv substitution — the user must reach for
///      a shell alias instead.
pub fn validate_aliases(map: &BTreeMap<String, String>, source_label: &str) -> Result<()> {
  // Pull the built-in subcommand + alias names directly from clap so
  // adding a new subcommand to `cli::Command` automatically extends
  // the shadow check.
  let cmd = crate::cli::Cli::command();
  let mut built_ins: std::collections::HashSet<String> = std::collections::HashSet::new();
  for sub in cmd.get_subcommands() {
    built_ins.insert(sub.get_name().to_string());
    for alias in sub.get_visible_aliases() {
      built_ins.insert(alias.to_string());
    }
    for alias in sub.get_all_aliases() {
      built_ins.insert(alias.to_string());
    }
  }
  // Always include the canonical subcommand list — keeps the gate
  // honest when `validate_aliases` is called before `Cli::command()`
  // has registered new subcommands (e.g. from a test that mutates
  // the command tree).
  for name in BUILT_IN_SUBCOMMANDS {
    built_ins.insert((*name).to_string());
  }

  for (name, value) in map {
    if built_ins.contains(name) {
      return Err(GwmError::Config(format!(
        "{}: alias '{}' would shadow a built-in subcommand or alias — pick a different name",
        source_label, name
      )));
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
      return Err(GwmError::Config(format!(
        "{}: alias '{}' has an empty value — provide a subcommand to expand to",
        source_label, name
      )));
    }
    // Shell pipeline metachar gate. The list is conservative — we
    // refuse anything that LOOKS like a pipeline so the failure
    // mode is "user reads the error" rather than "user sees a
    // half-honoured alias do mysterious things".
    for pat in SHELL_METACHARS {
      if trimmed.contains(pat) {
        return Err(GwmError::Config(format!(
          "{}: alias '{}' = {:?} contains shell metachar {:?} — \
           gwm aliases are argv substitution only (no pipelines); use a shell alias instead",
          source_label, name, value, pat
        )));
      }
    }
  }
  Ok(())
}

/// Forbidden shell metachars in alias values. The list intentionally
/// stays short — anything that even hints at "shell pipeline" gets
/// rejected. A user trying to do `path | pbcopy` hits the gate and
/// reads the error pointing at shell aliases as the right tool.
const SHELL_METACHARS: &[&str] = &["&&", "||", "|", ";", "`"];
