use super::state::confirm::{ConfirmKeyAction, ConfirmModal, CountdownTickOutcome};
use super::state::create_form::CreateForm;
use super::state::filter::{fuzzy_match_indices, FilterState};
use super::state::link_prompt::LinkPrompt;
use super::state::sidebar::SidebarState;
use crate::bootstrap::{self, BootstrapCtx, BootstrapReport, StepStatus};
use crate::config::BranchType;
use crate::config::{Config, TuiOpenConfig, TuiOpenMode};
use crate::error::{GwmError, Result};
use crate::github::{self, BranchLink, IssueStatus, PrStatus};
use crate::launcher::{self, ExpandedCommand, LauncherContext};
use crate::naming::BranchSpec;
use crate::worktree::{self, WorktreeInfo};
use git2::Repository;
use ratatui::widgets::TableState;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
}

/// Target of an open / link action.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LinkTarget {
  Issue,
  Pr,
}

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

/// State of a background GitHub fetch (issue or PR).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubFetchState<T> {
  Idle,
  Loading,
  Loaded(T),
  Error(String),
}

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

  // Create form state
  /// Create-worktree overlay state (extracted per #123). Holds field
  /// focus, type index, and the issue/slug input buffers.
  pub create_form: CreateForm,
  /// Branch types displayed in the create-form picker. Resolved once at
  /// startup from [`Config::resolved_branch_types`] so the picker
  /// honours any `[[branch_types]]` override in `.gwm.toml` without
  /// re-reading the file on every key event.
  pub branch_types: Vec<BranchType>,

  // Bootstrap report
  pub report: Option<BootstrapReport>,

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
  pub pending_g: bool,

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

  /// Safety countdown state for the confirm overlay (issue #30, extracted
  /// per #125). Holds the timer anchor and exposes the pure state-machine
  /// API; this `App` keeps the side-effecting wrappers below that compose
  /// the status messages and call `worktree::remove`.
  pub confirm: ConfirmModal,

  // ---- Issue/PR linking (issue #67) -------------------------------------
  /// Cached link for the currently selected worktree's branch.
  link: BranchLink,
  /// Repo slug parsed from `origin` (None when no GitHub remote).
  link_slug: Option<String>,
  issue_state: GitHubFetchState<IssueStatus>,
  pr_state: GitHubFetchState<PrStatus>,
  /// Two-stage issue/PR link prompt state (extracted per #126). Owns
  /// the stage + target + digit buffer; the orchestrator wraps the
  /// transitions to update the status bar and shell out to
  /// `github::link_{issue,pr}` on submit.
  link_prompt: LinkPrompt,

  /// TOFU trust mode for this TUI session (issue #95). Resolved at
  /// the CLI entrypoint from `--allow-bootstrap` / `--deny-bootstrap`
  /// / `GWM_ALLOW_BOOTSTRAP=1` and threaded down via `tui::run(mode)`.
  /// Used by `check_trust_for_bootstrap` to gate `submit_create` and
  /// `bootstrap_selected` — same security policy as the CLI, no
  /// bypass via the TUI. Default `Prompt` (preserves the safe
  /// default when callers construct `App` directly, e.g. tests that
  /// don't care about the gate).
  pub trust_mode: crate::trust::TrustMode,
}

impl App {
  pub fn new() -> Result<Self> {
    Self::new_at(None)
  }

  pub fn new_at(start: Option<&Path>) -> Result<Self> {
    let repo = worktree::discover_repo(start)?;
    let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
    let repo_name = worktree::repo_name(&repo);
    let config = Config::load_for_repo(&workdir)?;
    let branch_types = config.resolved_branch_types().types;
    let worktrees = worktree::list(&repo)?;
    let mut state = TableState::default();
    if !worktrees.is_empty() {
      state.select(Some(0));
    }
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
      create_form: CreateForm::new(),
      branch_types,
      report: None,
      sidebar: SidebarState::new(),
      pending_g: false,
      filter: FilterState::new(),
      picker_mode: false,
      picker_result: None,
      picker_should_exit: false,
      confirm: ConfirmModal::new(),
      link: BranchLink::empty(),
      link_slug: None,
      issue_state: GitHubFetchState::Idle,
      pr_state: GitHubFetchState::Idle,
      link_prompt: LinkPrompt::new(),
      trust_mode: crate::trust::TrustMode::Prompt,
    };
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
      .and_then(|r| r.url().map(String::from));
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
    let mut app = Self::new_at(start)?;
    app.picker_mode = true;
    app.filter.open();
    app.status = "switch picker — type to filter · enter selects · esc cancels".into();
    Ok(app)
  }

  pub fn refresh(&mut self) -> Result<()> {
    self.worktrees = worktree::list(&self.repo)?;
    // The cached fuzzy-match indices reference the previous worktrees
    // vec; drop them so the next render recomputes against the fresh
    // list. (A length-change auto-invalidates too, but a refresh that
    // produces a same-length vec with different contents would not —
    // so the explicit flush is the safe play.)
    self.filter.invalidate();
    // `clamp_selection_to_filter` re-resolves the link cache for us.
    self.clamp_selection_to_filter();
    self.invalidate_sidebar_cache();
    self.status = format!("refreshed — {} worktree(s)", self.worktrees.len());
    Ok(())
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
  pub fn handle_g(&mut self) {
    if self.pending_g {
      self.pending_g = false;
      self.first();
    } else {
      self.pending_g = true;
    }
  }

  pub fn cancel_pending_motion(&mut self) {
    self.pending_g = false;
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

  pub fn toggle_focus(&mut self) {
    self.sidebar.toggle_focus();
  }

  pub fn sidebar_scroll_down(&mut self) {
    self.sidebar.scroll_down();
  }

  pub fn sidebar_scroll_up(&mut self) {
    self.sidebar.scroll_up();
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
    let target = spec.worktree_path(&self.config.worktree, &self.repo_name)?;

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

    let created = worktree::add(&self.repo, &dirname, &target, &branch)?;

    let ctx = BootstrapCtx {
      main_repo: &self.workdir,
      worktree: &created,
      config: &self.config,
    };
    let report = bootstrap::run(&ctx)?;
    self.report = Some(report);
    self.view = View::Report;
    self.refresh()?;
    self.status = format!("created {} @ {}", branch, created.display());
    Ok(())
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
  }

  pub fn confirm_delete(&mut self) -> Result<()> {
    let (name, label) = match self.selected() {
      Some(s) => (s.name.clone(), s.path.display().to_string()),
      None => return Ok(()),
    };
    worktree::remove(&self.repo, &name, self.delete_branch_on_remove)?;
    self.status = format!("removed {} ({})", name, label);
    self.view = View::List;
    self.confirm.reset();
    self.refresh()
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
    self.confirm.dismiss();
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
    self.status = if self.filter.query.is_empty() {
      "press ? for help".into()
    } else {
      format!("filter sticky: {}", self.filter.query)
    };
  }

  /// Close the filter bar and clear the query: `Esc` returns to the full list.
  pub fn exit_filter_cancel(&mut self) {
    let had_query = !self.filter.query.is_empty();
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
    let before = self.filter.query.len();
    self.filter.pop_char();
    if self.filter.query.len() != before {
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

    let ctx = BootstrapCtx {
      main_repo: &self.workdir,
      worktree: &path,
      config: &self.config,
    };
    match bootstrap::run(&ctx) {
      Ok(r) => {
        let any_failed = r.steps.iter().any(|s| s.status == StepStatus::Failed);
        self.report = Some(r);
        self.view = View::Report;
        self.status = if any_failed {
          "bootstrap had failures".into()
        } else {
          "bootstrap ok".into()
        };
      }
      Err(e) => self.status = format!("bootstrap error: {}", e),
    }
  }

  // ---- Issue/PR linking (issue #67) -------------------------------------

  /// Re-read the link for the currently selected worktree's branch. Also
  /// re-resolves the repo slug from the origin remote, and resets any
  /// previously cached GitHub fetch state since it would refer to a
  /// different (issue, pr) tuple now.
  pub fn refresh_link(&mut self) {
    self.link = self.read_selected_link().unwrap_or_else(BranchLink::empty);
    self.link_slug = github::repo_slug(&self.repo).ok();
    self.issue_state = GitHubFetchState::Idle;
    self.pr_state = GitHubFetchState::Idle;
  }

  fn read_selected_link(&self) -> Option<BranchLink> {
    let branch = self
      .selected()
      .and_then(|w| w.branch.clone())
      .or_else(|| self.repo.head().ok().and_then(|h| h.shorthand().map(|s| s.to_string())))?;
    github::read_link(&self.repo, &branch).ok()
  }

  pub fn current_link(&self) -> &BranchLink {
    &self.link
  }

  pub fn current_slug(&self) -> Option<&str> {
    self.link_slug.as_deref()
  }

  pub fn issue_fetch_state(&self) -> &GitHubFetchState<IssueStatus> {
    &self.issue_state
  }

  pub fn pr_fetch_state(&self) -> &GitHubFetchState<PrStatus> {
    &self.pr_state
  }

  /// Drive the issue/PR fetch synchronously. Called from the event loop
  /// when the user presses `R`. Sets states to `Loading` first so the UI
  /// can flag the in-flight state, then runs the fetches.
  pub fn refresh_github_status(&mut self) {
    if self.link.issue.is_none() && self.link.pr.is_none() {
      self.status = "nothing linked — press L to link an issue or PR".into();
      return;
    }
    let Some(slug) = self.link_slug.clone() else {
      self.status = "no GitHub remote — cannot fetch status".into();
      return;
    };
    if let Some(n) = self.link.issue {
      self.issue_state = GitHubFetchState::Loading;
      let r = github::fetch_issue(&slug, n).map_err(|e| e.to_string());
      self.apply_issue_fetch_result(r);
    }
    if let Some(n) = self.link.pr {
      self.pr_state = GitHubFetchState::Loading;
      let r = github::fetch_pr(&slug, n).map_err(|e| e.to_string());
      self.apply_pr_fetch_result(r);
    }
    self.report_github_refresh_status();
  }

  /// Compute the post-refresh status line message based on the actual
  /// outcome of the issue / PR fetches. PR #68 Copilot review caught
  /// that always printing "refreshed" misled users when one of the
  /// fetches had failed.
  pub fn report_github_refresh_status(&mut self) {
    let issue_err = matches!(self.issue_state, GitHubFetchState::Error(_));
    let pr_err = matches!(self.pr_state, GitHubFetchState::Error(_));
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
    match &self.issue_state {
      GitHubFetchState::Error(e) => Some(e.clone()),
      _ => None,
    }
  }

  fn pr_error_message(&self) -> Option<String> {
    match &self.pr_state {
      GitHubFetchState::Error(e) => Some(e.clone()),
      _ => None,
    }
  }

  pub fn apply_issue_fetch_result(&mut self, r: std::result::Result<IssueStatus, String>) {
    self.issue_state = match r {
      Ok(s) => GitHubFetchState::Loaded(s),
      Err(e) => GitHubFetchState::Error(e),
    };
  }

  pub fn apply_pr_fetch_result(&mut self, r: std::result::Result<PrStatus, String>) {
    self.pr_state = match r {
      Ok(s) => GitHubFetchState::Loaded(s),
      Err(e) => GitHubFetchState::Error(e),
    };
  }

  // ---- Open menu ----------------------------------------------------------

  pub fn enter_open_menu(&mut self) {
    // Re-resolve link + slug in case the user just linked something
    // (`gwm link …` from a parallel terminal) or moved the origin remote.
    self.refresh_link();
    self.view = View::OpenMenu;
  }

  pub fn exit_open_menu(&mut self) {
    self.view = View::List;
  }

  /// Pick a target from the open menu. Returns the URL to open, or `None`
  /// when the link is missing (the status bar carries the explanation).
  pub fn open_menu_pick(&mut self, target: LinkTarget) -> Option<String> {
    self.view = View::List;
    let Some(slug) = self.link_slug.clone() else {
      self.status = "no GitHub remote — cannot build URL".into();
      return None;
    };
    let url = match target {
      LinkTarget::Issue => match self.link.issue {
        Some(n) => github::issue_url(&slug, n),
        None => {
          self.status = "no issue linked — press L to link one".into();
          return None;
        }
      },
      LinkTarget::Pr => match self.link.pr {
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
    self.status = "link: [i]ssue / [p]r · esc cancels".into();
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
      LinkTarget::Issue => "issue # — digits, enter to link, esc to cancel".into(),
      LinkTarget::Pr => "pr # — digits, enter to link, esc to cancel".into(),
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
      .or_else(|| self.repo.head().ok().and_then(|h| h.shorthand().map(|s| s.to_string())))
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
