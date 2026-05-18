use crate::bootstrap::{self, BootstrapCtx, BootstrapReport, StepStatus};
use crate::config::Config;
use crate::error::{GwmError, Result};
use crate::naming::{BranchSpec, BRANCH_TYPES};
use crate::worktree::{self, WorktreeInfo};
use git2::Repository;
use ratatui::widgets::ListState;
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
  pub list_state: ListState,
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
    let mut state = ListState::default();
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
    if self.worktrees.is_empty() {
      return;
    }
    let i = match self.list_state.selected() {
      Some(i) => (i + 1) % self.worktrees.len(),
      None => 0,
    };
    self.list_state.select(Some(i));
  }

  pub fn prev(&mut self) {
    if self.worktrees.is_empty() {
      return;
    }
    let i = match self.list_state.selected() {
      Some(0) | None => self.worktrees.len() - 1,
      Some(i) => i - 1,
    };
    self.list_state.select(Some(i));
  }

  pub fn selected(&self) -> Option<&WorktreeInfo> {
    self.list_state.selected().and_then(|i| self.worktrees.get(i))
  }

  pub fn copy_path_to_status(&mut self) {
    if let Some(w) = self.selected() {
      self.status = format!("path: {}", w.path.display());
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
