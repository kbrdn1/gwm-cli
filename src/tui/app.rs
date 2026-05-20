use crate::bootstrap::{self, BootstrapCtx, BootstrapReport, StepStatus};
use crate::config::Config;
use crate::error::{GwmError, Result};
use crate::github::{self, BranchLink, IssueStatus, PrStatus};
use crate::naming::{BranchSpec, BRANCH_TYPES};
use crate::worktree::{self, WorktreeInfo};
use git2::Repository;
use nucleo_matcher::{
  pattern::{CaseMatching, Normalization, Pattern},
  Config as NucleoConfig, Matcher, Utf32Str,
};
use ratatui::{text::Line, widgets::TableState};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Outcome of pressing `y` / Enter on the confirm overlay. The event loop
/// in `super::run_app` matches on this to decide whether to fire the
/// delete immediately or wait for the countdown tick.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ConfirmKeyAction {
  /// Classic modal (delete_branch OFF, or countdown_secs = 0). The caller
  /// must invoke `confirm_delete` right away.
  FireNow,
  /// Countdown just got armed by this keystroke.
  Armed,
  /// Countdown was armed and got disarmed by this second keystroke.
  Disarmed,
}

/// State of the safety countdown after a tick. Returned by
/// `App::tick_confirm_countdown` so the event loop can branch without
/// reaching into the App's internals.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CountdownTickOutcome {
  /// No countdown was running (modal closed, or classic confirm modal).
  NotArmed,
  /// Countdown is still running — the loop should keep drawing the bar.
  Pending,
  /// Countdown has elapsed; the caller must invoke `confirm_delete` and
  /// clear the modal. The App has already reset its own timer state.
  ReadyToFire,
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

/// Stage of the two-step link prompt.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LinkPromptStage {
  ChooseTarget,
  InputNumber,
}

/// State of a background GitHub fetch (issue or PR).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubFetchState<T> {
  Idle,
  Loading,
  Loaded(T),
  Error(String),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Field {
  Type,
  Issue,
  Desc,
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
  pub create_field: Field,
  pub create_type_index: usize,
  pub create_issue: String,
  pub create_desc: String,

  // Bootstrap report
  pub report: Option<BootstrapReport>,

  // Sidebar (git preview) state
  pub sidebar_open: bool,
  pub sidebar_focused: bool,
  pub sidebar_scroll: u16,
  /// Cache of rendered sidebar lines, keyed by the selected worktree path.
  /// Prevents re-shelling `git log` / `git status` on every TUI redraw — they
  /// only run when the selection actually changes (or on explicit refresh).
  pub sidebar_cache: Option<(PathBuf, Vec<Line<'static>>)>,
  /// Upper bound for `sidebar_scroll`, recomputed by the renderer each frame.
  /// Keeps scrolling clamped to the rendered content height so the user can't
  /// scroll the panel entirely off-screen.
  pub sidebar_max_scroll: u16,

  // Vim motion buffer: armed by first `g`, completed by the second.
  pub pending_g: bool,

  // Inline fuzzy filter on the worktree list (issue #21).
  // `filter_active` is true while the user is typing in the filter bar (`/`).
  // `filter_query` carries the current pattern; when non-empty, the list view
  // shows only matching worktrees ranked by `nucleo_matcher`. Esc clears the
  // query, Enter confirms (sticky filter, focus returns to the list).
  pub filter_active: bool,
  pub filter_query: String,

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

  /// Anchor for the confirm-overlay safety countdown (issue #30). When
  /// `Some`, the modal renders a progress bar and the event loop ticks
  /// it down before firing `confirm_delete`. `None` means the modal is
  /// either closed, in classic mode (delete_branch OFF or
  /// `confirm_countdown_secs = 0`), or armed-but-just-disarmed by a
  /// second `y` press.
  pub confirm_countdown_started_at: Option<Instant>,

  // ---- Issue/PR linking (issue #67) -------------------------------------
  /// Cached link for the currently selected worktree's branch.
  link: BranchLink,
  /// Repo slug parsed from `origin` (None when no GitHub remote).
  link_slug: Option<String>,
  issue_state: GitHubFetchState<IssueStatus>,
  pr_state: GitHubFetchState<PrStatus>,
  /// In `View::LinkPrompt`, which stage are we in.
  link_prompt_stage: LinkPromptStage,
  /// In `View::LinkPrompt::InputNumber`, the digits typed so far.
  link_prompt_number: String,
  /// In `View::LinkPrompt::InputNumber`, the chosen target.
  link_prompt_target: Option<LinkTarget>,
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
      create_field: Field::Type,
      create_type_index: 0,
      create_issue: String::new(),
      create_desc: String::new(),
      report: None,
      sidebar_open: true,
      sidebar_focused: false,
      sidebar_scroll: 0,
      sidebar_cache: None,
      sidebar_max_scroll: 0,
      pending_g: false,
      filter_active: false,
      filter_query: String::new(),
      picker_mode: false,
      picker_result: None,
      picker_should_exit: false,
      confirm_countdown_started_at: None,
      link: BranchLink::empty(),
      link_slug: None,
      issue_state: GitHubFetchState::Idle,
      pr_state: GitHubFetchState::Idle,
      link_prompt_stage: LinkPromptStage::ChooseTarget,
      link_prompt_number: String::new(),
      link_prompt_target: None,
    };
    out.refresh_link();
    Ok(out)
  }

  /// Constructor for `gwm switch`: same App, but picker mode is on and the
  /// fuzzy filter bar is open from the first frame so the user can start
  /// narrowing right away. Everything else (worktree list, sidebar, vim
  /// motions) behaves identically; only the event-loop interpretation of
  /// Enter / n / d / b changes.
  pub fn new_picker_at(start: Option<&Path>) -> Result<Self> {
    let mut app = Self::new_at(start)?;
    app.picker_mode = true;
    app.filter_active = true;
    app.status = "switch picker — type to filter · enter selects · esc cancels".into();
    Ok(app)
  }

  pub fn refresh(&mut self) -> Result<()> {
    self.worktrees = worktree::list(&self.repo)?;
    // `clamp_selection_to_filter` re-resolves the link cache for us.
    self.clamp_selection_to_filter();
    self.invalidate_sidebar_cache();
    self.status = format!("refreshed — {} worktree(s)", self.worktrees.len());
    Ok(())
  }

  /// Drop the cached sidebar content. Call on any change that may have altered
  /// what the sidebar shows: worktree list refresh, selection change, etc.
  pub fn invalidate_sidebar_cache(&mut self) {
    self.sidebar_cache = None;
  }

  pub fn next(&mut self) {
    // Route navigation to the sidebar when it's focused; otherwise move the list.
    if self.sidebar_open && self.sidebar_focused {
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
    self.sidebar_scroll = 0;
    self.invalidate_sidebar_cache();
    self.refresh_link();
  }

  pub fn prev(&mut self) {
    if self.sidebar_open && self.sidebar_focused {
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
    self.sidebar_scroll = 0;
    self.invalidate_sidebar_cache();
    self.refresh_link();
  }

  // ---- Vim-style motions / list jumps -------------------------------------

  pub fn first(&mut self) {
    let len = self.filtered_indices().len();
    if len > 0 {
      self.list_state.select(Some(0));
      self.sidebar_scroll = 0;
      self.invalidate_sidebar_cache();
      self.refresh_link();
    }
  }

  pub fn last(&mut self) {
    let len = self.filtered_indices().len();
    if len > 0 {
      self.list_state.select(Some(len - 1));
      self.sidebar_scroll = 0;
      self.invalidate_sidebar_cache();
      self.refresh_link();
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
    self.sidebar_open = !self.sidebar_open;
    if !self.sidebar_open {
      // Hidden sidebar can't be focused.
      self.sidebar_focused = false;
    }
    self.status = if self.sidebar_open {
      "sidebar shown".into()
    } else {
      "sidebar hidden".into()
    };
  }

  pub fn toggle_focus(&mut self) {
    if !self.sidebar_open {
      return;
    }
    self.sidebar_focused = !self.sidebar_focused;
  }

  pub fn sidebar_scroll_down(&mut self) {
    // Clamp to the last-known content max so scrolling stops at the bottom
    // instead of running off-screen. The renderer keeps `sidebar_max_scroll`
    // up to date with the visible content height.
    self.sidebar_scroll = self.sidebar_scroll.saturating_add(1).min(self.sidebar_max_scroll);
  }

  pub fn sidebar_scroll_up(&mut self) {
    self.sidebar_scroll = self.sidebar_scroll.saturating_sub(1);
  }

  /// Path to launch lazygit on, or `None` if nothing selected or lazygit is missing.
  /// The caller drives the actual TUI suspension/restoration around the spawn.
  pub fn launch_lazygit(&mut self) -> Option<PathBuf> {
    let path = self.selected()?.path.clone();
    if which::which("lazygit").is_err() {
      self.status = "lazygit not found in PATH".into();
      return None;
    }
    Some(path)
  }

  pub fn selected(&self) -> Option<&WorktreeInfo> {
    // The visible list is the filtered subset, so the table state's index is
    // into `filtered_indices()`, not the raw `worktrees` vec. Resolving the
    // selection means hopping through the filter map.
    let i = self.list_state.selected()?;
    let filtered = self.filtered_indices();
    let original = *filtered.get(i)?;
    self.worktrees.get(original)
  }

  pub fn copy_path_to_status(&mut self) {
    if let Some(w) = self.selected() {
      self.status = format!("path: {}", w.path.display());
    }
  }

  /// Reveal the selected worktree's directory in the OS file manager.
  /// macOS: `open`, Linux: `xdg-open`, Windows: `explorer`.
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

  pub fn toggle_delete_branch(&mut self) {
    self.delete_branch_on_remove = !self.delete_branch_on_remove;
    self.status = format!("delete branch on remove: {}", self.delete_branch_on_remove);
  }

  // ---- Create flow ---------------------------------------------------------

  pub fn enter_create(&mut self) {
    self.view = View::Create;
    self.create_field = Field::Type;
    self.create_type_index = 0;
    self.create_issue.clear();
    self.create_desc.clear();
    self.status = "tab/shift-tab: switch field — enter on desc: submit — esc: cancel".into();
  }

  pub fn create_next_field(&mut self) {
    self.create_field = match self.create_field {
      Field::Type => Field::Issue,
      Field::Issue => Field::Desc,
      Field::Desc => Field::Type,
    };
  }

  pub fn create_prev_field(&mut self) {
    self.create_field = match self.create_field {
      Field::Type => Field::Desc,
      Field::Issue => Field::Type,
      Field::Desc => Field::Issue,
    };
  }

  pub fn create_next_type(&mut self) {
    self.create_type_index = (self.create_type_index + 1) % BRANCH_TYPES.len();
  }

  pub fn create_prev_type(&mut self) {
    if self.create_type_index == 0 {
      self.create_type_index = BRANCH_TYPES.len() - 1;
    } else {
      self.create_type_index -= 1;
    }
  }

  pub fn create_push_char(&mut self, c: char) {
    match self.create_field {
      Field::Issue if c.is_ascii_digit() => self.create_issue.push(c),
      Field::Desc => self.create_desc.push(c),
      _ => {}
    }
  }

  pub fn create_pop_char(&mut self) {
    match self.create_field {
      Field::Issue => {
        self.create_issue.pop();
      }
      Field::Desc => {
        self.create_desc.pop();
      }
      _ => {}
    }
  }

  pub fn submit_create(&mut self) -> Result<()> {
    let type_ = BRANCH_TYPES[self.create_type_index].0.to_string();
    let spec = BranchSpec::new(type_, self.create_issue.clone(), self.create_desc.clone())?;
    let branch = spec.branch_name(&self.config.worktree, &self.repo_name)?;
    let dirname = spec.worktree_dirname(&self.config.worktree, &self.repo_name)?;
    let target = spec.worktree_path(&self.config.worktree, &self.repo_name)?;

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
    self.confirm_countdown_started_at = None;
  }

  pub fn confirm_delete(&mut self) -> Result<()> {
    let (name, label) = match self.selected() {
      Some(s) => (s.name.clone(), s.path.display().to_string()),
      None => return Ok(()),
    };
    worktree::remove(&self.repo, &name, self.delete_branch_on_remove)?;
    self.status = format!("removed {} ({})", name, label);
    self.view = View::List;
    self.confirm_countdown_started_at = None;
    self.refresh()
  }

  // ---- Confirm-overlay safety countdown (issue #30) -----------------------
  //
  // The countdown only applies when `delete_branch_on_remove` is ON AND the
  // configured `confirm_countdown_secs` is non-zero. In every other case
  // the classic single-keystroke confirm is preserved. Methods here are
  // pure (modulo `&mut self`) and take an explicit `Instant` so tests can
  // step through the countdown without sleeping.

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

  /// Handle a `y` / Enter press inside the confirm overlay.
  ///
  /// Returns the action the event loop should take next:
  /// - `FireNow` — classic modal, the loop must call `confirm_delete()`.
  /// - `Armed` — countdown started; the loop draws the progress bar and
  ///   waits for the next tick.
  /// - `Disarmed` — second `y` press cancelled the countdown; the loop
  ///   stays in the modal (Esc / n closes it).
  pub fn confirm_press_y(&mut self, now: Instant) -> ConfirmKeyAction {
    if !self.confirm_is_countdown_mode() {
      return ConfirmKeyAction::FireNow;
    }
    if self.confirm_countdown_started_at.is_some() {
      self.confirm_countdown_started_at = None;
      let secs = self.confirm_countdown_total().as_secs();
      self.status = format!("countdown cancelled — press y to re-arm ({secs}s safety delay)");
      ConfirmKeyAction::Disarmed
    } else {
      self.confirm_countdown_started_at = Some(now);
      let secs = self.confirm_countdown_total().as_secs();
      self.status = format!("armed — auto-fires in {secs}s · press y again or Esc to cancel");
      ConfirmKeyAction::Armed
    }
  }

  /// Handle the dismissal keys (`n` / `Esc`) inside the confirm overlay.
  /// Always disarms the countdown and returns to the list.
  pub fn confirm_dismiss(&mut self) {
    self.confirm_countdown_started_at = None;
    self.view = View::List;
  }

  /// Tick the countdown forward. Called from the event loop on every
  /// poll-timeout iteration (every 200ms). Returns `ReadyToFire` exactly
  /// once when the timer crosses the total duration; the App's own
  /// `confirm_countdown_started_at` is cleared at that point so a
  /// re-entrant tick would return `NotArmed`.
  pub fn tick_confirm_countdown(&mut self, now: Instant) -> CountdownTickOutcome {
    let Some(started) = self.confirm_countdown_started_at else {
      return CountdownTickOutcome::NotArmed;
    };
    let duration = self.confirm_countdown_total();
    if duration.is_zero() {
      // Defensive: if config changed mid-modal to 0s, treat as no-op.
      self.confirm_countdown_started_at = None;
      return CountdownTickOutcome::NotArmed;
    }
    if now.saturating_duration_since(started) < duration {
      CountdownTickOutcome::Pending
    } else {
      self.confirm_countdown_started_at = None;
      CountdownTickOutcome::ReadyToFire
    }
  }

  /// Countdown progress in `[0.0, 1.0]`. `0.0` when not armed, `1.0` once
  /// elapsed. Used by the UI to draw the gauge.
  pub fn confirm_countdown_progress(&self, now: Instant) -> f64 {
    let Some(started) = self.confirm_countdown_started_at else {
      return 0.0;
    };
    let duration = self.confirm_countdown_total();
    if duration.is_zero() {
      return 0.0;
    }
    let elapsed = now.saturating_duration_since(started).as_secs_f64();
    let total = duration.as_secs_f64();
    (elapsed / total).min(1.0)
  }

  /// Seconds remaining (rounded up to the next whole second) for the UI
  /// label. `0` when not armed or when the countdown has elapsed.
  pub fn confirm_countdown_remaining_secs(&self, now: Instant) -> u64 {
    let Some(started) = self.confirm_countdown_started_at else {
      return 0;
    };
    let duration = self.confirm_countdown_total();
    if duration.is_zero() {
      return 0;
    }
    let remaining = duration.saturating_sub(now.saturating_duration_since(started));
    if remaining.is_zero() {
      return 0;
    }
    let extra = if remaining.subsec_nanos() > 0 { 1 } else { 0 };
    remaining.as_secs() + extra
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
    self.filter_active = true;
    self.sidebar_focused = false;
    self.cancel_pending_motion();
    self.status = "/ filter — type to narrow · enter confirms · esc clears".into();
  }

  /// Close the filter bar but keep the query: `Enter` confirms the current
  /// match set and returns the cursor to list navigation.
  pub fn exit_filter_keep(&mut self) {
    self.filter_active = false;
    self.status = if self.filter_query.is_empty() {
      "press ? for help".into()
    } else {
      format!("filter sticky: {}", self.filter_query)
    };
  }

  /// Close the filter bar and clear the query: `Esc` returns to the full list.
  pub fn exit_filter_cancel(&mut self) {
    let had_query = !self.filter_query.is_empty();
    self.filter_active = false;
    self.filter_query.clear();
    self.clamp_selection_to_filter();
    self.invalidate_sidebar_cache();
    self.status = if had_query {
      "filter cleared".into()
    } else {
      "press ? for help".into()
    };
  }

  pub fn filter_push_char(&mut self, c: char) {
    self.filter_query.push(c);
    self.clamp_selection_to_filter();
    self.invalidate_sidebar_cache();
  }

  pub fn filter_pop_char(&mut self) {
    if self.filter_query.pop().is_some() {
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
  pub fn filtered_indices(&self) -> Vec<usize> {
    if self.filter_query.is_empty() {
      return (0..self.worktrees.len()).collect();
    }
    let pattern = Pattern::parse(self.filter_query.as_str(), CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(NucleoConfig::DEFAULT);
    let mut buf: Vec<char> = Vec::new();
    let mut scored: Vec<(u32, usize)> = Vec::with_capacity(self.worktrees.len());
    for (i, w) in self.worktrees.iter().enumerate() {
      let hay = Utf32Str::new(&w.name, &mut buf);
      if let Some(score) = pattern.score(hay, &mut matcher) {
        scored.push((score, i));
      }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
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

  pub fn enter_link_prompt(&mut self) {
    self.view = View::LinkPrompt;
    self.link_prompt_stage = LinkPromptStage::ChooseTarget;
    self.link_prompt_target = None;
    self.link_prompt_number.clear();
    self.status = "link: [i]ssue / [p]r · esc cancels".into();
  }

  pub fn link_prompt_cancel(&mut self) {
    self.view = View::List;
    self.link_prompt_stage = LinkPromptStage::ChooseTarget;
    self.link_prompt_target = None;
    self.link_prompt_number.clear();
  }

  pub fn link_prompt_stage(&self) -> LinkPromptStage {
    self.link_prompt_stage
  }

  pub fn link_prompt_number_input(&self) -> &str {
    &self.link_prompt_number
  }

  pub fn link_prompt_target(&self) -> Option<LinkTarget> {
    self.link_prompt_target
  }

  pub fn link_prompt_choose(&mut self, target: LinkTarget) {
    self.link_prompt_target = Some(target);
    self.link_prompt_stage = LinkPromptStage::InputNumber;
    self.link_prompt_number.clear();
    self.status = match target {
      LinkTarget::Issue => "issue # — digits, enter to link, esc to cancel".into(),
      LinkTarget::Pr => "pr # — digits, enter to link, esc to cancel".into(),
    };
  }

  pub fn link_prompt_push_char(&mut self, c: char) {
    if self.link_prompt_stage == LinkPromptStage::InputNumber && c.is_ascii_digit() {
      self.link_prompt_number.push(c);
    }
  }

  pub fn link_prompt_pop_char(&mut self) {
    if self.link_prompt_stage == LinkPromptStage::InputNumber {
      self.link_prompt_number.pop();
    }
  }

  pub fn link_prompt_submit(&mut self) -> Result<()> {
    let Some(target) = self.link_prompt_target else {
      self.status = "no target chosen".into();
      return Ok(());
    };
    let n: u64 = self
      .link_prompt_number
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
    self.link_prompt_stage = LinkPromptStage::ChooseTarget;
    self.link_prompt_target = None;
    self.link_prompt_number.clear();
    self.refresh_link();
    Ok(())
  }
}
