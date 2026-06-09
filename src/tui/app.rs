use super::keymap::{Action, ChordResolution, KeyStroke, Keymap};
use super::palette::PaletteState;
use super::state::async_task::{CreateWorktreeResult, TaskKind, TaskMsg, TaskRunner};
use super::state::command_logs::CommandLogs;
use super::state::config_panel::{ConfigPanel, FieldKind, SettingField, SettingsLayer};
use super::state::confirm::{ConfirmKeyAction, ConfirmModal, CountdownTickOutcome};
use super::state::create_form::{CreateForm, Field};
use super::state::filter::{fuzzy_match_indices, FilterState};
use super::state::github_fetch::{FetchKey, GitHubFetch};
use super::state::link_prompt::LinkPrompt;
use super::state::sidebar::SidebarState;
use super::state::spinner::Spinner;
use super::theme::Theme;
use crate::bootstrap::{self, BootstrapCtx, BootstrapReport, StepStatus};
use crate::config::BranchType;
use crate::config::{Config, TuiOpenConfig, TuiOpenMode};
use crate::error::{GwmError, Result};
use crate::github::{self, BranchLink, IssueStatus, PrStatus};
use crate::launcher::{self, ExpandedCommand, LauncherContext};
use crate::naming::BranchSpec;
use crate::worktree::{self, WorktreeInfo};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use git2::Repository;
use ratatui::widgets::TableState;
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

pub struct App {
  pub repo: Repository,
  pub repo_name: String,
  pub workdir: PathBuf,
  pub config: Config,
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
      pending_g: false,
      pending_chord: Vec::new(),
      keymap,
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
      task_tx,
      task_rx,
      command_logs: CommandLogs::new(),
      config_panel: ConfigPanel::new(),
      global_path: global_path.map(Path::to_path_buf),
    };
    // Seed the sidebar position from `[tui] sidebar_position` (issue
    // #188). Orientation stays at its `Auto` default — runtime-only.
    out.sidebar.position = out.config.tui.sidebar_position;
    out.refresh_link();
    Ok(out)
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
    let worktrees = worktree::list(&self.repo)?;
    self.apply_refreshed_worktrees(worktrees);
    Ok(())
  }

  /// Swap in a freshly-listed worktree vec and run the bookkeeping every
  /// refresh path shares: drop the cached fuzzy-match indices (they point
  /// at the previous vec — a length change auto-invalidates, but a
  /// same-length list with different contents would not, so the explicit
  /// flush is the safe play), re-clamp the selection (which re-resolves
  /// the link cache), invalidate the sidebar preview, and report the
  /// count. Called by the synchronous [`Self::refresh`] and by the
  /// off-thread drain in [`Self::drain_task_results`].
  fn apply_refreshed_worktrees(&mut self, worktrees: Vec<WorktreeInfo>) {
    self.worktrees = worktrees;
    self.filter.invalidate();
    self.clamp_selection_to_filter();
    self.invalidate_sidebar_cache();
    self.status = format!("refreshed — {} worktree(s)", self.worktrees.len());
  }

  /// Off-thread worktree list refresh for the `f` / `r` key (issue #231):
  /// spawn a worker that re-lists the worktrees and posts the result back
  /// to the event loop, so a large repo / slow filesystem no longer
  /// freezes the TUI. Coalesces onto an in-flight run (a second press
  /// while loading is a no-op) and seeds the loader label + spinner. The
  /// result is applied by [`Self::drain_task_results`].
  pub fn request_refresh(&mut self) {
    let Some(generation) = self.tasks.request(TaskKind::RefreshWorktrees) else {
      // A refresh is already in flight — coalesce onto it.
      return;
    };
    // Start the loader from a deterministic frame and surface the label.
    self.spinner.reset();
    self.status = TaskKind::RefreshWorktrees.loading_label().into();
    self.spawn_refresh(generation);
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
        TaskMsg::GithubIssue(generation, number, result) => {
          // Generation guard: a stale worker whose slot was bumped by an
          // intervening invalidate/re-request is dropped here, before it can
          // stamp the cache (the Codex-flagged race, fixed by the spine).
          if !self.tasks.complete(TaskKind::GithubIssue(number), generation) {
            continue;
          }
          self.github.complete_issue(number, result);
          applied = true;
          github_applied = true;
        }
        TaskMsg::GithubPr(generation, number, result) => {
          if !self.tasks.complete(TaskKind::GithubPr(number), generation) {
            continue;
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
      View::LinkPrompt => HintContext::LinkPrompt,
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
    self.config_panel.reset();
    self.view = View::Config;
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
      Ok(cfg) => self.config = cfg,
      Err(e) => {
        self.status = format!("settings saved, but reload failed: {}", e);
        return;
      }
    }
    match self.config.theme.resolve() {
      Ok(theme) => self.theme = theme,
      Err(e) => self.status = format!("theme: {}", e),
    }
    self.sidebar.position = self.config.tui.sidebar_position;
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

  /// Return the path that the `y: yank` key should push into the system
  /// clipboard, or `None` when nothing is selected. Pure — the actual
  /// shell-out (`pbcopy` / `wl-copy` / `xclip` / `clip`) is handled by
  /// the event loop so this method stays trivially testable.
  pub fn yank_selected_path(&self) -> Option<PathBuf> {
    self.selected().map(|w| w.path.clone())
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
    match key.code {
      KeyCode::Esc => return CreateKey::Cancel,
      KeyCode::Tab => self.create_next_field(),
      KeyCode::BackTab => self.create_prev_field(),
      KeyCode::Enter => {
        if self.create_form.field == Field::Desc {
          return CreateKey::Submit;
        }
        self.create_next_field();
      }
      KeyCode::Up | KeyCode::Left if on_type => self.create_prev_type(),
      KeyCode::Down | KeyCode::Right if on_type => self.create_next_type(),
      KeyCode::Char('h') if on_type => self.create_prev_type(),
      KeyCode::Char('l') if on_type => self.create_next_type(),
      KeyCode::Char(c) if self.create_form.field == Field::Issue && !c.is_ascii_digit() => {
        self.status = "issue accepts digits only".into();
      }
      KeyCode::Char(c) if !on_type => self.create_push_char(c),
      KeyCode::Backspace if !on_type => self.create_pop_char(),
      _ => {}
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
    let (name, label) = match self.selected() {
      Some(s) => (s.name.clone(), s.path.display().to_string()),
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
    self.spawn_delete_worktree(generation, name, label, delete_branch);
    Ok(())
  }

  fn spawn_delete_worktree(&self, generation: u64, name: String, label: String, delete_branch: bool) {
    let tx = self.task_tx.clone();
    let workdir = self.workdir.clone();
    std::thread::spawn(move || {
      let result = worktree::discover_repo(Some(&workdir))
        .and_then(|repo| worktree::remove(&repo, &name, delete_branch))
        .map_err(|e| e.to_string());
      let _ = tx.send(TaskMsg::DeleteWorktree(generation, name, label, result));
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
        self.status = format!("countdown cancelled — press y to re-arm ({secs}s safety delay)");
      }
      ConfirmKeyAction::Armed => {
        let secs = total.as_secs();
        self.status = format!("armed — auto-fires in {secs}s · press y again or Esc to cancel");
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

    // Drop a prior auto-detection so this refresh re-resolves it live
    // (issue #181): a detected PR must not stick across `F` presses if
    // the branch's PR changed. Explicit / branch-name links stay pinned.
    self.github.clear_detected_pr();

    // Auto-detect the selected branch's PR when none is linked (issue
    // #181). Synchronous (see method doc); needs a remote, so it's a no-op
    // without a slug. An explicit `gwm link --pr` wins — `apply_detected_pr`
    // only fills an empty slot.
    if self.github.link.pr.is_none() {
      if let (Some(slug), Some(branch)) = (slug.as_deref(), self.selected_branch_name()) {
        let detected = github::find_pr_for_branch(slug, &branch).ok().flatten();
        self.github.apply_detected_pr(detected);
        // Persist the detection (issue #283) so the no-fetch table read
        // path colours the PR pastille on every row, not just the selected
        // one. A vanished detection clears the stored key so it can't go
        // stale. Best-effort: a git-config write failure must not break the
        // refresh, so the result is intentionally discarded.
        let _ = match detected {
          Some(n) => github::persist_detected_pr(&self.repo, &branch, n),
          None => github::clear_persisted_detected_pr(&self.repo, &branch),
        };
      }
    }

    if self.github.link.issue.is_none() && self.github.link.pr.is_none() {
      self.status = "nothing linked — press L to link an issue or PR".into();
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
    self.github.apply_issue_result(r);
  }

  pub fn apply_pr_fetch_result(&mut self, r: std::result::Result<PrStatus, String>) {
    self.github.apply_pr_result(r);
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
          self.status = "no issue linked — press L to link one".into();
          return None;
        }
      },
      LinkTarget::Pr => match self.github.link.pr {
        Some(n) => github::pr_url(&slug, n),
        None => {
          self.status = "no PR linked — press L to link one".into();
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
    if self.key_matches_action(key, Action::FetchGithub) {
      return LinkPromptKey::Refresh;
    }
    match (self.link_prompt.stage, key.code) {
      (_, KeyCode::Esc) => return LinkPromptKey::Cancel,
      // ChooseTarget: a vertical selectable list. j/k (and arrows) move the
      // highlight, Enter links the highlighted row, i/p stay direct picks.
      // With exactly two targets, up and down land on the same other row, so
      // a single flip serves j/k/Up/Down alike.
      (LinkPromptStage::ChooseTarget, KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Down | KeyCode::Up) => {
        self.link_prompt.toggle_selection()
      }
      (LinkPromptStage::ChooseTarget, KeyCode::Char('i')) => self.link_prompt_choose(LinkTarget::Issue),
      (LinkPromptStage::ChooseTarget, KeyCode::Char('p')) => self.link_prompt_choose(LinkTarget::Pr),
      (LinkPromptStage::ChooseTarget, KeyCode::Enter) => {
        let target = self.link_prompt.selected;
        self.link_prompt_choose(target);
      }
      // InputNumber: type the digits, Enter submits, Backspace deletes.
      (LinkPromptStage::InputNumber, KeyCode::Enter) => return LinkPromptKey::Submit,
      (LinkPromptStage::InputNumber, KeyCode::Char(c)) => self.link_prompt_push_char(c),
      (LinkPromptStage::InputNumber, KeyCode::Backspace) => self.link_prompt_pop_char(),
      _ => {}
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
