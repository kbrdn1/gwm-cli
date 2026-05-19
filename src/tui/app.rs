use crate::bootstrap::{self, BootstrapCtx, BootstrapReport, StepStatus};
use crate::config::Config;
use crate::error::{GwmError, Result};
use crate::naming::{BranchSpec, BRANCH_TYPES};
use crate::worktree::{self, WorktreeInfo};
use git2::Repository;
use nucleo_matcher::{
  pattern::{CaseMatching, Normalization, Pattern},
  Config as NucleoConfig, Matcher, Utf32Str,
};
use ratatui::{text::Line, widgets::TableState};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum View {
  List,
  Create,
  Confirm,
  Report,
  Help,
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
    Ok(Self {
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
    })
  }

  pub fn refresh(&mut self) -> Result<()> {
    self.worktrees = worktree::list(&self.repo)?;
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
  }

  // ---- Vim-style motions / list jumps -------------------------------------

  pub fn first(&mut self) {
    let len = self.filtered_indices().len();
    if len > 0 {
      self.list_state.select(Some(0));
      self.sidebar_scroll = 0;
      self.invalidate_sidebar_cache();
    }
  }

  pub fn last(&mut self) {
    let len = self.filtered_indices().len();
    if len > 0 {
      self.list_state.select(Some(len - 1));
      self.sidebar_scroll = 0;
      self.invalidate_sidebar_cache();
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
  }

  pub fn confirm_delete(&mut self) -> Result<()> {
    let (name, label) = match self.selected() {
      Some(s) => (s.name.clone(), s.path.display().to_string()),
      None => return Ok(()),
    };
    worktree::remove(&self.repo, &name, self.delete_branch_on_remove)?;
    self.status = format!("removed {} ({})", name, label);
    self.view = View::List;
    self.refresh()
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
  /// worktree list itself changes (`refresh`).
  fn clamp_selection_to_filter(&mut self) {
    let len = self.filtered_indices().len();
    if len == 0 {
      self.list_state.select(None);
      return;
    }
    match self.list_state.selected() {
      Some(i) if i >= len => self.list_state.select(Some(len - 1)),
      Some(_) => {}
      None => self.list_state.select(Some(0)),
    }
  }

  // ---- Bootstrap flow ------------------------------------------------------

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
}
