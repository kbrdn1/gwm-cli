//! Unit tests for the [`gwm::pr_templates`] renderer (issue #84).
//!
//! The renderer is pure: it takes a `PrTemplateConfig`, the worktree
//! root, and a `PrTemplateContext` built by the caller. Tests construct
//! everything in-memory and drive `render_pr_body` directly so the
//! `gh pr create` and git plumbing live elsewhere.

use gwm::config::{PrTemplateConfig, PrTemplateTypeConfig};
use gwm::pr_templates::{render_pr_body, PrTemplateContext};
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

fn ctx(branch_type: &str, issue: &str, desc: &str) -> PrTemplateContext {
  PrTemplateContext {
    branch_type: branch_type.into(),
    issue: issue.into(),
    desc: desc.into(),
    base: "main".into(),
    head: format!("{}/#{}-{}", branch_type, issue, desc),
    commits: "- first commit\n- second commit".into(),
    files_changed: " src/foo.rs | 5 +++++".into(),
    repo: String::new(),
  }
}

#[test]
fn inline_body_substitutes_placeholders() {
  let mut by_type = BTreeMap::new();
  by_type.insert(
    "chore".to_string(),
    PrTemplateTypeConfig {
      path: None,
      body: Some("## Summary\n{desc}\n\nCloses #{issue}\n".into()),
    },
  );
  let cfg = PrTemplateConfig { default: None, by_type };
  let workdir = TempDir::new().unwrap();
  let body = render_pr_body(&cfg, workdir.path(), &ctx("chore", "42", "tidy-imports")).unwrap();
  assert!(body.contains("tidy-imports"), "{body}");
  assert!(body.contains("Closes #42"), "{body}");
  assert!(!body.contains("{desc}"), "unsubstituted placeholder leaked: {body}");
}

#[test]
fn path_resolves_workdir_relative_markdown_and_renders_commits_block() {
  let workdir = TempDir::new().unwrap();
  fs::create_dir_all(workdir.path().join(".github/pr-templates")).unwrap();
  fs::write(
    workdir.path().join(".github/pr-templates/feat.md"),
    "## Summary\n{desc}\n\n## Commits\n{commits}\n\n## Files changed\n{files_changed}\n",
  )
  .unwrap();

  let mut by_type = BTreeMap::new();
  by_type.insert(
    "feat".to_string(),
    PrTemplateTypeConfig {
      path: Some(".github/pr-templates/feat.md".into()),
      body: None,
    },
  );
  let cfg = PrTemplateConfig { default: None, by_type };

  let body = render_pr_body(&cfg, workdir.path(), &ctx("feat", "100", "add-x")).unwrap();
  assert!(body.contains("add-x"), "{body}");
  assert!(body.contains("- first commit"), "{body}");
  assert!(body.contains("- second commit"), "{body}");
  assert!(body.contains("src/foo.rs"), "{body}");
}

#[test]
fn falls_back_to_default_when_no_per_type_entry() {
  let workdir = TempDir::new().unwrap();
  fs::create_dir_all(workdir.path().join(".github")).unwrap();
  fs::write(
    workdir.path().join(".github/pull_request_template.md"),
    "fallback for {type}: {desc}",
  )
  .unwrap();
  let cfg = PrTemplateConfig {
    default: Some(".github/pull_request_template.md".into()),
    by_type: BTreeMap::new(),
  };
  let body = render_pr_body(&cfg, workdir.path(), &ctx("docs", "0", "tweak")).unwrap();
  assert_eq!(body.trim(), "fallback for docs: tweak");
}

#[test]
fn errors_when_no_template_configured_for_branch_type() {
  let cfg = PrTemplateConfig::default();
  let workdir = TempDir::new().unwrap();
  let err = render_pr_body(&cfg, workdir.path(), &ctx("feat", "1", "x")).unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("pr_template"), "{msg}");
  assert!(msg.contains("feat"), "{msg}");
}

#[test]
fn inline_body_wins_over_path_when_both_set() {
  // Per the [`PrTemplateTypeConfig`] docstring: `body` is the most
  // explicit override, so if both are set the inline value wins over
  // the on-disk file. This lets a repo carry a "stable default" body
  // while a `path` placeholder waits to be filled in later.
  let workdir = TempDir::new().unwrap();
  fs::create_dir_all(workdir.path().join(".github/pr-templates")).unwrap();
  fs::write(workdir.path().join(".github/pr-templates/fix.md"), "FROM-FILE: {desc}").unwrap();

  let mut by_type = BTreeMap::new();
  by_type.insert(
    "fix".to_string(),
    PrTemplateTypeConfig {
      path: Some(".github/pr-templates/fix.md".into()),
      body: Some("INLINE: {desc}".into()),
    },
  );
  let cfg = PrTemplateConfig { default: None, by_type };

  let body = render_pr_body(&cfg, workdir.path(), &ctx("fix", "9", "boom")).unwrap();
  assert!(body.starts_with("INLINE: boom"), "{body}");
  assert!(!body.contains("FROM-FILE"), "file body should not appear: {body}");
}

#[test]
fn rejects_path_traversal_in_template_path() {
  let workdir = TempDir::new().unwrap();
  let mut by_type = BTreeMap::new();
  by_type.insert(
    "feat".to_string(),
    PrTemplateTypeConfig {
      path: Some("../etc/passwd.md".into()),
      body: None,
    },
  );
  let cfg = PrTemplateConfig { default: None, by_type };

  let err = render_pr_body(&cfg, workdir.path(), &ctx("feat", "1", "x")).unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("escape") || msg.contains("must be relative"), "{msg}");
}

#[test]
fn rejects_absolute_template_path() {
  let workdir = TempDir::new().unwrap();
  let mut by_type = BTreeMap::new();
  by_type.insert(
    "feat".to_string(),
    PrTemplateTypeConfig {
      path: Some("/etc/passwd.md".into()),
      body: None,
    },
  );
  let cfg = PrTemplateConfig { default: None, by_type };
  let err = render_pr_body(&cfg, workdir.path(), &ctx("feat", "1", "x")).unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("relative") || msg.contains("escape"), "{msg}");
}

#[test]
fn renders_repo_placeholder() {
  // The renderer exposes `{repo}` so a PR body can hard-code paths to
  // the repo's wiki / docs / etc. without the author re-typing the
  // slug.
  let mut by_type = BTreeMap::new();
  by_type.insert(
    "feat".to_string(),
    PrTemplateTypeConfig {
      path: None,
      body: Some("see https://github.com/{repo}/wiki for context".into()),
    },
  );
  let cfg = PrTemplateConfig { default: None, by_type };
  let workdir = TempDir::new().unwrap();
  let mut c = ctx("feat", "1", "x");
  c.repo = "kbrdn1/gwm-cli".into();
  let body = render_pr_body(&cfg, workdir.path(), &c).unwrap();
  assert!(body.contains("kbrdn1/gwm-cli"), "{body}");
}
