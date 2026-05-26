use crate::error::{GwmError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = ".gwm.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
  #[serde(default)]
  pub worktree: WorktreeConfig,
  #[serde(default)]
  pub bootstrap: BootstrapConfig,
  #[serde(default)]
  pub hooks: LifecycleHooksConfig,
  #[serde(default)]
  pub doctor: DoctorConfig,
  #[serde(default)]
  pub tui: TuiConfig,
  #[serde(default)]
  pub git_tui: GitTuiConfig,
  #[serde(default)]
  pub review: ReviewConfig,
  /// `[[labels]]` table — declarative GitHub label set pushed via
  /// `gwm labels push`. Issue #81. Absent block resolves to an empty
  /// vec, so `gwm labels push` is a no-op on configs that never opt in.
  /// Whitespace in `name` is preserved verbatim (e.g. `"good first
  /// issue"`); colour falls back to a deterministic pastel hash at
  /// push time when omitted.
  #[serde(default)]
  pub labels: Vec<LabelConfig>,
  /// `[[milestones]]` table — declarative GitHub milestone set pushed
  /// via `gwm milestones push`. Issue #82. Same opt-in / no-op shape
  /// as `labels`. `due_on` accepts both `YYYY-MM-DD` (the milestones
  /// module materialises end-of-day UTC) and full RFC3339; `state`
  /// defaults to `"open"` when omitted.
  #[serde(default)]
  pub milestones: Vec<MilestoneConfig>,
  /// `[[branch_types]]` — per-repo override of the allowed branch types.
  /// Empty (the default) means the built-in list from `naming::BRANCH_TYPES`
  /// is used, keeping zero-friction for existing repos. See
  /// [`Config::resolved_branch_types`] for the single lookup site shared
  /// by `BranchSpec::validate`, `gwm types` and the TUI create picker.
  #[serde(rename = "branch_types", default)]
  pub branch_types: Vec<BranchType>,
  /// `[aliases]` table — repo-level CLI aliases expanded BEFORE clap
  /// parses argv (issue #86). Maps alias name to argv-shell-tokenised
  /// expansion (e.g. `wip = "create feat 0 wip"`). `BTreeMap` so the
  /// ordering surfaced by `gwm aliases list` is deterministic.
  ///
  /// Absent block resolves to an empty map — aliasing disabled, no
  /// behaviour change for repos that never opt in. Shadowing a
  /// built-in subcommand or visible alias is a config error surfaced
  /// at load time by [`crate::aliases::validate_aliases`]; same for
  /// values containing shell pipeline metachars.
  #[serde(default)]
  pub aliases: BTreeMap<String, String>,
  /// `[gitmoji]` table — branch type to Gitmoji shortcode overrides used
  /// by `gwm types --gitmoji` and `gwm commit-prefix`.
  #[serde(default)]
  pub gitmoji: BTreeMap<String, String>,
}

/// One `[[labels]]` entry. `name` is the GitHub key (unique per repo);
/// `description` and `color` are optional, with the colour resolved by
/// the labels module at push time (deterministic pastel by default,
/// overridable via `--random-colors`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelConfig {
  pub name: String,
  #[serde(default)]
  pub description: Option<String>,
  /// 6-character hex colour without a leading `#` (e.g. `"d73a4a"`).
  /// Validation is deferred to push time so a typo doesn't break
  /// config load for unrelated subcommands.
  #[serde(default)]
  pub color: Option<String>,
}

/// One `[[milestones]]` entry. `title` is the GitHub key (unique per
/// repo). `description`, `due_on`, and `state` are optional; the
/// milestones module validates `due_on` (YYYY-MM-DD or RFC3339) and
/// `state` (`"open"` | `"closed"`) at push time so a typo doesn't
/// break unrelated subcommands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MilestoneConfig {
  pub title: String,
  #[serde(default)]
  pub description: Option<String>,
  /// Due date. Accepted forms: `YYYY-MM-DD` (treated as end-of-day
  /// UTC at push time) or full RFC3339 (`2026-07-15T17:00:00Z`).
  #[serde(default)]
  pub due_on: Option<String>,
  /// `"open"` (default) or `"closed"`. Validated at push time.
  #[serde(default)]
  pub state: Option<String>,
}

/// One entry of the `[[branch_types]]` table in `.gwm.toml`. The struct
/// is also produced by [`crate::naming::default_branch_types`] when the
/// config block is absent, so both the configured and built-in flavours
/// share the same shape downstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchType {
  pub name: String,
  pub description: String,
}

/// Origin of the resolved branch-type list — surfaced verbatim under
/// `gwm types` so users can tell at a glance whether they're looking at
/// their `.gwm.toml` override or the built-in defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchTypesSource {
  /// No `[[branch_types]]` block in `.gwm.toml` (or it's empty) — the
  /// built-in list from `naming::BRANCH_TYPES` is in effect.
  Default,
  /// At least one `[[branch_types]]` entry was loaded from `.gwm.toml`.
  Config,
}

impl BranchTypesSource {
  /// Human-readable label rendered as the footer of `gwm types`.
  pub fn label(self) -> &'static str {
    match self {
      Self::Default => "built-in defaults",
      Self::Config => ".gwm.toml",
    }
  }
}

/// Pair returned by [`Config::resolved_branch_types`] — the list to feed
/// into validation / display, plus the [`BranchTypesSource`] that
/// produced it.
#[derive(Debug, Clone)]
pub struct ResolvedBranchTypes {
  pub types: Vec<BranchType>,
  pub source: BranchTypesSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeConfig {
  #[serde(default = "default_worktree_base")]
  pub base: String,
  #[serde(default = "default_path_pattern")]
  pub path_pattern: String,
  #[serde(default = "default_branch_pattern")]
  pub branch_pattern: String,
}

impl Default for WorktreeConfig {
  fn default() -> Self {
    Self {
      base: default_worktree_base(),
      path_pattern: default_path_pattern(),
      branch_pattern: default_branch_pattern(),
    }
  }
}

fn default_worktree_base() -> String {
  "{home}/cc-worktree/{repo}".into()
}

fn default_path_pattern() -> String {
  "{type}-{issue}-{desc}".into()
}

fn default_branch_pattern() -> String {
  "{type}/#{issue}-{desc}".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct NoSymlink {
  pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandStep {
  pub name: String,
  pub run: String,
  /// `file_exists:<path>` only for now.
  #[serde(default)]
  pub when: Option<String>,
  #[serde(default)]
  pub env: HashMap<String, String>,
}

/// `[hooks]` lifecycle automation. Each array uses the same command
/// shape as `[[bootstrap.command]]`, plus explicit failure handling.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleHooksConfig {
  #[serde(default)]
  pub pre_create: Vec<HookStep>,
  #[serde(default)]
  pub post_create: Vec<HookStep>,
  #[serde(default)]
  pub pre_bootstrap: Vec<HookStep>,
  #[serde(default)]
  pub post_bootstrap: Vec<HookStep>,
  #[serde(default)]
  pub pre_remove: Vec<HookStep>,
  #[serde(default)]
  pub post_remove: Vec<HookStep>,
}

impl LifecycleHooksConfig {
  pub fn has_any(&self) -> bool {
    !self.pre_create.is_empty()
      || !self.post_create.is_empty()
      || !self.pre_bootstrap.is_empty()
      || !self.post_bootstrap.is_empty()
      || !self.pre_remove.is_empty()
      || !self.post_remove.is_empty()
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookStep {
  pub name: String,
  pub run: String,
  #[serde(default)]
  pub when: Option<String>,
  #[serde(default)]
  pub env: HashMap<String, String>,
  #[serde(default)]
  pub on_fail: HookOnFail,
}

impl From<CommandStep> for HookStep {
  fn from(step: CommandStep) -> Self {
    Self {
      name: step.name,
      run: step.run,
      when: step.when,
      env: step.env,
      on_fail: HookOnFail::Abort,
    }
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookOnFail {
  #[default]
  Abort,
  Warn,
  Ignore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackContent {
  pub target: String,
  pub content: String,
}

/// `[doctor]` table — knobs for `gwm doctor`. Currently exposes the trunk
/// list used by the orphan-branch check; previously this was hardcoded to
/// `["dev", "main"]` in `doctor.rs`, which silently no-op'd the filter on
/// any repo using a different trunk convention (`master`, `trunk`,
/// `release-1.x`, …). Default preserves the previous behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoctorConfig {
  /// Trunk branches the orphan-branch check treats as "merge destinations".
  /// A gwm-style branch fully reachable from one of these is preserved per
  /// CONTRIBUTING.md ("never delete the source branch after merge") and is
  /// therefore not flagged as orphan. An empty list disables the filter
  /// entirely (every unclaimed gwm-style branch becomes orphan).
  #[serde(default = "default_trunks")]
  pub trunks: Vec<String>,
}

impl Default for DoctorConfig {
  fn default() -> Self {
    Self {
      trunks: default_trunks(),
    }
  }
}

fn default_trunks() -> Vec<String> {
  vec!["dev".into(), "main".into()]
}

/// `[tui]` table — runtime knobs for the worktree TUI. Currently exposes
/// the safety countdown on the delete-confirm overlay (issue #30): when
/// `delete_branch_on_remove` has been toggled ON, the modal forces the
/// user to wait N seconds (visualised by a progress bar) before the
/// destructive action actually fires. `0` disables the countdown and
/// falls back to the classic single-keystroke confirm even when delete-
/// branch is armed; the value is clamped to `5` at read time so a typo
/// like `confirm_countdown_secs = 300` can never strand a destructive
/// path behind a 300-second wait.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
  /// Safety countdown (in seconds) applied to the confirm overlay when
  /// `delete_branch_on_remove` is ON. Accepts any non-negative integer;
  /// values above [`Self::MAX_CONFIRM_COUNTDOWN_SECS`] are clamped on
  /// read via [`Self::effective_confirm_countdown_secs`]. The field is
  /// `u32` (rather than `u8`) so a typo like `confirm_countdown_secs = 300`
  /// still round-trips through TOML deserialization and reaches the
  /// clamp instead of erroring out at parse time.
  #[serde(default = "default_confirm_countdown_secs")]
  pub confirm_countdown_secs: u32,

  /// `[tui.open]` sub-table — drives the dispatch of the `o` key in the
  /// list view. Default mode is `shell` (lazygit-like worktree-manager
  /// workflow); pre-#73 behaviour (`open` / `xdg-open` / `explorer`) is
  /// kept available under `mode = "finder"`.
  #[serde(default)]
  pub open: TuiOpenConfig,
}

impl Default for TuiConfig {
  fn default() -> Self {
    Self {
      confirm_countdown_secs: default_confirm_countdown_secs(),
      open: TuiOpenConfig::default(),
    }
  }
}

/// `[tui.open]` — how the `o` key resolves the action on the selected
/// worktree. Adds a configurable hook on top of the historical "reveal
/// in OS file manager" so users with a worktree-heavy workflow can land
/// in a shell or `$EDITOR` directly, sharing the spawn-and-restore
/// lifecycle that `l: lazygit` already uses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiOpenConfig {
  #[serde(default)]
  pub mode: TuiOpenMode,
  /// Override `$SHELL` when `mode = "shell"`. Falls back to `$SHELL`,
  /// then `/bin/sh`. Empty TOML string reads as `None` so
  /// `shell_cmd = ""` and an omitted key are observationally identical.
  #[serde(default, deserialize_with = "deserialize_optional_non_empty")]
  pub shell_cmd: Option<String>,
  /// Override `$EDITOR` when `mode = "editor"`. Falls back to `$EDITOR`,
  /// then `vi`. Same empty-string-as-unset convention as `shell_cmd`.
  #[serde(default, deserialize_with = "deserialize_optional_non_empty")]
  pub editor_cmd: Option<String>,
}

/// The three documented behaviours of the `o` key. Serialised in
/// lowercase so `.gwm.toml` keys stay idiomatic (`mode = "shell"`); an
/// unknown value is a hard config error surfaced by
/// `Config::load_for_repo`, never silently coerced to a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TuiOpenMode {
  /// Spawn an interactive shell with `cwd` set to the worktree
  /// (lazygit-style suspend / spawn / restore). Default — matches the
  /// "I want to do work in this worktree" intent of a worktree manager.
  #[default]
  Shell,
  /// Spawn `$EDITOR <worktree-path>` and wait for it to exit. Useful
  /// for drive-by edits without dropping into a full shell session.
  Editor,
  /// Pre-#73 behaviour: ask the OS to reveal the worktree directory
  /// (`open` on macOS, `xdg-open` on Linux, `explorer` on Windows).
  Finder,
}

/// Serde helper: treat an empty TOML string as `None`. Keeps
/// `shell_cmd = ""` and an omitted key identical at the call site so
/// the TUI never has to special-case the empty command.
fn deserialize_optional_non_empty<'de, D>(d: D) -> std::result::Result<Option<String>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let opt = Option::<String>::deserialize(d)?;
  Ok(opt.filter(|s| !s.is_empty()))
}

impl TuiConfig {
  /// Documented range cap. Centralised so the TUI and the doctor share
  /// the same clamp logic.
  pub const MAX_CONFIRM_COUNTDOWN_SECS: u32 = 5;

  /// Effective countdown value used by the TUI, clamped to
  /// `[0, MAX_CONFIRM_COUNTDOWN_SECS]`. The raw field stays at the
  /// user's value so a future doctor check can surface "your config
  /// asked for X but we capped at 5".
  pub fn effective_confirm_countdown_secs(&self) -> u32 {
    self.confirm_countdown_secs.min(Self::MAX_CONFIRM_COUNTDOWN_SECS)
  }
}

fn default_confirm_countdown_secs() -> u32 {
  3
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
    cfg.validate_branch_types()?;
    cfg.validate_bootstrap_paths()?;
    cfg.validate_bootstrap_guards()?;
    cfg.validate_labels()?;
    cfg.validate_aliases()?;
    Ok(cfg)
  }

  /// Reject `[aliases]` entries that shadow built-in subcommands, are
  /// empty, or contain shell pipeline metachars (issue #86). Delegates
  /// to [`crate::aliases::validate_aliases`] so the same rules apply
  /// symmetrically to the user-level `~/.config/gwm/aliases.toml`.
  pub(crate) fn validate_aliases(&self) -> Result<()> {
    crate::aliases::validate_aliases(&self.aliases, ".gwm.toml `[aliases]`")
  }

  /// Reject `[[labels]]` entries whose `name` would be parsed as a flag
  /// by `gh label create` (or violate GitHub's naming rules). Delegates
  /// per-entry validation to [`crate::labels::validate_label_name`]; the
  /// error here prefixes the offending entry index so the user can
  /// locate the TOML coordinate without grepping the file (issue #100).
  ///
  /// The inner error is unwrapped from its `GwmError::Config` wrapping
  /// before being re-wrapped with the entry index — otherwise the
  /// `Display` impl reads `config error: labels[<i>]: config error:
  /// labels: …` with the prefix echoed twice, which is what the user
  /// actually sees on stderr.
  pub(crate) fn validate_labels(&self) -> Result<()> {
    for (i, l) in self.labels.iter().enumerate() {
      crate::labels::validate_label_name(&l.name).map_err(|e| {
        let inner = match e {
          GwmError::Config(msg) => msg,
          other => other.to_string(),
        };
        GwmError::Config(format!("labels[{}]: {}", i, inner))
      })?;
    }
    Ok(())
  }

  /// Pre-compile every `[[bootstrap.guard]].deny_patterns` entry so a
  /// malformed regex surfaces at config load instead of being silently
  /// dropped at evaluation time (issue #96).
  ///
  /// Historically `bootstrap.rs::guard_match` wrapped `Regex::new(pat)`
  /// in `if let Ok(re) = …`, which made a guard fail-open whenever one
  /// of its patterns failed to compile: the bad pattern vanished and
  /// the surviving patterns evaluated against the file as if nothing
  /// was wrong. A refusal mechanism that silently refuses to refuse is
  /// strictly worse than no mechanism — the user reads "guard passed"
  /// and trusts a file that never went through the rule it was meant
  /// to be filtered by.
  ///
  /// The compiled regexes are deliberately discarded here: the goal
  /// of this validator is to fail fast at load time, and caching a
  /// `Vec<Regex>` on the `Guard` struct would force `#[serde(skip)]`
  /// gymnastics on a type that round-trips through TOML.
  ///
  /// **Trust boundary**: `Config::load_for_repo` is the primary
  /// chokepoint this validator protects. `bootstrap::guard_match`
  /// holds the matching defence-in-depth for `Config` values that
  /// bypass the loader (test fixtures, programmatic constructors,
  /// future APIs): a runtime `Regex::new` failure surfaces as a
  /// `StepStatus::Failed` step and refuses the copy, instead of
  /// silently dropping the pattern as the original #96 fail-open
  /// did.
  pub fn validate_bootstrap_guards(&self) -> Result<()> {
    for (gi, g) in self.bootstrap.guard.iter().enumerate() {
      for (pi, pat) in g.deny_patterns.iter().enumerate() {
        regex::Regex::new(pat).map_err(|e| {
          // Include the guard index AND the pattern index so a `.gwm.toml`
          // with five guards × five patterns surfaces "bootstrap.guard[3].
          // deny_patterns[1]" — the exact TOML coordinate — instead of
          // forcing the user to grep for the pattern content. Mirrors the
          // shape used by `validate_bootstrap_paths` (e.g.
          // "bootstrap.copy[0].to").
          GwmError::Config(format!(
            "bootstrap.guard[{}].deny_patterns[{}] '{}': invalid pattern {:?} — regex: {}",
            gi, pi, g.name, pat, e
          ))
        })?;
      }
    }
    Ok(())
  }

  /// Reject `..` components and absolute paths in bootstrap path
  /// fields (issue #94). The runtime guard in `bootstrap::run_copies`
  /// is the last line of defence; this check surfaces violations with
  /// the TOML key in the error rather than failing mid-bootstrap.
  ///
  /// Three fields are validated:
  ///   - `bootstrap.copy[].to` — write target inside the worktree
  ///   - `bootstrap.guard[].example_file` — read source inside main repo
  ///   - `bootstrap.fallback.<key>.target` — declarative today, but a
  ///     `..` there still misrepresents intent and is rejected for
  ///     consistency with the other two
  ///
  /// `bootstrap.copy[].from` is intentionally NOT validated here: it
  /// is joined onto `ctx.main_repo` (the repo root that ships the
  /// config), so traversal there is bounded by who can edit the
  /// `.gwm.toml` itself — same trust boundary as for the rest of
  /// the file.
  ///
  /// **Trust-boundary note (revisit with #95)**: once the TOFU prompt
  /// on `.gwm.toml` lands, `.gwm.toml` may be sourced from a less
  /// trusted location (e.g. a freshly cloned hostile main repo
  /// during the first `gwm bootstrap`). At that point the `from`
  /// trust assumption no longer holds and this validator should be
  /// extended symmetrically — `check_relative_no_traversal` already
  /// accepts an arbitrary field label and is ready for it.
  pub(crate) fn validate_bootstrap_paths(&self) -> Result<()> {
    for (i, c) in self.bootstrap.copy.iter().enumerate() {
      check_relative_no_traversal(&c.to, &format!("bootstrap.copy[{}].to", i))?;
    }
    for (i, g) in self.bootstrap.guard.iter().enumerate() {
      if let Some(ex) = &g.example_file {
        check_relative_no_traversal(ex, &format!("bootstrap.guard[{}].example_file", i))?;
      }
    }
    for (key, fb) in &self.bootstrap.fallback {
      check_relative_no_traversal(&fb.target, &format!("bootstrap.fallback.{}.target", key))?;
    }
    Ok(())
  }

  /// Validate `[[branch_types]]` entries on load so a malformed config
  /// surfaces a clear error at startup instead of failing downstream in
  /// `parse_branch` / git itself with a cryptic message. Rules:
  ///   - `name` must be non-empty
  ///   - `name` must match `^[a-z]+$` (the regex `parse_branch` uses
  ///     for the type segment of a gwm-style branch name)
  ///   - `name`s must be unique across the table — duplicates would
  ///     silently override each other under `serde`'s `Vec` decoding
  ///     and make the resolved list non-deterministic
  pub(crate) fn validate_branch_types(&self) -> Result<()> {
    let name_re = regex::Regex::new(r"^[a-z]+$").expect("static regex compiles");
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for entry in &self.branch_types {
      if entry.name.is_empty() {
        return Err(GwmError::Config(
          "branch_types: entry has empty `name`; use a lowercase ASCII alpha token (e.g. \"feat\")".into(),
        ));
      }
      if !name_re.is_match(&entry.name) {
        return Err(GwmError::Config(format!(
          "branch_types: invalid `name = \"{}\"`; must match ^[a-z]+$ to be a valid branch-prefix \
           (lowercase letters only, no digits, no dashes — git refs and `parse_branch` rely on this)",
          entry.name
        )));
      }
      if !seen.insert(entry.name.as_str()) {
        return Err(GwmError::Config(format!(
          "branch_types: duplicate entry for `name = \"{}\"` — each branch type must be declared at most once",
          entry.name
        )));
      }
    }
    Ok(())
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

  /// Single lookup site for the allowed branch types. Returns the
  /// `[[branch_types]]` block from `.gwm.toml` when present, falling
  /// back to [`crate::naming::default_branch_types`] otherwise. Used
  /// by `BranchSpec::validate`, `gwm types`, the TUI create picker
  /// (and, future-pending, the pre-commit hook) so the list stays
  /// consistent across surfaces.
  pub fn resolved_branch_types(&self) -> ResolvedBranchTypes {
    if self.branch_types.is_empty() {
      ResolvedBranchTypes {
        types: crate::naming::default_branch_types(),
        source: BranchTypesSource::Default,
      }
    } else {
      ResolvedBranchTypes {
        types: self.branch_types.clone(),
        source: BranchTypesSource::Config,
      }
    }
  }
}

/// `[git_tui]` table — drives the `l` keybinding in the TUI worktree list
/// (issue #75). Absent ⇒ legacy default `lazygit -p {path}` with
/// fullscreen=true, so no `.gwm.toml` change is required for repos that
/// were happy with the previous behaviour. Sharing `[`ReviewConfig`]`'s
/// shape (placeholder expansion + `fullscreen` flag) keeps the user's
/// mental model consistent across the two launcher keybindings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitTuiConfig {
  /// Shell line. Accepts the `{path}` placeholder. When `None`, the
  /// resolved launcher uses `lazygit -p {path}`.
  #[serde(default)]
  pub command: Option<String>,
  /// Whether gwm should suspend its own TUI before exec'ing the command.
  /// Defaults to `true` for TUI tools like lazygit / gitui / tig; set to
  /// `false` to launch e.g. a GUI editor that should run alongside gwm.
  #[serde(default)]
  pub fullscreen: Option<bool>,
}

impl GitTuiConfig {
  /// Resolve to a concrete `(command, fullscreen)` pair. The default
  /// (no `[git_tui]` block in `.gwm.toml`) is `lazygit -p {path}` so the
  /// `l` keybinding behaves exactly as it did before issue #75 landed.
  pub fn resolved(&self) -> ResolvedLauncher {
    ResolvedLauncher {
      command: self.command.clone().unwrap_or_else(|| "lazygit -p {path}".into()),
      fullscreen: self.fullscreen.unwrap_or(true),
    }
  }
}

/// `[review]` table — drives the `R` keybinding in the TUI worktree list
/// (issue #75). The user picks one of three forms:
///
/// - `command = "<shell line>"` — the primary contract; any CLI on $PATH
///   with any arguments. Placeholders `{base} {head} {path} {diff}` are
///   substituted before the shell line is split with `shell-words`.
/// - `tool = "<preset>"` — sugar for a built-in `(command, fullscreen)`
///   pair (see [`ReviewConfig::resolved`]).
/// - neither — the `R` key is inert and the status bar carries a hint.
///
/// When both are set, `command` wins (and the TUI surfaces a status-bar
/// hint at startup so the user notices their `tool` choice is shadowed).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewConfig {
  /// Shell line. Accepts `{base} {head} {path} {diff}` placeholders.
  #[serde(default)]
  pub command: Option<String>,
  /// Whether gwm should suspend its own TUI before exec'ing the command.
  /// Defaults to `false` so non-TUI tools (a linter, `gh pr view --web`)
  /// don't black-out the screen.
  #[serde(default)]
  pub fullscreen: Option<bool>,
  /// Preset name; one of `lumen`, `claude`, `codex`, `aider`, `gh`.
  /// Resolved to a `(command, fullscreen)` pair by [`ReviewConfig::resolved`].
  #[serde(default)]
  pub tool: Option<String>,
  /// Skip the shell-out when `git rev-list --count {base}..{head} == 0`.
  /// Default `true`.
  #[serde(default = "default_skip_when_no_changes")]
  pub skip_when_no_changes: bool,
  /// Optional pin for the review base ref. Slots into the base-
  /// resolution chain *after* `branch.<n>.merge` (upstream) and
  /// `branch.<n>.gwm-base`, and *before* the static `dev` / `main`
  /// fallback. Setting it overrides only the `dev` / `main` step —
  /// upstream and gwm-base still win when present. See
  /// [`crate::launcher::resolve_review_base`] for the canonical
  /// order.
  #[serde(default)]
  pub default_base: Option<String>,
}

impl Default for ReviewConfig {
  fn default() -> Self {
    Self {
      command: None,
      fullscreen: None,
      tool: None,
      skip_when_no_changes: default_skip_when_no_changes(),
      default_base: None,
    }
  }
}

fn default_skip_when_no_changes() -> bool {
  true
}

/// Reject empty strings, absolute paths, Windows drive prefixes and
/// `..` traversal segments in bootstrap path fields (issue #94). The
/// field name is woven into the error message so the user can
/// pinpoint the offending TOML key. The wording stays neutral
/// ("base directory") because callers use this helper for both
/// `worktree`-relative (`copy.to`, `fallback.target`) and
/// `main_repo`-relative (`guard.example_file`) fields.
fn check_relative_no_traversal(value: &str, field: &str) -> Result<()> {
  if value.is_empty() {
    return Err(GwmError::Config(format!(
      "{}: empty path is not a valid bootstrap target",
      field
    )));
  }
  let p = Path::new(value);
  if p.is_absolute() {
    return Err(GwmError::Config(format!(
      "{}: {:?} is an absolute path — only relative paths under the base directory are allowed",
      field, value
    )));
  }
  // `Component::Prefix` covers Windows drive-relative paths like
  // `C:foo` which are NOT absolute (per `Path::is_absolute`) yet
  // make `PathBuf::join` drop the base. Unreachable on Unix, so
  // this is a defence-in-depth rejection for Windows targets.
  for comp in p.components() {
    match comp {
      std::path::Component::ParentDir => {
        return Err(GwmError::Config(format!(
          "{}: {:?} contains '..' traversal — only relative paths under the base directory are allowed",
          field, value
        )));
      }
      std::path::Component::Prefix(_) => {
        return Err(GwmError::Config(format!(
          "{}: {:?} contains a Windows drive prefix — only relative paths under the base directory are allowed",
          field, value
        )));
      }
      _ => {}
    }
  }
  Ok(())
}

impl ReviewConfig {
  /// Resolve the user's choice to a concrete `(command, fullscreen)`
  /// pair, or `None` when neither `command` nor a recognised `tool` was
  /// set. The `command` field wins when both are present.
  pub fn resolved(&self) -> Option<ResolvedLauncher> {
    if let Some(cmd) = self.command.as_ref().filter(|s| !s.trim().is_empty()) {
      return Some(ResolvedLauncher {
        command: cmd.clone(),
        fullscreen: self.fullscreen.unwrap_or(false),
      });
    }
    let tool = self.tool.as_deref()?.trim();
    if tool.is_empty() {
      return None;
    }
    let (cmd, fullscreen_default) = review_tool_preset(tool)?;
    Some(ResolvedLauncher {
      command: cmd.into(),
      fullscreen: self.fullscreen.unwrap_or(fullscreen_default),
    })
  }

  /// True when `command` and `tool` are both set — the TUI uses this to
  /// surface a one-shot warning ("your `tool = X` is shadowed by
  /// `command = Y`") on first render.
  pub fn has_shadowed_tool(&self) -> bool {
    self.command.as_ref().is_some_and(|s| !s.trim().is_empty())
      && self.tool.as_ref().is_some_and(|s| !s.trim().is_empty())
  }
}

/// Built-in preset table for `[review].tool`. Returns `Some((command,
/// fullscreen))` for known tools, `None` otherwise.
///
/// The table is the canonical place to add new presets; it's exposed
/// (via [`ReviewConfig::resolved`]) so docs and `gwm doctor` can refer
/// to a single source of truth instead of duplicating the strings.
pub fn review_tool_preset(tool: &str) -> Option<(&'static str, bool)> {
  Some(match tool {
    "lumen" => ("lumen diff {base}..{head}", true),
    "claude" => ("claude --print 'review the diff {base}..{head}'", false),
    "codex" => ("codex review {base}..{head}", false),
    "aider" => ("aider --message 'review {base}..{head}'", true),
    "gh" => ("gh pr view --web", false),
    _ => return None,
  })
}

/// Concrete `(command, fullscreen)` pair derived from a [`GitTuiConfig`]
/// or [`ReviewConfig`] by [`GitTuiConfig::resolved`] /
/// [`ReviewConfig::resolved`]. The launcher then expands placeholders
/// in `command` and decides whether to suspend the TUI based on
/// `fullscreen`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLauncher {
  pub command: String,
  pub fullscreen: bool,
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
  let mut out = template.replace("{home}", &home).replace("{repo}", repo);
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
