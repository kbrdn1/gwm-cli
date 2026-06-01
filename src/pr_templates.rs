//! PR body renderer for the `[pr_template]` config block (issue #84).
//!
//! Mirrors the issue-template flow from [`crate::issue_templates`] but
//! is deliberately pure: callers (e.g. `gwm pr`) collect the branch
//! context, base/head refs, commit list, and diff stats from libgit2 +
//! `gh`, then hand a [`PrTemplateContext`] over to [`render_pr_body`].
//! The renderer itself does not touch git so unit tests can drive it
//! against a `tempfile::TempDir` without seeding a repo.
//!
//! Body resolution precedence (most → least specific):
//!   1. `[pr_template.by_type.<type>].body` — inline override
//!   2. `[pr_template.by_type.<type>].path` — workdir-relative Markdown
//!   3. `[pr_template].default` — workdir-relative Markdown fallback
//!   4. otherwise an error so the caller can surface a config hint
//!
//! Path inputs are sandboxed against the same `Component::Prefix` /
//! `Component::RootDir` / parent-traversal hardening as
//! [`crate::issue_templates::resolve_template_path`], plus a
//! belt-and-braces `strip_prefix(workdir)` containment check.

use crate::config::{PrTemplateConfig, PrTemplateTypeConfig};
use crate::error::{GwmError, Result};
use crate::templating::{self, TemplateContext};
use std::path::{Component, Path, PathBuf};

/// Inputs the renderer substitutes into the template body. All fields
/// default to the empty string so callers can fill in only the ones
/// the configured template references.
#[derive(Debug, Clone, Default)]
pub struct PrTemplateContext {
  pub branch_type: String,
  pub issue: String,
  pub desc: String,
  pub base: String,
  pub head: String,
  pub commits: String,
  pub files_changed: String,
  pub repo: String,
}

/// Render the PR body for `ctx.branch_type` from `config`, anchored at
/// `workdir`. Resolves the most-specific template (inline body > per-
/// type path > default path), then runs the shared placeholder engine
/// against the supplied context.
pub fn render_pr_body(config: &PrTemplateConfig, workdir: &Path, ctx: &PrTemplateContext) -> Result<String> {
  let type_config = config.by_type.get(&ctx.branch_type);
  let raw_body = resolve_raw_body(config, workdir, &ctx.branch_type, type_config)?;
  let tctx = TemplateContext::from_pairs([
    ("type", ctx.branch_type.as_str()),
    ("issue", ctx.issue.as_str()),
    ("desc", ctx.desc.as_str()),
    ("base", ctx.base.as_str()),
    ("head", ctx.head.as_str()),
    ("commits", ctx.commits.as_str()),
    ("files_changed", ctx.files_changed.as_str()),
    ("repo", ctx.repo.as_str()),
  ]);
  Ok(templating::render_template(&raw_body, &tctx))
}

fn resolve_raw_body(
  config: &PrTemplateConfig,
  workdir: &Path,
  branch_type: &str,
  type_config: Option<&PrTemplateTypeConfig>,
) -> Result<String> {
  if let Some(tc) = type_config {
    if let Some(body) = tc.body.as_deref() {
      return Ok(body.to_string());
    }
    if let Some(path) = tc.path.as_deref() {
      return read_template_file(workdir, path, "pr_template");
    }
  }
  if let Some(path) = config.default.as_deref() {
    return read_template_file(workdir, path, "pr_template default");
  }
  Err(GwmError::Config(format!(
    "no pr_template configured for branch type '{}' (set [pr_template].default or [pr_template.by_type.{}].path/body)",
    branch_type, branch_type
  )))
}

fn read_template_file(workdir: &Path, path: &str, label: &str) -> Result<String> {
  let resolved = resolve_template_path(workdir, path)?;
  std::fs::read_to_string(&resolved)
    .map_err(|e| GwmError::Config(format!("{} '{}' could not be read: {}", label, path, e)))
}

fn resolve_template_path(workdir: &Path, path: &str) -> Result<PathBuf> {
  let rel = Path::new(path);
  let suspicious = rel.is_absolute()
    || rel
      .components()
      .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir));
  if suspicious {
    return Err(GwmError::Config(format!(
      "pr_template path '{}' must be relative and stay inside the worktree root",
      path
    )));
  }
  let joined = workdir.join(rel);
  if joined.strip_prefix(workdir).is_err() {
    return Err(GwmError::Config(format!(
      "pr_template path '{}' escapes the worktree root",
      path
    )));
  }
  Ok(joined)
}
