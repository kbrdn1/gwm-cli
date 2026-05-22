use crate::error::{GwmError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = ".gwm.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
  #[serde(default)]
  pub worktree: WorktreeConfig,
  #[serde(default)]
  pub bootstrap: BootstrapConfig,
  #[serde(default)]
  pub doctor: DoctorConfig,
  #[serde(default)]
  pub tui: TuiConfig,
  #[serde(default)]
  pub git_tui: GitTuiConfig,
  #[serde(default)]
  pub review: ReviewConfig,
  /// `[[branch_types]]` — per-repo override of the allowed branch types.
  /// Empty (the default) means the built-in list from `naming::BRANCH_TYPES`
  /// is used, keeping zero-friction for existing repos. See
  /// [`Config::resolved_branch_types`] for the single lookup site shared
  /// by `BranchSpec::validate`, `gwm types` and the TUI create picker.
  #[serde(rename = "branch_types", default)]
  pub branch_types: Vec<BranchType>,
}

/// One entry of the `[[branch_types]]` table in `.gwm.toml`. The struct
/// is also produced by [`crate::naming::default_branch_types`] when the
/// config block is absent, so both the configured and built-in flavours
/// share the same shape downstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct WorktreeConfig {
  pub base: String,
  pub path_pattern: String,
  pub branch_pattern: String,
}

impl Default for WorktreeConfig {
  fn default() -> Self {
    Self {
      base: "{home}/cc-worktree/{repo}".into(),
      path_pattern: "{type}-{issue}-{desc}".into(),
      branch_pattern: "{type}/#{issue}-{desc}".into(),
    }
  }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
pub struct NoSymlink {
  pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandStep {
  pub name: String,
  pub run: String,
  /// `file_exists:<path>` only for now.
  #[serde(default)]
  pub when: Option<String>,
  #[serde(default)]
  pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    Ok(cfg)
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
