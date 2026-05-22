use thiserror::Error;

pub type Result<T> = std::result::Result<T, GwmError>;

/// Which side of a `gwm link` / `gwm open` pair is missing on a branch.
/// Carried by [`GwmError::LinkMissing`] so the user sees whether the
/// issue or the PR slot is empty — both share the same git-config
/// shape (`branch.<name>.gwm-issue` / `gwm-pr`) so the error message
/// must spell out which one was queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
  Issue,
  Pr,
}

impl LinkKind {
  fn as_str(self) -> &'static str {
    match self {
      LinkKind::Issue => "issue",
      LinkKind::Pr => "PR",
    }
  }
}

impl std::fmt::Display for LinkKind {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(self.as_str())
  }
}

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

  #[error("invalid branch type '{got}' (allowed: {allowed})")]
  InvalidBranchType { got: String, allowed: String },

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

  /// Generic command/spawn failure. The variant is shared between
  /// every subcommand that shells out (bootstrap steps, `gwm tmux`,
  /// `gwm zellij`, the `git log` / `git status` previews in the TUI
  /// sidebar, …); callers prepend their own operation name into the
  /// inner string so the rendered message stays attributable to the
  /// verb the user actually typed.
  #[error("command failed: {0}")]
  CommandFailed(String),

  #[error("config error: {0}")]
  Config(String),

  /// Issue #105: HEAD is unborn (no commits yet) or detached when a
  /// command that needs the current branch shorthand is invoked.
  /// Split out of `Other` so callers (and the TUI status line) can
  /// distinguish "no current branch" from arbitrary string errors.
  #[error("{reason}")]
  UnbornHead { reason: String },

  /// Issue #105: failed to deserialize a `gh` CLI JSON payload.
  /// `kind` names the payload (`"issue"`, `"pr"`, `"pr list"`,
  /// `"labels"`, `"milestones"`) so the user can grep for which
  /// gh contract changed; `source` carries the underlying
  /// `serde_json::Error` for downstream introspection.
  #[error("failed to parse {kind} json: {source}")]
  GhJsonParse {
    kind: &'static str,
    #[source]
    source: serde_json::Error,
  },

  /// Issue #105: `gwm open` / `gwm link` was asked for an issue or PR
  /// linked to a branch but no such link is recorded in git-config.
  /// `kind` names which side (issue vs PR) is missing; `branch` is
  /// the branch shorthand the user queried.
  #[error("no {kind} linked to branch '{branch}'")]
  LinkMissing { kind: LinkKind, branch: String },

  #[error("{0}")]
  Other(String),
}

impl From<anyhow::Error> for GwmError {
  fn from(e: anyhow::Error) -> Self {
    GwmError::Other(e.to_string())
  }
}
