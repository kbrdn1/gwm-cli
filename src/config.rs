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
  pub theme: ThemeConfig,
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
  #[serde(default)]
  pub issue_template: IssueTemplateConfig,
  #[serde(default)]
  pub pr_template: PrTemplateConfig,
  /// `[exec]` — named command profiles for `gwm exec --profile <name>`
  /// (issue #324). Absent block resolves to no profiles, so the inline
  /// `gwm exec -- <cmd>` surface is unchanged. Frozen for 1.0: a profile's
  /// `command` is an argv **array** (no shell), diverging from the
  /// string-shell `command` of `[git_tui]` / `[review]`.
  #[serde(default)]
  pub exec: ExecConfig,
  /// `[clean]` — named directory-set profiles for `gwm clean --profile
  /// <name>` (issue #324). Absent block resolves to no profiles, so
  /// `gwm clean` keeps cleaning the built-in `target`/`node_modules`/
  /// `dist`/`build` set. A profile's `dirs` is a COMPLETE set that
  /// replaces the built-ins, never adds to them.
  #[serde(default)]
  pub clean: CleanConfig,
}

/// `[exec]` — named command profiles for `gwm exec` (issue #324).
///
/// Each `[exec.profiles.<name>]` carries the argv to run via
/// `gwm exec --profile <name>`. The block is opt-in: an absent `[exec]`
/// resolves to an empty profile map, leaving the inline `gwm exec -- <cmd>`
/// surface untouched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecConfig {
  /// `[exec] jobs` — the global default parallelism for `gwm exec` (issue
  /// #324). `1` or absent ⇒ sequential (live, inherited stdio — the MVP
  /// behaviour); `> 1` ⇒ bounded parallel with per-worktree captured output.
  /// Precedence: `--jobs` flag > `[exec.profiles.<name>].jobs` > this > `1`.
  #[serde(default)]
  pub jobs: Option<u32>,
  /// `[exec.profiles.<name>]` sub-tables. `BTreeMap` for a deterministic
  /// ordering when surfaced.
  #[serde(default)]
  pub profiles: BTreeMap<String, ExecProfile>,
}

/// One `[exec.profiles.<name>]` entry.
///
/// `command` is an argv **array** (`["cargo", "test"]`) executed with **no
/// shell** — the same contract as the inline `gwm exec -- <cmd>`. This
/// diverges from `[git_tui]` / `[review]`, whose `command` is a single
/// shell line; the divergence is intentional and frozen for 1.0.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecProfile {
  /// argv to run in each worktree. Required — a profile with no command is
  /// a config error at load time (`deny_unknown_fields` + no `serde(default)`).
  pub command: Vec<String>,
  /// Per-profile parallelism override (issue #324). Overrides `[exec] jobs`
  /// when this profile runs; the `--jobs` flag still wins over it.
  #[serde(default)]
  pub jobs: Option<u32>,
}

/// `[clean]` — named directory-set profiles for `gwm clean` (issue #324).
///
/// Opt-in like [`ExecConfig`]: an absent `[clean]` resolves to an empty
/// profile map, so `gwm clean` keeps using the built-in directory set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanConfig {
  /// `[clean.profiles.<name>]` sub-tables. The `default` profile, when
  /// present, is what `gwm clean` uses **without** `--profile`.
  #[serde(default)]
  pub profiles: BTreeMap<String, CleanProfile>,
}

/// One `[clean.profiles.<name>]` entry.
///
/// `dirs` is a **complete** directory set that **replaces** the built-in
/// `target`/`node_modules`/`dist`/`build` — it never adds to them. The
/// safety gate (git-ignored + no tracked files + skip symlinks) still
/// applies to every listed directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanProfile {
  /// Complete set of directory names to reclaim. Required — a profile with
  /// no `dirs` is a config error at load time.
  pub dirs: Vec<String>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueTemplateConfig {
  #[serde(default)]
  pub default: Option<String>,
  #[serde(default)]
  pub by_type: BTreeMap<String, IssueTemplateTypeConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueTemplateTypeConfig {
  #[serde(default)]
  pub template: Option<String>,
  #[serde(default)]
  pub surface: Option<String>,
  #[serde(default)]
  pub title_prefix: Option<String>,
  #[serde(default)]
  pub labels: Vec<String>,
}

/// `[pr_template]` config block (issue #84). `default` is a workdir-
/// relative path to a Markdown template used as the fallback PR body;
/// `by_type` maps a branch type to either a per-type `path` or an
/// inline `body` string. The resolver in `pr_templates` picks the most
/// specific entry (per-type wins over default) and runs the templating
/// engine on the result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrTemplateConfig {
  #[serde(default)]
  pub default: Option<String>,
  #[serde(default)]
  pub by_type: BTreeMap<String, PrTemplateTypeConfig>,
}

/// Per-branch-type override under `[pr_template.by_type.<type>]`. Either
/// `path` (a workdir-relative Markdown file) or `body` (an inline
/// string) must be set; setting both is allowed and inline `body` wins
/// over `path` so a stable on-disk template can be carried in `path`
/// while a focused `body` override takes precedence for a specific
/// branch type. The resolver in `pr_templates` enforces this ordering;
/// see `inline_body_wins_over_path_when_both_set` in
/// `tests/pr_templates_tests.rs` for the pinned contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrTemplateTypeConfig {
  #[serde(default)]
  pub path: Option<String>,
  #[serde(default)]
  pub body: Option<String>,
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

/// Which side the worktree-details sidebar sits on in the side-by-side
/// TUI layout (issue #188). `Right` preserves the pre-#188 behaviour and
/// is the default. In the stacked (narrow-terminal) layout the sidebar
/// always sits at the bottom, so this preference only governs the
/// side-by-side split. Toggled live with `v`; persisted here so the
/// choice survives across launches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPosition {
  /// Sidebar on the left, worktree table on the right.
  Left,
  /// Sidebar on the right of the table — pre-#188 behaviour. Default.
  #[default]
  Right,
}

impl SidebarPosition {
  /// Every variant, in Settings-panel cycle order. Lets the panel's choice
  /// list be derived from the enum instead of restating it — see
  /// [`SidebarOrientation::ALL`] for the full reasoning.
  pub const ALL: [SidebarPosition; 2] = [SidebarPosition::Right, SidebarPosition::Left];

  /// Human-readable label for the status bar (`sidebar position: left`).
  /// Also the serialised TOML spelling, and the Settings-panel choice
  /// string — `const` so the panel's list is built from this `match`
  /// rather than duplicating it.
  pub const fn label(self) -> &'static str {
    match self {
      SidebarPosition::Left => "left",
      SidebarPosition::Right => "right",
    }
  }

  /// `true` when the sidebar should be drawn to the left of the table.
  pub fn is_left(self) -> bool {
    matches!(self, SidebarPosition::Left)
  }
}

/// How the TUI puts yanked text on the clipboard (issue #367).
///
/// The host binaries (`pbcopy`, `wl-copy`, `xclip`, …) write to the
/// clipboard of the machine gwm runs on. Over SSH that is the *remote*
/// machine: `pbcopy` is found, succeeds, reports success — and the text is
/// unreachable from the user's actual desktop. OSC52 instead hands the text
/// to the terminal emulator, which owns the real clipboard.
///
/// `lowercase` is enough here (unlike [`SidebarOrientation`]): every variant
/// is a single word, so there is no `sidebyside`-style trap to dodge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardMode {
  /// OSC52 when an SSH session is detected, host tools otherwise. Default:
  /// fixes the SSH case with no configuration, and keeps the host tools
  /// (which are universally supported) for the local case.
  #[default]
  Auto,
  /// Always emit the OSC52 escape sequence. For local terminals that support
  /// it, or when the host tools are unwanted.
  Osc52,
  /// Always use the host binaries — the pre-#367 behaviour. The escape hatch
  /// when a terminal mangles or ignores OSC52, and for `$SSH_*` false
  /// positives (a tmux pane can carry a stale `SSH_CONNECTION`).
  Tools,
}

impl ClipboardMode {
  /// Every variant, in Settings-panel cycle order (default first).
  /// Keep in sync when adding a variant — the exhaustive `match` in
  /// [`Self::label`] is what fails to compile and brings you here.
  pub const ALL: [ClipboardMode; 3] = [ClipboardMode::Auto, ClipboardMode::Osc52, ClipboardMode::Tools];

  /// Status-bar label, equal to the serialised TOML spelling. `const` so the
  /// Settings-panel choice list is derived from this `match` (#365).
  pub const fn label(self) -> &'static str {
    match self {
      ClipboardMode::Auto => "auto",
      ClipboardMode::Osc52 => "osc52",
      ClipboardMode::Tools => "tools",
    }
  }
}

/// How the sidebar is arranged relative to the worktree table (issue
/// #188). Cycled live with `cycle_sidebar_layout`; seeded from
/// `[tui] sidebar_orientation` since #365, which is why the enum lives
/// here next to [`SidebarPosition`] rather than in the TUI state module
/// (`tui::state::sidebar` re-exports it).
///
/// `kebab-case`, not `lowercase`: the latter would spell `SideBySide`
/// as `sidebyside`. This is the trap [`MacroOpenMode`] hit on PR #292,
/// and it also keeps the serialised form equal to [`Self::label`], so a
/// Settings-panel write-back produces a file that still loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SidebarOrientation {
  /// Width-driven: side-by-side at `>= SIDEBAR_MIN_WIDTH`, stacked
  /// below it. Restores a usable sidebar on narrow terminals where it
  /// was previously hidden entirely.
  Auto,
  /// Always beside the table (table | sidebar), even when narrow.
  SideBySide,
  /// Always stacked (table on top, sidebar below), even when wide.
  /// Default since issue #217: the status pane reads best under the
  /// worktrees table, where it gets the full terminal width.
  #[default]
  Stacked,
}

impl SidebarOrientation {
  /// Every variant, in Settings-panel cycle order (default first).
  ///
  /// The panel's choice list is built from these variants' labels rather than
  /// restating the strings, so the displayed choices cannot drift from the
  /// serde spelling — a drift would make the panel write a `.gwm.toml` that
  /// no longer loads.
  ///
  /// Keep in sync when adding a variant. Nothing enforces that statically; the
  /// exhaustive `match` in [`Self::label`] is what fails to compile and brings
  /// you here.
  pub const ALL: [SidebarOrientation; 3] = [
    SidebarOrientation::Stacked,
    SidebarOrientation::SideBySide,
    SidebarOrientation::Auto,
  ];

  /// Status-bar label (`sidebar layout: auto`). Equal to the serialised
  /// TOML spelling — see the type-level note. `const` so the Settings-panel
  /// choice list can be derived from this `match`.
  pub const fn label(self) -> &'static str {
    match self {
      SidebarOrientation::Auto => "auto",
      SidebarOrientation::SideBySide => "side-by-side",
      SidebarOrientation::Stacked => "stacked",
    }
  }

  /// Advance to the next orientation in the cycle
  /// `Auto → SideBySide → Stacked → Auto`. Drives `cycle_sidebar_layout`.
  pub fn next(self) -> Self {
    match self {
      SidebarOrientation::Auto => SidebarOrientation::SideBySide,
      SidebarOrientation::SideBySide => SidebarOrientation::Stacked,
      SidebarOrientation::Stacked => SidebarOrientation::Auto,
    }
  }
}

/// Where a macro command runs when fired from the TUI (#290).
/// Serialised as snake_case strings in `[tui.macro1]` / `[tui.macro2]`
/// (`open_in = "pty"` / `open_in = "mux_pane"`) — `snake_case`, not
/// `lowercase`, so the documented `"mux_pane"` value deserialises (Codex
/// review on PR #292: `lowercase` produced `"muxpane"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacroOpenMode {
  /// Open the command in an embedded PTY overlay (same as lazygit-pty /
  /// terminal-pty). The TUI suspends until the command exits.
  #[default]
  Pty,
  /// Open the command in a new pane of the running multiplexer (tmux /
  /// Zellij / GNU Screen). Falls back to `Pty` when no multiplexer is
  /// detected.
  MuxPane,
}

/// `[tui.macro1]` / `[tui.macro2]` sub-table — a user-defined command
/// that the `h` / `H` keys fire from inside the worktree TUI (#290).
/// Absent → the key does nothing (no-op). Present → the command is run
/// in the worktree's directory in the mode requested by `open_in`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiMacroConfig {
  /// Shell command to execute. Forwarded to the OS shell (`sh -c`).
  pub command: String,
  /// How the command is opened. Defaults to `pty`.
  #[serde(default)]
  pub open_in: MacroOpenMode,
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

  /// Periodic worktree-list refresh interval in seconds. Default `60`
  /// keeps Issue/PR table state reasonably fresh; `0` disables the
  /// automatic refresh loop.
  #[serde(default = "default_auto_refresh_secs")]
  pub auto_refresh_secs: u64,

  /// `[tui.open]` sub-table — drives the dispatch of the `o` key in the
  /// list view. Default mode is `shell` (lazygit-like worktree-manager
  /// workflow); pre-#73 behaviour (`open` / `xdg-open` / `explorer`) is
  /// kept available under `mode = "finder"`.
  #[serde(default)]
  pub open: TuiOpenConfig,

  /// Which side the worktree-details sidebar sits on in the side-by-side
  /// layout (issue #188). Default `right` preserves pre-#188 behaviour;
  /// `left` flips the split. Toggled live in the TUI with `v`. Ignored by
  /// the stacked (narrow-terminal) layout, where the sidebar is always at
  /// the bottom.
  #[serde(default)]
  pub sidebar_position: SidebarPosition,

  /// How the sidebar is arranged relative to the table (issue #365):
  /// `auto` (width-driven), `side-by-side`, or `stacked`. Default
  /// `stacked` preserves the #217 launch layout. Cycled live in the TUI
  /// with `cycle_sidebar_layout`; before #365 that choice was runtime-only
  /// and reset on every launch.
  #[serde(default)]
  pub sidebar_orientation: SidebarOrientation,

  /// How yanked text reaches the clipboard (issue #367): `auto` (OSC52 over
  /// SSH, host tools otherwise), `osc52`, or `tools`. Default `auto` fixes
  /// yanking over SSH — where `pbcopy` & co. silently write to the *remote*
  /// clipboard — without needing configuration.
  #[serde(default)]
  pub clipboard: ClipboardMode,

  /// `[tui.keys]` sub-table (issue #87) — user overrides for the
  /// remappable keymap. Absent → keymap stays at the built-in
  /// defaults. Present → every listed action *replaces* its default
  /// binding set; actions left unmentioned keep their defaults. An
  /// empty array (`down = []`) unbinds the action entirely.
  #[serde(default)]
  pub keys: TuiKeysConfig,

  /// `[tui.macro1]` — user-defined command bound to `h` by default (#290).
  /// Absent → the key does nothing.
  #[serde(default)]
  pub macro1: Option<TuiMacroConfig>,

  /// `[tui.macro2]` — user-defined command bound to `H` by default (#290).
  /// Absent → the key does nothing.
  #[serde(default)]
  pub macro2: Option<TuiMacroConfig>,
}

impl Default for TuiConfig {
  fn default() -> Self {
    Self {
      confirm_countdown_secs: default_confirm_countdown_secs(),
      auto_refresh_secs: default_auto_refresh_secs(),
      open: TuiOpenConfig::default(),
      sidebar_position: SidebarPosition::default(),
      sidebar_orientation: SidebarOrientation::default(),
      clipboard: ClipboardMode::default(),
      keys: TuiKeysConfig::default(),
      macro1: None,
      macro2: None,
    }
  }
}

/// `[tui.keys]` — user-facing override table for the TUI keymap.
///
/// Two kinds of entry live side by side, disambiguated by value type
/// (issue #219):
///
/// - **array value** → a *global* `View::List` verb, e.g. `quit = ["q"]`.
///   Resolved by [`Self::resolved_keymap`] into a
///   [`crate::tui::keymap::Keymap`].
/// - **table value** → a *contextual* modal sub-table, e.g.
///   `[tui.keys.modal.confirm]` or `[tui.keys.modal.link.choose_target]`. Resolved by
///   [`Self::resolved_modal_keymap`] into a
///   [`crate::tui::modal_keymap::ModalKeymap`].
///
/// A few names exist in both worlds (`create`, `help`, `command_logs`,
/// `link` are global actions *and* modal contexts). TOML forbids defining
/// the same key twice, so a user picks one per file; the value type is
/// what the walkers below key off. Resolution / validation runs at
/// `Config::load_for_repo` time via [`Config::validate_tui_keys`] so a
/// malformed override (unknown action / context / verb, parse error,
/// per-context conflict, multi-stroke modal chord) is surfaced at load
/// rather than as a silent no-op in the TUI. The raw table is preserved so
/// `gwm tui keys` can show both the user's source and the resolved set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TuiKeysConfig {
  pub raw: toml::Table,
}

impl TuiKeysConfig {
  /// Resolve the **global** `View::List` keymap: the top-level array
  /// entries of `[tui.keys]`. The `[tui.keys.modal]` sub-table (modal
  /// contexts) is skipped here and handled by [`Self::resolved_modal_keymap`].
  pub fn resolved_keymap(&self) -> Result<crate::tui::keymap::Keymap> {
    use crate::tui::keymap::{Action, KeyStroke, Keymap};

    let mut km = Keymap::defaults();
    for (action_slug, value) in &self.raw {
      // Modal contexts live under `[tui.keys.modal.<context>]` and are
      // resolved by `resolved_modal_keymap`. The dedicated namespace (issue
      // #219 review) keeps a global action and a same-named modal context
      // (`create` / `help` / `command_logs` / `link`) from colliding at the
      // `tui.keys.<name>` path during the layered merge — which previously
      // replaced the global array with the modal table and silently dropped
      // the user's global override.
      if action_slug == TUI_KEYS_MODAL_NAMESPACE {
        continue;
      }
      let chord_strings = match value {
        toml::Value::Array(_) => as_chord_list(action_slug, value)?,
        other => {
          return Err(GwmError::Config(format!(
            "tui.keys.{}: expected an array of chords; modal contexts go under [tui.keys.modal.<context>], got {}",
            action_slug,
            other.type_str()
          )))
        }
      };
      let action = Action::from_slug_compat(action_slug).ok_or_else(|| {
        GwmError::Config(format!(
          "tui.keys: unknown action {:?} (run `gwm tui keys` for the full list)",
          action_slug
        ))
      })?;
      let mut parsed = Vec::with_capacity(chord_strings.len());
      for chord_str in &chord_strings {
        let chord = KeyStroke::parse_chord(chord_str).map_err(|e| rewrap(&format!("tui.keys.{}", action_slug), e))?;
        parsed.push(chord);
      }
      km.apply_override(action, parsed)
        .map_err(|e| rewrap(&format!("tui.keys.{}", action_slug), e))?;
    }
    Ok(km)
  }

  /// Resolve the **contextual** modal keymap from the `[tui.keys.modal]`
  /// sub-table, recursing into stages (`link.choose_target`, `config.edit`).
  /// A missing `[tui.keys.modal]` table means no overrides — the built-in
  /// defaults stand. The dedicated namespace (issue #219 review) keeps modal
  /// contexts from colliding with same-named global actions during the
  /// layered merge.
  pub fn resolved_modal_keymap(&self) -> Result<crate::tui::modal_keymap::ModalKeymap> {
    use crate::tui::modal_keymap::ModalKeymap;

    let mut mk = ModalKeymap::defaults();
    let Some(modal_val) = self.raw.get(TUI_KEYS_MODAL_NAMESPACE) else {
      return Ok(mk);
    };
    let table = modal_val.as_table().ok_or_else(|| {
      GwmError::Config(format!(
        "tui.keys.modal: expected a table of modal contexts, got {}",
        modal_val.type_str()
      ))
    })?;
    for (ctx_key, value) in table {
      match value {
        toml::Value::Table(sub) => walk_modal_context(ctx_key, sub, &mut mk)?,
        other => {
          return Err(GwmError::Config(format!(
            "tui.keys.modal.{}: expected a context table, got {}",
            ctx_key,
            other.type_str()
          )))
        }
      }
    }
    Ok(mk)
  }
}

/// Sub-table key under `[tui.keys]` that holds the contextual modal bindings
/// (`[tui.keys.modal.<context>]`). See [`TuiKeysConfig::resolved_modal_keymap`].
const TUI_KEYS_MODAL_NAMESPACE: &str = "modal";

/// Extract a `["a", "b"]` chord list from a TOML array value, erroring on a
/// non-string element. `coord` is the dotted `tui.keys.…` path for messages.
fn as_chord_list(coord: &str, value: &toml::Value) -> Result<Vec<String>> {
  let arr = value
    .as_array()
    .expect("as_chord_list called on a non-array — caller must match Value::Array first");
  let mut out = Vec::with_capacity(arr.len());
  for v in arr {
    let s = v.as_str().ok_or_else(|| {
      GwmError::Config(format!(
        "tui.keys.{}: chord list must contain strings, got {}",
        coord,
        v.type_str()
      ))
    })?;
    out.push(s.to_string());
  }
  Ok(out)
}

/// `true` when `path` is a non-leaf context *group* — i.e. some real
/// context nests below it (`link` → `link.choose_target`). Used to give a
/// precise "bind under a stage" error instead of "unknown context".
fn is_modal_context_group(path: &str) -> bool {
  let prefix = format!("{}.", path);
  crate::tui::modal_keymap::KeyContext::all()
    .iter()
    .any(|c| c.config_path().starts_with(&prefix))
}

/// Walk one `[tui.keys.modal.<ctx_path>]` sub-table, applying every `verb = [keys]`
/// entry to `mk` and recursing into nested stage sub-tables.
fn walk_modal_context(
  ctx_path: &str,
  table: &toml::Table,
  mk: &mut crate::tui::modal_keymap::ModalKeymap,
) -> Result<()> {
  use crate::tui::modal_keymap::{parse_single, KeyContext, ModalAction};

  let ctx = KeyContext::from_config_path(ctx_path);
  if ctx.is_none() && !is_modal_context_group(ctx_path) {
    return Err(GwmError::Config(format!(
      "tui.keys.modal.{}: unknown modal context (run `gwm tui keys` for the list)",
      ctx_path
    )));
  }

  for (key, value) in table {
    match value {
      toml::Value::Array(_) => {
        let ctx = ctx.ok_or_else(|| {
          GwmError::Config(format!(
            "tui.keys.modal.{path}: {path:?} is a context group, not a leaf — bind under a stage (e.g. {path}.<stage>)",
            path = ctx_path
          ))
        })?;
        let coord = format!("modal.{}.{}", ctx_path, key);
        let chords = as_chord_list(&coord, value)?;
        let action = ModalAction::from_context_verb(ctx, key).ok_or_else(|| {
          GwmError::Config(format!(
            "tui.keys.modal.{}: unknown verb {:?} (run `gwm tui keys` for the list)",
            ctx_path, key
          ))
        })?;
        let mut parsed = Vec::with_capacity(chords.len());
        for s in &chords {
          parsed.push(parse_single(s).map_err(|e| rewrap(&coord, e))?);
        }
        mk.apply_override(action, parsed)
          .map_err(|e| rewrap(&format!("tui.keys.modal.{}", ctx_path), e))?;
      }
      toml::Value::Table(sub) => {
        let child = format!("{}.{}", ctx_path, key);
        walk_modal_context(&child, sub, mk)?;
      }
      other => {
        return Err(GwmError::Config(format!(
          "tui.keys.modal.{}.{}: expected an array of keys or a sub-table, got {}",
          ctx_path,
          key,
          other.type_str()
        )))
      }
    }
  }
  Ok(())
}

/// Re-wrap a parser / keymap error so the user sees the `tui.keys.<coord>`
/// coordinate rather than the bare `keymap:` prefix from the inner layer.
fn rewrap(coord: &str, e: GwmError) -> GwmError {
  let inner = match e {
    GwmError::Config(msg) => msg,
    other => other.to_string(),
  };
  GwmError::Config(format!("{}: {}", coord, inner))
}

/// `[theme]` block (issue #33) — role-based TUI colour scheme.
///
/// Two knobs:
///
/// - `preset` (optional string) — pick a built-in palette
///   (`catppuccin`, `gruvbox`, `tokyo-night`). When absent, the
///   resolved theme starts from [`crate::tui::theme::Theme::default`]
///   (the pre-#33 hardcoded scheme).
/// - Per-role keys (`focus`, `accent`, `branch`, …) — override the
///   colour of a single role on top of the preset (or default).
///   Recognised colours: named (`cyan`, `bright_blue`), indexed
///   (`220`), or hex (`#89b4fa`).
///
/// Validation runs in `Config::load_for_repo` via
/// [`Self::resolve`], so unknown presets, unknown roles, and bad
/// colour values fail at load instead of silently picking the
/// default colour at render time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeConfig {
  /// Optional preset name. `None` → start from the default scheme.
  /// `Some("catppuccin")` → seed every role from that preset.
  pub preset: Option<String>,
  /// Per-role overrides. Keys must match an entry in
  /// [`crate::tui::theme::Theme`]; values must parse via
  /// [`crate::tui::theme::parse_color`].
  #[serde(flatten)]
  pub overrides: std::collections::BTreeMap<String, String>,
}

impl ThemeConfig {
  /// Resolve this config into a [`crate::tui::theme::Theme`]:
  ///
  /// 1. Start from the preset if any (else default).
  /// 2. Apply every per-role override on top.
  ///
  /// Returns `Err(GwmError::Config(_))` on unknown preset, unknown
  /// role, or bad colour value.
  pub fn resolve(&self) -> Result<crate::tui::theme::Theme> {
    use crate::tui::theme::Theme;
    let mut theme = match &self.preset {
      Some(name) => Theme::preset(name).ok_or_else(|| {
        let known = crate::tui::theme::preset_names().join(", ");
        GwmError::Config(format!("theme.preset: unknown preset {:?} (known: {})", name, known))
      })?,
      None => Theme::default(),
    };
    for (role, value) in &self.overrides {
      // `preset` lands in `overrides` via `#[serde(flatten)]` only if
      // a user happens to also write `[theme] preset = "x"` (it
      // doesn't — the dedicated field absorbs it first). Defensive
      // guard anyway in case a future refactor moves the field.
      if role == "preset" {
        continue;
      }
      theme.apply_override(role, value)?;
    }
    Ok(theme)
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

fn default_auto_refresh_secs() -> u64 {
  60
}

/// Read a config file as a raw `toml::Value` (always a table at the
/// document root). Kept separate from `toml::from_str::<Config>` so the
/// two layers can be deep-merged at the value level before a single
/// `deny_unknown_fields` deserialization runs on the result. Issue #190.
fn read_config_value(path: &Path) -> Result<toml::Value> {
  let raw = std::fs::read_to_string(path)?;
  let val: toml::Value = toml::from_str(&raw)?;
  Ok(val)
}

/// Deep-merge `over` onto `base`: two tables merge key-by-key
/// recursively (so disjoint sections from both files coexist and a
/// nested table override keeps the untouched sibling keys); for every
/// other shape — scalars AND arrays — `over` wins wholesale. Arrays are
/// intentionally replaced, never element-wise unioned, so a repo's
/// `[[labels]]` fully supersedes the global set rather than producing a
/// confusing concatenation. Issue #190.
fn merge_toml(base: toml::Value, over: toml::Value) -> toml::Value {
  match (base, over) {
    (toml::Value::Table(mut b), toml::Value::Table(o)) => {
      for (k, ov) in o {
        let merged = match b.remove(&k) {
          Some(bv) => merge_toml(bv, ov),
          None => ov,
        };
        b.insert(k, merged);
      }
      toml::Value::Table(b)
    }
    (_, over) => over,
  }
}

/// Load a single top-level config section (e.g. `[exec]` / `[clean]`) from the
/// layered config (global `~/.config/gwm/config.toml` then repo `.gwm.toml`),
/// **tolerant of errors elsewhere** in the file but **strict on the section
/// itself**.
///
/// `gwm exec --profile` / `gwm clean` each consult exactly one section. An
/// unrelated problem — a stray top-level key, a semantic `[tui.keys]` error,
/// another section's shape — must not block them, so only the requested
/// subtree is deserialized; the rest is never validated. But the requested
/// section IS deserialized with its `deny_unknown_fields` / required-field
/// rules, so its OWN error still surfaces. That distinction matters for the
/// destructive `gwm clean`: a malformed `[clean.profiles.default]` must error
/// rather than silently revert to the built-in directory set (#324 review).
///
/// A missing section yields `T::default()`. A TOML *syntax* error (an
/// unreadable file) still surfaces — there is no section to read.
///
/// `repo_root` is `None` for a **bare** repo (no workdir, hence no repo
/// `.gwm.toml`): the repo layer is skipped, but the user-level GLOBAL config
/// is still read, so a global `[exec] jobs = N` applies even there (#324
/// review).
fn load_config_section<T>(repo_root: Option<&Path>, key: &str) -> Result<T>
where
  T: serde::de::DeserializeOwned + Default,
{
  load_config_section_layered(global_config_path().as_deref(), repo_root, key)
}

/// Core of [`load_config_section`] with the global config path injected, so
/// the layering can be pinned by a test without touching the runner's real
/// `$HOME` / `$XDG_CONFIG_HOME`.
fn load_config_section_layered<T>(global: Option<&Path>, repo_root: Option<&Path>, key: &str) -> Result<T>
where
  T: serde::de::DeserializeOwned + Default,
{
  let global_val = match global {
    Some(p) if p.exists() => Some(read_config_value(p)?),
    _ => None,
  };
  let repo_val = match repo_root {
    Some(root) => {
      let repo_path = root.join(CONFIG_FILE);
      if repo_path.exists() {
        Some(read_config_value(&repo_path)?)
      } else {
        None
      }
    }
    None => None,
  };
  let merged = match (global_val, repo_val) {
    (None, None) => return Ok(T::default()),
    (Some(g), None) => g,
    (None, Some(r)) => r,
    (Some(g), Some(r)) => merge_toml(g, r),
  };
  match merged.get(key) {
    Some(section) => section
      .clone()
      .try_into()
      .map_err(|e| GwmError::Config(format!("invalid `[{key}]` config: {e}"))),
    None => Ok(T::default()),
  }
}

/// Flatten a (possibly nested) TOML value into `key = display` rows,
/// dotted for tables and `name[i]` for arrays-of-tables, leaving scalar
/// leaves as-is. Shared by `gwm config list` (the CLI surface) and the
/// in-TUI Configuration panel (issue #232) so neither can drift from the
/// other's key shape. Pure — pushes onto `rows` in table-iteration order
/// (`toml::Value::Table` is a `BTreeMap`, so keys come out sorted).
pub(crate) fn flatten_value(prefix: &str, value: &toml::Value, rows: &mut Vec<(String, String)>) {
  match value {
    toml::Value::Table(table) => {
      for (key, value) in table {
        let next = if prefix.is_empty() {
          key.to_string()
        } else {
          format!("{}.{}", prefix, key)
        };
        flatten_value(&next, value, rows);
      }
    }
    toml::Value::Array(values) if values.iter().all(toml::Value::is_table) => {
      for (i, value) in values.iter().enumerate() {
        flatten_value(&format!("{}[{}]", prefix, i), value, rows);
      }
    }
    _ => rows.push((prefix.to_string(), format_list_value(value))),
  }
}

/// Which configuration layer a resolved value came from (issue #232).
/// Ordered by precedence so the panel can colour the strongest source
/// distinctly: a repo `.gwm.toml` overrides the user-level global, which
/// overrides the built-in defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
  /// The value is a built-in default — set in neither config file.
  Default,
  /// The value comes from the user-level global config
  /// (`~/.config/gwm/config.toml`).
  User,
  /// The value comes from the repo's `.gwm.toml`.
  Repo,
}

impl ConfigSource {
  /// Stable lowercase label for the panel's source column and tests.
  pub fn label(self) -> &'static str {
    match self {
      ConfigSource::Default => "default",
      ConfigSource::User => "user",
      ConfigSource::Repo => "repo",
    }
  }
}

/// One resolved configuration row: the flattened `key`, its `value`
/// rendered exactly as `gwm config list` prints it, and the layer that
/// `source`d it. Consumed by the in-TUI Configuration panel (issue #232).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRow {
  pub key: String,
  pub value: String,
  pub source: ConfigSource,
}

/// Flatten the raw (un-defaulted) TOML at `path` into the set of keys it
/// literally declares — the membership probe behind source attribution.
/// An absent file contributes no keys (every value then comes from a
/// lower layer). Issue #232.
fn declared_keys(path: &Path) -> Result<std::collections::HashSet<String>> {
  if !path.exists() {
    return Ok(std::collections::HashSet::new());
  }
  let value = read_config_value(path)?;
  let mut rows = Vec::new();
  flatten_value("", &value, &mut rows);
  Ok(rows.into_iter().map(|(key, _)| key).collect())
}

/// Resolve the effective configuration the way `gwm config list` does
/// (user-level global deep-merged under the repo `.gwm.toml`, defaults
/// filled), then attribute each flattened key to the layer that provided
/// it — repo over user over default. `global_path` is injected so the
/// attribution can be pinned by a test without touching the real
/// `$HOME` / `$XDG_CONFIG_HOME` (mirrors [`Config::load_layered`]).
/// Issue #232.
pub fn resolved_rows(repo_root: &Path, global_path: Option<&Path>) -> Result<Vec<ConfigRow>> {
  // The merged + defaulted config, serialised back to a value so the
  // key/value shape is identical to `gwm config list`.
  let cfg = Config::load_layered(repo_root, global_path)?;
  let value = toml::Value::try_from(cfg).map_err(|e| GwmError::Config(e.to_string()))?;
  let mut flat = Vec::new();
  flatten_value("", &value, &mut flat);

  // Per-layer declared keys probe which layer owns each merged key. Both
  // layers and the merged value flow through the same `flatten_value`, so
  // a key present in a layer's raw file matches the merged key verbatim.
  let repo_keys = declared_keys(&repo_root.join(CONFIG_FILE))?;
  let user_keys = match global_path {
    Some(p) => declared_keys(p)?,
    None => std::collections::HashSet::new(),
  };

  Ok(
    flat
      .into_iter()
      .map(|(key, value)| {
        let source = if repo_keys.contains(&key) {
          ConfigSource::Repo
        } else if user_keys.contains(&key) {
          ConfigSource::User
        } else {
          ConfigSource::Default
        };
        ConfigRow { key, value, source }
      })
      .collect(),
  )
}

/// Render a single TOML scalar (or non-table aggregate) the way
/// `gwm config list` does: strings quoted, scalars bare, arrays/tables
/// via their `Display`. Shared with the Configuration panel (issue #232).
pub(crate) fn format_list_value(value: &toml::Value) -> String {
  match value {
    toml::Value::String(s) => format!("{:?}", s),
    toml::Value::Integer(i) => i.to_string(),
    toml::Value::Float(f) => f.to_string(),
    toml::Value::Boolean(b) => b.to_string(),
    toml::Value::Datetime(d) => d.to_string(),
    toml::Value::Array(_) | toml::Value::Table(_) => value.to_string(),
  }
}

/// On-disk location of the user-level global config under a given
/// XDG config-home directory: `<config_home>/gwm/config.toml`. Pure
/// (no env / FS access) so the path contract is unit-testable. Issue
/// #190.
pub fn global_config_path_in(config_home: &Path) -> PathBuf {
  config_home.join("gwm").join("config.toml")
}

/// Pure resolver behind [`global_config_path`]: given the candidate config
/// homes and an existence predicate, pick the effective `gwm/config.toml`.
///
/// Precedence:
///   1. `$XDG_CONFIG_HOME/gwm/config.toml` — an explicit user override wins
///      outright, whether or not it exists (mirrors the pre-#372 contract).
///   2. `~/.config/gwm/config.toml` — the documented cross-platform path.
///   3. `dirs::config_dir()/gwm/config.toml` — the platform fallback
///      (`Application Support` on macOS, `%APPDATA%` on Windows).
///
/// With no `$XDG_CONFIG_HOME`, #2 and #3 are the candidate set: the first
/// that EXISTS wins, so a macOS user who put their config at the documented
/// `~/.config` path is finally honoured (issue #372), while an existing
/// `Application Support` config keeps working. When neither exists the
/// canonical `~/.config` path is returned so doctor / error messages point
/// at the documented location. On Linux #2 and #3 coincide, so resolution is
/// byte-for-byte the pre-#372 behaviour.
///
/// Pure (existence is injected) so the OS-dependent contract is unit-testable
/// against a tempdir without touching the runner's real `$HOME`. Issue #372.
pub fn resolve_global_config_path(
  xdg_config_home: Option<&Path>,
  home_dir: Option<&Path>,
  platform_config_dir: Option<&Path>,
  exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
  // An explicit $XDG_CONFIG_HOME is an intentional user choice — honour it
  // outright, existent or not (the pre-#372 contract; `merge_layered` treats
  // an absent file as no global).
  if let Some(xdg) = xdg_config_home {
    return Some(global_config_path_in(xdg));
  }
  let dotconfig = home_dir.map(|h| global_config_path_in(&h.join(".config")));
  let platform = platform_config_dir.map(global_config_path_in);
  // First existing candidate wins (documented ~/.config before the platform
  // fallback); else the canonical ~/.config path, else the platform dir.
  if let Some(p) = dotconfig.as_ref().filter(|p| exists(p)) {
    return Some(p.clone());
  }
  if let Some(p) = platform.as_ref().filter(|p| exists(p)) {
    return Some(p.clone());
  }
  dotconfig.or(platform)
}

/// Resolve the user-level global config path, honouring `$XDG_CONFIG_HOME`
/// first, then the documented `~/.config/gwm/config.toml`, then the platform
/// config dir (`dirs::config_dir()`). Returns `None` on systems where none
/// resolves (sandboxed CI / containers without `$HOME`), in which case
/// loading degrades to repo-only. Issues #190, #372.
pub fn global_config_path() -> Option<PathBuf> {
  // Opt-out: `GWM_NO_GLOBAL_CONFIG=1` reports no global path, forcing
  // repo-only loading. `load_for_repo` reads the real user-level file,
  // so this keeps `cargo test` / CI deterministic on a machine that
  // happens to have a `~/.config/gwm/config.toml`, and lets a user pin
  // strictly repo-local config. Uses the same truthy parsing as the
  // other `GWM_*` flags (`GWM_ALLOW_BOOTSTRAP`). Issue #190.
  if crate::trust::env_truthy("GWM_NO_GLOBAL_CONFIG") {
    return None;
  }
  let xdg = std::env::var("XDG_CONFIG_HOME").ok().filter(|s| !s.is_empty());
  let home = dirs::home_dir();
  let platform = dirs::config_dir();
  resolve_global_config_path(
    xdg.as_deref().map(Path::new),
    home.as_deref(),
    platform.as_deref(),
    |p| p.exists(),
  )
}

impl Config {
  /// Look for `.gwm.toml` in the given repo root, layered over the
  /// user-level global config at [`global_config_path`] (issue #190).
  /// Falls back to defaults when neither exists.
  pub fn load_for_repo(repo_root: &Path) -> Result<Self> {
    Self::load_layered(repo_root, global_config_path().as_deref())
  }

  /// Load just the `[exec]` section (layered global → repo), tolerant of
  /// errors elsewhere in the config but strict on `[exec]` itself. Used by
  /// `gwm exec --profile` so an unrelated `.gwm.toml` problem doesn't block
  /// it. See [`load_config_section`].
  ///
  /// "Strict on itself" means EVERY `[exec.profiles.*]` is validated (not just
  /// the one the command selects), so `gwm exec --profile good` rejects the
  /// same file `Config::load_for_repo` / `gwm config validate` / doctor reject
  /// — a sibling profile's semantic error can't pass on the command path only.
  pub fn load_exec_config(repo_root: &Path) -> Result<ExecConfig> {
    let cfg: ExecConfig = load_config_section(Some(repo_root), "exec")?;
    for (name, p) in &cfg.profiles {
      crate::exec::validate_exec_profile_command(name, &p.command)?;
    }
    Ok(cfg)
  }

  /// Read ONLY the `[exec] jobs` default (issue #324), without validating the
  /// `[exec.profiles.*]` semantics. Used by inline `gwm exec -- <cmd>` (no
  /// `--profile`, no `--jobs`) which needs the parallelism default but uses no
  /// profile — so a sibling profile's *semantic* issue must not block it. A
  /// shape error in `[exec]` (unknown field, wrong type) still surfaces.
  ///
  /// `repo_root` is `None` for a bare repo (no workdir): the repo `.gwm.toml`
  /// is skipped but the GLOBAL `[exec] jobs` still applies.
  pub fn load_exec_jobs_default(repo_root: Option<&Path>) -> Result<Option<u32>> {
    let cfg: ExecConfig = load_config_section(repo_root, "exec")?;
    Ok(cfg.jobs)
  }

  /// Like [`Self::load_exec_jobs_default`] but with the global config path
  /// injected, so the global-vs-repo layering can be pinned by a test without
  /// touching the runner's real `$HOME` / `$XDG_CONFIG_HOME`.
  pub fn load_exec_jobs_default_layered(global: Option<&Path>, repo_root: Option<&Path>) -> Result<Option<u32>> {
    let cfg: ExecConfig = load_config_section_layered(global, repo_root, "exec")?;
    Ok(cfg.jobs)
  }

  /// Load just the `[clean]` section (layered global → repo), tolerant of
  /// errors elsewhere but strict on `[clean]` itself. Used by `gwm clean` so
  /// an unrelated `.gwm.toml` problem doesn't block the built-in clean, while
  /// a malformed `[clean.profiles.default]` still errors rather than silently
  /// reverting to the built-in set before a destructive `--yes`. See
  /// [`load_config_section`].
  ///
  /// As with [`Self::load_exec_config`], EVERY `[clean.profiles.*]` is
  /// validated — a sibling profile that escapes the worktree can't slip
  /// through `gwm clean --profile good` while `gwm config validate` rejects it.
  ///
  /// `repo_root` is `None` for a bare repo (no workdir): the repo `.gwm.toml`
  /// is skipped but the GLOBAL `[clean]` section still applies (the built-ins
  /// are used when no `default` profile is defined).
  pub fn load_clean_config(repo_root: Option<&Path>) -> Result<CleanConfig> {
    let cfg: CleanConfig = load_config_section(repo_root, "clean")?;
    for (name, p) in &cfg.profiles {
      crate::clean::validate_clean_profile_dirs(name, &p.dirs)?;
    }
    Ok(cfg)
  }

  /// Load the effective config by deep-merging the user-level global
  /// config (`global_path`, the base) under the repo's `.gwm.toml`
  /// (the override). Issue #190.
  ///
  /// Merge rule: the repo wins on conflicting scalars; tables merge
  /// key-by-key recursively; arrays are replaced wholesale by the repo
  /// when present. Validation runs on the merged result, so a bad
  /// value from either layer fails at load. When neither file exists
  /// the bare default is returned — identical to the pre-#190
  /// behaviour, which the absent-global case preserves byte-for-byte.
  ///
  /// `global_path` is injected (rather than resolved internally) so
  /// the merge contract can be pinned by a test without touching the
  /// runner's real `$HOME` / `$XDG_CONFIG_HOME`.
  pub fn load_layered(repo_root: &Path, global_path: Option<&Path>) -> Result<Self> {
    let cfg = Self::merge_layered(repo_root, global_path)?;
    cfg.validate_branch_types()?;
    cfg.validate_bootstrap_paths()?;
    cfg.validate_bootstrap_guards()?;
    cfg.validate_labels()?;
    cfg.validate_aliases()?;
    cfg.validate_tui_keys()?;
    cfg.validate_theme()?;
    cfg.validate_profiles()?;
    Ok(cfg)
  }

  /// Reject semantically invalid `[exec.profiles.*]` / `[clean.profiles.*]`
  /// entries — an empty exec `command`, or a clean `dirs` entry that escapes
  /// the worktree (absolute, `..`, `.`/root, nested) — at config-load time, so
  /// `gwm config validate` / `gwm doctor` reject exactly what `gwm exec
  /// --profile` / `gwm clean` would (issue #324 review). The per-command
  /// resolvers share the same validators, so the two paths can't drift.
  pub(crate) fn validate_profiles(&self) -> Result<()> {
    for (name, p) in &self.exec.profiles {
      crate::exec::validate_exec_profile_command(name, &p.command)?;
    }
    for (name, p) in &self.clean.profiles {
      crate::clean::validate_clean_profile_dirs(name, &p.dirs)?;
    }
    Ok(())
  }

  /// Build the effective (deep-merged) config from disk **without** running
  /// the semantic validators. Same merge rule as [`Self::load_layered`]; only
  /// the TOML structure must be sound (a parse / shape error still fails).
  ///
  /// `gwm doctor` uses this to re-check one section against the real on-disk
  /// config even when the lenient `repo_context_lenient` defaulted the whole
  /// config away after `load_for_repo` rejected it — otherwise a check would
  /// validate the default and mask the very error it promises to surface
  /// (issue #219 review).
  pub(crate) fn merge_layered(repo_root: &Path, global_path: Option<&Path>) -> Result<Self> {
    let repo_path = repo_root.join(CONFIG_FILE);
    let global_val = match global_path {
      Some(p) if p.exists() => Some(read_config_value(p)?),
      _ => None,
    };
    let repo_val = if repo_path.exists() {
      Some(read_config_value(&repo_path)?)
    } else {
      None
    };

    Ok(match (global_val, repo_val) {
      (None, None) => Self::default(),
      (Some(g), None) => g.try_into()?,
      (None, Some(r)) => r.try_into()?,
      (Some(g), Some(r)) => merge_toml(g, r).try_into()?,
    })
  }

  /// Reject `[tui.keys]` entries that name an unknown action, list a
  /// chord that does not parse, or create a conflict / prefix
  /// collision with another binding (issue #87). Delegates to
  /// [`TuiKeysConfig::resolved_keymap`] which does the full layering +
  /// validation in one pass — the resolved keymap is discarded here
  /// (it's rebuilt by the TUI at startup); the call is only kept for
  /// its error side-effects.
  pub(crate) fn validate_tui_keys(&self) -> Result<()> {
    // Global verbs (array entries) and contextual modal verbs (table
    // entries) are validated in one pass each; the resolved keymaps are
    // discarded — they're rebuilt by the TUI at startup.
    self.tui.keys.resolved_keymap().map(|_| ())?;
    self.tui.keys.resolved_modal_keymap().map(|_| ())
  }

  /// Reject `[theme]` entries that name an unknown preset, unknown
  /// role, or unparsable colour value (issue #33). Delegates to
  /// [`ThemeConfig::resolve`] which does the full preset + override
  /// pass in one shot. The resolved theme is discarded here — the
  /// TUI rebuilds it at startup; the call is kept only for its
  /// error side-effects.
  pub(crate) fn validate_theme(&self) -> Result<()> {
    self.theme.resolve().map(|_| ())
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
    Self::write_preset(repo_root, crate::presets::GENERIC_BODY)
  }

  /// Write a `.gwm.toml` body (a built-in preset, see [`crate::presets`])
  /// to the repo root, refusing to clobber an existing file. Factored out
  /// of [`Self::write_default`] so `gwm init --preset <name>` seeds a
  /// stack-specific template through the same idempotency guard.
  pub fn write_preset(repo_root: &Path, body: &str) -> Result<PathBuf> {
    let target = repo_root.join(CONFIG_FILE);
    if target.exists() {
      return Err(GwmError::Config(format!("{} already exists", target.display())));
    }
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

/// Expand `{home}`, `{repo}`, `{repo_path}`, `{repo_parent}`, `{type}`,
/// `{issue}`, `{desc}` in a template string.
///
/// `{repo}` is the repo **name**; `{repo_path}` is the main repo's
/// absolute working directory and `{repo_parent}` its parent directory —
/// both resolved from `repo_path`. These two let a `base` be expressed
/// relative to the repo on disk (e.g. `{repo_parent}/worktrees`, matching
/// an editor's `../worktrees` convention). When `repo_path` is `None` the
/// disk-path tokens are left untouched rather than collapsed to empty.
pub fn expand_placeholders(
  template: &str,
  repo: &str,
  type_: Option<&str>,
  issue: Option<&str>,
  desc: Option<&str>,
  repo_path: Option<&Path>,
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
  if let Some(p) = repo_path {
    out = out.replace("{repo_path}", &p.to_string_lossy());
    if let Some(parent) = p.parent() {
      out = out.replace("{repo_parent}", &parent.to_string_lossy());
    }
  }
  // Tilde expansion in case the template starts with ~/...
  let expanded = shellexpand::tilde(&out).to_string();
  Ok(expanded)
}
