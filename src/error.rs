use thiserror::Error;

pub type Result<T> = std::result::Result<T, GwmError>;

#[derive(Debug, Error)]
pub enum GwmError {
  #[error("not inside a git repository")]
  NotInGitRepo,

  #[error("git error: {0}")]
  Git(#[from] git2::Error),

  #[error("io error: {0}")]
  Io(#[from] std::io::Error),

  #[error("toml parse error: {0}")]
  TomlParse(#[from] toml::de::Error),

  #[error("toml serialize error: {0}")]
  TomlSer(#[from] toml::ser::Error),

  #[error("regex error: {0}")]
  Regex(#[from] regex::Error),

  #[error("shell expand error: {0}")]
  ShellExpand(#[from] shellexpand::LookupError<std::env::VarError>),

  #[error("invalid branch type '{0}' (allowed: feat, fix, hotfix, docs, test, refactor, chore, perf, ci, build)")]
  InvalidBranchType(String),

  #[error("invalid issue number '{0}' (digits only)")]
  InvalidIssue(String),

  #[error("invalid description '{0}' (kebab-case alphanumeric)")]
  InvalidDescription(String),

  #[error("worktree '{0}' not found")]
  WorktreeNotFound(String),

  #[error("worktree '{0}' already exists at {1}")]
  WorktreeExists(String, String),

  #[error("guard '{name}' tripped: file {file} matches deny pattern")]
  GuardTripped { name: String, file: String },

  #[error("bootstrap command failed: {0}")]
  CommandFailed(String),

  #[error("config error: {0}")]
  Config(String),

  #[error("{0}")]
  Other(String),
}

impl From<anyhow::Error> for GwmError {
  fn from(e: anyhow::Error) -> Self {
    GwmError::Other(e.to_string())
  }
}
