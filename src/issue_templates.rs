use crate::config::{Config, IssueTemplateTypeConfig};
use crate::error::{GwmError, Result};
use crate::templating::{self, FormDefaults, TemplateContext};
use crate::worktree;
use git2::Repository;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct IssueDraft {
  pub title: String,
  pub labels: Vec<String>,
  pub body_file: tempfile::NamedTempFile,
}

pub fn render_issue_draft(repo: &Repository, config: &Config, branch_type: &str, desc: &str) -> Result<IssueDraft> {
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?;
  let type_config = config.issue_template.by_type.get(branch_type);
  let template_name = type_config
    .and_then(|cfg| cfg.template.as_deref())
    .or(config.issue_template.default.as_deref())
    .ok_or_else(|| {
      GwmError::Config(format!(
        "no issue template configured for branch type '{}' (set [issue_template].default or [issue_template.by_type.{}].template)",
        branch_type, branch_type
      ))
    })?;
  let template_path = resolve_template_path(workdir, template_name)?;
  let raw = std::fs::read_to_string(&template_path)?;
  let meta = templating::issue_form_metadata(&raw)?;
  let ctx = TemplateContext::from_pairs([
    ("type", branch_type),
    ("issue", ""),
    ("desc", desc),
    ("repo", &worktree::repo_name(repo)),
  ]);
  let defaults = defaults_for(type_config);
  let body = templating::render_form_markdown(&raw, &ctx, &defaults)?;
  let mut body_file = tempfile::NamedTempFile::new()?;
  body_file.write_all(body.as_bytes())?;
  body_file.flush()?;

  let title_prefix = type_config
    .and_then(|cfg| cfg.title_prefix.as_deref())
    .or(meta.title.as_deref())
    .unwrap_or_default();
  let mut labels = meta.labels;
  if let Some(cfg) = type_config {
    labels.extend(cfg.labels.clone());
  }
  labels.sort();
  labels.dedup();

  Ok(IssueDraft {
    title: format!("{}{}", title_prefix, desc),
    labels,
    body_file,
  })
}

fn defaults_for(type_config: Option<&IssueTemplateTypeConfig>) -> FormDefaults {
  let mut fields = BTreeMap::new();
  if let Some(surface) = type_config.and_then(|cfg| cfg.surface.as_deref()) {
    fields.insert("surface".to_string(), surface.to_string());
  }
  FormDefaults { fields }
}

fn resolve_template_path(workdir: &Path, template_name: &str) -> Result<PathBuf> {
  let rel = Path::new(template_name);
  // Reject anything that could escape the worktree root or the
  // `.github/ISSUE_TEMPLATE` base:
  //   - absolute paths (Unix `/etc/passwd`, Windows `C:\Windows\…`)
  //   - parent traversals (`..`)
  //   - Windows drive prefixes on relative paths (`C:foo.yml` parses as a
  //     relative path with a `Prefix` component but joining it onto `workdir`
  //     can ignore the base)
  //   - root-only segments (`\foo.yml` is not absolute on Windows but has a
  //     `RootDir` component that resets the joined path)
  let suspicious = rel.is_absolute()
    || rel
      .components()
      .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir));
  if suspicious {
    return Err(GwmError::Config(format!(
      "issue template path '{}' must be relative and stay inside .github/ISSUE_TEMPLATE",
      template_name
    )));
  }
  let joined = if rel.starts_with(".github") {
    workdir.join(rel)
  } else {
    workdir.join(".github").join("ISSUE_TEMPLATE").join(rel)
  };
  if joined.strip_prefix(workdir).is_err() {
    return Err(GwmError::Config(format!(
      "issue template path '{}' escapes the worktree root",
      template_name
    )));
  }
  Ok(joined)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolve_rejects_parent_traversal() {
    let workdir = Path::new("/tmp/wd");
    let err = resolve_template_path(workdir, "../etc/passwd.yml").unwrap_err();
    assert!(matches!(err, GwmError::Config(_)), "got {err:?}");
  }

  #[test]
  fn resolve_rejects_absolute_paths() {
    let workdir = Path::new("/tmp/wd");
    let err = resolve_template_path(workdir, "/etc/passwd.yml").unwrap_err();
    assert!(matches!(err, GwmError::Config(_)), "got {err:?}");
  }

  #[cfg(windows)]
  #[test]
  fn resolve_rejects_windows_drive_prefix() {
    let workdir = Path::new(r"C:\tmp\wd");
    let err = resolve_template_path(workdir, "C:foo.yml").unwrap_err();
    assert!(matches!(err, GwmError::Config(_)), "got {err:?}");
  }

  #[cfg(windows)]
  #[test]
  fn resolve_rejects_windows_rootdir_prefix() {
    let workdir = Path::new(r"C:\tmp\wd");
    let err = resolve_template_path(workdir, r"\Windows\System32\config").unwrap_err();
    assert!(matches!(err, GwmError::Config(_)), "got {err:?}");
  }

  #[test]
  fn resolve_accepts_plain_template_name() {
    let workdir = Path::new("/tmp/wd");
    let path = resolve_template_path(workdir, "feature_request.yml").unwrap();
    assert_eq!(
      path,
      workdir
        .join(".github")
        .join("ISSUE_TEMPLATE")
        .join("feature_request.yml")
    );
  }

  #[test]
  fn resolve_accepts_explicit_dot_github_prefix() {
    let workdir = Path::new("/tmp/wd");
    let path = resolve_template_path(workdir, ".github/ISSUE_TEMPLATE/bug.yml").unwrap();
    assert_eq!(path, workdir.join(".github").join("ISSUE_TEMPLATE").join("bug.yml"));
  }
}
