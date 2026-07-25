use super::keymap::{Action, ChordResolution, KeyStroke, Keymap};
use super::modal_keymap::{KeyContext, ModalAction, ModalKeymap};
use super::palette::PaletteState;
use super::state::async_task::{CreateWorktreeResult, EditWorktreeResult, TaskKind, TaskMsg, TaskRunner};
use super::state::clean_overlay::CleanOverlay;
use super::state::command_logs::CommandLogs;
use super::state::config_panel::{ConfigPanel, FieldKind, KeyTarget, SettingField, SettingsLayer};
use super::state::confirm::{ConfirmKeyAction, ConfirmModal, CountdownTickOutcome};
use super::state::create_form::{CreateForm, Field};
use super::state::exec_picker::ExecPicker;
use super::state::filter::{fuzzy_match_indices, FilterState};
use super::state::github_fetch::{FetchKey, GitHubFetch};
use super::state::link_prompt::LinkPrompt;
use super::state::pty_overlay::PtyOverlay;
use super::state::sidebar::SidebarState;
use super::state::spinner::Spinner;
use super::theme::Theme;
use crate::bootstrap::{self, BootstrapCtx, BootstrapReport, StepStatus};
use crate::config::BranchType;
use crate::config::{CleanConfig, Config, ExecConfig, TuiOpenConfig, TuiOpenMode};
use crate::error::{GwmError, Result};
use crate::github::{self, BranchLink, IssueState, IssueStatus, PrStatus};
use crate::launcher::{self, ExpandedCommand, LauncherContext};
use crate::naming::BranchSpec;
use crate::worktree::{self, WorktreeInfo};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use git2::Repository;
use ratatui::widgets::TableState;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// Re-export the GitHub fetch state enum at its historical path
// (`tui::app::GitHubFetchState`) so callers that imported it from
// `tui::app` (or via `tui::GitHubFetchState` before the new
// `state::github_fetch` re-export landed) keep compiling. The owning
// module is now `tui::state::github_fetch` — see #128.
pub use super::state::github_fetch::GitHubFetchState;

/// Spawnable launcher plan handed to the event loop by
/// [`App::prepare_git_tui`] / [`App::prepare_review`]. Carries the
/// expanded argv, the cwd to set on the child, and the `fullscreen`
/// toggle that decides whether gwm suspends its own TUI for the call.
///
/// The `diff_file` inside `expanded` (when set) is kept alive for the
/// lifetime of the plan, so a `{diff}` tempfile survives until the
/// spawned reviewer has had a chance to consume it.
#[derive(Debug)]
pub struct LauncherPlan {
  pub expanded: ExpandedCommand,
  pub cwd: std::path::PathBuf,
  pub fullscreen: bool,
  /// Resolved base ref, when the launcher cares about it (review).
  /// `None` for the git_tui launcher. Surfaced so the status bar /
  /// caller can mention which ref was used.
  pub base: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum View {
  List,
  Create,
  Confirm,
  Report,
  Help,
  /// Compact menu to pick which GitHub URL to open (issue / pr).
  OpenMenu,
  /// Two-stage prompt: pick the link kind, then enter the number.
  LinkPrompt,
  /// Command palette (issue #32). A bottom overlay where the user
  /// types an action by name (`:create`, `:bootstrap`, …). State
  /// lives on [`App::palette`]; orchestrator methods are
  /// `open_command_palette` / `palette_push_char` / `palette_pop_char`
  /// / `palette_cycle_*` / `accept_command_palette` /
  /// `close_command_palette`.
  CommandPalette,
  /// Command Logs overlay (issue #226). A ~90% fullscreen modal over a
  /// dimmed list showing the lazygit-style transcript of the external
  /// commands gwm ran. Opened on `3`, scrolled like the help overlay;
  /// state lives on [`App::command_logs`].
  CommandLogs,
  /// Configuration panel (issue #232). A ~90% fullscreen modal over a
  /// dimmed list showing the resolved `.gwm.toml` (user-level global
  /// deep-merged under the repo file) with a per-row source column
  /// (repo / user / default). Opened on `4`, scrolled like the help
  /// overlay; state lives on [`App::config_panel`].
  Config,
  /// Embedded PTY overlay (issue #35). A ~90% fullscreen modal that renders
  /// a live PTY session (lazygit on `l`, native terminal on `o`) over the
  /// worktree list. All keys are forwarded to the child process; `Esc`
  /// kills the child and returns to the list. State lives on
  /// [`App::pty_overlay`].
  Pty,
  /// Exec profile picker overlay (issue #325). A small centred modal that
  /// lists the `[exec.profiles.*]` names; `Enter` resolves the highlight
  /// to an argv and the run loop spawns it in a PTY overlay
  /// ([`PtyKind::Exec`]) rooted at the selected worktree. State lives on
  /// [`App::exec_picker`]; keys resolve through
  /// [`crate::tui::modal_keymap::KeyContext::ExecPicker`].
  ExecPicker,
  /// Clean reclaim overlay (issue #325). A centred modal showing the gated
  /// `clean::scan_worktree_safe` report for the selected worktree, an
  /// optional `[clean.profiles.*]` picker, and a safety countdown; the run
  /// loop fires `clean::delete_reclaim` when the countdown elapses. State
  /// lives on [`App::clean_overlay`]; keys resolve through
  /// [`crate::tui::modal_keymap::KeyContext::Clean`].
  CleanReport,
  /// Worktree-rename modal (#290). Reuses the Create form (Type / Issue /
  /// Desc) pre-filled by parsing the current branch; submitting renames the
  /// local + remote branch and moves the worktree directory. State lives on
  /// [`App::create_form`] plus [`App::edit_original_branch`] /
  /// [`App::edit_original_path`].
  Edit,
  /// Generic detail overlay (issue #408). A centred row-list modal — its
  /// first consumer is the agent-session view (`a` on the worktree list);
  /// the content contract is deliberately generic so the planned rich
  /// PR/Issue view reuses it. State lives on [`App::detail_overlay`]; keys
  /// resolve through [`crate::tui::modal_keymap::KeyContext::Detail`].
  DetailOverlay,
}

/// What the run loop must do after [`App::handle_exec_picker_key`]
/// processes a key in the exec picker overlay (issue #325). Mirrors
/// [`CreateKey`] / [`LinkPromptKey`]: the testable handler owns the
/// highlight movement, the loop owns the two side effects (resolve the
/// argv + spawn the PTY overlay, or close back to the list).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ExecPickerKey {
  /// The key moved the highlight (or was ignored); stay in the picker.
  Handled,
  /// `Enter` — the loop should resolve the highlighted profile and spawn
  /// the PTY overlay.
  Submit,
  /// `Esc` — the loop should close the picker back to the list.
  Cancel,
}

/// What the run loop must do after [`App::handle_create_key`] processes a
/// key in the create overlay (issue #217). Keeps the side effects
/// (worktree creation, view transition) in the loop while the form
/// mutations stay in the testable handler.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CreateKey {
  /// The key mutated form state (or was ignored); stay in the overlay.
  Handled,
  /// `Enter` on the description field — the loop should run `submit_create`.
  Submit,
  /// `Esc` — the loop should close the overlay back to the list.
  Cancel,
}

/// What the run loop must do after [`App::handle_link_prompt_key`] processes
/// a key in the link prompt (issue #217). Mirrors [`CreateKey`]: the testable
/// handler owns the picker / digit-buffer mutations, the loop owns the two
/// side effects (the `github::link_*` shell-out, the view transition).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LinkPromptKey {
  /// The key moved the highlight, committed a target, or edited the number
  /// buffer (or was ignored); stay in the prompt.
  Handled,
  /// `Enter` on the number field — the loop should run `link_prompt_submit`.
  Submit,
  /// The resolved `fetch_github` key — the loop should refresh status.
  Refresh,
  /// `Esc` — the loop should close the prompt back to the list.
  Cancel,
}

/// Target of an open / link action. Canonical definition lives in
/// `crate::cli::LinkTarget` (it carries the `clap::ValueEnum` derive
/// for the CLI surface); the TUI re-exports the same type so a value
/// crossing the cli/tui boundary doesn't need a manual conversion
/// (issue #106).
pub use crate::cli::LinkTarget;

/// Dispatch target for the `o` key (issue #73). Resolved by
/// [`App::resolve_open_target`] from the current selection + the
/// `[tui.open]` config so the event loop can hand off to the right
/// runner (shell suspend, editor suspend, OS file manager) without
/// re-reading the config or `$SHELL` / `$EDITOR` itself.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum OpenTarget {
  /// Spawn `command` with `cwd = path`. Caller suspends the TUI and
  /// restores it on the child's exit (same lifecycle as `l: lazygit`).
  Shell { path: PathBuf, command: String },
  /// Spawn `command <path>` and wait. Same suspend/restore lifecycle
  /// as `Shell`.
  Editor { path: PathBuf, command: String },
  /// Hand off to the OS opener (`open` / `xdg-open` / `explorer`).
  /// Doesn't suspend the TUI — the opener detaches.
  Finder { path: PathBuf },
}

/// Stage of the two-step link prompt. Re-export from the extracted
/// `LinkPrompt` sub-struct (issue #126) so the existing public surface
/// (`gwm::tui::LinkPromptStage`) keeps compiling without callers
/// learning the new module path.
pub use super::state::link_prompt::LinkPromptStage;

/// One repo's session-stable metadata in workspace mode (issue #36). The live
/// `git2::Repository` is *not* stored here (it isn't `Send`/`Clone` and would
/// duplicate `App.repo`); it is re-opened from `workdir` when this repo
/// becomes the active one. `config` is cloned into `App.config` on activation
/// so per-row actions (`create`, bootstrap, hooks) read the right repo's
/// `.gwm.toml` — matching the issue's "each row inherits its own repo's
/// config" contract. Keymap/theme stay session-level (resolved once from the
/// first repo), the same "resolved once, relaunch to change" contract as
/// single-repo mode.
#[derive(Debug, Clone)]
pub struct RepoMeta {
  pub name: String,
  pub workdir: PathBuf,
  pub config: Config,
}

/// Pins per worktree path, read from each row's owning repo (its branch
/// config). Runs in the detection worker on the periodic path (round P) and
/// synchronously on user-action paths; repos are opened at most once per
/// distinct workdir. Pub: the state tests pin the owning-repo contract
/// through it without spawning the worker thread.
pub fn read_pins_from_sources(
  sources: &[(String, String, PathBuf)],
) -> std::collections::BTreeMap<String, Vec<String>> {
  let mut repos: std::collections::BTreeMap<&PathBuf, Option<Repository>> = std::collections::BTreeMap::new();
  let mut out = std::collections::BTreeMap::new();
  for (path, branch, repo_dir) in sources {
    let repo = repos.entry(repo_dir).or_insert_with(|| Repository::open(repo_dir).ok());
    let Some(repo) = repo.as_ref() else {
      continue;
    };
    let pins = crate::github::agent_pins(repo, branch).unwrap_or_default();
    if !pins.is_empty() {
      out.insert(path.clone(), pins);
    }
  }
  out
}

/// Workspace-mode state (issue #36). `None` in single-repo mode (the default).
/// The *active* repo lives in `App`'s core fields (`repo`/`repo_name`/
/// `workdir`/`config`); this holds everything needed to swap a different repo
/// into those fields as the selection moves between repos.
#[derive(Debug, Clone)]
pub struct WorkspaceState {
  /// The root `--workspace` pointed at.
  pub root: PathBuf,
  /// Session-stable repo metadata, in discovery (alphabetical) order.
  pub repos: Vec<RepoMeta>,
  /// The owning repo index for each `App.worktrees[i]` row, parallel to that
  /// vec. Rebuilt by every workspace refresh so it never drifts.
  pub row_repo: Vec<usize>,
  /// Index into `repos` of the currently active repo (mirrors `App.repo*`).
  pub active: usize,
}

pub struct App {
  pub repo: Repository,
  pub repo_name: String,
  pub workdir: PathBuf,
  pub config: Config,
  /// Workspace-mode state (issue #36); `None` in single-repo mode.
  pub workspace: Option<WorkspaceState>,
  /// Set when the selected row's repo could not be activated in workspace mode
  /// (moved / deleted / corrupt since listing). While true, `repo`/`workdir`/
  /// `config` still point at the previously active repo, so repo-mutating
  /// actions are blocked to avoid a wrong-target write (#304). Always `false`
  /// in single-repo mode and once a selection activates cleanly.
  pub workspace_active_stale: bool,
  pub worktrees: Vec<WorktreeInfo>,
  pub list_state: TableState,
  pub view: View,
  pub status: String,
  pub delete_branch_on_remove: bool,
  pub open_menu_selected: LinkTarget,

  // Create form state
  /// Create-worktree overlay state (extracted per #123). Holds field
  /// focus, type index, and the issue/slug input buffers.
  pub create_form: CreateForm,
  /// Last asynchronous create failure shown inside the Create modal.
  pub create_failure: Option<String>,
  /// Branch types displayed in the create-form picker. Resolved once at
  /// startup from [`Config::resolved_branch_types`] so the picker
  /// honours any `[[branch_types]]` override in `.gwm.toml` without
  /// re-reading the file on every key event.
  pub branch_types: Vec<BranchType>,

  // Bootstrap report
  pub report: Option<BootstrapReport>,

  /// Keybindings (help) overlay scroll offset, in rows. Reset to 0 every
  /// time the overlay opens; clamped to `help_max_scroll` (#217).
  pub help_scroll: u16,
  /// Keybindings (help) overlay horizontal scroll offset, in columns (#222).
  pub help_x_scroll: u16,
  /// Maximum help scroll offset, republished by [`super::ui::draw_help`]
  /// each frame as `content_rows.saturating_sub(viewport_rows)` so the
  /// offset can never scroll past the last line into the void.
  pub help_max_scroll: u16,
  /// Maximum horizontal help scroll offset, republished by the renderer.
  pub help_max_x_scroll: u16,

  /// Sidebar (git preview) panel state (extracted per #127). Owns the
  /// visibility / focus flags, the scroll offset + max bound, and the
  /// cached pre-rendered sections keyed by the selected worktree's
  /// path. The cache prevents re-shelling `git log` / `git status` on
  /// every TUI redraw — they only run when the selection actually
  /// changes (via [`SidebarState::on_navigation`]) or on explicit
  /// refresh ([`SidebarState::invalidate`]). The renderer publishes
  /// `sidebar.max_scroll` every frame against the actual rendered
  /// Recent Commits height; [`SidebarState::scroll_down`] clamps
  /// against it.
  pub sidebar: SidebarState,

  /// Last completed agent-session snapshot, keyed by worktree path string
  /// (issue #408). `None` until the first detection lands — the table then
  /// renders without agent cells, no placeholder noise. Replaced atomically
  /// by [`Self::apply_agent_snapshot`]; the render path only reads it.
  pub agent_snapshot: Option<std::collections::BTreeMap<String, crate::agent_sessions::WorktreeAgents>>,
  /// When the current snapshot was taken — drives the periodic re-detection
  /// in [`Self::maybe_refresh_agent_sessions`] so freshness colours do not
  /// fossilise at their startup value.
  pub agent_snapshot_at: Option<std::time::Instant>,
  /// Every session the last detection saw, matched or not — the candidate
  /// pool of the overlay's attach-by-id prompt (user feedback 2026-07-22).
  pub agent_all_sessions: Vec<crate::agent_sessions::AgentSession>,
  /// Pinned session ids per worktree path — the sidebar Agents pane shows
  /// ONLY these (user feedback 2026-07-22), and the render path must not
  /// read git config, so the map is refreshed off-render (each detection
  /// cycle + immediately after attach/detach). Empty in workspace mode
  /// (same single-repo ceiling as the pins themselves).
  pub agent_pins: std::collections::BTreeMap<String, Vec<String>>,
  /// A full pool scan was requested while a detection run was in flight —
  /// it chains after that run lands instead of walking the store
  /// concurrently (Codex review round R).
  agent_pool_wanted: bool,
  /// A pin changed while a detection run was in flight — the re-scan (and
  /// the pins refresh) chains after that run lands instead of racing a
  /// second walk against it (Codex review round U).
  agent_redetect_wanted: bool,

  // Vim motion buffer: armed by first `g`, completed by the second.
  // **Kept for backward compatibility** with pre-#87 tests that read
  // it directly. Now a *mirror* of [`Self::pending_chord`] —
  // [`Self::dispatch_key`] keeps the two synchronised via
  // [`Self::sync_legacy_pending`]. New code should consume
  // [`Self::pending_chord_is_empty`] instead.
  pub pending_g: bool,

  /// Generic pending-keys buffer for the configurable keymap
  /// (issue #87). Empty most of the time; populated with the
  /// strokes seen so far whenever the user is partway through a
  /// chord that is a prefix of a bound binding (e.g. after the
  /// first `g` of the default `g g → Top`).
  pub pending_chord: Vec<KeyStroke>,

  /// Resolved keymap for this TUI session. Built from
  /// [`Config::tui.keys`] at construction time and never mutated
  /// thereafter — the user has to relaunch gwm to pick up a config
  /// change, mirroring how every other knob in `[tui]` behaves.
  pub keymap: Keymap,

  /// Resolved contextual keymap for modals / overlays (issue #219).
  /// Built from the `[tui.keys.modal.<context>]` sub-tables at construction
  /// time alongside [`Self::keymap`]; consulted by the modal routing in
  /// `src/tui/mod.rs` to turn a keystroke into a typed [`ModalAction`].
  pub modal_keymap: ModalKeymap,

  /// Resolved colour theme for this TUI session (issue #33). Built
  /// from `[theme]` in `.gwm.toml` at construction time. Threaded
  /// through `draw_*` calls so user overrides reach every visual
  /// signal. Same hot-reload-on-relaunch contract as the keymap.
  pub theme: Theme,

  // Inline fuzzy filter on the worktree list (issue #21, extracted per
  // #124 with memoisation closing #104). The sub-struct owns the buffer
  // (`query`), the typing-bar flag (`active`), and a cached indices vec
  // so the 3–5 `tui/ui.rs` call sites per render frame don't each rerun
  // the `nucleo_matcher` pass. `App::refresh` calls
  // `self.filter.invalidate()` to drop the cache when `worktrees`
  // changes; a worktrees-length mismatch auto-invalidates too.
  pub filter: FilterState,

  // Picker mode (issue #22): `gwm switch` runs the TUI as a stripped-down
  // picker. Create / delete / bootstrap keys are inert; Enter records the
  // highlighted worktree path into `picker_result` and the event loop quits
  // so the CLI caller can print the path on stdout for `cd "$(gwm switch)"`.
  pub picker_mode: bool,
  pub picker_result: Option<PathBuf>,
  /// Event-loop exit signal for picker mode. Driven by `picker_confirm`
  /// (only when a worktree is actually selected) and `picker_cancel` (Esc
  /// from inside the filter bar, where a blanket `break` would clash with
  /// the regular TUI's clear-filter behaviour). Keeps the loop running on
  /// Enter-with-no-match so the user can back-space and refine the filter
  /// instead of being kicked out with exit code 1.
  pub picker_should_exit: bool,

  /// Event-loop exit signal for `Action::Quit` fired from a path
  /// that cannot itself `break` the loop (issue #32: the command
  /// palette routes accepted actions through `run_action`, which
  /// returns `Result<()>` and has no `break` channel). Set by
  /// `run_action` when it sees `Action::Quit`; checked at the top
  /// of every event-loop iteration alongside `picker_should_exit`.
  pub should_quit: bool,

  /// Safety countdown state for the confirm overlay (issue #30, extracted
  /// per #125). Holds the timer anchor and exposes the pure state-machine
  /// API; this `App` keeps the side-effecting wrappers below that compose
  /// the status messages and call `worktree::remove`.
  pub confirm: ConfirmModal,

  /// Last delete-worktree failure shown inside the confirm modal (issue
  /// #257). Kept on `App`, not `ConfirmModal`, because it is the outcome of
  /// the async worktree deletion side effect rather than countdown state.
  pub delete_failure: Option<String>,

  /// Animated loader for overlays (issue #187). Advanced by the event
  /// loop's 200ms poll tick while the confirm countdown is armed and
  /// read by the renderer; pure state lives in
  /// [`super::state::spinner::Spinner`].
  pub spinner: Spinner,

  // ---- Issue/PR linking (issue #67) -------------------------------------
  /// GitHub fetch state slice — owns the cached link for the currently
  /// selected worktree's branch, the repo slug parsed from `origin`,
  /// and the per-target `gh issue view` / `gh pr view` fetch state
  /// (extracted per #128, part 6/6 of the `App` god-struct
  /// decomposition #102). The orchestrator methods below
  /// (`refresh_link`, `refresh_github_status`,
  /// `apply_issue_fetch_result`, `apply_pr_fetch_result`) are thin
  /// wrappers that compose the status-bar copy + drive the actual
  /// `gh` shell-outs; the pure state machine lives on
  /// `GitHubFetch`.
  pub github: GitHubFetch,
  /// Two-stage issue/PR link prompt state (extracted per #126). Owns
  /// the stage + target + digit buffer; the orchestrator wraps the
  /// transitions to update the status bar and shell out to
  /// `github::link_{issue,pr}` on submit.
  link_prompt: LinkPrompt,

  /// Command palette overlay state (issue #32). Opened by
  /// `Action::CommandPalette` (default `:` binding). The pure state
  /// machine — buffer, fuzzy-matched candidates, highlight cursor —
  /// lives on `PaletteState`; this `App` owns the view transition
  /// and routes the accepted `Action` back through the normal
  /// dispatcher so palette and keymap fire identical side effects.
  pub palette: PaletteState,

  /// TOFU trust mode for this TUI session (issue #95). Resolved at
  /// the CLI entrypoint from `--allow-bootstrap` / `--deny-bootstrap`
  /// / `GWM_ALLOW_BOOTSTRAP=1` and threaded down via `tui::run(mode)`.
  /// Used by `check_trust_for_bootstrap` to gate `submit_create` and
  /// `bootstrap_selected` — same security policy as the CLI, no
  /// bypass via the TUI. Default `Prompt` (preserves the safe
  /// default when callers construct `App` directly, e.g. tests that
  /// don't care about the gate).
  pub trust_mode: crate::trust::TrustMode,

  /// Generic off-thread task spine (issue #231; GitHub fetch folded in by
  /// #255): coalescing + per-key generation late-drop for slow one-shot
  /// ops — the worktree list refresh and the `gh issue/pr view` fetches.
  /// Public for the same reason `github` is — the state-machine tests
  /// claim a generation directly without spawning an OS thread.
  pub tasks: TaskRunner,
  /// Last point at which the periodic TUI worktree refresh was armed.
  /// Tests set this directly to simulate elapsed time without sleeping.
  pub last_auto_refresh_at: Instant,
  /// Sender cloned into each background task worker (issue #231; carries the
  /// GitHub fetch results too since #255).
  task_tx: mpsc::Sender<TaskMsg>,
  /// Receiver drained by [`Self::drain_task_results`] each event-loop tick.
  /// A worker whose `App` has dropped simply fails its `send` and is ignored.
  task_rx: mpsc::Receiver<TaskMsg>,

  /// Command Logs overlay state (issue #226): the scroll cursor plus an
  /// owned snapshot of the [`crate::command_log`] global, so the modal
  /// renders off `App` state rather than locking the global mid-frame.
  pub command_logs: CommandLogs,

  /// Configuration panel overlay state (issue #232): the scroll cursor
  /// plus the resolved-row snapshot, filled by [`Self::enter_config_panel`].
  pub config_panel: ConfigPanel,

  /// The user-level global config path this `App` was constructed with
  /// (issue #232). Stored so [`Self::enter_config_panel`] resolves the
  /// panel's source attribution against the *same* layers the running
  /// config was loaded from — `None` in tests / sandboxed runs with no
  /// global file, matching [`Config::load_layered`]'s injection point.
  global_path: Option<PathBuf>,

  /// Live PTY overlay state (issue #35). `Some` while a lazygit or native
  /// terminal PTY session is open; `None` at all other times.
  /// Managed by [`Self::open_pty_overlay`] / [`Self::close_pty_overlay`].
  pub pty_overlay: Option<PtyOverlay>,

  /// Exec profile picker overlay state (issue #325). Populated by
  /// [`Self::enter_exec_picker`] from `[exec.profiles.*]`; on `Enter` the
  /// run loop resolves the highlight to an argv and spawns a PTY overlay
  /// ([`PtyKind::Exec`]) in the selected worktree's directory.
  pub exec_picker: ExecPicker,

  /// The `[exec]` config captured when the exec picker opened (issue #325).
  /// In workspace mode `sync_active_repo` can swap `self.config` to another
  /// repo while the overlay is open, so `Enter` resolves the argv against
  /// this snapshot — the active repo's `[exec]` at open time — not the live
  /// config (Codex #333 review).
  exec_picker_cfg: ExecConfig,

  /// Clean overlay state (issue #325). Holds the gated reclaim scan of the
  /// selected worktree, the `[clean.profiles.*]` picker, and a dedicated
  /// safety countdown. Filled by [`Self::enter_clean_overlay`]; the run loop
  /// fires [`crate::clean::delete_reclaim`] when the countdown elapses.
  pub clean_overlay: CleanOverlay,

  /// The `[clean]` config captured when the clean overlay opened (issue
  /// #325) — every re-scan and the delete resolve their dir-set against this
  /// snapshot, not the live `self.config.clean`, which a workspace
  /// auto-refresh could swap to another repo's (Codex #333 review).
  clean_overlay_cfg: CleanConfig,

  /// The safety-countdown duration (seconds) captured when the clean overlay
  /// opened (issue #325). Pinned alongside [`Self::clean_overlay_cfg`] so a
  /// workspace config swap can't shorten — or clear to `0` — the delay
  /// before an armed reclaim fires (Codex #333 review).
  clean_overlay_countdown_secs: u32,

  /// Generic detail overlay content (issue #408) — filled by
  /// [`Self::open_agent_overlay`] while [`View::DetailOverlay`] is up.
  pub detail_overlay: crate::tui::state::detail_overlay::DetailOverlay,

  /// The worktree the open detail overlay was built for — `(path, branch)`
  /// captured at open so attach/detach pin against it even if an
  /// auto-refresh drifts the live selection (clean-overlay pattern).
  detail_overlay_target: Option<(PathBuf, Option<String>)>,

  /// CI-consumer counterpart of `detail_overlay_target` (Codex review
  /// #455): the PR number the open CI checks overlay was built for,
  /// captured by [`Self::enter_ci_checks`]. A refresh whose re-detected
  /// link disagrees (the PR changed or disappeared) closes the overlay up
  /// front — otherwise the stale checks stay up through the new fetch,
  /// and forever if it fails, with `Enter` opening an old PR's check URL.
  detail_overlay_pr: Option<u64>,

  /// Set by `Action::ExitToWorktree` (#290): the path the main loop
  /// should print to stdout just before quitting so the shell wrapper
  /// (`cd "$(gwm)"`) can change directory. `None` → plain quit.
  pub should_exit_to: Option<PathBuf>,

  /// The selected worktree's branch name captured when the rename modal
  /// (`View::Edit`, #290) opens — the `<old>` in `git branch -m <old> <new>`.
  /// `None` while the modal is closed.
  pub edit_original_branch: Option<String>,

  /// The selected worktree's on-disk path captured when the rename modal
  /// opens — the source for `git worktree move <old_path> <new_path>`.
  pub edit_original_path: Option<PathBuf>,

  /// Last rename failure, surfaced inside the Edit modal (mirrors
  /// [`Self::create_failure`]) so the user can correct and retry without
  /// losing the form. Cleared when the modal reopens.
  pub edit_failure: Option<String>,
}

impl App {
  pub fn new() -> Result<Self> {
    Self::new_at(None)
  }

  pub fn new_at(start: Option<&Path>) -> Result<Self> {
    Self::new_at_layered(start, crate::config::global_config_path().as_deref())
  }

  /// Injectable variant of [`Self::new_at`] (issue #194): `global_path`
  /// is the user-level global config layered under the repo's `.gwm.toml`
  /// (`None` = repo-only, no environment read). Tests pass `None` so `App`
  /// construction never depends on the runner's real
  /// `~/.config/gwm/config.toml`. `new_at` delegates with the real
  /// `global_config_path()`, so runtime behaviour is unchanged.
  pub fn new_at_layered(start: Option<&Path>, global_path: Option<&Path>) -> Result<Self> {
    let repo = worktree::discover_repo(start)?;
    let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
    let repo_name = worktree::repo_name(&repo);
    let config = Config::load_layered(&workdir, global_path)?;
    let branch_types = config.resolved_branch_types().types;
    // Resolve the keymap once at construction. Config::load_for_repo
    // already validated the overrides, so this should not surface a
    // fresh error — but we re-`?` it rather than `.expect()` so a
    // future hot-reload path could exercise the same call.
    let keymap = config.tui.keys.resolved_keymap()?;
    // Issue #219: resolve the contextual modal keymap once, same lifecycle
    // as the global keymap above. Pre-validated by `Config::load_for_repo`.
    let modal_keymap = config.tui.keys.resolved_modal_keymap()?;
    // Issue #33: resolve the colour theme once at construction.
    // Validated by `Config::load_for_repo` already, so this can
    // only surface a fresh error if the loader pre-validation is
    // bypassed (e.g. a future hot-reload path) — `?` is still the
    // right propagation policy.
    let theme = config.theme.resolve()?;
    let worktrees = worktree::list(&repo)?;
    let mut state = TableState::default();
    if !worktrees.is_empty() {
      state.select(Some(0));
    }
    let (task_tx, task_rx) = mpsc::channel();
    let mut out = Self {
      repo,
      repo_name,
      workdir,
      config,
      workspace: None,
      workspace_active_stale: false,
      worktrees,
      list_state: state,
      view: View::List,
      status: String::from("press ? for help"),
      delete_branch_on_remove: false,
      open_menu_selected: LinkTarget::Issue,
      create_form: CreateForm::new(),
      create_failure: None,
      branch_types,
      report: None,
      help_scroll: 0,
      help_x_scroll: 0,
      help_max_scroll: 0,
      help_max_x_scroll: 0,
      sidebar: SidebarState::new(),
      agent_snapshot: None,
      agent_snapshot_at: None,
      agent_all_sessions: Vec::new(),
      agent_pins: std::collections::BTreeMap::new(),
      agent_pool_wanted: false,
      agent_redetect_wanted: false,
      pending_g: false,
      pending_chord: Vec::new(),
      keymap,
      modal_keymap,
      theme,
      filter: FilterState::new(),
      picker_mode: false,
      picker_result: None,
      picker_should_exit: false,
      should_quit: false,
      confirm: ConfirmModal::new(),
      delete_failure: None,
      spinner: Spinner::new(),
      github: GitHubFetch::new(),
      link_prompt: LinkPrompt::new(),
      palette: PaletteState::new(),
      trust_mode: crate::trust::TrustMode::Prompt,
      tasks: TaskRunner::new(),
      last_auto_refresh_at: Instant::now(),
      task_tx,
      task_rx,
      command_logs: CommandLogs::new(),
      config_panel: ConfigPanel::new(),
      global_path: global_path.map(Path::to_path_buf),
      pty_overlay: None,
      exec_picker: ExecPicker::new(),
      exec_picker_cfg: ExecConfig::default(),
      clean_overlay: CleanOverlay::new(),
      clean_overlay_cfg: CleanConfig::default(),
      clean_overlay_countdown_secs: 0,
      detail_overlay: crate::tui::state::detail_overlay::DetailOverlay::default(),
      detail_overlay_target: None,
      detail_overlay_pr: None,
      should_exit_to: None,
      edit_original_branch: None,
      edit_original_path: None,
      edit_failure: None,
    };
    out.apply_sidebar_config();
    out.refresh_link();
    let spawned = out.refresh_linked_github_statuses_for_worktrees();
    if spawned > 0 {
      out.status = String::from("fetching GitHub status…");
    }
    Ok(out)
  }

  /// Workspace-mode constructor (issue #36): open the TUI over every git repo
  /// one level below `root`, merging their worktree listings into one
  /// repo-tagged table. Anchors the session on the first repo (alphabetical)
  /// for keymap/theme resolution and the event-loop channels, then swaps the
  /// merged list and per-row repo map in. Errors with [`GwmError::EmptyWorkspace`]
  /// when no repo sits directly under `root`.
  pub fn new_workspace_at_layered(root: &Path, global_path: Option<&Path>) -> Result<Self> {
    let ws = crate::workspace::discover(root)?;
    if ws.is_empty() {
      return Err(GwmError::EmptyWorkspace {
        root: root.display().to_string(),
      });
    }

    // Load each repo's `.gwm.toml` once — session-stable metadata swapped into
    // the active slot on navigation.
    let mut repos: Vec<RepoMeta> = Vec::with_capacity(ws.repos.len());
    for r in &ws.repos {
      let config = Config::load_layered(&r.path, global_path)?;
      repos.push(RepoMeta {
        name: r.name.clone(),
        workdir: r.path.clone(),
        config,
      });
    }

    // Anchor the session on the first repo: this resolves the keymap, theme,
    // branch types, and sets up the task channels exactly as single-repo mode.
    let mut app = Self::new_at_layered(Some(&repos[0].workdir), global_path)?;

    // Replace the single-repo list with the merged, repo-tagged one. Map each
    // row to its repo by the repo's *workdir path*, not its display name —
    // names can collide (a linked worktree resolving to an owner outside the
    // root, symlinks), and a name-keyed map would then point rows at the wrong
    // repo handle/config (Codex review #303 round-2 P2).
    let path_to_idx: HashMap<&Path, usize> = repos
      .iter()
      .enumerate()
      .map(|(i, m)| (m.workdir.as_path(), i))
      .collect();
    let rows = crate::workspace::merge_worktrees(&ws)?;
    let mut worktrees = Vec::with_capacity(rows.len());
    let mut row_repo = Vec::with_capacity(rows.len());
    for row in &rows {
      let idx = path_to_idx.get(row.repo_path.as_path()).copied().unwrap_or(0);
      worktrees.push(row.info.clone());
      row_repo.push(idx);
    }

    let repo_count = repos.len();
    let wt_count = worktrees.len();
    app.worktrees = worktrees;
    app.workspace = Some(WorkspaceState {
      root: root.to_path_buf(),
      repos,
      row_repo,
      active: 0,
    });
    app.filter.invalidate();
    app.list_state.select(if wt_count == 0 { None } else { Some(0) });
    // Resolve the initially-selected row's GitHub link/slug against its own
    // repo (the anchor). Workspace mode fetches GitHub state per-selection, not
    // in one cross-repo bulk pass — see `refresh_linked_github_statuses_for_worktrees`.
    app.refresh_link();
    app.status = format!(
      "workspace {} — {} repo(s), {} worktree(s) · press ? for help",
      root.display(),
      repo_count,
      wt_count
    );
    Ok(app)
  }

  /// True when the TUI is in workspace mode (issue #36).
  pub fn is_workspace(&self) -> bool {
    self.workspace.is_some()
  }

  /// Display name of the repo owning raw worktree row `raw_index` (the index
  /// into [`Self::worktrees`], not the filtered view). `None` in single-repo
  /// mode or for an out-of-range index. Drives the TUI `REPO` column.
  pub fn row_repo_name(&self, raw_index: usize) -> Option<&str> {
    let ws = self.workspace.as_ref()?;
    let idx = *ws.row_repo.get(raw_index)?;
    ws.repos.get(idx).map(|m| m.name.as_str())
  }

  /// Raw `worktrees` index of the current selection, hopping through the fuzzy
  /// filter map (the selection indexes the filtered view, not the raw vec).
  fn selected_raw_index(&self) -> Option<usize> {
    let i = self.list_state.selected()?;
    let filtered = self.filter.snapshot_indices(&self.worktrees, fuzzy_match_indices);
    filtered.get(i).copied()
  }

  /// Align the active repo (`repo`/`repo_name`/`workdir`/`config`) with the
  /// selected worktree's repo (issue #36). A no-op in single-repo mode and
  /// when the selection still belongs to the active repo, so the event loop
  /// can call it every frame cheaply. On the repo actually changing it
  /// re-opens the `git2::Repository` from the target workdir and invalidates
  /// the sidebar preview; an open failure keeps the current repo and reports
  /// on the status bar rather than panicking mid-render.
  /// Apply the `[tui]` sidebar knobs from the live config onto the sidebar
  /// state. Called from every point where `self.config` becomes authoritative:
  /// construction, the Settings-panel reload, and the workspace repo swap
  /// (`sync_active_repo`). Kept as one call rather than open-coded assignments
  /// so a future knob can't be wired into two of the three and silently drift —
  /// which is precisely how the repo-swap path came to ignore
  /// `sidebar_position` (Codex review #366 P2).
  fn apply_sidebar_config(&mut self) {
    self.sidebar.position = self.config.tui.sidebar_position;
    self.sidebar.orientation = self.config.tui.sidebar_orientation;
  }

  pub fn sync_active_repo(&mut self) {
    let Some(ws) = self.workspace.as_ref() else {
      return;
    };
    let Some(raw) = self.selected_raw_index() else {
      // No visible/selected row (e.g. the filter hides everything): there is no
      // active repo the selection points at, so writes must not fall through to
      // the previously active repo — mark stale to block them (#304). Reached
      // only in workspace mode (the `ws` guard above returns in single-repo).
      self.workspace_active_stale = true;
      return;
    };
    let Some(&target) = ws.row_repo.get(raw) else {
      self.workspace_active_stale = true;
      return;
    };
    if target == ws.active {
      // The selection is on the live, already-activated repo — clear any stale
      // flag left over from a previous unreachable selection.
      self.workspace_active_stale = false;
      return;
    }
    let Some(meta) = ws.repos.get(target).cloned() else {
      return;
    };
    match Repository::open(&meta.workdir) {
      Ok(repo) => {
        self.repo = repo;
        self.repo_name = meta.name;
        self.workdir = meta.workdir;
        self.config = meta.config;
        self.workspace_active_stale = false;
        // The branch types drive the create form; re-resolve them from the
        // newly-active repo's config so a per-repo `[[branch_types]]` override
        // applies to the row being acted on (Codex review #303 P2).
        self.branch_types = self.config.resolved_branch_types().types;
        // Same reasoning for the sidebar layout: the swap replaced `self.config`
        // wholesale, so a per-repo `[tui]` sidebar override would otherwise be
        // ignored until a reload (Codex review #366 P2).
        self.apply_sidebar_config();
        if let Some(ws) = self.workspace.as_mut() {
          ws.active = target;
        }
        self.invalidate_sidebar_cache();
        // Re-resolve the GitHub link + slug against the now-active repo so the
        // Issue/PR panel and the `F` refresh act on the selected row's own
        // repo, not the previously-active one (Codex review #303 P2). The
        // per-repo nav hook (`on_navigation`) ran `refresh_link` *before* this
        // swap, while `self.repo` still pointed at the old repo.
        self.refresh_link();
      }
      Err(e) => {
        // Keep the previously active repo live but mark the selection stale so
        // repo-mutating actions are blocked until the user moves to a
        // reachable row (or a refresh drops the dead repo) — #304.
        self.workspace_active_stale = true;
        self.status = format!(
          "workspace: repo '{}' is unavailable ({}) — press r to refresh",
          meta.name, e
        );
      }
    }
  }

  /// Set the active config, keeping the workspace cache coherent. In
  /// workspace mode the per-repo `RepoMeta.config` is the source of truth that
  /// `sync_active_repo` restores on activation, so a settings/keymap reload
  /// that only updated `self.config` would be reverted the next time the user
  /// navigated away and back (Codex review #303 P3). Write the reloaded config
  /// through to the active repo's cached meta too.
  fn set_active_config(&mut self, cfg: Config) {
    self.config = cfg;
    if let Some(ws) = self.workspace.as_mut() {
      if let Some(meta) = ws.repos.get_mut(ws.active) {
        meta.config = self.config.clone();
      }
    }
  }

  /// Reload every workspace repo's cached config from disk (issue #36). Called
  /// after a Global-layer settings edit, which changes the deep-merged config
  /// for *all* repos — without this, navigating to a non-active repo would
  /// restore the config it was loaded with at startup, reverting the edit for
  /// that repo until relaunch (Codex review #303 P2). The active repo's live
  /// `self.config` is already current (set by `set_active_config`); this
  /// re-syncs its cached meta too, so it stays the single source of truth.
  fn reload_workspace_repo_configs(&mut self) {
    let Some(ws) = self.workspace.as_ref() else {
      return;
    };
    let global = self.global_path.clone();
    let targets: Vec<(usize, PathBuf)> = ws
      .repos
      .iter()
      .enumerate()
      .map(|(i, m)| (i, m.workdir.clone()))
      .collect();
    for (i, workdir) in targets {
      if let Ok(cfg) = Config::load_layered(&workdir, global.as_deref()) {
        if let Some(ws) = self.workspace.as_mut() {
          if let Some(meta) = ws.repos.get_mut(i) {
            meta.config = cfg;
          }
        }
      }
    }
  }

  /// Per-row mask of whether each `worktrees` row belongs to the currently
  /// active repo. `None` in single-repo mode (every row qualifies). Issue/PR
  /// numbers are only unique *within* a repo, so the number-keyed GitHub state
  /// stamping must be scoped to the active repo's rows in workspace mode —
  /// otherwise a fetch for repo A's `#1` would stamp (and persist to the wrong
  /// repo) every other repo's `#1` row (Codex review #303 P2).
  fn active_repo_row_mask(&self) -> Option<Vec<bool>> {
    let ws = self.workspace.as_ref()?;
    Some(ws.row_repo.iter().map(|&r| r == ws.active).collect())
  }

  /// Re-list every repo in the workspace and rebuild the merged table +
  /// row→repo map (issue #36). The single-repo async refresh would clobber the
  /// merged list with one repo's worktrees, so workspace refresh runs
  /// synchronously across all repos instead. Repos are fixed for the session
  /// (a new repo under the root needs a relaunch, matching the config "resolved
  /// once" contract), so this re-lists the stored metas rather than re-walking
  /// the root.
  fn refresh_workspace(&mut self) {
    let targets = self.workspace_refresh_targets();
    let rows = Self::list_workspace(&targets);
    self.apply_workspace_worktrees(rows);
  }

  /// The `(repo_index, workdir)` targets a workspace re-list walks — every
  /// repo's stored `workdir` (repos are fixed for the session). Owned `Send`
  /// data, so the async worker ([`Self::spawn_refresh_workspace`], issue #343)
  /// can move it across the thread boundary; the synchronous path uses it too.
  fn workspace_refresh_targets(&self) -> Vec<(usize, PathBuf)> {
    self
      .workspace
      .as_ref()
      .map(|ws| {
        ws.repos
          .iter()
          .enumerate()
          .map(|(i, m)| (i, m.workdir.clone()))
          .collect()
      })
      .unwrap_or_default()
  }

  /// Open + list every workspace target into merged `(worktree, repo_index)`
  /// rows (issue #343 / #36). A static fn taking owned targets so it runs
  /// unchanged on the async worker thread or the synchronous path. Per-repo
  /// open / list errors are swallowed — a broken repo drops its rows, the rest
  /// still list — matching the pre-#343 synchronous behaviour.
  fn list_workspace(targets: &[(usize, PathBuf)]) -> Vec<(WorktreeInfo, usize)> {
    let mut rows = Vec::new();
    for (idx, workdir) in targets {
      if let Ok(repo) = Repository::open(workdir) {
        if let Ok(trees) = worktree::list(&repo) {
          for t in trees {
            rows.push((t, *idx));
          }
        }
      }
    }
    rows
  }

  /// Apply merged workspace rows: rebuild the row→repo map, swap in the merged
  /// worktree list, and re-align the active repo (issue #343 / #36). Shared by
  /// the synchronous [`Self::refresh_workspace`] and the async
  /// `RefreshWorkspace` drain so the two can never drift.
  fn apply_workspace_worktrees(&mut self, rows: Vec<(WorktreeInfo, usize)>) {
    let mut worktrees = Vec::with_capacity(rows.len());
    let mut row_repo = Vec::with_capacity(rows.len());
    for (t, idx) in rows {
      worktrees.push(t);
      row_repo.push(idx);
    }
    if let Some(ws) = self.workspace.as_mut() {
      ws.row_repo = row_repo;
    }
    self.apply_refreshed_worktrees(worktrees);
    // The selection may now land on a different repo's row — re-align the
    // active repo. `sync_active_repo` only refreshes the link when the repo
    // actually changes, so re-resolve the selected row's link/slug here too
    // (the bulk prefetch is a no-op in workspace mode).
    self.sync_active_repo();
    self.refresh_link();
  }

  /// Builder-style setter for `trust_mode`. The TUI entrypoint
  /// (`tui::run`) calls this after construction to thread through
  /// the CLI flags / env resolution; tests can use it directly to
  /// exercise each variant of the gate.
  pub fn with_trust_mode(mut self, mode: crate::trust::TrustMode) -> Self {
    self.trust_mode = mode;
    self
  }

  /// Silent TOFU gate for the TUI's bootstrap call sites
  /// (`submit_create`, `bootstrap_selected`). Returns:
  ///
  /// * `Ok(None)` — caller is cleared to invoke `bootstrap::run`.
  /// * `Ok(Some(msg))` — caller MUST NOT run bootstrap; show `msg`
  ///   to the user (e.g. assign to `self.status`). Untrusted
  ///   configs and `TrustMode::Deny` both land here — the TUI
  ///   alternate-screen can't host a stdin prompt today, so we
  ///   refuse with a hint pointing the user at the CLI gate
  ///   (`gwm bootstrap` from another terminal).
  /// * `Err(e)` — ledger I/O / config read error propagated verbatim.
  pub fn check_trust_for_bootstrap(&self) -> Result<Option<String>> {
    use crate::trust::{self, TrustOutcome};

    let origin_url = self
      .repo
      .find_remote("origin")
      .ok()
      .and_then(|r| r.url().ok().map(String::from));
    let origin = trust::resolve_origin_key(origin_url.as_deref(), &self.workdir);

    match trust::evaluate(&self.workdir, &origin, self.trust_mode)? {
      TrustOutcome::Proceed => Ok(None),
      TrustOutcome::Refuse { message } => Ok(Some(message)),
      TrustOutcome::Prompt { cfg_path, sha, .. } => {
        let short_sha: String = sha.chars().take(12).collect();
        Ok(Some(format!(
          ".gwm.toml at {} not in trust ledger (hash {}) — \
           run `gwm bootstrap` from a CLI in another terminal to approve, \
           or relaunch with GWM_ALLOW_BOOTSTRAP=1 / --allow-bootstrap",
          cfg_path.display(),
          short_sha
        )))
      }
    }
  }

  /// Constructor for `gwm switch`: same App, but picker mode is on and the
  /// fuzzy filter bar is open from the first frame so the user can start
  /// narrowing right away. Everything else (worktree list, sidebar, vim
  /// motions) behaves identically; only the event-loop interpretation of
  /// Enter / n / d / b changes.
  pub fn new_picker_at(start: Option<&Path>) -> Result<Self> {
    Self::new_picker_at_layered(start, crate::config::global_config_path().as_deref())
  }

  /// Injectable variant of [`Self::new_picker_at`] (issue #196): mirrors
  /// [`Self::new_at_layered`] so picker-mode tests never read the runner's
  /// real `~/.config/gwm/config.toml`. `new_picker_at` delegates with the
  /// real `global_config_path()`.
  pub fn new_picker_at_layered(start: Option<&Path>, global_path: Option<&Path>) -> Result<Self> {
    let mut app = Self::new_at_layered(start, global_path)?;
    app.picker_mode = true;
    app.filter.open();
    app.status = "switch picker — type to filter · enter selects · esc cancels".into();
    Ok(app)
  }

  /// Synchronous worktree list refresh. Kept for internal post-mutation
  /// callers (create / delete / report-close) that need the list fresh
  /// *before* the next render; the user-initiated `f` / `r` key path goes
  /// through the off-thread [`Self::request_refresh`] instead (issue
  /// #231). Both converge on [`Self::apply_refreshed_worktrees`] so the
  /// two paths can never drift on the post-list bookkeeping.
  pub fn refresh(&mut self) -> Result<()> {
    // A synchronous re-list (create / delete / report-close) produces
    // authoritative fresh state, so any older async refresh still in flight
    // is by definition stale — bump its generation so `drain_task_results`
    // drops the late result instead of clobbering this post-mutation list
    // with a pre-mutation snapshot (issue #231, the #138 race class). A
    // harmless counter bump when no task is running. Lives here and not in
    // `apply_refreshed_worktrees` so the async drain, which shares that
    // tail, does not re-invalidate the run it just applied.
    self.tasks.invalidate(TaskKind::RefreshWorktrees);
    // Same for an in-flight async workspace re-list (issue #343): this
    // synchronous path produces authoritative post-mutation state, so drop the
    // stale run's generation. (The in-flight *sidebar* rebuild is dropped by
    // `apply_refreshed_worktrees`, the tail every refresh path shares.)
    self.tasks.invalidate(TaskKind::RefreshWorkspace);
    if self.is_workspace() {
      // Workspace mode re-lists every repo, not just the active one (#36).
      self.refresh_workspace();
      return Ok(());
    }
    let worktrees = worktree::list(&self.repo)?;
    self.apply_refreshed_worktrees(worktrees);
    Ok(())
  }

  /// Swap in a freshly-listed worktree vec and run the bookkeeping every
  /// refresh path shares: drop the cached fuzzy-match indices (they point
  /// at the previous vec — a length change auto-invalidates, but a
  /// same-length list with different contents would not, so the explicit
  /// flush is the safe play), re-clamp the selection (which re-resolves
  /// the link cache), refresh every Issue/PR status linked by the listed
  /// rows, invalidate the sidebar preview, and report the count. Called by
  /// the synchronous [`Self::refresh`] and by the off-thread drain in
  /// [`Self::drain_task_results`].
  fn apply_refreshed_worktrees(&mut self, mut worktrees: Vec<WorktreeInfo>) {
    let old_keys: std::collections::BTreeSet<(PathBuf, Option<String>)> = self
      .worktrees
      .iter()
      .map(|w| (w.path.clone(), w.branch.clone()))
      .collect();
    // The carry-over preserves this session's in-memory fetched issue/PR state
    // across a re-list, keyed by number. In workspace mode that key collides
    // across repos (two repos can both own `#1`), so skip it: the freshly
    // listed rows already carry each repo's own *persisted* state from
    // `read_link`, which is per-repo-correct (Codex review #303 P2).
    if !self.is_workspace() {
      let issue_states: HashMap<u64, IssueState> = self
        .worktrees
        .iter()
        .filter_map(|w| Some((w.link.issue?, w.issue_state?)))
        .collect();
      let pr_states = self
        .worktrees
        .iter()
        .filter_map(|w| Some((w.link.pr?, w.pr_state?)))
        .collect::<HashMap<_, _>>();

      for w in &mut worktrees {
        if let Some(issue) = w.link.issue {
          if let Some(state) = issue_states.get(&issue).copied() {
            w.issue_state = Some(state);
          }
        }
        if let Some(pr) = w.link.pr {
          if let Some(state) = pr_states.get(&pr).copied() {
            w.pr_state = Some(state);
          }
        }
      }
    }

    self.worktrees = worktrees;
    self.filter.invalidate();
    self.clamp_selection_to_filter();
    let spawned = self.refresh_linked_github_statuses_for_worktrees();
    self.invalidate_sidebar_cache();
    // The re-list re-read git state, so any in-flight sidebar rebuild is now
    // reading *pre-refresh* data — bump its generation so a late result is
    // dropped by the drain instead of stored under the current key and rendered
    // as fresh until the next navigation (issue #343). Lives here, in the tail
    // every refresh path shares, so the OFF-thread drains (`RefreshWorktrees` /
    // `RefreshWorkspace`) get it too, not just the synchronous `refresh`.
    self.tasks.invalidate(TaskKind::Sidebar);
    // Agent staleness is keyed to the SET of (path, branch) pairs, not to
    // the refresh itself (Codex review rounds P + Q): invalidating
    // unconditionally freed the in-flight slot while its scan thread kept
    // running, so an auto-refresh faster than the scan piled up concurrent
    // scans whose results were each dropped as stale — no snapshot ever
    // landed. The branch is part of the key because pins live in BRANCH
    // config: a same-path checkout that switched branch must drop the old
    // branch's pins instead of showing them for up to 30 s. A same-keys
    // refresh keeps the in-flight run (the 30 s TTL owns freshness).
    let new_keys: std::collections::BTreeSet<(PathBuf, Option<String>)> = self
      .worktrees
      .iter()
      .map(|w| (w.path.clone(), w.branch.clone()))
      .collect();
    if old_keys != new_keys {
      self.tasks.invalidate(TaskKind::AgentSessions);
      self.agent_snapshot_at = None;
    }
    self.status = if spawned > 0 {
      format!(
        "refreshed — {} worktree(s); fetching GitHub status…",
        self.worktrees.len()
      )
    } else {
      format!("refreshed — {} worktree(s)", self.worktrees.len())
    };
  }

  /// Off-thread worktree list refresh for the `f` / `r` key (issue #231):
  /// spawn a worker that re-lists the worktrees and posts the result back
  /// to the event loop, so a large repo / slow filesystem no longer
  /// freezes the TUI. Coalesces onto an in-flight run (a second press
  /// while loading is a no-op) and seeds the loader label + spinner. The
  /// result is applied by [`Self::drain_task_results`].
  pub fn request_refresh(&mut self) {
    if self.is_workspace() {
      // Workspace mode re-lists every repo off-thread on its own slot (issue
      // #343): the single-repo worker can't be reused (it would clobber the
      // merged list with one repo's worktrees, #36), so route through
      // `RefreshWorkspace` instead of the pre-#343 synchronous `refresh()`.
      let Some(generation) = self.tasks.request(TaskKind::RefreshWorkspace) else {
        return;
      };
      self.spinner.reset();
      self.status = TaskKind::RefreshWorkspace.loading_label().into();
      self.spawn_refresh_workspace(generation);
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::RefreshWorktrees) else {
      // A refresh is already in flight — coalesce onto it.
      return;
    };
    // Start the loader from a deterministic frame and surface the label.
    self.spinner.reset();
    self.status = TaskKind::RefreshWorktrees.loading_label().into();
    self.spawn_refresh(generation);
  }

  /// Periodic worktree-list refresh for the TUI event loop. Returns `true`
  /// only when a new async refresh task was actually started. `0` disables
  /// the feature, and an in-flight refresh coalesces so the renderer is never
  /// blocked by repeated relist attempts.
  pub fn maybe_auto_refresh(&mut self, now: Instant) -> bool {
    let secs = self.config.tui.auto_refresh_secs;
    if secs == 0 {
      return false;
    }
    if now.saturating_duration_since(self.last_auto_refresh_at) < Duration::from_secs(secs) {
      return false;
    }
    self.last_auto_refresh_at = now;
    if self.is_workspace() {
      // Off-thread merged refresh in workspace mode (issue #343 / #36): the
      // per-repo `Repository::open` + `worktree::list` loop no longer freezes
      // the event loop on a many-repo workspace. Coalesces onto an in-flight
      // run so a slow relist never stacks.
      let Some(generation) = self.tasks.request(TaskKind::RefreshWorkspace) else {
        return false;
      };
      self.spinner.reset();
      self.status = "auto-refreshing worktrees…".into();
      self.spawn_refresh_workspace(generation);
      return true;
    }
    let Some(generation) = self.tasks.request(TaskKind::RefreshWorktrees) else {
      return false;
    };
    self.spinner.reset();
    self.status = "auto-refreshing worktrees…".into();
    self.spawn_refresh(generation);
    true
  }

  /// Spawn one background worktree-list worker tagged with `generation`
  /// (issue #231). A thin shell, mirroring [`Self::spawn_github_fetch`]:
  /// it owns only the off-thread dispatch + send, no state logic (the
  /// coalescing / late-drop contract lives in [`TaskRunner`], tested in
  /// `tui_state_async_task_tests.rs`). `git2::Repository` is not `Send`,
  /// so the worker opens its *own* repo from the owned `workdir` path
  /// rather than borrowing `self.repo` — the same "only owned `Send` data
  /// crosses the boundary" discipline as the GitHub worker. A `send`
  /// failure (the `App`/receiver dropped) is ignored.
  fn spawn_refresh(&self, generation: u64) {
    let tx = self.task_tx.clone();
    let workdir = self.workdir.clone();
    std::thread::spawn(move || {
      let result = worktree::discover_repo(Some(&workdir))
        .and_then(|repo| worktree::list(&repo))
        .map_err(|e| e.to_string());
      let _ = tx.send(TaskMsg::RefreshWorktrees(generation, result));
    });
  }

  /// Spawn one background workspace re-list worker tagged with `generation`
  /// (issue #343 / #36). Mirrors [`Self::spawn_refresh`] for workspace mode:
  /// the owned `(repo_index, workdir)` targets are the only data crossing the
  /// boundary, and [`Self::list_workspace`] opens each repo + lists off-thread.
  /// The drain applies the merged rows via [`Self::apply_workspace_worktrees`].
  fn spawn_refresh_workspace(&self, generation: u64) {
    let tx = self.task_tx.clone();
    let targets = self.workspace_refresh_targets();
    std::thread::spawn(move || {
      let rows = Self::list_workspace(&targets);
      let _ = tx.send(TaskMsg::RefreshWorkspace(generation, rows));
    });
  }

  /// Keep the details sidebar's git-backed preview off the render path (issue
  /// #343). Called once per event-loop tick (after `sync_active_repo`, so the
  /// active repo's `doctor.trunks` are correct in workspace mode). When the
  /// cached payload was NOT built for the current selection + mode — a cold
  /// cache, a navigation (`on_navigation` nulled it), a mode toggle, or a
  /// post-mutation `invalidate` — this spawns one worker to rebuild it, keyed
  /// to the *currently selected* worktree.
  ///
  /// Pure navigation deliberately does NOT [`TaskRunner::invalidate`] the
  /// `Sidebar` slot, so a held `j` coalesces onto the single in-flight worker
  /// instead of spawning a thread per row: the render shows the placeholder
  /// while scrolling and the settled selection is fetched once the burst ends.
  /// That coalescing IS the debounce — no timer needed. A worker whose
  /// selection has since moved stores a payload the render key-check ignores;
  /// the next tick requests the settled one.
  pub fn maybe_refresh_sidebar(&mut self) {
    // A hidden sidebar is not drawn (`draw_body` skips `draw_sidebar`), so
    // rebuilding its preview would run git work for an invisible panel —
    // restoring the pre-#343 behaviour where hiding the sidebar (`v`) did no
    // preview work at all. Opening it (`v`) re-arms the fetch on the next tick.
    if !self.sidebar.open {
      return;
    }
    let Some(w) = self.selected().cloned() else {
      return;
    };
    let mode = self.sidebar.mode;
    // Already authoritative for this selection + mode → nothing to rebuild.
    if matches!(&self.sidebar.cache, Some(((p, m), _)) if *p == w.path && *m == mode) {
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::Sidebar) else {
      // A rebuild is already in flight — coalesce onto it (the debounce).
      return;
    };
    let trunks = self.config.doctor.trunks.clone();
    let theme = self.theme;
    self.spawn_sidebar(generation, w, mode, trunks, theme);
  }

  /// Spawn one background sidebar-rebuild worker tagged with `generation`
  /// (issue #343). Mirrors [`Self::spawn_refresh`]: only owned `Send` data
  /// crosses the boundary (the [`WorktreeInfo`], mode, the active repo's
  /// `trunks`, and the `Copy` [`Theme`]), and the worker runs
  /// [`crate::tui::ui::build_sidebar_payload`], which fires every sidebar git
  /// subprocess off-thread. A `send` failure (the `App`/receiver dropped) is
  /// ignored.
  fn spawn_sidebar(
    &self,
    generation: u64,
    w: WorktreeInfo,
    mode: crate::tui::state::sidebar::SidebarMode,
    trunks: Vec<String>,
    theme: crate::tui::theme::Theme,
  ) {
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let path = w.path.clone();
      let sections = crate::tui::ui::build_sidebar_payload(&w, mode, &trunks, &theme);
      let _ = tx.send(TaskMsg::Sidebar(generation, path, mode, sections));
    });
  }

  /// Keep agent-session detection off the render path (issue #408). Called
  /// once per event-loop tick, next to [`Self::maybe_refresh_sidebar`]: a
  /// cold snapshot (startup, or nulled by a refresh) or one older than the
  /// re-detection period spawns one worker; a tick that finds a run already
  /// in flight coalesces onto it — same no-timer debounce as the sidebar.
  pub fn maybe_refresh_agent_sessions(&mut self) {
    const REDETECT_PERIOD: std::time::Duration = std::time::Duration::from_secs(30);
    let fresh = self.agent_snapshot_at.is_some_and(|at| at.elapsed() < REDETECT_PERIOD);
    if fresh {
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::AgentSessions) else {
      return; // detection already in flight — coalesce
    };
    let rows: Vec<(String, PathBuf)> = self
      .worktrees
      .iter()
      .map(|w| (crate::agent_sessions::path_display_key(&w.path), w.path.clone()))
      .collect();
    // Pin reads are branch-config I/O — in workspace mode one repo open
    // per row. That happens in the WORKER, not here (Codex review round P:
    // the event loop must not touch the disk on the periodic path); the
    // main thread only assembles (path, branch, owning repo dir) triples,
    // resolved via `row_repo` so each row reads its own repo (round I).
    let pin_sources = self.agent_pin_sources();
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let pins_map = read_pins_from_sources(&pin_sources);
      let pins: Vec<(String, String)> = pins_map
        .iter()
        .flat_map(|(path, sids)| sids.iter().map(move |sid| (path.clone(), sid.clone())))
        .collect();
      // Summary-only: the matched-per-worktree scan, NOT the full
      // foreign-dir sweep — that one is linear in the whole artefact
      // history and runs only when the attach prompt opens (round Q).
      let map = match crate::agent_sessions::agents_home() {
        Some(home) => crate::agent_sessions::detect_all(&home, &rows, &pins, std::time::SystemTime::now()),
        None => Default::default(), // no home: detection degrades to empty (FR-009)
      };
      let _ = tx.send(TaskMsg::AgentSessions(generation, map, None, pins_map));
    });
  }

  /// Spawn the FULL detection — foreign-dir sweep included — to feed the
  /// attach prompt's candidate pool. Prompt-open only (round Q): the sweep
  /// costs a bounded read of every recent foreign artefact and must not
  /// ride the 30 s periodic tick. Drops a coalescing in-flight periodic
  /// run: this result supersedes it anyway.
  fn refresh_agent_pool(&mut self) {
    // A run in flight keeps walking the store even after `invalidate`
    // frees its slot — starting the full scan NOW would double the I/O.
    // Queue it instead; `apply_agent_snapshot` chains it on landing
    // (round R).
    if self.tasks.is_loading(TaskKind::AgentSessions) {
      self.agent_pool_wanted = true;
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::AgentSessions) else {
      return;
    };
    let rows: Vec<(String, PathBuf)> = self
      .worktrees
      .iter()
      .map(|w| (crate::agent_sessions::path_display_key(&w.path), w.path.clone()))
      .collect();
    let pin_sources = self.agent_pin_sources();
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let pins_map = read_pins_from_sources(&pin_sources);
      let pins: Vec<(String, String)> = pins_map
        .iter()
        .flat_map(|(path, sids)| sids.iter().map(move |sid| (path.clone(), sid.clone())))
        .collect();
      let (map, all) = match crate::agent_sessions::agents_home() {
        Some(home) => crate::agent_sessions::detect_with_sessions(&home, &rows, &pins, std::time::SystemTime::now()),
        None => Default::default(), // no home: detection degrades to empty (FR-009)
      };
      let _ = tx.send(TaskMsg::AgentSessions(generation, map, Some(all), pins_map));
    });
  }

  /// The (worktree path, branch, owning repo workdir) triples the detection
  /// worker reads pins from — assembled here without touching the disk. In
  /// workspace mode the owner comes from the `row_repo` mapping; in
  /// single-repo mode every row belongs to the active repo.
  pub fn agent_pin_sources(&self) -> Vec<(String, String, PathBuf)> {
    self
      .worktrees
      .iter()
      .enumerate()
      .filter_map(|(i, w)| {
        let branch = crate::github::pinnable_branch(w.branch.as_deref())?;
        let repo_dir = if let Some(ws) = &self.workspace {
          ws.repos.get(*ws.row_repo.get(i)?)?.workdir.clone()
        } else {
          self.workdir.clone()
        };
        Some((
          crate::agent_sessions::path_display_key(&w.path),
          branch.to_string(),
          repo_dir,
        ))
      })
      .collect()
  }

  /// Store a completed detection snapshot if its generation is still
  /// authoritative (issue #408). Returns `true` when applied; a late result
  /// superseded by [`TaskRunner::invalidate`] is dropped and the previous
  /// snapshot survives. Extracted from the drain so the state contract is
  /// pinned ratatui-free by `tests/tui_app_tests.rs`.
  pub fn apply_agent_snapshot(
    &mut self,
    generation: u64,
    map: std::collections::BTreeMap<String, crate::agent_sessions::WorktreeAgents>,
    all: Option<Vec<crate::agent_sessions::AgentSession>>,
    pins: std::collections::BTreeMap<String, Vec<String>>,
  ) -> bool {
    if !self.tasks.complete(TaskKind::AgentSessions, generation) {
      return false;
    }
    self.agent_snapshot = Some(map);
    self.agent_snapshot_at = Some(std::time::Instant::now());
    // `None` = summary-only run: the previous pool survives so an open
    // attach prompt keeps its candidates (round Q).
    let landed_pool = all.is_some();
    if let Some(all) = all {
      self.agent_all_sessions = all;
    }
    // A pool scan queued while this run was in flight chains now that the
    // slot is free (round R); a landing that already carried the pool
    // satisfies the request outright, and a prompt closed in the meantime
    // abandons it — nobody would consume the sweep (round T).
    if self.agent_pool_wanted {
      self.agent_pool_wanted = false;
      let prompt_open = self.view == View::DetailOverlay
        && self.detail_overlay.mode == crate::tui::state::detail_overlay::DetailMode::Input;
      if !landed_pool && prompt_open {
        self.refresh_agent_pool();
      }
    }
    // The worker read the pins from each row's owning repo (round P);
    // store them before the overlay rebuild below reads the map — UNLESS
    // a pin changed while this run was in flight: its map predates the
    // change, so the fresh event-path read stands and a re-detection is
    // chained by clearing the snapshot timestamp (round U).
    if self.agent_redetect_wanted {
      self.agent_redetect_wanted = false;
      self.agent_snapshot_at = None;
    } else {
      self.agent_pins = pins;
    }
    // A landing detection refreshes the open overlay in place (user
    // feedback: attach/detach used to leave stale rows until reopened).
    // Gated on the AGENTS consumer (Codex review #455): a stale target
    // left by an interrupted agents overlay must never rebuild the CI
    // checks rows into session rows under an unchanged CiChecks kind.
    if self.view == View::DetailOverlay
      && self.detail_overlay.kind == crate::tui::state::detail_overlay::DetailKind::Agents
    {
      if let Some((path, _)) = self.detail_overlay_target.clone() {
        if let Some(w) = self.worktrees.iter().find(|w| w.path == path).cloned() {
          let rows = self.build_agent_rows(&w);
          self.detail_overlay.set_rows(rows);
        }
      }
    }
    true
  }

  /// The agent sessions matched to `w`, if a snapshot has landed and holds
  /// any (issue #408). Pure lookup — the render path's only entry point.
  pub fn agents_for(&self, w: &crate::worktree::WorktreeInfo) -> Option<&crate::agent_sessions::WorktreeAgents> {
    self
      .agent_snapshot
      .as_ref()
      .and_then(|map| map.get(&crate::agent_sessions::path_display_key(&w.path)))
  }

  /// Any session in the landed snapshot at all? Drives the table's AGENT
  /// column visibility (Codex review round D): with no agent tooling the
  /// table must stay visually pre-#408, not carry an empty 8-cell column
  /// squeezing NAME/BRANCH/PATH on narrow terminals. Keyed to the whole
  /// snapshot — not the visible rows — so filtering/scrolling never makes
  /// the column flicker.
  pub fn any_agent_sessions(&self) -> bool {
    self
      .agent_snapshot
      .as_ref()
      .is_some_and(|map| map.values().any(|a| !a.sessions.is_empty()))
  }

  /// Off-thread `gwm sync` of the selected worktree for the `S` key (issue
  /// #258): fetch + rebase its branch onto upstream on a worker thread, so a
  /// slow network fetch / rebase does not freeze the event loop. Coalesces
  /// onto an in-flight sync (a second `S` while one runs is a no-op, so two
  /// rebases never race). The outcome is applied by
  /// [`Self::drain_task_results`], which reports it and refreshes the list so
  /// the new ahead/behind state shows. Default strategy is rebase (the repo
  /// convention); a `--merge` variant is deferred (see #258).
  pub fn request_sync(&mut self) {
    let Some((path, name)) = self.selected().map(|w| (w.path.clone(), w.name.clone())) else {
      self.status = "no worktree selected to sync".into();
      return;
    };
    if self.tasks.has_mutating_task_in_flight() && !self.tasks.is_loading(TaskKind::Sync) {
      self.status = self.busy_mutation_status("syncing");
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::Sync) else {
      // A sync is already in flight — coalesce onto it.
      return;
    };
    self.spinner.reset();
    self.status = TaskKind::Sync.loading_label().into();
    self.spawn_sync(generation, path, name);
  }

  /// Spawn one background `gwm sync` worker tagged with `generation` (issue
  /// #258). Mirrors [`Self::spawn_refresh`]: it moves only owned `Send` data
  /// (the worktree `path` + `name`) across the boundary and runs the existing
  /// [`crate::sync::sync`] logic, which discovers its own repo from `path` and
  /// shells out to `git` for fetch/rebase. A `send` failure (the `App`/receiver
  /// dropped) is ignored.
  fn spawn_sync(&self, generation: u64, path: PathBuf, name: String) {
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let result = crate::sync::sync(&path, crate::sync::SyncStrategy::Rebase).map_err(|e| e.to_string());
      let _ = tx.send(TaskMsg::Sync(generation, name, result));
    });
  }

  /// Apply every background task result that has arrived since the last
  /// call (issue #231; GitHub fetch results folded in by #255), draining
  /// the channel without blocking. Each result goes through
  /// [`TaskRunner::complete`], so a result whose per-key generation was
  /// bumped mid-flight is dropped (#138 guard, generalised) — this is what
  /// makes a stale GitHub worker lose to a fresh one in the retry race.
  ///
  /// A failed refresh surfaces on the status bar and leaves the list
  /// intact — what used to be a fatal `refresh()?` that tore down the
  /// event loop is now a graceful message. A GitHub result is stamped into
  /// the per-key cache via `complete_{issue,pr}` (pure writes now that the
  /// drop decision lives on the spine); once nothing GitHub-side is left
  /// loading, the aggregate outcome is re-reported on the status bar — the
  /// same end state `drain_github_results` produced pre-#255. Returns `true`
  /// if at least one result was applied, so the loop can force a redraw.
  pub fn drain_task_results(&mut self) -> bool {
    let mut applied = false;
    let mut github_applied = false;
    let mut refresh_applied = false;
    while let Ok(msg) = self.task_rx.try_recv() {
      match msg {
        TaskMsg::CreateWorktree(generation, result) => {
          if !self.tasks.complete(TaskKind::CreateWorktree, generation) {
            // Late result — a newer run (or an invalidate) superseded it.
            continue;
          }
          match result {
            Ok(result) => {
              self.create_failure = None;
              self.report = Some(result.report);
              self.view = View::Report;
              let refresh_result = self.refresh();
              self.status = match refresh_result {
                Ok(()) => format!("created {} @ {}", result.branch, result.created.display()),
                Err(e) => format!(
                  "created {} @ {}; refresh failed: {}",
                  result.branch,
                  result.created.display(),
                  e
                ),
              };
            }
            Err(e) => {
              self.create_failure = Some(e.clone());
              self.view = View::Create;
              self.status = format!("create failed: {}", e);
            }
          }
          applied = true;
          // Create owns the status line this tick.
          refresh_applied = true;
        }
        TaskMsg::RefreshWorktrees(generation, result) => {
          if !self.tasks.complete(TaskKind::RefreshWorktrees, generation) {
            // Late result — a newer run (or an invalidate) superseded it.
            continue;
          }
          match result {
            Ok(worktrees) => self.apply_refreshed_worktrees(worktrees),
            Err(e) => self.status = format!("refresh failed: {}", e),
          }
          applied = true;
          refresh_applied = true;
        }
        TaskMsg::RefreshWorkspace(generation, rows) => {
          if !self.tasks.complete(TaskKind::RefreshWorkspace, generation) {
            // Late result — a newer run (or a synchronous `refresh`) superseded it.
            continue;
          }
          self.apply_workspace_worktrees(rows);
          applied = true;
          refresh_applied = true;
        }
        TaskMsg::GithubIssue(generation, number, result) => {
          // Generation guard: a stale worker whose slot was bumped by an
          // intervening invalidate/re-request is dropped here, before it can
          // stamp the cache (the Codex-flagged race, fixed by the spine).
          if !self.tasks.complete(TaskKind::GithubIssue(number), generation) {
            continue;
          }
          if let Ok(status) = &result {
            self.persist_loaded_issue_title(status);
          }
          self.github.complete_issue(number, result);
          applied = true;
          github_applied = true;
        }
        TaskMsg::GithubPr(generation, number, result) => {
          if !self.tasks.complete(TaskKind::GithubPr(number), generation) {
            continue;
          }
          if let Ok(status) = &result {
            self.persist_loaded_pr_title(status);
            if self.refresh_ci_overlay_on_pr_landing(status) {
              // The overlay-close message owns the status line this tick.
              refresh_applied = true;
            }
          }
          self.github.complete_pr(number, result);
          applied = true;
          github_applied = true;
        }
        TaskMsg::Sync(generation, name, result) => {
          if !self.tasks.complete(TaskKind::Sync, generation) {
            // Late result — a newer sync (or an invalidate) superseded it.
            continue;
          }
          match result {
            Ok(report) => {
              // Re-list so the new ahead/behind state shows (this also bumps
              // the refresh generation — the #138 race guard). The worker
              // mutated refs in a subprocess, but a libgit2 read re-reads them
              // from disk, so the synchronous `self.refresh()` (`self.repo`)
              // sees the rebased state — verified end-to-end by the
              // ahead/behind assertion in `sync_tests::
              // tui_sync_action_relists_to_the_rebased_state_from_disk`.
              // `refresh` sets its own "refreshed — N" status, so overwrite it
              // with the sync outcome afterwards — the user pressed `S`, the
              // sync result is what they want to read.
              let _ = self.refresh();
              self.status = crate::cli::format_sync_report(&name, &report).trim_end().to_string();
            }
            Err(e) => self.status = format!("sync failed: {}", e),
          }
          applied = true;
          // The sync owns the status line this tick — keep the post-loop
          // GitHub report from overwriting it (same guard the refresh uses).
          refresh_applied = true;
        }
        TaskMsg::Bootstrap(generation, result) => {
          if !self.tasks.complete(TaskKind::Bootstrap, generation) {
            // Late result — a newer run (or an invalidate) superseded it, so
            // it must not flip the view to a stale report.
            continue;
          }
          match result {
            Ok(report) => {
              // Same outcome as the old synchronous path (issue #256): show
              // the report and surface whether any step failed.
              let any_failed = report.steps.iter().any(|s| s.status == StepStatus::Failed);
              self.report = Some(report);
              self.view = View::Report;
              self.status = if any_failed {
                "bootstrap had failures".into()
              } else {
                "bootstrap ok".into()
              };
            }
            Err(e) => self.status = format!("bootstrap error: {}", e),
          }
          applied = true;
          // The bootstrap owns the status line (and the view) this tick — keep
          // the post-loop GitHub report from overwriting it (same guard the
          // refresh / sync arms use).
          refresh_applied = true;
        }
        TaskMsg::DeleteWorktree(generation, name, label, result) => {
          if !self.tasks.complete(TaskKind::DeleteWorktree, generation) {
            // Late result — a newer run (or an invalidate) superseded it.
            continue;
          }
          match result {
            Ok(()) => {
              self.delete_failure = None;
              self.view = View::List;
              self.confirm.reset();
              let refresh_result = self.refresh();
              self.status = match refresh_result {
                Ok(()) => format!("removed {} ({})", name, label),
                Err(e) => format!("removed {} ({}); refresh failed: {}", name, label, e),
              };
            }
            Err(e) => {
              self.delete_failure = Some(e.clone());
              self.view = View::Confirm;
              self.status = format!("delete failed: {}", e);
            }
          }
          applied = true;
          // Delete owns the status line this tick.
          refresh_applied = true;
        }
        TaskMsg::Pull(generation, name, result) => {
          if !self.tasks.complete(TaskKind::Pull, generation) {
            continue;
          }
          // Refresh on both arms: a failed pull can still mutate the tree (a
          // merge/rebase conflict leaves it dirty / mid-rebase), so the table
          // must not keep showing the pre-pull clean state (Codex review #292).
          let _ = self.refresh();
          match result {
            Ok(msg) => self.status = format!("pulled {}: {}", name, msg),
            Err(e) => self.status = format!("pull failed: {}", e),
          }
          applied = true;
          refresh_applied = true;
        }
        TaskMsg::Push(generation, name, result) => {
          if !self.tasks.complete(TaskKind::Push, generation) {
            continue;
          }
          match result {
            Ok(msg) => {
              // Pushing updates the remote-tracking ref + ahead/behind, so
              // refresh the table before overwriting the status, mirroring
              // the pull/sync path (Codex review on PR #292).
              let _ = self.refresh();
              self.status = format!("pushed {}: {}", name, msg);
            }
            Err(e) => self.status = format!("push failed: {}", e),
          }
          applied = true;
          refresh_applied = true;
        }
        TaskMsg::EditWorktree(generation, result) => {
          if !self.tasks.complete(TaskKind::EditWorktree, generation) {
            continue;
          }
          match result {
            Ok(res) => {
              let _ = self.refresh();
              self.status = if res.remote_renamed {
                format!("renamed to {} (local + remote)", res.new_branch)
              } else {
                format!("renamed to {} (local only)", res.new_branch)
              };
              // Re-select the renamed worktree by its new path so the cursor
              // stays on the row the user just edited (mapped through the
              // filter — Codex review on PR #292).
              self.reselect_by_path(&res.new_path);
              self.edit_original_branch = None;
              self.edit_original_path = None;
              self.edit_failure = None;
              self.create_form.reset();
              self.view = View::List;
            }
            // Keep the modal open so the user can fix the form and retry, and
            // replace the "renaming worktree…" loading status so the bar no
            // longer reads as in-progress (Codex review on PR #292, P3).
            Err(e) => {
              self.status = format!("rename failed: {}", e);
              self.edit_failure = Some(e);
            }
          }
          applied = true;
          refresh_applied = true;
        }
        TaskMsg::Sidebar(generation, path, mode, sections) => {
          // Late result — the selection moved and `refresh` bumped the slot's
          // generation (a mutation invalidated a pre-mutation rebuild), so this
          // payload is stale. Drop it; the next tick requests the current one.
          if !self.tasks.complete(TaskKind::Sidebar, generation) {
            continue;
          }
          // Store keyed by the worktree + mode it was built for. If the
          // selection has since moved this key won't match the current one, so
          // the render shows the placeholder and `maybe_refresh_sidebar` fetches
          // the settled selection next tick — no stale worktree's git preview
          // is ever shown under the live header.
          self.sidebar.cache = Some(((path, mode), sections));
          applied = true;
        }
        TaskMsg::AgentSessions(generation, map, all, pins) => {
          // Late-drop + store live in `apply_agent_snapshot` so the state
          // contract is pinned ratatui-free (issue #408). Deliberately does
          // NOT set `applied`: agent detection reads no git state, so there
          // is nothing for the post-drain refresh bookkeeping to do.
          self.apply_agent_snapshot(generation, map, all, pins);
        }
      }
    }
    // Once nothing GitHub-side is left loading, swap the "fetching…"
    // placeholder for the real outcome (refreshed / partial failure /
    // failure) — only when a GitHub result actually applied, so a dropped
    // stale result never overwrites the current status (issue #217 review P2).
    //
    // Skip it when a worktree refresh also landed this tick: pre-#255 the
    // event loop drained the GitHub channel *before* the task channel, so a
    // simultaneous completion left `apply_refreshed_worktrees`' "refreshed —
    // N" message standing last. The `!refresh_applied` guard preserves that
    // ordering now that both drain in one pass.
    if github_applied && !refresh_applied && !self.is_github_loading() {
      self.report_github_refresh_status();
    }
    applied
  }

  /// `true` while any background task is in flight (issue #231) — drives
  /// the statusbar spinner alongside [`Self::is_github_loading`].
  pub fn is_task_loading(&self) -> bool {
    self.tasks.is_any_loading()
  }

  /// `true` while the create-worktree worker is in flight (issue #276).
  pub fn is_create_worktree_loading(&self) -> bool {
    self.tasks.is_loading(TaskKind::CreateWorktree)
  }

  /// `true` while the delete-worktree worker is in flight (issue #257).
  pub fn is_delete_worktree_loading(&self) -> bool {
    self.tasks.is_loading(TaskKind::DeleteWorktree)
  }

  /// `true` when a requested quit can safely leave the event loop now.
  /// Mutating spine workers keep running until their result is drained so
  /// `sync` / `bootstrap` / delete-worktree are not abandoned mid-operation.
  pub fn can_quit_now(&self) -> bool {
    !self.should_quit || !self.tasks.has_mutating_task_in_flight()
  }

  /// Surface why a requested quit is being held. The event loop keeps
  /// ticking/draining while this status is visible.
  pub fn defer_quit_for_mutating_task(&mut self) {
    if let Some(label) = self.tasks.mutating_loading_label() {
      self.status = format!("finishing {} before quit…", label.trim_end_matches('…'));
    } else {
      self.status = "finishing task before quit…".into();
    }
  }

  /// A clone of the task channel sender background workers report over
  /// (issue #231; GitHub fetch workers too since #255). Exposed so the
  /// async-apply path ([`Self::drain_task_results`]) can be driven
  /// deterministically in tests — inject a [`TaskMsg`] exactly as a worker
  /// would, then drain — without spawning an OS thread or a real `gh`.
  pub fn task_result_sender(&self) -> mpsc::Sender<TaskMsg> {
    self.task_tx.clone()
  }

  /// Drop the cached sidebar content. Call on any change that may have altered
  /// what the sidebar shows: worktree list refresh, filter narrowing, etc.
  /// Pure delegate over [`SidebarState::invalidate`]; navigation-driven
  /// invalidation goes through [`Self::on_navigation`] which also resets
  /// the scroll offset.
  pub fn invalidate_sidebar_cache(&mut self) {
    self.sidebar.invalidate();
  }

  /// Selection-change reaction: drop the sidebar's scroll back to the
  /// top, invalidate its cached preview, and resolve the link cache
  /// against the freshly selected worktree. Collapses the verbatim
  /// `sidebar.scroll = 0; invalidate_sidebar_cache(); refresh_link();`
  /// triple that was repeated across `next`, `prev`, `first`, `last`
  /// pre-extraction (issue #127, part of #102). The first two pieces
  /// live on [`SidebarState::on_navigation`]; the link refresh is
  /// orchestrator-shaped (it touches `self.link` / `self.link_slug` /
  /// `self.issue_state` / `self.pr_state` via [`Self::refresh_link`])
  /// so it stays here. Every navigation entry point now goes through
  /// this single call so the triple cannot drift back into duplicated
  /// literals.
  pub fn on_navigation(&mut self) {
    self.sidebar.on_navigation();
    self.refresh_link();
  }

  pub fn next(&mut self) {
    // Route navigation to the sidebar when it's focused; otherwise move the list.
    if self.sidebar.open && self.sidebar.focused {
      self.sidebar_scroll_down();
      return;
    }
    let len = self.filtered_indices().len();
    if len == 0 {
      return;
    }
    let i = match self.list_state.selected() {
      Some(i) => (i + 1) % len,
      None => 0,
    };
    self.list_state.select(Some(i));
    self.on_navigation();
  }

  pub fn prev(&mut self) {
    if self.sidebar.open && self.sidebar.focused {
      self.sidebar_scroll_up();
      return;
    }
    let len = self.filtered_indices().len();
    if len == 0 {
      return;
    }
    let i = match self.list_state.selected() {
      Some(0) | None => len - 1,
      Some(i) => i - 1,
    };
    self.list_state.select(Some(i));
    self.on_navigation();
  }

  // ---- Vim-style motions / list jumps -------------------------------------

  pub fn first(&mut self) {
    let len = self.filtered_indices().len();
    if len > 0 {
      self.list_state.select(Some(0));
      self.on_navigation();
    }
  }

  pub fn last(&mut self) {
    let len = self.filtered_indices().len();
    if len > 0 {
      self.list_state.select(Some(len - 1));
      self.on_navigation();
    }
  }

  /// Drive the two-keystroke `gg` motion. First press arms it, second jumps to top.
  ///
  /// **Compatibility shim** — kept so the existing tests in
  /// `tests/tui_app_tests.rs::handle_g_motion_tracks_pending_then_jumps_to_first`
  /// and the not-yet-migrated event-loop branch keep working
  /// verbatim. The implementation routes through
  /// [`Self::dispatch_key`] so the legacy and generic paths cannot
  /// drift on the chord semantics.
  pub fn handle_g(&mut self) {
    let ev = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::empty());
    if let Some(Action::Top) = self.dispatch_key(ev) {
      self.first();
    }
  }

  /// Drop any in-flight chord prefix. Called by the legacy event-loop
  /// branch on any non-`g` keystroke (pre-#87 contract). New call
  /// sites that route through [`Self::dispatch_key`] don't need it —
  /// `dispatch_key` already clears the buffer on `NoMatch`.
  pub fn cancel_pending_motion(&mut self) {
    self.pending_chord.clear();
    self.sync_legacy_pending_flag();
  }

  /// True iff no chord prefix is currently armed. Surface for tests
  /// and for the help / status-bar code that may want to show a
  /// "waiting for next key" hint once chord support is wired up.
  pub fn pending_chord_is_empty(&self) -> bool {
    self.pending_chord.is_empty()
  }

  /// Drive a raw `KeyEvent` through the keymap.
  ///
  /// Returns `Some(action)` when the buffer (current pending chord +
  /// this stroke) matches a binding — caller fires the action and the
  /// buffer is left cleared. Returns `None` when the buffer is now a
  /// strict prefix of a longer binding (caller waits for the next
  /// keystroke) **or** when the stroke matches nothing at all
  /// (caller drops it).
  ///
  /// Vim-style fallback: if appending the stroke to a non-empty
  /// buffer produces a `NoMatch`, the buffer is cleared and the
  /// stroke is re-tried on its own. This mirrors the historical
  /// `g j` behaviour where the stray `g` is forgotten and `j`
  /// still navigates down.
  pub fn dispatch_key(&mut self, key: KeyEvent) -> Option<Action> {
    let stroke = KeyStroke::from_event(&key);
    let mut tentative = self.pending_chord.clone();
    tentative.push(stroke.clone());

    let outcome = match self.keymap.lookup(&tentative) {
      ChordResolution::Matched(action) => {
        self.pending_chord.clear();
        Some(action)
      }
      ChordResolution::PendingPrefix => {
        self.pending_chord = tentative;
        None
      }
      ChordResolution::NoMatch if self.pending_chord.is_empty() => {
        // Single stroke, no binding. Nothing to retry.
        None
      }
      ChordResolution::NoMatch => {
        // Mismatched continuation. Drop the in-flight prefix and
        // retry the new stroke on its own so the user's keypress
        // is not silently swallowed when it has a single-key
        // binding (the `g j` case).
        self.pending_chord.clear();
        let single = vec![stroke];
        match self.keymap.lookup(&single) {
          ChordResolution::Matched(action) => Some(action),
          ChordResolution::PendingPrefix => {
            self.pending_chord = single;
            None
          }
          ChordResolution::NoMatch => None,
        }
      }
    };

    self.sync_legacy_pending_flag();
    outcome
  }

  pub fn key_matches_action(&self, key: KeyEvent, action: Action) -> bool {
    matches!(
      self.keymap.lookup(&[KeyStroke::from_event(&key)]),
      ChordResolution::Matched(found) if found == action
    )
  }

  /// Resolve a keystroke against the contextual modal keymap (issue #219).
  /// Returns the [`ModalAction`] bound to `key` in `ctx`, or `None` when
  /// nothing in that context binds it — the modal routing then applies its
  /// text-input / default fallback (digits, free-text, sub-state guards).
  pub fn resolve_modal(&self, ctx: KeyContext, key: KeyEvent) -> Option<ModalAction> {
    self.modal_keymap.resolve(ctx, &KeyStroke::from_event(&key))
  }

  /// Mirror the new `pending_chord` buffer into the legacy
  /// `pending_g` boolean so pre-#87 tests that read it as a field
  /// stay green. Removed when those tests migrate to
  /// [`Self::pending_chord_is_empty`].
  fn sync_legacy_pending_flag(&mut self) {
    let g = KeyStroke::new(KeyCode::Char('g'), KeyModifiers::empty());
    self.pending_g = self.pending_chord.len() == 1 && self.pending_chord[0] == g;
  }

  // ---- Command palette (issue #32) ----------------------------------------

  /// Open the command palette overlay. Transitions the active view
  /// to `View::CommandPalette` and arms the pure state machine on
  /// `self.palette` with a fresh empty buffer. Status bar shows a
  /// short hint so the user knows what to type.
  pub fn open_command_palette(&mut self) {
    self.palette.open();
    self.view = View::CommandPalette;
    self.status = "command palette — type, Enter to run, Esc to cancel".into();
  }

  /// Close the palette without firing anything. Called on `Esc` from
  /// inside the overlay. Returns the view to `View::List` and drops
  /// the buffer.
  pub fn close_command_palette(&mut self) {
    self.palette.close();
    self.view = View::List;
    self.status = "palette cancelled".into();
  }

  /// Append a character to the palette input buffer. The pure state
  /// machine re-runs its fuzzy match and resets the highlight to 0.
  pub fn palette_push_char(&mut self, c: char) {
    self.palette.push_char(c);
  }

  /// Remove the trailing character from the palette input buffer.
  pub fn palette_pop_char(&mut self) {
    self.palette.pop_char();
  }

  /// Move the palette highlight one row down (wraps at the end).
  pub fn palette_cycle_down(&mut self) {
    self.palette.cycle_highlight_down();
  }

  /// Move the palette highlight one row up (wraps at the start).
  pub fn palette_cycle_up(&mut self) {
    self.palette.cycle_highlight_up();
  }

  /// Accept the highlighted entry. Returns the resolved `Action` and
  /// drops the palette overlay; the caller (event loop) routes the
  /// action through the same dispatcher branch as a keystroke so
  /// palette + key fire identical side effects.
  ///
  /// When the input buffer matches nothing the palette stays open
  /// and `None` is returned — the user can backspace and retry
  /// without losing context.
  pub fn accept_command_palette(&mut self) -> Option<Action> {
    let action = self.palette.accept()?;
    self.view = View::List;
    self.status = format!("palette: {}", action.slug());
    Some(action)
  }

  // ---- Sidebar ------------------------------------------------------------

  pub fn toggle_sidebar(&mut self) {
    self.sidebar.toggle_open();
    self.status = if self.sidebar.open {
      "sidebar shown".into()
    } else {
      "sidebar hidden".into()
    };
  }

  /// Cycle the sidebar preview mode between Commits and Stashes
  /// (issue #34). Drives the pure-state cycle on `SidebarState`
  /// plus the status-bar copy: orchestrator-shaped because the
  /// status bar is owned by `App`, not by the sub-struct.
  pub fn cycle_sidebar_mode(&mut self) {
    self.sidebar.cycle_mode();
    self.status = format!("sidebar: {}", self.sidebar.mode.label());
  }

  /// Cycle the sidebar orientation `auto → side-by-side → stacked`
  /// (issue #188). Orchestrator-shaped for the status-bar copy, like
  /// [`Self::cycle_sidebar_mode`].
  pub fn cycle_sidebar_layout(&mut self) {
    self.sidebar.cycle_orientation();
    self.status = format!("sidebar layout: {}", self.sidebar.orientation.label());
  }

  /// Flip the side-by-side sidebar position left ↔ right (issue #188).
  pub fn toggle_sidebar_position(&mut self) {
    self.sidebar.toggle_position();
    self.status = format!("sidebar position: {}", self.sidebar.position.label());
  }

  pub fn toggle_focus(&mut self) {
    self.sidebar.toggle_focus();
  }

  /// Direct-focus the worktree table (issue #217, `1`). Orchestrator-shaped
  /// for the status-bar copy, like the sidebar toggles.
  pub fn focus_worktrees(&mut self) {
    self.sidebar.focus_table();
    self.status = "focus: worktrees".into();
  }

  /// Direct-focus the status (sidebar) pane (issue #217, `2`). Opens the
  /// sidebar if needed and moves focus onto it.
  pub fn focus_status(&mut self) {
    self.sidebar.focus_panel();
    self.status = "focus: status".into();
  }

  /// The live UI context driving the statusbar chip + help subtitle (issue
  /// #217). An open modal / overlay wins over the pane focus (issue #217
  /// review P2): when the create form is up, the statusbar must advertise
  /// the form's keys, not the worktrees pane's `n new` — pressing `n` there
  /// types text. Only `View::List` falls through to the pane context
  /// (`Picker` in `gwm switch`, `Status` when the sidebar holds focus, else
  /// `Worktrees`).
  pub fn hint_context(&self) -> super::ui::HintContext {
    use super::ui::HintContext;
    match self.view {
      View::Create => HintContext::Create,
      View::Confirm => HintContext::Confirm,
      View::OpenMenu => HintContext::OpenMenu,
      // #219: the two link-prompt stages advertise different keys — the
      // choose-target picker vs the number-input submit/cancel — so the
      // statusbar tracks whichever stage is live.
      View::LinkPrompt => {
        if self.link_prompt_stage() == crate::tui::state::link_prompt::LinkPromptStage::InputNumber {
          HintContext::LinkInputNumber
        } else {
          HintContext::LinkPrompt
        }
      }
      View::CommandPalette => HintContext::CommandPalette,
      View::Report => HintContext::Report,
      View::Help => HintContext::Help,
      // The Command Logs overlay (issue #226) is a ~90% fullscreen modal;
      // the statusbar behind it shows the underlying pane's context, as the
      // List view does.
      View::CommandLogs => self.pane_hint_context(),
      // The Configuration panel (issue #232) is likewise a ~90% fullscreen
      // modal; the statusbar behind it keeps the underlying pane context.
      View::Config => self.pane_hint_context(),
      View::Pty => super::ui::HintContext::Pty,
      View::ExecPicker => HintContext::ExecPicker,
      View::CleanReport => HintContext::Clean,
      View::Edit => HintContext::Rename,
      // Issue #408: the detail overlay advertises its close/scroll keys.
      View::DetailOverlay => {
        if self.detail_overlay.kind == crate::tui::state::detail_overlay::DetailKind::CiChecks {
          HintContext::CiChecks
        } else {
          HintContext::Detail
        }
      }
      View::List => self.pane_hint_context(),
    }
  }

  /// The underlying list-view pane context (issue #217), ignoring any open
  /// overlay. Drives the help overlay's subtitle + picker-section gating:
  /// `?` documents the keys for the pane you were on, so it must NOT collapse
  /// to the `Help` context that [`Self::hint_context`] returns while the
  /// overlay is up.
  pub fn pane_hint_context(&self) -> super::ui::HintContext {
    use super::ui::HintContext;
    if self.picker_mode {
      HintContext::Picker
    } else if self.sidebar.open && self.sidebar.focused {
      HintContext::Status
    } else {
      HintContext::Worktrees
    }
  }

  /// `true` while a GitHub issue / PR fetch for the current link is inflight
  /// (issue #217) — drives the statusbar loading spinner.
  pub fn is_github_loading(&self) -> bool {
    matches!(self.issue_fetch_state(), GitHubFetchState::Loading)
      || matches!(self.pr_fetch_state(), GitHubFetchState::Loading)
  }

  pub fn sidebar_scroll_down(&mut self) {
    self.sidebar.scroll_down();
  }

  pub fn sidebar_scroll_up(&mut self) {
    self.sidebar.scroll_up();
  }

  /// Scroll the Working Tree pane down (issue #437, `J`). Gated on the
  /// status pane holding the navigation focus — the same condition that
  /// routes `j` / `k` to the sidebar in [`Self::next`] / [`Self::prev`] —
  /// so the keys stay inert (and reusable) in the worktrees context.
  pub fn wt_scroll_down(&mut self) {
    if self.sidebar.open && self.sidebar.focused {
      self.sidebar.wt_scroll_down();
    }
  }

  /// Scroll the Working Tree pane up (issue #437, `K`). Same focus gate
  /// as [`Self::wt_scroll_down`].
  pub fn wt_scroll_up(&mut self) {
    if self.sidebar.open && self.sidebar.focused {
      self.sidebar.wt_scroll_up();
    }
  }

  /// Open the Keybindings (help) overlay from the top (#217). Resetting
  /// the scroll offset here keeps re-opens predictable.
  pub fn enter_help(&mut self) {
    self.view = View::Help;
    self.help_scroll = 0;
    self.help_x_scroll = 0;
  }

  /// Open the Command Logs overlay (issue #226). Snapshots the global
  /// command log into owned state and resets the scroll cursor so a
  /// previously-scrolled session starts fresh at the top. The renderer
  /// republishes `max_scroll` against the live viewport.
  pub fn enter_command_logs(&mut self) {
    self.command_logs.sync();
    self.command_logs.reset();
    self.view = View::CommandLogs;
  }

  /// Open the Configuration panel (issue #232). Resolves the effective
  /// config — the user-level global deep-merged under the repo `.gwm.toml`,
  /// with per-row source attribution — into owned state, then resets the
  /// scroll cursor so a re-open starts fresh at the top. The reads are
  /// cheap local TOML parses; on failure the panel still opens (empty)
  /// with the error on the statusbar rather than refusing to open.
  pub fn enter_config_panel(&mut self) {
    match crate::config::resolved_rows(&self.workdir, self.global_path.as_deref()) {
      Ok(rows) => self.config_panel.rows = rows,
      Err(e) => {
        self.config_panel.rows = Vec::new();
        self.status = format!("error: {}", e);
      }
    }
    self.refresh_key_rows();
    self.config_panel.reset();
    self.view = View::Config;
  }

  /// Rebuild the Keys-tab rows (issue #294) from the live keymaps, attributing
  /// each binding's source via the resolved-row snapshot (the same layer
  /// attribution the `All` tab shows). Called on panel open and after a
  /// successful rebind so the displayed key(s) + badge track the edit.
  fn refresh_key_rows(&mut self) {
    let rows = self.config_panel.rows.clone();
    let key_rows = super::state::config_panel::build_key_rows(&self.keymap, &self.modal_keymap, |key| {
      rows
        .iter()
        .find(|r| r.key == key)
        .map(|r| r.source)
        .unwrap_or(crate::config::ConfigSource::Default)
    });
    self.config_panel.key_rows = key_rows;
  }

  /// Feed a raw key event into the in-progress Keys-tab capture (issue #294),
  /// normalising it to a [`KeyStroke`] first. No-op when no capture is armed.
  pub fn push_key_capture(&mut self, key: KeyEvent) {
    self.config_panel.capture_push(KeyStroke::from_event(&key));
  }

  /// Drive a key through an armed Keys-tab capture (issue #294). The event loop
  /// owns no logic — it just routes here when a capture is armed, mirroring
  /// `handle_create_key` / `handle_link_prompt_key`. Controls (resolved through
  /// the `config.edit` context so a rebind shows through):
  ///
  /// - `cancel` (def Esc) aborts the capture;
  /// - `submit` (def Enter) commits a **multi-stroke global chord**;
  /// - `Backspace` drops the last stroke of a global chord;
  /// - any other key is captured — a **single-stroke modal** verb auto-commits
  ///   on the first one, a global chord accumulates until `submit`.
  ///
  /// `Esc` / `Enter` / `Backspace` stay reserved controls in **both** modes and
  /// are never themselves captured (a modal verb can't be bound to them via the
  /// UI — hand-edit `.gwm.toml`), matching the documented capture controls and
  /// the hard-coded escape-hatch policy.
  pub fn handle_capture_key(&mut self, key: KeyEvent) {
    let single = self
      .config_panel
      .capture
      .as_ref()
      .map(|c| c.single_only)
      .unwrap_or(false);
    // Reserved capture controls. The *physical* Esc / Enter / Backspace are
    // always controls (never captured) regardless of any `config.edit` rebind,
    // so a custom `submit = ["Ctrl+s"]` can't make Enter assignable (Codex #297
    // review). The resolved `config.edit` verbs are honoured *in addition*, so a
    // custom key also cancels / commits.
    let resolved = self.resolve_modal(KeyContext::ConfigEdit, key);
    let is_cancel = key.code == KeyCode::Esc || resolved == Some(ModalAction::ConfigEditCancel);
    let is_submit = key.code == KeyCode::Enter || resolved == Some(ModalAction::ConfigEditSubmit);
    if is_cancel {
      self.config_panel.cancel_capture();
    } else if is_submit {
      // Enter commits an accumulated global chord; a reserved control (ignored)
      // for a single-stroke modal capture.
      if !single {
        self.commit_key_capture();
      }
    } else if key.code == KeyCode::Backspace {
      // Backspace edits a global chord; reserved (ignored) for a modal capture.
      if !single {
        self.config_panel.capture_pop();
      }
    } else {
      self.push_key_capture(key);
      if single {
        self.commit_key_capture();
      }
    }
  }

  /// Commit the in-progress Keys-tab capture (issue #294): write the captured
  /// chord as a TOML array to the selected target's `[tui.keys]` /
  /// `[tui.keys.modal.<context>]` key in the active layer, then reload the
  /// config + both keymaps so the rebind is live immediately. An empty capture
  /// writes `[]` (unbind). Validation (conflict / prefix-collision) happens in
  /// the writer's validate-before-write gate; on failure the file and the live
  /// keymaps are left untouched and the error is surfaced on the statusbar.
  pub fn commit_key_capture(&mut self) {
    let Some(cap) = self.config_panel.take_capture() else {
      return;
    };
    let target = match self.config_panel.key_rows.get(cap.row) {
      Some(row) => row.target,
      None => return,
    };
    let config_key = target.config_key();
    let items = cap.as_config_items();

    // A Project-layer write targets `self.workdir/.gwm.toml`. In workspace mode
    // with a stale selection that path is the *previously* active repo, so
    // refuse rather than rebind keys in the wrong repo (#304).
    if self.workspace_active_stale && self.config_panel.layer == SettingsLayer::Project {
      self.status = "workspace: selected repo is unavailable — can't edit its project keymap".into();
      return;
    }
    let path = match self.config_panel.layer {
      SettingsLayer::Project => self.workdir.join(crate::config::CONFIG_FILE),
      SettingsLayer::Global => match self.global_path.clone() {
        Some(p) => p,
        None => {
          self.status = "keys: no global config path (set $XDG_CONFIG_HOME or $HOME)".into();
          return;
        }
      },
    };

    // Snapshot the target file first: `set_array_at` only validates the file
    // it writes, not the layered merge, so a rebind that is valid in this file
    // alone but collides with the *other* layer once merged (e.g. a prefix
    // collision the global layer reveals) would slip past and brick the
    // config for the next launch. Keep the prior bytes so we can roll back
    // (Codex #297 review P2).
    let prior = std::fs::read(&path).ok();

    if let Err(e) = crate::config_cli::set_array_at(&path, &config_key, &items) {
      // `write_and_validate` writes the edit *before* erroring when the file
      // was already invalid on its own (the recovery path for #281 — here the
      // target value can be shadowed by another layer so the app still
      // loaded). Roll back so a rebind reported as failed never persists or
      // takes effect on the next launch (Codex #297 review P2).
      Self::restore_file(&path, prior);
      self.status = format!("keys: {}", e);
      return;
    }

    // Strip any pre-#290 alias of this action from the same file: a legacy
    // config that still carries e.g. `tui.keys.open_menu` would, on reload,
    // re-apply the alias after the canonical `browse_links` in the sorted
    // override walk and silently shadow the new binding (Codex #297 review).
    // Best-effort: the canonical key is already written, so a cleanup error
    // is surfaced but does not abort the rebind.
    for alias_key in target.compat_alias_keys() {
      if let Err(e) = crate::config_cli::unset_at(&path, &alias_key) {
        self.status = format!("keys: {}", e);
      }
    }

    // Reload the merged config and rebuild both keymaps so the new binding
    // fires without a restart.
    match Config::load_layered(&self.workdir, self.global_path.as_deref()) {
      Ok(cfg) => self.set_active_config(cfg),
      Err(e) => {
        // The single-file write validated but the layered merge is invalid —
        // roll the file back to its prior state so the config is never left
        // broken on disk, and keep the previous live keymaps.
        Self::restore_file(&path, prior);
        self.status = format!("keys: rebind rejected — would break the merged config: {}", e);
        return;
      }
    }
    match self.config.tui.keys.resolved_keymap() {
      Ok(km) => self.keymap = km,
      Err(e) => {
        self.status = format!("keys: {}", e);
        return;
      }
    }
    match self.config.tui.keys.resolved_modal_keymap() {
      Ok(mk) => self.modal_keymap = mk,
      Err(e) => {
        self.status = format!("keys: {}", e);
        return;
      }
    }
    if let Ok(rows) = crate::config::resolved_rows(&self.workdir, self.global_path.as_deref()) {
      self.config_panel.rows = rows;
    }
    self.refresh_key_rows();

    let desc = if items.is_empty() {
      "unbound".to_string()
    } else {
      items.join(" ")
    };
    let mut status = format!("set {} = {} ({})", config_key, desc, self.config_panel.layer.label());
    // Verify the capture actually took effect in the *merged* keymap: a
    // higher-precedence layer, or a pre-#290 alias still declared in another
    // layer (which we deliberately don't edit), can shadow the write so the new
    // key never fires — or, for an unbind, keeps the action bound — even though
    // it persisted. Warn instead of reporting a clean success (Codex #297
    // review).
    if !self.capture_took_effect(target, &cap.pending) {
      status.push_str(" — shadowed (a higher layer or legacy alias still binds it)");
    }
    self.status = status;
  }

  /// Restore a config file to a snapshot taken before a rebind write: rewrite
  /// the prior bytes, or remove the file if it did not exist before. Used to
  /// roll back a failed / merge-invalid Keys-tab write (issue #294).
  fn restore_file(path: &std::path::Path, prior: Option<Vec<u8>>) {
    match prior {
      Some(bytes) => {
        let _ = std::fs::write(path, bytes);
      }
      None => {
        let _ = std::fs::remove_file(path);
      }
    }
  }

  /// Whether the just-committed capture is the *effective* state in the live
  /// (merged) keymap, i.e. not shadowed by another layer / a lingering legacy
  /// alias. For a rebind (`strokes` non-empty) the captured chord must resolve
  /// to the target's action; for an unbind (`strokes` empty) the action must
  /// have no remaining binding. Issue #294 (Codex #297 review).
  fn capture_took_effect(&self, target: KeyTarget, strokes: &[KeyStroke]) -> bool {
    match target {
      KeyTarget::Global(action) => {
        if strokes.is_empty() {
          self.keymap.keys_display(action).is_empty()
        } else {
          matches!(self.keymap.lookup(strokes), ChordResolution::Matched(a) if a == action)
        }
      }
      KeyTarget::Modal(action) => {
        if strokes.is_empty() {
          self.modal_keymap.keys_display(action).is_empty()
        } else {
          strokes
            .first()
            .map(|s| self.modal_keymap.resolve(action.context(), s) == Some(action))
            .unwrap_or(false)
        }
      }
    }
  }

  // ── PTY overlay (issue #35) ────────────────────────────────────────────

  /// Open the PTY overlay: store `pty` and switch to [`View::Pty`].
  pub fn open_pty_overlay(&mut self, pty: super::state::pty_overlay::PtyOverlay) {
    self.pty_overlay = Some(pty);
    self.view = View::Pty;
  }

  /// Close the PTY overlay: kill the child process, drop the state, and
  /// return to [`View::List`]. Safe to call when no overlay is open.
  pub fn close_pty_overlay(&mut self) {
    if let Some(ref mut pty) = self.pty_overlay {
      pty.kill();
    }
    self.pty_overlay = None;
    if self.view == View::Pty {
      self.view = View::List;
    }
  }

  // ── Exec picker overlay (issue #325) ───────────────────────────────────

  /// `true` while a destructive overlay — the exec picker or the clean
  /// report — is open (issue #325). The run loop suspends `maybe_auto_refresh`
  /// and `sync_active_repo` while one is up, so the worktree list (and thus
  /// the live selection / active repo) cannot reshuffle under an armed reclaim
  /// or a pending exec run. This closes the drift class at its source (Codex
  /// #333 review); the per-overlay open-time snapshots stay as defence in
  /// depth against an already-in-flight refresh landing its result.
  pub fn destructive_overlay_open(&self) -> bool {
    matches!(self.view, View::ExecPicker | View::CleanReport)
  }

  /// Open the exec profile picker (issue #325). Populates it from
  /// `[exec.profiles.*]` and switches to [`View::ExecPicker`]. Refuses
  /// (status-bar message, no transition) when nothing is selected or no
  /// exec profiles are configured — there is nothing to pick.
  pub fn enter_exec_picker(&mut self) {
    let Some(cwd) = self.selected().map(|wt| wt.path.clone()) else {
      self.status = "nothing selected".into();
      return;
    };
    let names: Vec<String> = self.config.exec.profiles.keys().cloned().collect();
    if names.is_empty() {
      self.status = "no [exec.profiles] configured — add one to .gwm.toml".into();
      return;
    }
    // Capture the target worktree path AND the active repo's `[exec]` config
    // now: an auto-refresh can drift the live selection (and, in workspace
    // mode, the active repo) while the picker is open, so `Enter` must run in
    // *this* worktree against *this* config — not whatever is live later
    // (Codex #333 review).
    self.exec_picker_cfg = self.config.exec.clone();
    self.exec_picker.open(names, cwd);
    self.view = View::ExecPicker;
  }

  /// Handle a key inside the exec picker overlay (issue #325). The
  /// testable handler owns the highlight movement; the run loop owns the
  /// two side effects (resolve + spawn, or close). Keys resolve through
  /// [`KeyContext::ExecPicker`] so they honour `[tui.keys.modal.exec]`.
  pub fn handle_exec_picker_key(&mut self, key: KeyEvent) -> ExecPickerKey {
    match self.resolve_modal(KeyContext::ExecPicker, key) {
      Some(ModalAction::ExecPickerCancel) => ExecPickerKey::Cancel,
      Some(ModalAction::ExecPickerAccept) => ExecPickerKey::Submit,
      Some(ModalAction::ExecPickerNext) => {
        self.exec_picker.next();
        ExecPickerKey::Handled
      }
      Some(ModalAction::ExecPickerPrev) => {
        self.exec_picker.prev();
        ExecPickerKey::Handled
      }
      _ => ExecPickerKey::Handled,
    }
  }

  /// Resolve the highlighted exec profile to an `(argv, cwd)` pair for the
  /// run loop to spawn in a PTY overlay (issue #325). `None` (with a
  /// status-bar message) when nothing is selected or the profile fails to
  /// resolve — e.g. an empty `command` array. The argv is the frozen
  /// `[exec.profiles.<name>].command` verbatim (no shell), matching the
  /// 1.0 exec contract; the run loop spawns `argv[0]` directly.
  pub fn exec_picker_resolve(&mut self) -> Option<(Vec<String>, PathBuf)> {
    let profile = self.exec_picker.selected_profile()?.to_string();
    // Resolve against the worktree captured when the picker opened, NOT the
    // live selection (which an auto-refresh may have drifted) — #333 review.
    let Some(cwd) = self.exec_picker.cwd().map(Path::to_path_buf) else {
      self.status = "nothing selected".into();
      return None;
    };
    // Resolve against the `[exec]` config captured at open, not the live one.
    match crate::exec::resolve_exec_command(Some(&profile), &[], &self.exec_picker_cfg) {
      Ok(mut argv) => {
        // Pin a worktree-relative executable (`./run.sh`, `scripts/build`) to
        // the captured worktree, exactly like the CLI exec path — otherwise
        // `argv[0]` would resolve against gwm's own cwd (Codex #333 review).
        // A bare command (`cargo`) or an absolute path is returned unchanged
        // (PATH lookup / as-is).
        if let Some(first) = argv.first_mut() {
          *first = crate::exec::resolve_program(&cwd, first).to_string_lossy().into_owned();
        }
        Some((argv, cwd))
      }
      Err(e) => {
        self.status = format!("exec profile {profile:?}: {e}");
        None
      }
    }
  }

  /// Close the exec picker without running anything (issue #325). Returns
  /// to [`View::List`].
  pub fn close_exec_picker(&mut self) {
    if self.view == View::ExecPicker {
      self.view = View::List;
    }
  }

  // ── Clean overlay (issue #325) ─────────────────────────────────────────

  /// Open the clean overlay (issue #325). Populates the `[clean.profiles]`
  /// picker, scans the selected worktree through the safety gate
  /// ([`crate::clean::scan_worktree_safe`]), and switches to
  /// [`View::CleanReport`]. Refuses (status-bar message, no transition) when
  /// nothing is selected. A scan that finds nothing safe still opens — the
  /// report says so.
  /// Open the agent-session detail overlay for the selected worktree
  /// (issue #408, `a`). Rows come from the pure
  /// [`crate::tui::state::detail_overlay::agent_detail_rows`] mapping over
  /// the last completed snapshot — a session-less worktree opens with an
  /// explicit "no agent session found" row, never blank.
  pub fn open_agent_overlay(&mut self) {
    let Some(sel) = self.selected().cloned() else {
      self.status = "nothing selected".into();
      return;
    };
    // Capture the target now (clean-overlay pattern, Codex #333): an
    // auto-refresh can drift the live selection while the overlay is open,
    // and attach/detach must pin against THIS worktree's branch.
    self.detail_overlay_target = Some((
      sel.path.clone(),
      crate::github::pinnable_branch(sel.branch.as_deref()).map(str::to_string),
    ));
    let rows = self.build_agent_rows(&sel);
    self.detail_overlay.open(
      crate::tui::state::detail_overlay::DetailKind::Agents,
      "Agent Sessions".into(),
      rows,
    );
    self.view = View::DetailOverlay;
  }

  /// Open the CI checks overlay (issue #436): one row per classified
  /// `statusCheckRollup` entry of the linked PR, in rollup order. With no
  /// linked PR or an empty rollup the overlay would be a bordered void —
  /// explain on the status bar instead.
  pub fn enter_ci_checks(&mut self) {
    let checks = match self.pr_fetch_state() {
      GitHubFetchState::Loaded(pr) if !pr.checks.is_empty() => pr.checks.clone(),
      _ => {
        // Resolve the active fetch binding instead of hard-coding `F`
        // (Codex review #455); an unbound action drops the parenthetical.
        self.status = match self.keymap.primary_chord(Action::FetchGithub) {
          Some(key) => format!("no CI checks to show — link a PR and fetch ({key}) first"),
          None => "no CI checks to show — link a PR and fetch first".into(),
        };
        return;
      }
    };
    let rows = crate::tui::state::detail_overlay::ci_check_rows(&checks, std::time::SystemTime::now());
    // Drop any stale agents target (an interrupted agents overlay leaves
    // one behind) — it belongs to the agents consumer only (Codex #455).
    self.detail_overlay_target = None;
    // Pin the overlay to the PR it renders, so a refresh whose re-detected
    // link disagrees can close it (Codex review #455).
    self.detail_overlay_pr = self.github.link.pr;
    self.detail_overlay.open(
      crate::tui::state::detail_overlay::DetailKind::CiChecks,
      "CI Checks".into(),
      rows,
    );
    self.view = View::DetailOverlay;
  }

  /// Contextual KEY routing (issue #436) — same mechanism that turns
  /// `j` / `k` into sidebar scroll: while the status pane holds the
  /// focus, the `c` keystroke (EditWorktree) opens the CI checks
  /// overlay instead of the rename modal. Applied by the event loop on
  /// the **key path only** (Codex review #455): the command palette
  /// dispatches actions by their NAME, so its `edit-worktree` entry
  /// must stay a rename in every context (a dedicated `ci-checks`
  /// entry already exists there). Pure, so the contract is pinned
  /// without an event loop.
  pub fn resolve_contextual_action(&self, action: Action) -> Action {
    if action == Action::EditWorktree && self.sidebar.open && self.sidebar.focused {
      Action::CiChecks
    } else {
      action
    }
  }

  // ---- CI checks overlay `f` filter (issue #436) ---------------------------
  // Same shell machinery as the agent attach prompt right above (mode +
  // input buffer + candidate cursor), filtering the overlay's own rows.

  pub fn ci_input_open(&mut self) {
    self.detail_overlay.mode = crate::tui::state::detail_overlay::DetailMode::Input;
    self.detail_overlay.input.clear();
    self.detail_overlay.input_selected = 0;
  }

  pub fn ci_input_push(&mut self, c: char) {
    self.detail_overlay.input.push(c);
    self.detail_overlay.input_selected = 0;
  }

  pub fn ci_input_pop(&mut self) {
    self.detail_overlay.input.pop();
    self.detail_overlay.input_selected = 0;
  }

  /// Indices of the rows matching the live query, in row order.
  pub fn ci_input_matches(&self) -> Vec<usize> {
    crate::tui::state::detail_overlay::filter_rows(&self.detail_overlay.rows, &self.detail_overlay.input)
  }

  pub fn ci_input_next(&mut self) {
    let len = self.ci_input_matches().len();
    self.detail_overlay.input_selected = (self.detail_overlay.input_selected + 1).min(len.saturating_sub(1));
  }

  pub fn ci_input_prev(&mut self) {
    self.detail_overlay.input_selected = self.detail_overlay.input_selected.saturating_sub(1);
  }

  pub fn ci_input_cancel(&mut self) {
    self.detail_overlay.mode = crate::tui::state::detail_overlay::DetailMode::List;
    self.detail_overlay.input.clear();
  }

  /// The details URL of the highlighted filtered row (Enter inside the
  /// filter). Pure so the event loop owns the actual browser spawn; also
  /// re-anchors the List selection on the picked row and leaves the
  /// filter, so Esc-free flows land where the user expects.
  pub fn ci_input_selected_url(&mut self) -> Option<String> {
    let matches = self.ci_input_matches();
    let row_idx = matches.get(self.detail_overlay.input_selected).copied()?;
    self.detail_overlay.selected = row_idx;
    self.ci_input_cancel();
    self.detail_overlay.rows.get(row_idx).and_then(|r| r.meta.clone())
  }

  /// The details URL of the selected row in List mode (Enter). `None` when
  /// the check carries no URL — the caller reports on the status bar.
  pub fn ci_selected_url(&self) -> Option<String> {
    self
      .detail_overlay
      .rows
      .get(self.detail_overlay.selected)
      .and_then(|r| r.meta.clone())
  }

  /// Rows for the captured worktree: sessions from the snapshot, the manual
  /// pins marked (issue #408 US4 + user feedback 2026-07-22 — multi-pin).
  fn build_agent_rows(&self, w: &crate::worktree::WorktreeInfo) -> Vec<crate::tui::state::detail_overlay::DetailRow> {
    // Pins come from the per-path map — built per OWNING repo, so a
    // workspace active-repo swap under the open overlay cannot yield
    // absent or wrong markers (round N), and the snapshot-landing rebuild
    // does no branch-config I/O on the event loop (round P): the map is
    // refreshed by the landing itself and by every attach/detach.
    let pinned = self
      .agent_pins
      .get(&crate::agent_sessions::path_display_key(&w.path))
      .cloned()
      .unwrap_or_default();
    crate::tui::state::detail_overlay::agent_detail_rows(self.agents_for(w), &pinned, std::time::SystemTime::now())
  }

  /// Fresh pins per worktree path from branch config — the synchronous
  /// read for USER-ACTION paths (attach/detach refresh); the periodic
  /// detection reads the same sources in its worker instead (round P).
  /// Each row reads from its OWNING repo via [`Self::agent_pin_sources`]
  /// (rounds A + I: a same-named branch elsewhere cannot leak its pins).
  fn read_agent_pins(&self) -> std::collections::BTreeMap<String, Vec<String>> {
    read_pins_from_sources(&self.agent_pin_sources())
  }

  /// The current pinnable branch of the worktree at `path`, freshly read
  /// from the listed rows (which every refresh re-lists) — never the
  /// branch captured when an overlay opened (Codex review round U).
  fn current_branch_of(&self, path: &Path) -> Option<String> {
    let w = self.worktrees.iter().find(|w| w.path == path)?;
    crate::github::pinnable_branch(w.branch.as_deref()).map(str::to_string)
  }

  /// Pin the selected overlay row's session to the overlay's target worktree
  /// (`a` inside the modal). Auto-detection stays the default; the pin only
  /// adds (issue #408 US4).
  pub fn attach_selected_agent(&mut self) {
    let Some(sid) = self.detail_overlay.selected_meta().map(str::to_string) else {
      // Only the "no agent session found" placeholder carries no id: with
      // nothing to select, `a` falls through to the attach-by-id prompt
      // instead of dead-ending (user feedback 2026-07-22).
      self.open_agent_input();
      return;
    };
    self.attach_agent_by_id(&sid);
  }

  /// Pin `sid` to the overlay's target worktree — shared by the row action
  /// and the attach-by-id prompt. Returns `true` when the pin was written.
  fn attach_agent_by_id(&mut self, sid: &str) -> bool {
    if self.is_workspace() {
      // Pins are single-repo (same ceiling as the CLI surfaces): in
      // workspace mode `sync_active_repo` may swap `self.repo` under the
      // open overlay, which would write the pin into the wrong repo's
      // config (Codex review round B).
      self.status = "agent pins are per-repo — not available in workspace mode".into();
      return false;
    }
    let Some((path, _)) = self.detail_overlay_target.clone() else {
      self.status = "cannot pin: no worktree captured".into();
      return false;
    };
    // The CURRENT branch, not the one captured at overlay open: a branch
    // flipped externally while the overlay stayed open would otherwise
    // receive the pin under `branch.<old>.` (Codex review round U).
    let Some(branch) = self.current_branch_of(&path) else {
      self.status = "cannot pin: worktree has no branch (detached HEAD)".into();
      return false;
    };
    if let Err(e) = crate::github::add_agent_pin(&self.repo, &branch, sid) {
      self.status = format!("pin failed: {e}");
      return false;
    }
    self.status = format!("pinned {sid}");
    self.refresh_agent_overlay_rows(&path);
    true
  }

  /// Enter the attach-by-id prompt (`i` in the overlay): palette-style
  /// query over EVERY detected session — a session matched to no worktree
  /// is exactly the one worth pinning manually.
  pub fn open_agent_input(&mut self) {
    self.detail_overlay.mode = crate::tui::state::detail_overlay::DetailMode::Input;
    self.detail_overlay.input.clear();
    self.detail_overlay.input_selected = 0;
    // The candidate pool needs the full sweep — refreshed on open, not on
    // the periodic tick (round Q); until it lands the prompt filters the
    // last landed pool.
    self.refresh_agent_pool();
  }

  pub fn agent_input_push(&mut self, c: char) {
    self.detail_overlay.input.push(c);
    self.detail_overlay.input_selected = 0;
  }

  pub fn agent_input_pop(&mut self) {
    self.detail_overlay.input.pop();
    self.detail_overlay.input_selected = 0;
  }

  pub fn agent_input_next(&mut self) {
    let len = self.agent_input_candidates().len();
    self.detail_overlay.input_selected = (self.detail_overlay.input_selected + 1).min(len.saturating_sub(1));
  }

  pub fn agent_input_prev(&mut self) {
    self.detail_overlay.input_selected = self.detail_overlay.input_selected.saturating_sub(1);
  }

  pub fn agent_input_cancel(&mut self) {
    self.detail_overlay.mode = crate::tui::state::detail_overlay::DetailMode::List;
    self.detail_overlay.input.clear();
  }

  /// The prompt's filtered candidate pool (owned clones — the borrow of
  /// `agent_all_sessions` must not outlive `&mut self` call sites).
  pub fn agent_input_candidates(&self) -> Vec<crate::agent_sessions::AgentSession> {
    crate::tui::state::detail_overlay::filter_sessions(&self.agent_all_sessions, &self.detail_overlay.input)
      .into_iter()
      .cloned()
      .collect()
  }

  /// Attach the highlighted candidate (or the literal query when nothing
  /// matches a known session — validated before persisting). Unknown id
  /// keeps the prompt open for correction.
  pub fn agent_input_submit(&mut self) {
    let candidates = self.agent_input_candidates();
    let sid = candidates
      .get(self.detail_overlay.input_selected)
      .map(|s| s.id.clone())
      .unwrap_or_else(|| self.detail_overlay.input.trim().to_string());
    let known = candidates.iter().any(|s| s.id == sid);
    if sid.is_empty() || !known {
      self.status = format!("no agent session matching '{sid}' — run gwm agents for ids");
      return;
    }
    if self.attach_agent_by_id(&sid) {
      self.detail_overlay.mode = crate::tui::state::detail_overlay::DetailMode::List;
      self.detail_overlay.input.clear();
    }
  }

  /// Unpin the SELECTED session (`d` inside the modal). Pins are
  /// multi-valued (user feedback 2026-07-22): only the highlighted
  /// session's pin is removed, the others stay.
  pub fn detach_selected_agent(&mut self) {
    if self.is_workspace() {
      self.status = "agent pins are per-repo — not available in workspace mode".into();
      return;
    }
    let Some(sid) = self.detail_overlay.selected_meta().map(str::to_string) else {
      self.status = "no session selected to unpin".into();
      return;
    };
    let Some((path, _)) = self.detail_overlay_target.clone() else {
      self.status = "cannot detach: no worktree captured".into();
      return;
    };
    // Same round-U rule as attach: unpin from the CURRENT branch.
    let Some(branch) = self.current_branch_of(&path) else {
      self.status = "cannot detach: worktree has no branch (detached HEAD)".into();
      return;
    };
    match crate::github::remove_agent_pin(&self.repo, &branch, &sid) {
      Ok(true) => self.status = format!("unpinned {sid}"),
      Ok(false) => {
        self.status = "session is not pinned".into();
        return;
      }
      Err(e) => {
        self.status = format!("detach failed: {e}");
        return;
      }
    }
    self.refresh_agent_overlay_rows(&path);
  }

  /// Rebuild the open overlay's rows after a pin change, refresh the
  /// render-side pins copy (the Agents pane shows pinned-only), and push
  /// the new pin state to every other surface (snapshot re-detection).
  fn refresh_agent_overlay_rows(&mut self, path: &Path) {
    // Map first: `build_agent_rows` reads the [`Self::agent_pins`] copy
    // (round P), so the fresh read must land before the rows rebuild.
    self.agent_pins = self.read_agent_pins();
    if let Some(w) = self.worktrees.iter().find(|w| w.path == path).cloned() {
      let rows = self.build_agent_rows(&w);
      self.detail_overlay.set_rows(rows);
    }
    if self.tasks.is_loading(TaskKind::AgentSessions) {
      // The in-flight thread keeps walking the store even if its slot is
      // dropped — invalidating here raced a second scan against it
      // (round U, same hazard as rounds P/R). Let it land and chain the
      // re-detection; its pre-change pins are skipped on landing.
      self.agent_redetect_wanted = true;
    } else {
      self.tasks.invalidate(TaskKind::AgentSessions);
      self.agent_snapshot_at = None;
    }
  }

  /// Close the detail overlay back to the list, leaving list state as it was.
  pub fn close_detail_overlay(&mut self) {
    self.detail_overlay_target = None;
    self.detail_overlay_pr = None;
    self.view = View::List;
  }

  pub fn enter_clean_overlay(&mut self) {
    let Some(sel) = self.selected() else {
      self.status = "nothing selected".into();
      return;
    };
    // Capture the target worktree AND the active repo's `[clean]` config now:
    // an auto-refresh can drift the live selection (and, in workspace mode,
    // the active repo) while the overlay is open / armed, so every re-scan
    // and the delete must pin to *this* worktree against *this* config
    // (Codex #333 review).
    let name = sel.name.clone();
    let path = sel.path.clone();
    self.clean_overlay_cfg = self.config.clean.clone();
    self.clean_overlay_countdown_secs = self.config.tui.effective_confirm_countdown_secs();
    let names: Vec<String> = self.clean_overlay_cfg.profiles.keys().cloned().collect();
    self.clean_overlay.open(names, name, path);
    if let Err(e) = self.clean_overlay_rescan() {
      self.status = format!("clean: {e}");
      return;
    }
    self.view = View::CleanReport;
  }

  /// Re-resolve the highlighted profile's dirs and re-scan the *captured*
  /// target worktree (not the live selection), storing the gated snapshot.
  /// Surfaces a profile-resolution error (e.g. an invalid `[clean.profiles]`
  /// dir) to the caller.
  fn clean_overlay_rescan(&mut self) -> Result<()> {
    let Some((name, path)) = self
      .clean_overlay
      .target()
      .map(|(n, p)| (n.to_string(), p.to_path_buf()))
    else {
      return Ok(());
    };
    let profile = self.clean_overlay.selected_profile().map(str::to_string);
    let dirs = crate::clean::resolve_clean_dirs(profile.as_deref(), &self.clean_overlay_cfg)?;
    let (reclaim, skipped) = crate::clean::scan_worktree_safe(&name, &path, &dirs);
    self.clean_overlay.set_scan(reclaim, skipped);
    Ok(())
  }

  /// Cycle the clean profile picker forward and re-scan, but ONLY when the
  /// highlight actually moved (issue #325 / Codex #333). A no-op move (only
  /// the `(default)` choice) must not re-scan — that would reset the
  /// `ConfirmModal` and silently disarm a pending reclaim while the status
  /// bar still reads `armed`.
  pub fn clean_overlay_next(&mut self) {
    if self.clean_overlay.select_next() {
      if let Err(e) = self.clean_overlay_rescan() {
        self.status = format!("clean: {e}");
      }
    }
  }

  /// Cycle the clean profile picker backward and re-scan, only when the
  /// highlight actually moved (issue #325 / Codex #333).
  pub fn clean_overlay_prev(&mut self) {
    if self.clean_overlay.select_prev() {
      if let Err(e) = self.clean_overlay_rescan() {
        self.status = format!("clean: {e}");
      }
    }
  }

  /// Total duration of the clean safety countdown. Unlike the delete-confirm
  /// modal, clean has no `delete_branch_on_remove` gate — it reads
  /// `[tui] confirm_countdown_secs` directly. `Duration::ZERO` ⇒ classic
  /// single-keystroke confirm.
  pub fn clean_countdown_total(&self) -> Duration {
    // The value captured at open (Codex #333) — never the live config, which a
    // workspace refresh could swap (e.g. to `0`, erasing the safety delay).
    Duration::from_secs(u64::from(self.clean_overlay_countdown_secs))
  }

  /// Handle the clean confirm key. Arms / disarms / fires the countdown via
  /// the dedicated [`CleanOverlay`] modal. Nothing-to-reclaim is a no-op
  /// guard so the user cannot arm a delete that would free zero bytes.
  pub fn clean_confirm_press(&mut self, now: Instant) -> ConfirmKeyAction {
    if self.clean_overlay.is_empty_scan() {
      self.status = "nothing to reclaim".into();
      return ConfirmKeyAction::Disarmed;
    }
    let total = self.clean_countdown_total();
    let action = self.clean_overlay.confirm.press_y(now, total);
    match action {
      ConfirmKeyAction::Armed => {
        self.status = format!(
          "armed — reclaiming {} in {}s",
          crate::clean::human_size(self.clean_overlay.total_bytes()),
          total.as_secs()
        );
      }
      ConfirmKeyAction::Disarmed => self.status = "clean cancelled".into(),
      ConfirmKeyAction::FireNow => {}
    }
    action
  }

  /// Tick the clean safety countdown. Called from the event loop on every
  /// poll-timeout iteration while the overlay is open.
  pub fn tick_clean_countdown(&mut self, now: Instant) -> CountdownTickOutcome {
    self.clean_overlay.confirm.tick(now, self.clean_countdown_total())
  }

  /// Clean countdown progress in `[0.0, 1.0]` for the UI gauge.
  pub fn clean_countdown_progress(&self, now: Instant) -> f64 {
    self.clean_overlay.confirm.progress(now, self.clean_countdown_total())
  }

  /// Seconds remaining (rounded up) on the clean countdown, for the UI label.
  pub fn clean_countdown_remaining_secs(&self, now: Instant) -> u64 {
    self
      .clean_overlay
      .confirm
      .remaining_secs(now, self.clean_countdown_total())
  }

  /// Delete the gated reclaim of the current clean snapshot (issue #325) and
  /// return to the list. The snapshot was already filtered to the
  /// git-ignored, untracked artifacts by [`crate::clean::scan_worktree_safe`],
  /// so this only removes what the CLI `gwm clean --yes` would. Reports the
  /// freed size (or the failure) on the status bar.
  pub fn clean_overlay_delete(&mut self) {
    // Re-scan + re-gate IMMEDIATELY before deleting rather than trusting the
    // snapshot shown in the overlay (Codex #333 review). That snapshot can be
    // seconds old — the safety countdown, or just the overlay sitting open —
    // and a directory may have turned unsafe meanwhile (e.g. `git add -f
    // target/file` under an ignored `target/`). Deleting a freshly gated
    // reclaim closes that TOCTOU window, matching the CLI's scan-then-delete.
    // Pin to the CAPTURED target worktree, not the live selection (an
    // auto-refresh may have drifted it while the countdown ran) — #333.
    let Some((name, path)) = self
      .clean_overlay
      .target()
      .map(|(n, p)| (n.to_string(), p.to_path_buf()))
    else {
      self.close_clean_overlay();
      return;
    };
    let profile = self.clean_overlay.selected_profile().map(str::to_string);
    let dirs = match crate::clean::resolve_clean_dirs(profile.as_deref(), &self.clean_overlay_cfg) {
      Ok(d) => d,
      Err(e) => {
        self.status = format!("clean: {e}");
        self.close_clean_overlay();
        return;
      }
    };
    let (reclaim, _skipped) = crate::clean::scan_worktree_safe(&name, &path, &dirs);
    if reclaim.artifacts.is_empty() {
      self.status = "nothing to reclaim".into();
      self.close_clean_overlay();
      return;
    }
    match crate::clean::delete_reclaim(&reclaim) {
      Ok(freed) => {
        self.status = format!("reclaimed {} from {}", crate::clean::human_size(freed), reclaim.name);
      }
      Err(e) => self.status = format!("clean failed: {e}"),
    }
    self.close_clean_overlay();
  }

  /// Close the clean overlay, disarming the countdown, and return to
  /// [`View::List`] (issue #325).
  pub fn close_clean_overlay(&mut self) {
    self.clean_overlay.confirm.dismiss();
    if self.view == View::CleanReport {
      self.view = View::List;
    }
  }

  /// Activate the selected Settings field (issue #279): cycle a choice field
  /// to its next value (writing + applying live), or arm the numeric input
  /// buffer for a `Uint` field. No-op on the read-only `All` tab.
  pub fn activate_selected_setting(&mut self) {
    let Some(field) = self.config_panel.selected_field() else {
      return;
    };
    match field.kind() {
      FieldKind::Choice => {
        if let Some(next) = field.next_choice(&self.config) {
          self.apply_setting(field, &next);
        }
      }
      FieldKind::Uint | FieldKind::Text => {
        let current = field.current(&self.config);
        self.config_panel.begin_edit(&current);
      }
    }
  }

  /// Commit the in-progress numeric edit (issue #279): write the buffered
  /// value to the selected field and apply it live. Clearing the buffer
  /// reads as `0` (see [`ConfigPanel::take_edit`]).
  pub fn commit_settings_edit(&mut self) {
    let Some(field) = self.config_panel.selected_field() else {
      self.config_panel.cancel_edit();
      return;
    };
    if let Some(value) = self.config_panel.take_edit() {
      // A cleared numeric input is a valid zero; a cleared text input is a
      // legitimate empty / unset value.
      let value = if field.kind() == FieldKind::Uint && value.is_empty() {
        "0".to_string()
      } else {
        value
      };
      self.apply_setting(field, &value);
    }
  }

  /// Persist `field = value` into the active layer's TOML file and apply the
  /// change live (issue #279). The write targets the per-project `.gwm.toml`
  /// or the user-global `config.toml` per the panel's layer selector; on
  /// success the config is reloaded, the theme re-resolved, the sidebar
  /// position re-seeded and the resolved-rows snapshot refreshed so the
  /// `All` tab and the source attribution track the edit. Every fallible
  /// step routes its error to the status line — no `unwrap` on this path.
  pub fn apply_setting(&mut self, field: SettingField, value: &str) {
    // A Project-layer write targets `self.workdir/.gwm.toml`. In workspace mode
    // with a stale selection that path is the *previously* active repo, so
    // refuse rather than write settings into the wrong repo (#304). Global-layer
    // edits are repo-independent and stay allowed.
    if self.workspace_active_stale && self.config_panel.layer == SettingsLayer::Project {
      self.status = "workspace: selected repo is unavailable — can't edit its project config".into();
      return;
    }
    let path = match self.config_panel.layer {
      SettingsLayer::Project => self.workdir.join(crate::config::CONFIG_FILE),
      SettingsLayer::Global => match self.global_path.clone() {
        Some(p) => p,
        None => {
          self.status = "settings: no global config path (set $XDG_CONFIG_HOME or $HOME)".into();
          return;
        }
      },
    };

    // Numeric fields write a TOML integer; choices and free text write a
    // TOML string, so a value like `123` / `true` in a shell command or
    // worktree pattern is preserved as text rather than coerced (review P2).
    let write = match field.kind() {
      FieldKind::Uint => crate::config_cli::set_value_at(&path, field.key_path(), value),
      FieldKind::Choice | FieldKind::Text => crate::config_cli::set_string_at(&path, field.key_path(), value),
    };
    if let Err(e) = write {
      self.status = format!("settings: {}", e);
      return;
    }

    // Reload the merged config so every live read (open mode, confirm
    // countdown) and the re-seeded state below reflect the edit.
    match Config::load_layered(&self.workdir, self.global_path.as_deref()) {
      Ok(cfg) => self.set_active_config(cfg),
      Err(e) => {
        self.status = format!("settings saved, but reload failed: {}", e);
        return;
      }
    }
    // A Global-layer edit changes config for *every* repo, not just the active
    // one — refresh each cached `RepoMeta.config` so navigating to another repo
    // doesn't restore the pre-edit global value (Codex review #303 P2). A
    // Project-layer edit only touched the active repo's `.gwm.toml`, already
    // handled by `set_active_config`.
    if self.config_panel.layer == SettingsLayer::Global {
      self.reload_workspace_repo_configs();
    }
    match self.config.theme.resolve() {
      Ok(theme) => self.theme = theme,
      Err(e) => self.status = format!("theme: {}", e),
    }
    self.apply_sidebar_config();
    if let Ok(rows) = crate::config::resolved_rows(&self.workdir, self.global_path.as_deref()) {
      self.config_panel.rows = rows;
    }

    let mut status = format!(
      "set {} = {} ({})",
      field.key_path(),
      value,
      self.config_panel.layer.label()
    );
    // Surface a shadowed edit: writing global for a key the repo overrides
    // leaves the effective value unchanged (repo wins).
    if self.config_panel.layer == SettingsLayer::Global
      && self.config_panel.field_source(field) == Some(crate::config::ConfigSource::Repo)
    {
      status.push_str(" — shadowed by .gwm.toml");
    }
    self.status = status;
  }

  /// Render the Command Logs transcript as plain text for the clipboard
  /// (issue #279, `y`): newest-first, mirroring the overlay's layout
  /// (`$ argv`, the outcome line, then the full captured output — not the
  /// tail-capped view), entries separated by a blank line. Pure + owned so
  /// the format is unit-testable without a clipboard. Empty when no commands
  /// have run.
  pub fn command_logs_transcript(&self) -> String {
    use crate::command_log::CommandStatus;
    let mut out = String::new();
    for entry in self.command_logs.entries.iter().rev() {
      out.push_str(&format!("$ {}\n", entry.command));
      let detail = match &entry.status {
        CommandStatus::Exited(Some(0)) => format!("→ exit 0 ({} ms)", entry.duration.as_millis()),
        CommandStatus::Exited(Some(code)) => format!("→ exit {} ({} ms)", code, entry.duration.as_millis()),
        CommandStatus::Exited(None) => format!("→ terminated ({} ms)", entry.duration.as_millis()),
        CommandStatus::Spawn => "✗ failed to spawn".to_string(),
      };
      out.push_str(&format!("  {}\n", detail));
      for line in entry.output.lines() {
        out.push_str(&format!("    {}\n", line));
      }
      out.push('\n');
    }
    out.trim_end().to_string()
  }

  /// Scroll the help overlay down one row, clamped to the renderer-published
  /// `help_max_scroll` so it never scrolls past the last line.
  pub fn help_scroll_down(&mut self) {
    self.help_scroll = (self.help_scroll + 1).min(self.help_max_scroll);
  }

  /// Scroll the help overlay up one row, clamped at the top.
  pub fn help_scroll_up(&mut self) {
    self.help_scroll = self.help_scroll.saturating_sub(1);
  }

  pub fn help_scroll_right(&mut self) {
    self.help_x_scroll = (self.help_x_scroll + 1).min(self.help_max_x_scroll);
  }

  pub fn help_scroll_left(&mut self) {
    self.help_x_scroll = self.help_x_scroll.saturating_sub(1);
  }

  /// Path to launch lazygit on, or `None` if nothing selected or lazygit is missing.
  /// The caller drives the actual TUI suspension/restoration around the spawn.
  ///
  /// Retained for callers that still want the legacy "lazygit only"
  /// path; new code should go through [`Self::prepare_git_tui`], which
  /// honours the configurable `[git_tui]` block (issue #75).
  pub fn launch_lazygit(&mut self) -> Option<PathBuf> {
    let path = self.selected()?.path.clone();
    if which::which("lazygit").is_err() {
      self.status = "lazygit not found in PATH".into();
      return None;
    }
    Some(path)
  }

  // ---- Configurable launchers (issue #75) ---------------------------------

  /// Build the [`LauncherPlan`] for the `l` keybinding. Reads
  /// `[git_tui]` from `.gwm.toml` (default `lazygit -p {path}`
  /// fullscreen=true) and expands the `{path}` placeholder against
  /// the selected worktree. Returns `None` (and sets a status hint)
  /// when nothing is selected or the template is malformed.
  pub fn prepare_git_tui(&mut self) -> Option<LauncherPlan> {
    let Some(wt) = self.selected().cloned() else {
      self.status = "nothing selected".into();
      return None;
    };
    let resolved = self.config.git_tui.resolved();
    let ctx = LauncherContext {
      worktree_path: &wt.path,
      base: None,
      head: None,
      repo_workdir: Some(&self.workdir),
    };
    match launcher::expand_command(&resolved.command, &ctx) {
      Ok(expanded) => Some(LauncherPlan {
        expanded,
        cwd: wt.path,
        fullscreen: resolved.fullscreen,
        base: None,
      }),
      Err(e) => {
        self.status = format!("git_tui template error: {}", e);
        None
      }
    }
  }

  /// Build the [`LauncherPlan`] for the `R` keybinding. Implements the
  /// full review contract from issue #75:
  ///
  /// 1. `[review]` must resolve to a concrete launcher (`command`
  ///    set, or `tool = "<preset>"` matched).
  /// 2. The selected worktree must carry a branch name.
  /// 3. The review base is resolved via the documented chain (upstream
  ///    → `gwm-base` → `[review].default_base` → `"dev"` → `"main"`).
  /// 4. When `skip_when_no_changes` is on (default), a zero
  ///    `git rev-list --count {base}..HEAD` short-circuits with a
  ///    status-bar hint naming the base.
  /// 5. The template is expanded; `{diff}` lazily materialises a
  ///    tempfile so unused placeholders never spawn `git diff`.
  pub fn prepare_review(&mut self) -> Option<LauncherPlan> {
    let resolved = match self.config.review.resolved() {
      Some(r) => r,
      None => {
        self.status = "review tool not configured — set [review] in .gwm.toml".into();
        return None;
      }
    };
    let Some(wt) = self.selected().cloned() else {
      self.status = "nothing selected".into();
      return None;
    };
    let Some(head) = wt.branch.clone() else {
      self.status = "selected worktree has no branch — cannot review".into();
      return None;
    };

    let base = launcher::resolve_review_base(&self.repo, &head, self.config.review.default_base.as_deref());

    if self.config.review.skip_when_no_changes {
      let n = launcher::count_commits_ahead(&wt.path, &base, "HEAD");
      if n == 0 {
        self.status = format!("no changes to review (already at {})", base);
        return None;
      }
    }

    let ctx = LauncherContext {
      worktree_path: &wt.path,
      base: Some(&base),
      head: Some(&head),
      repo_workdir: Some(&self.workdir),
    };
    match launcher::expand_command(&resolved.command, &ctx) {
      Ok(expanded) => {
        if self.config.review.has_shadowed_tool() {
          self.status = format!("review: command overrides tool — running {}", base);
        } else {
          self.status = format!("review: {} vs {}", head, base);
        }
        Some(LauncherPlan {
          expanded,
          cwd: wt.path,
          fullscreen: resolved.fullscreen,
          base: Some(base),
        })
      }
      Err(e) => {
        self.status = format!("review template error: {}", e);
        None
      }
    }
  }

  pub fn selected(&self) -> Option<&WorktreeInfo> {
    // The visible list is the filtered subset, so the table state's index is
    // into `filtered_indices()`, not the raw `worktrees` vec. Resolving the
    // selection means hopping through the filter map.
    //
    // `selected` keeps its `&self` signature so callers holding a
    // shared borrow (e.g. `ui.rs` render path, `copy_path_to_status`)
    // don't have to upgrade. `snapshot_indices` reads the cache when
    // it's warm (which the per-frame render path guarantees, since
    // the table renderer calls `filtered_indices` first) and falls
    // back to a fresh compute when it isn't.
    let i = self.list_state.selected()?;
    let filtered = self.filter.snapshot_indices(&self.worktrees, fuzzy_match_indices);
    let original = *filtered.get(i)?;
    self.worktrees.get(original)
  }

  pub fn copy_path_to_status(&mut self) {
    if let Some(w) = self.selected() {
      self.status = format!("path: {}", w.path.display());
    }
  }

  /// Reveal the selected worktree's directory in the OS file manager.
  /// macOS: `open`, Linux: `xdg-open`, Windows: `explorer`. Used by
  /// `resolve_open_target` when the config picks `mode = "finder"`,
  /// and by the event loop directly to spawn the opener.
  pub fn open_selected_in_finder(&mut self) {
    let path = match self.selected() {
      Some(w) => w.path.clone(),
      None => {
        self.status = "nothing selected".into();
        return;
      }
    };
    let opener = if cfg!(target_os = "macos") {
      "open"
    } else if cfg!(target_os = "windows") {
      "explorer"
    } else {
      "xdg-open"
    };
    match std::process::Command::new(opener).arg(&path).spawn() {
      Ok(_) => self.status = format!("opened {} in {}", path.display(), opener),
      Err(e) => self.status = format!("failed to open {}: {}", path.display(), e),
    }
  }

  /// Return the path that the `Y: yank-path` key should push into the
  /// system clipboard, or `None` when nothing is selected. Pure — the
  /// shell-out is handled by the event loop.
  pub fn yank_selected_path(&self) -> Option<PathBuf> {
    self.selected().map(|w| w.path.clone())
  }

  /// Return the branch name for the `y: yank-branch-name` key (#290).
  pub fn yank_selected_branch(&self) -> Option<String> {
    self.selected()?.branch.clone()
  }

  /// Return the worktree slug/name for the `w: yank-worktree-name` key (#290).
  pub fn yank_selected_worktree_name(&self) -> Option<String> {
    self.selected().map(|w| w.name.clone())
  }

  /// Signal the event loop to print the selected worktree path to stdout
  /// before quitting (`e: exit-to-worktree`, #290). The loop checks
  /// `should_exit_to` after `can_quit_now` to emit the path.
  pub fn exit_to_worktree(&mut self) {
    let Some(path) = self.selected().map(|w| w.path.clone()) else {
      self.status = "no worktree selected".into();
      return;
    };
    self.should_exit_to = Some(path);
    self.should_quit = true;
  }

  /// Request an off-thread `git pull` of the selected worktree's branch
  /// (#290). Coalesces if a pull is already in flight, and refuses to start
  /// while a *different* mutating task (sync / bootstrap / push / rename /
  /// create / delete) runs in the same worktree (Codex review on PR #292).
  pub fn request_pull(&mut self) {
    let Some((path, name)) = self.selected().map(|w| (w.path.clone(), w.name.clone())) else {
      self.status = "no worktree selected".into();
      return;
    };
    if self.tasks.has_mutating_task_in_flight() && !self.tasks.is_loading(TaskKind::Pull) {
      self.status = self.busy_mutation_status("pulling");
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::Pull) else {
      return;
    };
    self.spinner.reset();
    self.status = TaskKind::Pull.loading_label().into();
    self.spawn_pull(generation, path, name);
  }

  /// Status line shown when a mutating verb is pressed while another mutating
  /// task is in flight. `action` is the gerund of the blocked verb
  /// (e.g. "pulling", "pushing").
  fn busy_mutation_status(&self, action: &str) -> String {
    match self.tasks.mutating_loading_label() {
      Some(label) => format!("finish {} before {}", label.trim_end_matches('…'), action),
      None => format!("finish current task before {}", action),
    }
  }

  fn spawn_pull(&self, generation: u64, path: PathBuf, name: String) {
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let mut cmd = std::process::Command::new("git");
      cmd.args(["pull"]).current_dir(&path);
      // Route through the command-log chokepoint so `git pull` lands in the
      // Command Logs modal (#290) — a user-triggered mutating op the user
      // expects to find in the transcript.
      let result = crate::command_log::run_logged(&mut cmd, "git pull".to_string())
        .map_err(|e| e.to_string())
        .and_then(|out| {
          if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
          } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
          }
        });
      let _ = tx.send(TaskMsg::Pull(generation, name, result));
    });
  }

  /// Request an off-thread `git push` of the selected worktree's branch
  /// (#290). Coalesces if a push is already in flight, and refuses to start
  /// while a *different* mutating task runs in the same worktree (Codex review
  /// on PR #292).
  pub fn request_push(&mut self) {
    let Some((path, name)) = self.selected().map(|w| (w.path.clone(), w.name.clone())) else {
      self.status = "no worktree selected".into();
      return;
    };
    if self.tasks.has_mutating_task_in_flight() && !self.tasks.is_loading(TaskKind::Push) {
      self.status = self.busy_mutation_status("pushing");
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::Push) else {
      return;
    };
    self.spinner.reset();
    self.status = TaskKind::Push.loading_label().into();
    self.spawn_push(generation, path, name);
  }

  fn spawn_push(&self, generation: u64, path: PathBuf, name: String) {
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let mut cmd = std::process::Command::new("git");
      cmd.args(["push"]).current_dir(&path);
      // Route through the command-log chokepoint so `git push` lands in the
      // Command Logs modal (#290). git writes its progress to stderr, so the
      // status line still reads stderr on success.
      let result = crate::command_log::run_logged(&mut cmd, "git push".to_string())
        .map_err(|e| e.to_string())
        .and_then(|out| {
          if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stderr).trim().to_string())
          } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
          }
        });
      let _ = tx.send(TaskMsg::Push(generation, name, result));
    });
  }

  /// Open the rename modal for the selected worktree (`c`, #290). Reuses the
  /// Create form (Type / Issue / Desc) pre-filled by parsing the current
  /// branch name, so renaming is symmetric with creating. A branch that does
  /// not match the `<type>/#<issue>-<desc>` pattern can't be decomposed into
  /// the form, so the modal refuses to open and explains why.
  pub fn enter_edit_worktree(&mut self) {
    let Some((branch, path)) = self
      .selected()
      .and_then(|w| w.branch.clone().map(|b| (b, w.path.clone())))
    else {
      self.status = "no branch to rename (detached HEAD or nothing selected)".into();
      return;
    };
    let Some(spec) = crate::naming::parse_branch(&branch) else {
      self.status = format!(
        "branch '{}' doesn't match <type>/#<issue>-<desc>; can't rename here",
        branch
      );
      return;
    };
    // Refuse rather than silently preselect type index 0: a branch whose
    // parsed type isn't configured (config change, manual branch) would
    // otherwise be renamed to the first configured type on Enter (Codex
    // review on PR #292).
    let Some(type_index) = self.branch_types.iter().position(|t| t.name == spec.type_) else {
      self.status = format!("branch type '{}' is not configured; can't rename here", spec.type_);
      return;
    };
    self.create_form.reset();
    self.create_form.type_index = type_index;
    self.create_form.issue = spec.issue;
    self.create_form.desc = spec.desc;
    self.create_form.field = Field::Desc;
    self.edit_original_branch = Some(branch);
    self.edit_original_path = Some(path);
    self.edit_failure = None;
    self.view = View::Edit;
  }

  /// `true` while the async rename worker is in flight (#290). The run loop
  /// swallows input in `View::Edit` while this holds, mirroring create.
  pub fn is_edit_worktree_loading(&self) -> bool {
    self.tasks.is_loading(TaskKind::EditWorktree)
  }

  /// Cancel the rename modal (`Esc`): drop the captured original branch/path
  /// and return to the list without touching git.
  pub fn cancel_edit_worktree(&mut self) {
    self.edit_original_branch = None;
    self.edit_original_path = None;
    self.edit_failure = None;
    self.create_form.reset();
    self.view = View::List;
  }

  /// Submit the rename from the `View::Edit` modal (#290). Composes the new
  /// branch name + worktree path from the form, then spawns an off-thread
  /// worker that renames the local branch (`git branch -m`), the remote
  /// branch when it exists (`git push origin :<old> <new>:<new>` + re-track),
  /// and moves the worktree directory (`git worktree move`). A no-op rename
  /// (nothing changed) just closes the modal.
  pub fn submit_edit_worktree(&mut self) -> Result<()> {
    let type_ = self
      .branch_types
      .get(self.create_form.type_index)
      .map(|t| t.name.clone())
      .unwrap_or_default();
    let spec = match BranchSpec::new_with_types(
      type_,
      self.create_form.issue.clone(),
      self.create_form.desc.clone(),
      &self.branch_types,
    ) {
      Ok(s) => s,
      Err(e) => {
        self.edit_failure = Some(e.to_string());
        return Ok(());
      }
    };
    let new_branch = spec.branch_name(&self.config.worktree, &self.repo_name)?;
    let new_name = spec.worktree_dirname(&self.config.worktree, &self.repo_name)?;
    let new_path = spec.worktree_path(&self.config.worktree, &self.repo_name, &self.workdir)?;

    let Some(old_branch) = self.edit_original_branch.clone() else {
      self.cancel_edit_worktree();
      return Ok(());
    };
    let Some(old_path) = self.edit_original_path.clone() else {
      self.cancel_edit_worktree();
      return Ok(());
    };

    // Nothing changed — close without shelling out to git.
    if new_branch == old_branch && new_path == old_path {
      self.status = "no change".into();
      self.cancel_edit_worktree();
      return Ok(());
    }

    if self.tasks.has_mutating_task_in_flight() {
      if let Some(label) = self.tasks.mutating_loading_label() {
        self.status = format!("finish {} before renaming", label.trim_end_matches('…'));
      } else {
        self.status = "finish current task before renaming".into();
      }
      return Ok(());
    }
    let Some(generation) = self.tasks.request(TaskKind::EditWorktree) else {
      return Ok(());
    };
    self.edit_failure = None;
    self.spinner.reset();
    self.status = TaskKind::EditWorktree.loading_label().into();
    self.spawn_edit_worktree(
      generation,
      old_branch,
      old_path,
      new_branch,
      new_path,
      new_name,
      self.workdir.clone(),
    );
    Ok(())
  }

  // The rename worker takes each piece of the edit as an owned, `Send`
  // parameter because only owned data may cross the `thread::spawn` boundary
  // (`self` / `git2::Repository` are not `Send`) — the same flat signature the
  // other `spawn_*` workers use. Bundling them into a struct would just add an
  // indirection between the call site and the move-closure for no gain, so the
  // arg count is deliberate.
  #[allow(clippy::too_many_arguments)]
  fn spawn_edit_worktree(
    &self,
    generation: u64,
    old_branch: String,
    old_path: PathBuf,
    new_branch: String,
    new_path: PathBuf,
    new_name: String,
    workdir: PathBuf,
  ) {
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let result = crate::worktree::rename_worktree(&workdir, &old_path, &old_branch, &new_path, &new_branch)
        .map(|remote_renamed| EditWorktreeResult {
          new_branch,
          new_path,
          new_name,
          remote_renamed,
        })
        .map_err(|e| e.to_string());
      let _ = tx.send(TaskMsg::EditWorktree(generation, result));
    });
  }

  /// Open the selected worktree in a new multiplexer pane/tab (`t`, #290).
  /// Detects tmux / zellij at runtime via environment variables; prints a
  /// status message when no supported multiplexer is active.
  pub fn open_in_mux_pane(&mut self) {
    use crate::multiplexer::{build_tmux_command, build_zellij_command, detect_tmux, detect_zellij, SpawnMode};
    let Some(w) = self.selected() else {
      self.status = "no worktree selected".into();
      return;
    };
    let path = w.path.clone();
    let name = w.name.clone();
    // `mux_pane` promises a pane, so split the current pane (tmux
    // `split-window` / zellij `new-pane`) rather than opening a new
    // window/tab (Codex review on PR #292).
    let cmd = if detect_tmux(std::env::var("TMUX").ok()) {
      build_tmux_command(&name, &path, SpawnMode::Split)
    } else if detect_zellij(std::env::var("ZELLIJ").ok()) {
      build_zellij_command(&name, &path, SpawnMode::Split)
    } else {
      self.status = "no multiplexer detected ($TMUX / $ZELLIJ not set)".into();
      return;
    };
    let bin = cmd[0].as_str();
    match std::process::Command::new(bin).args(&cmd[1..]).spawn() {
      Ok(_) => self.status = format!("opened {} in new pane", name),
      Err(e) => self.status = format!("mux-pane failed: {}", e),
    }
  }

  /// Resolve what the `o` key should do for the currently selected
  /// worktree. Returns `None` when nothing is selected (the event loop
  /// surfaces a status message in that case). The exact command is
  /// resolved once here (config override > env var > hardcoded fallback)
  /// so the event loop never has to reason about precedence.
  pub fn resolve_open_target(&self) -> Option<OpenTarget> {
    let path = self.selected()?.path.clone();
    Some(match self.config.tui.open.mode {
      TuiOpenMode::Shell => OpenTarget::Shell {
        path,
        command: resolve_shell_command(&self.config.tui.open),
      },
      TuiOpenMode::Editor => OpenTarget::Editor {
        path,
        command: resolve_editor_command(&self.config.tui.open),
      },
      TuiOpenMode::Finder => OpenTarget::Finder { path },
    })
  }

  pub fn toggle_delete_branch(&mut self) {
    self.delete_branch_on_remove = !self.delete_branch_on_remove;
    self.status = format!("delete branch on remove: {}", self.delete_branch_on_remove);
  }

  // ---- Create flow ---------------------------------------------------------

  pub fn enter_create(&mut self) {
    self.view = View::Create;
    self.create_form.reset();
    self.create_failure = None;
    // Open focused on Issue rather than the cycle-only Type field (#217 UX):
    // the first keypress then edits text instead of being a silent no-op on
    // Type. The type keeps its `reset()` default and stays reachable via
    // Shift-Tab / the field rotation.
    self.create_form.field = Field::Issue;
    self.status = "tab/shift-tab: switch field — enter on desc: submit — esc: cancel".into();
  }

  pub fn create_next_field(&mut self) {
    self.create_form.next_field();
  }

  pub fn create_prev_field(&mut self) {
    self.create_form.prev_field();
  }

  pub fn create_next_type(&mut self) {
    self.create_form.next_type(self.branch_types.len());
  }

  pub fn create_prev_type(&mut self) {
    self.create_form.prev_type(self.branch_types.len());
  }

  pub fn create_push_char(&mut self, c: char) {
    self.create_form.push_char(c);
  }

  pub fn create_pop_char(&mut self) {
    self.create_form.pop_char();
  }

  /// Handle one key in the create overlay and report what the run loop must
  /// do next. Extracted from the inline `View::Create` match (issue #217)
  /// so the input path — typing, type cycling, submit/cancel — is
  /// unit-testable rather than only reachable through a live terminal.
  ///
  /// `h` / `l` mirror the `←` / `→` horizontal type selector, but **only**
  /// when the Type field is focused; on a text field they are literal input
  /// so the letters are never swallowed.
  pub fn handle_create_key(&mut self, key: KeyEvent) -> CreateKey {
    if self.is_create_worktree_loading() {
      return CreateKey::Handled;
    }
    let on_type = self.create_form.field == Field::Type;
    // #219: verbs resolve through the `create` context. The type-cycling
    // verbs (`prev_type` / `next_type`, def arrows + h/l) only fire on the
    // Type field; on a text field their keys fall through to literal input
    // so `h` / `l` are never swallowed while typing a description.
    match self.resolve_modal(KeyContext::Create, key) {
      Some(ModalAction::CreateCancel) => return CreateKey::Cancel,
      Some(ModalAction::CreateNextField) => self.create_next_field(),
      Some(ModalAction::CreatePrevField) => self.create_prev_field(),
      Some(ModalAction::CreateSubmit) => {
        if self.create_form.field == Field::Desc {
          return CreateKey::Submit;
        }
        self.create_next_field();
      }
      Some(ModalAction::CreatePrevType) if on_type => self.create_prev_type(),
      Some(ModalAction::CreateNextType) if on_type => self.create_next_type(),
      _ => match key.code {
        KeyCode::Char(c) if self.create_form.field == Field::Issue && !c.is_ascii_digit() => {
          self.status = "issue accepts digits only".into();
        }
        KeyCode::Char(c) if !on_type => self.create_push_char(c),
        KeyCode::Backspace if !on_type => self.create_pop_char(),
        _ => {}
      },
    }
    CreateKey::Handled
  }

  pub fn submit_create(&mut self) -> Result<()> {
    let type_ = self
      .branch_types
      .get(self.create_form.type_index)
      .map(|t| t.name.clone())
      .unwrap_or_default();
    let spec = BranchSpec::new_with_types(
      type_,
      self.create_form.issue.clone(),
      self.create_form.desc.clone(),
      &self.branch_types,
    )?;
    let branch = spec.branch_name(&self.config.worktree, &self.repo_name)?;
    let dirname = spec.worktree_dirname(&self.config.worktree, &self.repo_name)?;
    let target = spec.worktree_path(&self.config.worktree, &self.repo_name, &self.workdir)?;

    // Gate the bootstrap RCE primitive on the TOFU ledger BEFORE
    // creating the worktree on disk (issue #95). A refusal here
    // leaves the user's disk state untouched — no orphaned
    // worktree to clean up. Mirrors `cmd_create` in src/cli.rs.
    if let Some(msg) = self.check_trust_for_bootstrap()? {
      self.status = msg;
      // Stay in the create form so the user can retry after
      // approving the config via the CLI gate. Returning Ok here
      // (rather than Err) keeps the event loop alive — an Err
      // would print to stderr and tear down the alternate screen.
      return Ok(());
    }

    if self.tasks.has_mutating_task_in_flight() {
      if let Some(label) = self.tasks.mutating_loading_label() {
        self.status = format!("finish {} before creating worktree", label.trim_end_matches('…'));
      } else {
        self.status = "finish current task before creating worktree".into();
      }
      return Ok(());
    }
    let Some(generation) = self.tasks.request(TaskKind::CreateWorktree) else {
      return Ok(());
    };
    self.create_failure = None;
    self.spinner.reset();
    self.status = TaskKind::CreateWorktree.loading_label().into();
    self.spawn_create_worktree(
      generation,
      dirname,
      target,
      branch,
      self.workdir.clone(),
      self.config.clone(),
    );
    Ok(())
  }

  fn spawn_create_worktree(
    &self,
    generation: u64,
    dirname: String,
    target: PathBuf,
    branch: String,
    workdir: PathBuf,
    config: Config,
  ) {
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let result = (|| -> Result<CreateWorktreeResult> {
        let repo = worktree::discover_repo(Some(&workdir))?;
        let created = worktree::add(&repo, &dirname, &target, &branch, false)?;
        let ctx = BootstrapCtx {
          main_repo: &workdir,
          worktree: &created,
          config: &config,
        };
        let report = bootstrap::run(&ctx)?;
        Ok(CreateWorktreeResult {
          branch,
          created,
          report,
        })
      })()
      .map_err(|e| e.to_string());
      let _ = tx.send(TaskMsg::CreateWorktree(generation, result));
    });
  }

  // ---- Delete flow ---------------------------------------------------------

  pub fn enter_confirm_delete(&mut self) {
    let Some(sel) = self.selected() else {
      self.status = "nothing selected".into();
      return;
    };
    if sel.is_main {
      self.status = "cannot remove the main worktree".into();
      return;
    }
    self.view = View::Confirm;
    self.confirm.reset();
    self.delete_failure = None;
    // Start the loader animation from a deterministic frame each time
    // the modal opens (#187).
    self.spinner.reset();
  }

  pub fn confirm_delete(&mut self) -> Result<()> {
    // `worktree::remove` resolves by the internal git id, which can diverge
    // from the display name after a rename (#290), so pass `id` here.
    let (id, label) = match self.selected() {
      Some(s) => (s.id.clone(), s.path.display().to_string()),
      None => return Ok(()),
    };
    if self.is_delete_worktree_loading() {
      return Ok(());
    }
    if self.tasks.has_mutating_task_in_flight() {
      if let Some(label) = self.tasks.mutating_loading_label() {
        self.status = format!("finish {} before deleting worktree", label.trim_end_matches('…'));
      } else {
        self.status = "finish current task before deleting worktree".into();
      }
      return Ok(());
    }
    let Some(generation) = self.tasks.request(TaskKind::DeleteWorktree) else {
      return Ok(());
    };
    let delete_branch = self.delete_branch_on_remove;
    self.delete_failure = None;
    self.confirm.dismiss();
    self.spinner.reset();
    self.status = TaskKind::DeleteWorktree.loading_label().into();
    self.spawn_delete_worktree(generation, id, label, delete_branch);
    Ok(())
  }

  fn spawn_delete_worktree(&self, generation: u64, id: String, label: String, delete_branch: bool) {
    let tx = self.task_tx.clone();
    let workdir = self.workdir.clone();
    std::thread::spawn(move || {
      let result = worktree::discover_repo(Some(&workdir))
        .and_then(|repo| worktree::remove(&repo, &id, delete_branch))
        .map_err(|e| e.to_string());
      let _ = tx.send(TaskMsg::DeleteWorktree(generation, id, label, result));
    });
  }

  // ---- Confirm-overlay safety countdown (issue #30, extracted per #125) ---
  //
  // The countdown only applies when `delete_branch_on_remove` is ON AND the
  // configured `confirm_countdown_secs` is non-zero. The pure state lives
  // on `self.confirm` (see `src/tui/state/confirm.rs`); the wrappers below
  // own the side effects (status messages, view transitions).

  /// Total duration of the safety countdown for the current modal state.
  /// `Duration::ZERO` means "no countdown — classic modal".
  pub fn confirm_countdown_total(&self) -> Duration {
    if self.delete_branch_on_remove {
      Duration::from_secs(u64::from(self.config.tui.effective_confirm_countdown_secs()))
    } else {
      Duration::ZERO
    }
  }

  /// True when the confirm overlay should render the countdown variant
  /// (progress bar + footer "y arm / y again to cancel"). False for the
  /// classic single-keystroke confirm.
  pub fn confirm_is_countdown_mode(&self) -> bool {
    self.confirm_countdown_total() > Duration::ZERO
  }

  /// Handle a `y` / Enter press inside the confirm overlay. Delegates to
  /// `ConfirmModal::press_y` and composes the status-bar message based on
  /// the returned action.
  pub fn confirm_press_y(&mut self, now: Instant) -> ConfirmKeyAction {
    let total = self.confirm_countdown_total();
    let action = self.confirm.press_y(now, total);
    match action {
      ConfirmKeyAction::FireNow => {}
      ConfirmKeyAction::Disarmed => {
        let secs = total.as_secs();
        // #219 review: name the live confirm key, and drop the clause entirely
        // when it is unbound — never advertise a key that no longer re-arms.
        self.status = match self.modal_keymap.primary_key(ModalAction::ConfirmConfirm) {
          Some(c) => format!("countdown cancelled — press {c} to re-arm ({secs}s safety delay)"),
          None => format!("countdown cancelled ({secs}s safety delay)"),
        };
      }
      ConfirmKeyAction::Armed => {
        let secs = total.as_secs();
        // #219 review: name the live confirm / cancel keys (rebindable via
        // `[tui.keys.modal.confirm]`), dropping either clause when its verb is
        // unbound rather than advertising a phantom key while the timer runs.
        let confirm = self.modal_keymap.primary_key(ModalAction::ConfirmConfirm);
        let cancel = self.modal_keymap.primary_key(ModalAction::ConfirmCancel);
        let tail = match (confirm, cancel) {
          (Some(c), Some(x)) => format!(" · press {c} again or {x} to cancel"),
          (Some(c), None) => format!(" · press {c} again to disarm"),
          (None, Some(x)) => format!(" · press {x} to cancel"),
          (None, None) => String::new(),
        };
        self.status = format!("armed — auto-fires in {secs}s{tail}");
      }
    }
    action
  }

  /// Handle the dismissal keys (`n` / `Esc`) inside the confirm overlay.
  /// Always disarms the countdown and returns to the list.
  pub fn confirm_dismiss(&mut self) {
    if self.is_delete_worktree_loading() {
      self.status = TaskKind::DeleteWorktree.loading_label().into();
      return;
    }
    self.confirm.dismiss();
    self.delete_failure = None;
    self.view = View::List;
  }

  /// Tick the countdown forward. Called from the event loop on every
  /// poll-timeout iteration (every 200ms).
  pub fn tick_confirm_countdown(&mut self, now: Instant) -> CountdownTickOutcome {
    self.confirm.tick(now, self.confirm_countdown_total())
  }

  /// Countdown progress in `[0.0, 1.0]`. `0.0` when not armed, `1.0` once
  /// elapsed. Used by the UI to draw the gauge.
  pub fn confirm_countdown_progress(&self, now: Instant) -> f64 {
    self.confirm.progress(now, self.confirm_countdown_total())
  }

  /// Seconds remaining (rounded up to the next whole second) for the UI
  /// label. `0` when not armed or when the countdown has elapsed.
  pub fn confirm_countdown_remaining_secs(&self, now: Instant) -> u64 {
    self.confirm.remaining_secs(now, self.confirm_countdown_total())
  }

  // ---- Fuzzy filter (issue #21) -------------------------------------------

  /// Open the inline filter bar. The existing query is preserved so the user
  /// can refine an already-sticky filter; `Esc` is the way to start fresh.
  /// Disarms any pending `gg` motion so `/g` doesn't half-trigger it.
  ///
  /// Forces focus back onto the list: opening `/` is an intent to narrow the
  /// list, and the post-`Enter` contract is "navigation returns to the
  /// table". Leaving the sidebar focused would make `j` / `k` scroll it
  /// instead of walking the filtered worktrees after the filter sticks.
  pub fn enter_filter(&mut self) {
    self.filter.open();
    self.sidebar.focused = false;
    self.cancel_pending_motion();
    self.status = "/ filter — type to narrow · enter confirms · esc clears".into();
  }

  /// Close the filter bar but keep the query: `Enter` confirms the current
  /// match set and returns the cursor to list navigation.
  pub fn exit_filter_keep(&mut self) {
    self.filter.close_keep();
    self.status = if self.filter.query().is_empty() {
      "press ? for help".into()
    } else {
      format!("filter sticky: {}", self.filter.query())
    };
  }

  /// Close the filter bar and clear the query: `Esc` returns to the full list.
  pub fn exit_filter_cancel(&mut self) {
    let had_query = !self.filter.query().is_empty();
    self.filter.close_cancel();
    self.clamp_selection_to_filter();
    self.invalidate_sidebar_cache();
    self.status = if had_query {
      "filter cleared".into()
    } else {
      "press ? for help".into()
    };
  }

  pub fn filter_push_char(&mut self, c: char) {
    self.filter.push_char(c);
    self.clamp_selection_to_filter();
    self.invalidate_sidebar_cache();
  }

  pub fn filter_pop_char(&mut self) {
    let before = self.filter.query().len();
    self.filter.pop_char();
    if self.filter.query().len() != before {
      self.clamp_selection_to_filter();
      self.invalidate_sidebar_cache();
    }
  }

  /// Indices into `self.worktrees`, in display order:
  /// - empty query: identity (every worktree in source order).
  /// - non-empty: only worktrees whose name matches the query via
  ///   `nucleo_matcher`, ranked by descending score (nucleo intrinsically
  ///   ranks exact/substring/prefix matches above subsequence matches).
  ///
  /// Score ties are broken by original index so output is stable.
  ///
  /// Memoised on `FilterState` since #124 / #104: the per-frame render
  /// path calls this 3–5× (table height, visible rows, title hint,
  /// footer counter, selection resolver), but the result only changes
  /// when the query OR the worktrees vec changes. The cache holds the
  /// previous result and the worktrees length it was computed against;
  /// any buffer mutation (`push_char` / `pop_char` / `set_query` /
  /// `clear`), an explicit `filter.invalidate()`, or a length change
  /// invalidates it. `App::refresh` calls `invalidate` after replacing
  /// `worktrees` so a same-length-different-contents refresh is also
  /// caught.
  pub fn filtered_indices(&mut self) -> &[usize] {
    self.filter.filtered_indices(&self.worktrees, fuzzy_match_indices)
  }

  /// Reposition the selection so it stays inside the current filtered subset.
  /// Called whenever the filter mutates (`/`-mode typing, `Esc`-clear) or the
  /// worktree list itself changes (`refresh`). Also re-resolves the issue/PR
  /// link cache so the right-panel block tracks the new selection — PR #68
  /// Copilot review caught that selection changes were leaving the cache
  /// pointing at the previously selected worktree.
  fn clamp_selection_to_filter(&mut self) {
    let len = self.filtered_indices().len();
    if len == 0 {
      self.list_state.select(None);
      self.refresh_link();
      return;
    }
    match self.list_state.selected() {
      Some(i) if i >= len => self.list_state.select(Some(len - 1)),
      Some(_) => {}
      None => self.list_state.select(Some(0)),
    }
    self.refresh_link();
  }

  /// Move the cursor onto the worktree at `path`, mapping its raw index in
  /// `self.worktrees` to its slot in the *filtered* list — `list_state`
  /// indexes `filtered_indices()`, not the raw vec, so selecting a raw index
  /// under an active filter lands on the wrong visible row or none (Codex
  /// review on PR #292). A no-op when the path is filtered out.
  /// The chord that opens the issue/PR link prompt (`i` by default since
  /// #290), resolved from the live keymap so "press X to link" status hints
  /// track the binding and any `[tui.keys]` override (Codex review on PR
  /// #292, P3).
  fn link_prompt_chord(&self) -> String {
    self
      .keymap
      .primary_chord(Action::LinkPrompt)
      .unwrap_or_else(|| "i".into())
  }

  pub fn reselect_by_path(&mut self, path: &Path) {
    let Some(raw) = self.worktrees.iter().position(|w| w.path == path) else {
      return;
    };
    let pos = self.filtered_indices().iter().position(|&idx| idx == raw);
    if let Some(pos) = pos {
      self.list_state.select(Some(pos));
    }
  }

  // ---- Bootstrap flow ------------------------------------------------------

  // ---- Picker mode (issue #22) --------------------------------------------

  /// Commit the highlighted worktree as the picker's result. The event loop
  /// breaks once `picker_should_exit` flips so `run_picker` can surface the
  /// path to the CLI caller, which prints it on stdout for `cd "$(gwm
  /// switch)"`.
  ///
  /// Outside picker mode the call is inert. When picker mode is on but
  /// nothing is selected (e.g. the filter narrowed the list to zero
  /// matches), the loop stays open and a status hint asks the user to
  /// refine — addresses Copilot's PR #53 review: Enter on an empty match
  /// set used to break with `None`, which read as "cancel" instead of
  /// "nothing to pick".
  pub fn picker_confirm(&mut self) {
    if !self.picker_mode {
      return;
    }
    match self.selected() {
      Some(w) => {
        self.picker_result = Some(w.path.clone());
        self.picker_should_exit = true;
      }
      None => {
        self.status = "no worktree selected — adjust the filter and try again".into();
      }
    }
  }

  /// Esc-equivalent for picker mode: leave without recording a path. The
  /// regular TUI uses Esc to clear an active filter, which conflicts with
  /// the picker footer's `esc:cancel` contract; this method exists so the
  /// event loop can route Esc-during-filter to a clean picker cancel.
  pub fn picker_cancel(&mut self) {
    if !self.picker_mode {
      return;
    }
    self.picker_should_exit = true;
  }

  pub fn bootstrap_selected(&mut self) {
    let path = match self.selected() {
      Some(s) => s.path.clone(),
      None => {
        self.status = "nothing selected".into();
        return;
      }
    };

    // Same TOFU gate as `submit_create` — pressing `b` to re-run
    // bootstrap on an existing worktree is just as much an RCE
    // primitive as creating a new one. Issue #95.
    match self.check_trust_for_bootstrap() {
      Ok(None) => {}
      Ok(Some(msg)) => {
        self.status = msg;
        return;
      }
      Err(e) => {
        self.status = format!("trust gate error: {}", e);
        return;
      }
    }

    // Run off-thread on the async-task spine (issue #256): `bootstrap::run`
    // (file copies, guards, command hooks) used to block the event loop. The
    // TOFU gate above stays synchronous on the main thread; only the run
    // itself moves to a worker, with the `View::Report` transition deferred
    // to `drain_task_results`. A second `b` press while one is in flight
    // coalesces (no `Some(generation)`), so two bootstraps never race.
    if self.tasks.has_mutating_task_in_flight() && !self.tasks.is_loading(TaskKind::Bootstrap) {
      self.status = self.busy_mutation_status("bootstrapping");
      return;
    }
    let Some(generation) = self.tasks.request(TaskKind::Bootstrap) else {
      return;
    };
    self.spinner.reset();
    self.status = TaskKind::Bootstrap.loading_label().into();
    self.spawn_bootstrap(generation, self.workdir.clone(), path, self.config.clone());
  }

  /// Spawn the off-thread bootstrap worker (issue #256). Only owned, `Send`
  /// data crosses the thread boundary — the `main_repo` / `worktree` paths
  /// and a clone of the resolved `Config` — so the worker rebuilds its own
  /// `BootstrapCtx` rather than borrowing `self`. The result is posted back
  /// over the task channel for `drain_task_results` to apply.
  fn spawn_bootstrap(&self, generation: u64, main_repo: PathBuf, worktree: PathBuf, config: Config) {
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let ctx = BootstrapCtx {
        main_repo: &main_repo,
        worktree: &worktree,
        config: &config,
      };
      let result = bootstrap::run(&ctx).map_err(|e| e.to_string());
      let _ = tx.send(TaskMsg::Bootstrap(generation, result));
    });
  }

  // ---- Issue/PR linking (issue #67) -------------------------------------

  /// Re-read the link for the currently selected worktree's branch. Also
  /// re-resolves the repo slug from the origin remote, and resets any
  /// previously cached GitHub fetch state since it would refer to a
  /// different (issue, pr) tuple now. Delegates to
  /// [`GitHubFetch::refresh_link`] for the pure state mutation; the
  /// branch resolution still lives here because it depends on
  /// `App`'s `selected()` + `repo.head()` fallback.
  pub fn refresh_link(&mut self) {
    let branch = self.selected_branch_name();
    self.github.refresh_link(&self.repo, branch.as_deref());
    // Navigation invariant (issue #255): the cache clear above must be paired
    // with a spine generation-bump so any in-flight `gh` worker for the
    // previous worktree's link is dropped instead of stamping the now-active
    // worktree's cache. `refresh_link` no longer holds the old issue/PR
    // numbers, so invalidate by predicate.
    self.tasks.invalidate_matching(TaskKind::is_github);
  }

  fn selected_branch_name(&self) -> Option<String> {
    self.selected().and_then(|w| w.branch.clone()).or_else(|| {
      self
        .repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().ok().map(|s| s.to_string()))
    })
  }

  pub fn current_link(&self) -> &BranchLink {
    &self.github.link
  }

  /// Mirror the live resolved `github.link` onto the selected worktree's
  /// snapshot (issue #283 / Codex review #284). The table renders the PR/
  /// issue pastilles from `self.worktrees[*].link`, captured at list time,
  /// so a freshly persisted auto-detection would otherwise stay invisible on
  /// the selected row until a full relist. Resolves the selection through
  /// the same filter map as [`Self::selected`].
  fn sync_selected_link_into_table(&mut self) {
    let Some(i) = self.list_state.selected() else {
      return;
    };
    let filtered = self.filter.snapshot_indices(&self.worktrees, fuzzy_match_indices);
    let Some(&original) = filtered.get(i) else {
      return;
    };
    let link = self.github.link.clone();
    if let Some(w) = self.worktrees.get_mut(original) {
      if w.link.issue != link.issue {
        w.issue_state = None;
      }
      if w.link.pr != link.pr {
        w.pr_state = None;
      }
      w.link = link;
    }
  }

  fn sync_issue_status_into_table(&mut self, status: &IssueStatus) {
    if self.github.link.issue == Some(status.number) {
      self.github.link.issue_title = Some(status.title.clone());
      self.github.link.issue_state = Some(status.state);
      if let Some(branch) = self.selected_branch_name() {
        let _ = github::persist_issue_title(&self.repo, &branch, &status.title);
        let _ = github::persist_issue_state(&self.repo, &branch, status.state);
      }
    }
    // In workspace mode the fetch was for the active repo's selected issue, so
    // only stamp/persist rows belonging to that repo — a number-only match
    // would otherwise carry repo A's state onto repo B's same-numbered row and
    // persist it through the wrong repo handle (Codex review #303 P2).
    let mask = self.active_repo_row_mask();
    for (i, w) in self.worktrees.iter_mut().enumerate() {
      if mask.as_ref().is_some_and(|m| !m[i]) {
        continue;
      }
      if w.link.issue != Some(status.number) {
        continue;
      }
      w.issue_state = Some(status.state);
      w.link.issue_title = Some(status.title.clone());
      w.link.issue_state = Some(status.state);
      if let Some(branch) = w.branch.as_deref() {
        let _ = github::persist_issue_title(&self.repo, branch, &status.title);
        let _ = github::persist_issue_state(&self.repo, branch, status.state);
      }
    }
  }

  fn sync_pr_status_into_table(&mut self, status: &PrStatus) {
    if self.github.link.pr == Some(status.number) {
      self.github.link.pr_title = Some(status.title.clone());
      self.github.link.pr_state = Some(status.state);
      if let Some(branch) = self.selected_branch_name() {
        let _ = match self.github.link.pr_source {
          github::LinkSource::Detected => github::persist_detected_pr_title(&self.repo, &branch, &status.title)
            .and_then(|()| github::persist_detected_pr_state(&self.repo, &branch, status.state)),
          github::LinkSource::Explicit => github::persist_pr_title(&self.repo, &branch, &status.title)
            .and_then(|()| github::persist_pr_state(&self.repo, &branch, status.state)),
          github::LinkSource::BranchName | github::LinkSource::None => Ok(()),
        };
      }
    }
    // Scope to the active repo's rows in workspace mode — see the matching
    // note in `sync_issue_status_into_table` (Codex review #303 P2).
    let mask = self.active_repo_row_mask();
    for (i, w) in self.worktrees.iter_mut().enumerate() {
      if mask.as_ref().is_some_and(|m| !m[i]) {
        continue;
      }
      if w.link.pr != Some(status.number) {
        continue;
      }
      w.pr_state = Some(status.state);
      w.link.pr_title = Some(status.title.clone());
      w.link.pr_state = Some(status.state);
      if let Some(branch) = w.branch.as_deref() {
        let _ = match w.link.pr_source {
          github::LinkSource::Detected => github::persist_detected_pr_title(&self.repo, branch, &status.title)
            .and_then(|()| github::persist_detected_pr_state(&self.repo, branch, status.state)),
          github::LinkSource::Explicit => github::persist_pr_title(&self.repo, branch, &status.title)
            .and_then(|()| github::persist_pr_state(&self.repo, branch, status.state)),
          github::LinkSource::BranchName | github::LinkSource::None => Ok(()),
        };
      }
    }
  }

  pub fn current_slug(&self) -> Option<&str> {
    self.github.link_slug.as_deref()
  }

  /// Read the cached issue fetch state for the *currently-linked*
  /// issue. Returns `&GitHubFetchState::Idle` when no issue is linked
  /// (or when the linked issue has never been fetched) — the cache is
  /// per-number (post-#138), so reading "the" state means resolving
  /// via `self.github.link.issue` first.
  pub fn issue_fetch_state(&self) -> &GitHubFetchState<IssueStatus> {
    match self.github.link.issue {
      Some(n) => self.github.issue_fetch_state(n),
      None => &GitHubFetchState::Idle,
    }
  }

  /// PR-side counterpart to [`Self::issue_fetch_state`].
  pub fn pr_fetch_state(&self) -> &GitHubFetchState<PrStatus> {
    match self.github.link.pr {
      Some(n) => self.github.pr_fetch_state(n),
      None => &GitHubFetchState::Idle,
    }
  }

  /// Kick off the issue/PR fetch. Called from the event loop when the
  /// user presses `F` (refresh GitHub status). Each `gh issue view` /
  /// `gh pr view` shell-out runs **off-thread** on the shared async-task
  /// spine (issue #255, migrated from #217's dedicated channel): the `App`
  /// checks the per-key cache, claims a generation from
  /// [`TaskRunner::request`], marks the cache `Loading`, and spawns a
  /// worker tagged with that generation. The worker reports a
  /// `TaskMsg::Github{Issue,Pr}` back; [`Self::drain_task_results`] applies
  /// it only if the generation is still authoritative — so a stale worker
  /// from a previous fetch loses the retry race to a fresh one.
  ///
  /// The PR auto-detection (`gh pr list`, issue #181) stays synchronous:
  /// it mutates `link` which the very next render needs, and it is a single
  /// cheap call rather than the two `view` shell-outs the spinner is for.
  ///
  /// This call path is the explicit user-initiated refresh, so it flushes
  /// the cache + drops any in-flight worker via [`Self::invalidate_github`]
  /// first — the user just asked for fresh data, a cache short-circuit here
  /// would be a bug.
  pub fn refresh_github_status(&mut self) {
    let slug = self.github.link_slug.clone();

    // Re-resolve a non-explicit PR live on `F` (issue #181/#283): only an
    // explicit `gwm link --pr` pins the PR; a branch-name / none / persisted-
    // detected (#283) PR is re-probed so a number that changed since the last
    // detection is refreshed. The in-memory detection is dropped *only* once
    // we have a fresh successful result (the `Ok` arm), so a refresh that
    // cannot probe — no origin slug, no resolvable branch, or a failed `gh`
    // call — keeps the persisted detection visible instead of blanking the
    // pane/table (Codex review #284). `apply_detected_pr` only fills an empty
    // slot, hence the clear-then-apply to replace a stale detection.
    if self.github.link.pr_source != github::LinkSource::Explicit {
      if let (Some(slug), Some(branch)) = (slug.as_deref(), self.selected_branch_name()) {
        if let Ok(detected) = github::find_pr_for_branch(slug, &branch) {
          self.github.clear_detected_pr();
          self.github.apply_detected_pr(detected);
          // Persist the detection (issue #283) so the no-fetch table read
          // path colours the PR pastille on every row, not just the selected
          // one. Only a successful probe is authoritative: store a hit, clear
          // the key on a proven `Ok(None)`. Best-effort write — a git-config
          // failure must not break the refresh, so the result is discarded.
          let _ = match detected {
            Some(n) => github::persist_detected_pr(&self.repo, &branch, n),
            None => github::clear_persisted_detected_pr(&self.repo, &branch),
          };
        }
        // On a `gh` failure (Err) nothing was cleared, so the link keeps
        // whatever `read_link` resolved (possibly a persisted detection).
        //
        // Mirror the resolved link onto the selected row's snapshot so the
        // table pastille reflects the detection immediately, without waiting
        // for a separate relist (Codex review #284). The table renders from
        // `self.worktrees[*].link`, not the live `github.link`.
        self.sync_selected_link_into_table();
      }
    }

    // The re-probe can also CHANGE the PR identity — a persisted detection
    // coming back None, or re-detecting a different number (#61 → #62).
    // The open CI checks overlay then shows checks for a PR the link no
    // longer carries: with no fetch to land nothing would ever close it,
    // and on a mere change the stale rows stay up through the new fetch
    // (forever if it fails), `Enter` opening an old PR's check URL (Codex
    // review #455, twice). The overlay is pinned to the PR that opened it
    // (`detail_overlay_pr`); a disagreeing link closes it up front, and
    // the flow below owns the status line ("nothing linked" / "fetching…").
    if self.view == View::DetailOverlay
      && self.detail_overlay.kind == crate::tui::state::detail_overlay::DetailKind::CiChecks
      && self.github.link.pr != self.detail_overlay_pr
    {
      self.close_detail_overlay();
    }

    if self.github.link.issue.is_none() && self.github.link.pr.is_none() {
      self.status = format!(
        "nothing linked — press {} to link an issue or PR",
        self.link_prompt_chord()
      );
      return;
    }
    let Some(slug) = slug else {
      self.status = "no GitHub remote — cannot fetch status".into();
      return;
    };
    // Explicit user-initiated refresh: flush the cache (so the cold-cache
    // branch fires instead of a hit) and drop any in-flight worker on the
    // spine, so previously-loaded keys re-fetch.
    self.invalidate_github();
    let mut spawned = 0u32;
    if let Some(n) = self.github.link.issue {
      if self.spawn_github_issue(n, &slug) {
        spawned += 1;
      }
    }
    if let Some(n) = self.github.link.pr {
      if self.spawn_github_pr(n, &slug) {
        spawned += 1;
      }
    }
    if spawned > 0 {
      // Loading state is live; the spinner animates until `drain` applies
      // the results and re-reports the outcome.
      self.spinner.reset();
      self.status = "fetching GitHub status…".into();
    } else {
      // Nothing actually spawned (all keys already terminal in cache) —
      // report the current outcome immediately.
      self.report_github_refresh_status();
    }
  }

  fn refresh_linked_github_statuses_for_worktrees(&mut self) -> u32 {
    // Workspace mode (#36): this bulk prefetch resolves every merged row's
    // issue/PR against a single repo's slug (`self.github.link_slug`), which
    // mis-attributes numbers across child repos with different remotes (Codex
    // review #303 P2). In workspace mode GitHub state is fetched per-selection
    // instead — `sync_active_repo`/`on_navigation` call `refresh_link`, which
    // re-resolves the slug from the selected row's own repo. So skip the bulk
    // cross-repo prefetch here.
    if self.is_workspace() {
      return 0;
    }
    let Some(slug) = self.github.link_slug.clone() else {
      return 0;
    };
    let issues = self
      .worktrees
      .iter()
      .filter_map(|w| w.link.issue)
      .collect::<BTreeSet<_>>()
      .into_iter()
      .collect::<Vec<_>>();
    let prs = self
      .worktrees
      .iter()
      .filter_map(|w| w.link.pr)
      .collect::<BTreeSet<_>>()
      .into_iter()
      .collect::<Vec<_>>();
    if issues.is_empty() && prs.is_empty() {
      return 0;
    }

    self.invalidate_github();
    let mut spawned = 0u32;
    for n in issues {
      if self.spawn_github_issue(n, &slug) {
        spawned += 1;
      }
    }
    for n in prs {
      if self.spawn_github_pr(n, &slug) {
        spawned += 1;
      }
    }
    if spawned > 0 {
      self.spinner.reset();
    }
    spawned
  }

  /// Flush the GitHub result cache **and** drop any in-flight GitHub worker
  /// on the spine (issue #255). The navigation invariant: the cache clear
  /// and the spine generation-bump must always move together, or a stale
  /// worker's late result could outlive the cache flush. Routed through one
  /// helper so the pairing can't desync — `refresh_github_status` and
  /// (via the predicate) `refresh_link` are the only callers.
  fn invalidate_github(&mut self) {
    self.github.invalidate();
    self.tasks.invalidate_matching(TaskKind::is_github);
  }

  /// Claim a spine generation for `Issue(n)` and spawn its `gh issue view`
  /// worker (issue #255), returning `true` when a worker was actually
  /// started. A terminal cache hit (the explicit refresh flushed the cache
  /// first, so this only fires on a redundant call) or a coalesced spine
  /// slot (a worker for this key is already in flight) returns `false`
  /// without spawning a second subprocess.
  fn spawn_github_issue(&mut self, n: u64, slug: &str) -> bool {
    let key = FetchKey::Issue(n);
    if self.github.is_cached(key) {
      return false;
    }
    let Some(generation) = self.tasks.request(TaskKind::GithubIssue(n)) else {
      return false;
    };
    self.github.mark_loading(key);
    self.spawn_github_fetch(key, slug.to_string(), generation);
    true
  }

  /// PR-side counterpart to [`Self::spawn_github_issue`] (issue #255).
  fn spawn_github_pr(&mut self, n: u64, slug: &str) -> bool {
    let key = FetchKey::Pr(n);
    if self.github.is_cached(key) {
      return false;
    }
    let Some(generation) = self.tasks.request(TaskKind::GithubPr(n)) else {
      return false;
    };
    self.github.mark_loading(key);
    self.spawn_github_fetch(key, slug.to_string(), generation);
    true
  }

  /// Spawn one background `gh` shell-out for `key` tagged with `generation`
  /// and wire its result back over the shared task channel (issue #255,
  /// migrated from #217's dedicated channel). Deliberately a thin shell: it
  /// owns only the off-thread dispatch + send, no state logic — the
  /// coalescing / late-drop contract lives on the [`TaskRunner`] spine. A
  /// `send` failure (the `App`/receiver was dropped) is ignored: there is
  /// no longer anyone to apply the result.
  fn spawn_github_fetch(&self, key: FetchKey, slug: String, generation: u64) {
    let tx = self.task_tx.clone();
    // Resolve the `gh` program on THIS (main) thread and hand it to the
    // worker, so the worker never reads `GWM_GH` / the process environment
    // concurrently with env-mutating code elsewhere (the `env_lock`
    // unsoundness the worker would otherwise reintroduce — issue #217).
    let program = github::gh_program();
    std::thread::spawn(move || {
      let msg = match key {
        FetchKey::Issue(n) => TaskMsg::GithubIssue(
          generation,
          n,
          github::fetch_issue_with(&program, &slug, n).map_err(|e| e.to_string()),
        ),
        FetchKey::Pr(n) => TaskMsg::GithubPr(
          generation,
          n,
          github::fetch_pr_with(&program, &slug, n).map_err(|e| e.to_string()),
        ),
      };
      let _ = tx.send(msg);
    });
  }

  /// Compute the post-refresh status line message based on the actual
  /// outcome of the issue / PR fetches. PR #68 Copilot review caught
  /// that always printing "refreshed" misled users when one of the
  /// fetches had failed.
  pub fn report_github_refresh_status(&mut self) {
    let issue_err = matches!(self.issue_fetch_state(), GitHubFetchState::Error(_));
    let pr_err = matches!(self.pr_fetch_state(), GitHubFetchState::Error(_));
    self.status = match (issue_err, pr_err) {
      (false, false) => "github status refreshed".into(),
      (true, false) => format!(
        "issue fetch failed: {}",
        self.issue_error_message().unwrap_or("?".into())
      ),
      (false, true) => format!("pr fetch failed: {}", self.pr_error_message().unwrap_or("?".into())),
      (true, true) => format!(
        "issue + pr fetch failed — issue: {} · pr: {}",
        self.issue_error_message().unwrap_or("?".into()),
        self.pr_error_message().unwrap_or("?".into())
      ),
    };
  }

  fn issue_error_message(&self) -> Option<String> {
    match self.issue_fetch_state() {
      GitHubFetchState::Error(e) => Some(e.clone()),
      _ => None,
    }
  }

  fn pr_error_message(&self) -> Option<String> {
    match self.pr_fetch_state() {
      GitHubFetchState::Error(e) => Some(e.clone()),
      _ => None,
    }
  }

  pub fn apply_issue_fetch_result(&mut self, r: std::result::Result<IssueStatus, String>) {
    if let Ok(status) = &r {
      self.persist_loaded_issue_title(status);
    }
    self.github.apply_issue_result(r);
  }

  pub fn apply_pr_fetch_result(&mut self, r: std::result::Result<PrStatus, String>) {
    if let Ok(status) = &r {
      self.persist_loaded_pr_title(status);
      self.refresh_ci_overlay_on_pr_landing(status);
    }
    self.github.apply_pr_result(r);
  }

  /// Rebuild the open CI checks overlay from a landed PR fetch (validation
  /// feedback on PR #455, `f` = refresh inside the overlay) — same
  /// convention as the agents landing. Gated on the CI consumer AND on the
  /// linked PR: the worktree-wide bulk prefetch lands other PRs' results
  /// through the same drain arm, and those must not clobber the rows.
  /// `set_rows` clamps the selection to the new count. Called from both
  /// landing paths — the drain (`TaskMsg::GithubPr`, the real worker path)
  /// and the `apply_pr_fetch_result` test seam — so they cannot desync
  /// again (the first cut lived only in the seam, so the running TUI never
  /// refreshed the overlay).
  ///
  /// Returns `true` when the landing closed the overlay and claimed the
  /// status line (empty rollup) so the drain suppresses its end-of-drain
  /// `report_github_refresh_status` — which otherwise overwrote the close
  /// message with "github status refreshed" (Codex review #455); same
  /// guard the sync arm uses.
  fn refresh_ci_overlay_on_pr_landing(&mut self, status: &PrStatus) -> bool {
    if self.view != View::DetailOverlay
      || self.detail_overlay.kind != crate::tui::state::detail_overlay::DetailKind::CiChecks
      || self.github.link.pr != Some(status.number)
    {
      return false;
    }
    // An empty rollup (a fresh commit whose workflows have not started
    // yet) would blank the rows while leaving the overlay open — exactly
    // the empty overlay `enter_ci_checks` refuses to open (Codex review
    // #455). Close it and say why instead.
    if status.checks.is_empty() {
      self.close_detail_overlay();
      self.status = "no CI checks reported by the refreshed PR".into();
      return true;
    }
    let rows = crate::tui::state::detail_overlay::ci_check_rows(&status.checks, std::time::SystemTime::now());
    self.detail_overlay.set_rows(rows);
    false
  }

  fn persist_loaded_issue_title(&mut self, status: &IssueStatus) {
    self.sync_issue_status_into_table(status);
  }

  fn persist_loaded_pr_title(&mut self, status: &PrStatus) {
    self.sync_pr_status_into_table(status);
  }

  // ---- Open menu ----------------------------------------------------------

  pub fn enter_open_menu(&mut self) {
    // Re-resolve link + slug in case the user just linked something
    // (`gwm link …` from a parallel terminal) or moved the origin remote.
    self.refresh_link();
    self.open_menu_selected = LinkTarget::Issue;
    self.view = View::OpenMenu;
  }

  pub fn exit_open_menu(&mut self) {
    self.view = View::List;
  }

  pub fn open_menu_toggle_selection(&mut self) {
    self.open_menu_selected = match self.open_menu_selected {
      LinkTarget::Issue => LinkTarget::Pr,
      LinkTarget::Pr => LinkTarget::Issue,
    };
  }

  /// Pick a target from the open menu. Returns the URL to open, or `None`
  /// when the link is missing (the status bar carries the explanation).
  pub fn open_menu_pick(&mut self, target: LinkTarget) -> Option<String> {
    self.view = View::List;
    let Some(slug) = self.github.link_slug.clone() else {
      self.status = "no GitHub remote — cannot build URL".into();
      return None;
    };
    let url = match target {
      LinkTarget::Issue => match self.github.link.issue {
        Some(n) => github::issue_url(&slug, n),
        None => {
          self.status = format!("no issue linked — press {} to link one", self.link_prompt_chord());
          return None;
        }
      },
      LinkTarget::Pr => match self.github.link.pr {
        Some(n) => github::pr_url(&slug, n),
        None => {
          self.status = format!("no PR linked — press {} to link one", self.link_prompt_chord());
          return None;
        }
      },
    };
    Some(url)
  }

  // ---- Link prompt --------------------------------------------------------
  //
  // Pure state lives in `self.link_prompt` (`tui::state::link_prompt`,
  // extracted per #126). The methods below are thin orchestrator
  // wrappers: they update `self.view` / `self.status` / drive the
  // `github::link_{issue,pr}` shell-out on submit, then delegate the
  // buffer / stage transitions to `LinkPrompt`.

  pub fn enter_link_prompt(&mut self) {
    self.view = View::LinkPrompt;
    self.link_prompt.reset();
    self.status = "pick".into();
  }

  /// Highlighted row in the `ChooseTarget` picker (for the renderer).
  pub fn link_prompt_selected(&self) -> LinkTarget {
    self.link_prompt.selected
  }

  /// Testable key handler for the link prompt (issue #217), mirroring
  /// [`App::handle_create_key`]. The picker / digit-buffer mutations and
  /// the per-stage status copy stay here; the loop only acts on the
  /// returned [`LinkPromptKey`] for the two genuine side effects
  /// (submit shell-out, view transition).
  pub fn handle_link_prompt_key(&mut self, key: KeyEvent) -> LinkPromptKey {
    use crate::tui::state::link_prompt::LinkPromptStage;
    // #219: each stage is its own modal context. ChooseTarget is a vertical
    // two-row picker — `next` / `prev` both flip the highlight (a single
    // flip serves j/k/Up/Down alike), while `issue` / `pr` are direct picks.
    // InputNumber routes `submit` / `cancel` through the context and treats
    // everything else as digit input. The global `fetch_github` key is a
    // FALLBACK after the stage context, so a contextual binding on that key
    // (e.g. `submit = ["F"]`) wins over the fetch shortcut (#293 review).
    match self.link_prompt.stage {
      LinkPromptStage::ChooseTarget => match self.resolve_modal(KeyContext::LinkChooseTarget, key) {
        Some(ModalAction::LinkChooseCancel) => return LinkPromptKey::Cancel,
        Some(ModalAction::LinkChooseNext) | Some(ModalAction::LinkChoosePrev) => self.link_prompt.toggle_selection(),
        Some(ModalAction::LinkChooseIssue) => self.link_prompt_choose(LinkTarget::Issue),
        Some(ModalAction::LinkChoosePr) => self.link_prompt_choose(LinkTarget::Pr),
        Some(ModalAction::LinkChooseAccept) => {
          let target = self.link_prompt.selected;
          self.link_prompt_choose(target);
        }
        _ if self.key_matches_action(key, Action::FetchGithub) => return LinkPromptKey::Refresh,
        _ => {}
      },
      LinkPromptStage::InputNumber => match self.resolve_modal(KeyContext::LinkInputNumber, key) {
        Some(ModalAction::LinkInputCancel) => return LinkPromptKey::Cancel,
        Some(ModalAction::LinkInputSubmit) => return LinkPromptKey::Submit,
        _ if self.key_matches_action(key, Action::FetchGithub) => return LinkPromptKey::Refresh,
        _ => match key.code {
          KeyCode::Char(c) => self.link_prompt_push_char(c),
          KeyCode::Backspace => self.link_prompt_pop_char(),
          _ => {}
        },
      },
    }
    LinkPromptKey::Handled
  }

  pub fn link_prompt_cancel(&mut self) {
    self.view = View::List;
    self.link_prompt.reset();
  }

  pub fn link_prompt_stage(&self) -> LinkPromptStage {
    self.link_prompt.stage
  }

  pub fn link_prompt_number_input(&self) -> &str {
    &self.link_prompt.number
  }

  pub fn link_prompt_target(&self) -> Option<LinkTarget> {
    self.link_prompt.target
  }

  pub fn link_prompt_choose(&mut self, target: LinkTarget) {
    self.link_prompt.commit_target(target);
    self.status = match target {
      LinkTarget::Issue | LinkTarget::Pr => "num".into(),
    };
  }

  pub fn link_prompt_push_char(&mut self, c: char) {
    self.link_prompt.push_char(c);
  }

  pub fn link_prompt_pop_char(&mut self) {
    self.link_prompt.pop_char();
  }

  pub fn link_prompt_submit(&mut self) -> Result<()> {
    let Some(target) = self.link_prompt.target else {
      self.status = "no target chosen".into();
      return Ok(());
    };
    let n: u64 = self
      .link_prompt
      .number
      .parse()
      .map_err(|_| GwmError::Other("number is empty or invalid".into()))?;
    let branch = self
      .selected()
      .and_then(|w| w.branch.clone())
      .or_else(|| {
        self
          .repo
          .head()
          .ok()
          .and_then(|h| h.shorthand().ok().map(|s| s.to_string()))
      })
      .ok_or_else(|| GwmError::Other("no branch resolved for selected worktree".into()))?;
    match target {
      LinkTarget::Issue => github::link_issue(&self.repo, &branch, n)?,
      LinkTarget::Pr => github::link_pr(&self.repo, &branch, n)?,
    }
    self.status = match target {
      LinkTarget::Issue => format!("linked issue #{} to {}", n, branch),
      LinkTarget::Pr => format!("linked PR #{} to {}", n, branch),
    };
    self.view = View::List;
    self.link_prompt.reset();
    self.refresh_link();
    Ok(())
  }
}

/// Resolve the shell command for `mode = "shell"`. Precedence:
/// `shell_cmd` in `.gwm.toml` → `$SHELL` env var → `/bin/sh`. The
/// hardcoded fallback exists for the (rare) case where neither is set —
/// the TUI's spawn-and-restore loop assumes a non-empty command string.
fn resolve_shell_command(cfg: &TuiOpenConfig) -> String {
  cfg
    .shell_cmd
    .clone()
    .or_else(|| std::env::var("SHELL").ok())
    .unwrap_or_else(|| "/bin/sh".into())
}

/// Resolve the editor command for `mode = "editor"`. Precedence:
/// `editor_cmd` in `.gwm.toml` → `$EDITOR` env var → `vi` (POSIX
/// baseline). Mirrors `resolve_shell_command` so the two flows share
/// the same precedence story.
fn resolve_editor_command(cfg: &TuiOpenConfig) -> String {
  cfg
    .editor_cmd
    .clone()
    .or_else(|| std::env::var("EDITOR").ok())
    .unwrap_or_else(|| "vi".into())
}
