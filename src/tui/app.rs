use crate::bootstrap::{self, BootstrapCtx, BootstrapReport, StepStatus};
use crate::config::Config;
use crate::error::{GwmError, Result};
use crate::naming::{BranchSpec, BRANCH_TYPES};
use crate::worktree::{self, WorktreeInfo};
use git2::Repository;
use ratatui::widgets::TableState;
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

  // Vim motion buffer: armed by first `g`, completed by the second.
  pub pending_g: bool,
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
      pending_g: false,
    })
  }

  pub fn refresh(&mut self) -> Result<()> {
    self.worktrees = worktree::list(&self.repo)?;
    if self.worktrees.is_empty() {
      self.list_state.select(None);
    } else if self.list_state.selected().is_none() {
      self.list_state.select(Some(0));
    } else if let Some(i) = self.list_state.selected() {
      if i >= self.worktrees.len() {
        self.list_state.select(Some(self.worktrees.len() - 1));
      }
    }
    self.status = format!("refreshed — {} worktree(s)", self.worktrees.len());
    Ok(())
  }

  pub fn next(&mut self) {
    // Route navigation to the sidebar when it's focused; otherwise move the list.
    if self.sidebar_open && self.sidebar_focused {
      self.sidebar_scroll_down();
      return;
    }
    if self.worktrees.is_empty() {
      return;
    }
    let i = match self.list_state.selected() {
      Some(i) => (i + 1) % self.worktrees.len(),
      None => 0,
    };
    self.list_state.select(Some(i));
    self.sidebar_scroll = 0;
  }

  pub fn prev(&mut self) {
    if self.sidebar_open && self.sidebar_focused {
      self.sidebar_scroll_up();
      return;
    }
    if self.worktrees.is_empty() {
      return;
    }
    let i = match self.list_state.selected() {
      Some(0) | None => self.worktrees.len() - 1,
      Some(i) => i - 1,
    };
    self.list_state.select(Some(i));
    self.sidebar_scroll = 0;
  }

  // ---- Vim-style motions / list jumps -------------------------------------

  pub fn first(&mut self) {
    if !self.worktrees.is_empty() {
      self.list_state.select(Some(0));
      self.sidebar_scroll = 0;
    }
  }

  pub fn last(&mut self) {
    if !self.worktrees.is_empty() {
      self.list_state.select(Some(self.worktrees.len() - 1));
      self.sidebar_scroll = 0;
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
    self.sidebar_scroll = self.sidebar_scroll.saturating_add(1);
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
    self.list_state.selected().and_then(|i| self.worktrees.get(i))
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
