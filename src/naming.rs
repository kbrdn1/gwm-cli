use crate::config::{expand_placeholders, WorktreeConfig};
use crate::error::{GwmError, Result};
use regex::Regex;

pub const BRANCH_TYPES: &[(&str, &str)] = &[
  ("feat", "New feature implementation"),
  ("fix", "Bug fix"),
  ("hotfix", "Critical production bug fix"),
  ("docs", "Documentation changes"),
  ("test", "Test additions or modifications"),
  ("refactor", "Code restructuring"),
  ("chore", "Maintenance tasks"),
  ("perf", "Performance improvements"),
  ("ci", "CI/CD configuration"),
  ("build", "Build system changes"),
];

#[derive(Debug, Clone)]
pub struct BranchSpec {
  pub type_: String,
  pub issue: String,
  pub desc: String,
}

impl BranchSpec {
  pub fn new(type_: impl Into<String>, issue: impl Into<String>, desc: impl Into<String>) -> Result<Self> {
    let s = Self {
      type_: type_.into(),
      issue: issue.into(),
      desc: kebab(&desc.into()),
    };
    s.validate()?;
    Ok(s)
  }

  pub fn validate(&self) -> Result<()> {
    if !BRANCH_TYPES.iter().any(|(t, _)| *t == self.type_) {
      return Err(GwmError::InvalidBranchType(self.type_.clone()));
    }
    if !Regex::new(r"^\d+$").unwrap().is_match(&self.issue) {
      return Err(GwmError::InvalidIssue(self.issue.clone()));
    }
    if !Regex::new(r"^[a-z0-9][a-z0-9-]*$").unwrap().is_match(&self.desc) {
      return Err(GwmError::InvalidDescription(self.desc.clone()));
    }
    Ok(())
  }

  pub fn branch_name(&self, cfg: &WorktreeConfig, repo: &str) -> Result<String> {
    expand_placeholders(&cfg.branch_pattern, repo, Some(&self.type_), Some(&self.issue), Some(&self.desc))
  }

  pub fn worktree_dirname(&self, cfg: &WorktreeConfig, repo: &str) -> Result<String> {
    expand_placeholders(&cfg.path_pattern, repo, Some(&self.type_), Some(&self.issue), Some(&self.desc))
  }

  pub fn worktree_path(&self, cfg: &WorktreeConfig, repo: &str) -> Result<std::path::PathBuf> {
    let base = expand_placeholders(&cfg.base, repo, Some(&self.type_), Some(&self.issue), Some(&self.desc))?;
    let dir = self.worktree_dirname(cfg, repo)?;
    Ok(std::path::PathBuf::from(base).join(dir))
  }
}

/// Try to recover a BranchSpec from a free-form branch name like `feat/#123-my-desc`.
pub fn parse_branch(branch: &str) -> Option<BranchSpec> {
  let re = Regex::new(r"^([a-z]+)/#(\d+)-([a-z0-9-]+)$").ok()?;
  let cap = re.captures(branch)?;
  Some(BranchSpec {
    type_: cap.get(1)?.as_str().to_string(),
    issue: cap.get(2)?.as_str().to_string(),
    desc: cap.get(3)?.as_str().to_string(),
  })
}

pub fn kebab(input: &str) -> String {
  let lower = input.to_lowercase();
  let mut out = String::with_capacity(lower.len());
  let mut prev_dash = false;
  for c in lower.chars() {
    let ok = c.is_ascii_alphanumeric();
    if ok {
      out.push(c);
      prev_dash = false;
    } else if c == '-' || c == ' ' || c == '_' {
      if !prev_dash && !out.is_empty() {
        out.push('-');
        prev_dash = true;
      }
    }
  }
  out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn kebab_normalizes() {
    assert_eq!(kebab("Hello World"), "hello-world");
    assert_eq!(kebab("Foo_BAR  baz"), "foo-bar-baz");
    assert_eq!(kebab("--leading--"), "leading");
  }

  #[test]
  fn branch_validation() {
    assert!(BranchSpec::new("feat", "123", "user-auth").is_ok());
    assert!(BranchSpec::new("nope", "123", "x").is_err());
    assert!(BranchSpec::new("feat", "abc", "x").is_err());
    assert!(BranchSpec::new("feat", "123", "").is_err());
  }

  #[test]
  fn parse_roundtrip() {
    let parsed = parse_branch("feat/#42-cool-feature").unwrap();
    assert_eq!(parsed.type_, "feat");
    assert_eq!(parsed.issue, "42");
    assert_eq!(parsed.desc, "cool-feature");
  }

  #[test]
  fn renders_paths() {
    let cfg = WorktreeConfig::default();
    let spec = BranchSpec::new("feat", "10", "x").unwrap();
    assert_eq!(spec.branch_name(&cfg, "myrepo").unwrap(), "feat/#10-x");
    assert_eq!(spec.worktree_dirname(&cfg, "myrepo").unwrap(), "feat-10-x");
    let p = spec.worktree_path(&cfg, "myrepo").unwrap();
    assert!(p.to_string_lossy().ends_with("/cc-worktree/myrepo/feat-10-x"));
  }
}
