use crate::config::{Config, IssueTemplateConfig, IssueTemplateTypeConfig};
use crate::error::{GwmError, Result};
use crate::naming::{kebab, sanitise_diagnostic_for_terminal};
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
  let template_name = template_name(config, branch_type).ok_or_else(|| {
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

  let title_prefix = title_prefix(type_config, meta.title.as_deref());
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

/// The template file `gwm new` renders for `branch_type`: the per-type
/// override, else `[issue_template].default`.
fn template_name<'a>(config: &'a Config, branch_type: &str) -> Option<&'a str> {
  config
    .issue_template
    .by_type
    .get(branch_type)
    .and_then(|cfg| cfg.template.as_deref())
    .or(config.issue_template.default.as_deref())
}

/// The prefix `gwm new` puts in front of the `<desc>` it types into an issue
/// title: the configured `title_prefix`, else the issue form's own `title:`.
///
/// One combinator for both directions of the flow (issue #617). `gwm create
/// --issue` has to take back off exactly what `gwm new` put on, and the
/// fallback to the form's `title:` is the half a reverse derivation written
/// from `[issue_template]` alone would miss — on a repo that leaves
/// `title_prefix` unset the two would then produce different slugs for the
/// same title.
fn title_prefix(type_config: Option<&IssueTemplateTypeConfig>, form_title: Option<&str>) -> String {
  type_config
    .and_then(|cfg| cfg.title_prefix.as_deref())
    .or(form_title)
    .unwrap_or_default()
    .to_string()
}

/// [`title_prefix`] resolved from disk, for the reverse direction — which has
/// no rendered draft to read the form metadata off.
///
/// A template that cannot be read contributes no prefix. The derivation then
/// keeps the whole title, which is wrong in the cosmetic way (a `feature-`
/// rider on the slug) rather than in the way that loses the issue, and is the
/// right trade for a repo whose `[issue_template]` points at a file that has
/// since moved.
pub fn title_prefix_for(repo: &Repository, config: &Config, branch_type: &str) -> String {
  let form_title = repo
    .workdir()
    .zip(template_name(config, branch_type))
    .and_then(|(workdir, name)| resolve_template_path(workdir, name).ok())
    .and_then(|path| std::fs::read_to_string(path).ok())
    .and_then(|raw| templating::issue_form_metadata(&raw).ok())
    .and_then(|meta| meta.title);
  title_prefix(config.issue_template.by_type.get(branch_type), form_title.as_deref())
}

/// Longest `<desc>` derived from an issue title, in characters.
///
/// The slug feeds `path_pattern` alongside `{type}` and `{issue}`, so it
/// becomes part of a directory name; 48 leaves the rest of that name well
/// inside the 255-byte path component limit `WorktreeName::freeform` already
/// pins, and is longer than any `<desc>` this repo's own branch history
/// carries. The cap applies to the **derived** slug only: a hand-typed
/// `<desc>` is still taken exactly as typed.
pub const DERIVED_DESC_MAX: usize = 48;

/// The branch type whose `[issue_template.by_type.<type>].labels` the issue's
/// labels select — `gwm new`'s type-to-labels map, read backwards (#617).
///
/// A type declaring no labels is never a candidate. An empty list says
/// nothing about which issues belong to the type, and reading it as "matches
/// everything" would hand the first such type every issue in the repo.
///
/// Nothing is guessed: zero matches and two matches are both refusals that
/// name what was seen and point at `--type`.
pub fn type_from_labels(config: &IssueTemplateConfig, labels: &[String]) -> Result<String> {
  let declared: Vec<(&str, &Vec<String>)> = config
    .by_type
    .iter()
    .filter(|(_, cfg)| !cfg.labels.is_empty())
    .map(|(name, cfg)| (name.as_str(), &cfg.labels))
    .collect();
  if declared.is_empty() {
    return Err(GwmError::Config(
      "no [issue_template.by_type.*] declares `labels`, so the branch type cannot be derived from the issue: pass --type <TYPE>".into(),
    ));
  }

  // Label names are arbitrary text from the forge, so the echo is sanitised
  // even though the comparison is not affected by it.
  let seen = sanitise_diagnostic_for_terminal(&labels.join(", "));
  let matched: Vec<&str> = declared
    .iter()
    .filter(|(_, want)| {
      want
        .iter()
        .any(|w| labels.iter().any(|got| got.eq_ignore_ascii_case(w)))
    })
    .map(|(name, _)| *name)
    .collect();

  match matched.as_slice() {
    [only] => Ok((*only).to_string()),
    [] => Err(GwmError::Config(format!(
      "issue labels [{}] match no [issue_template.by_type.*].labels: pass --type <TYPE>",
      seen
    ))),
    many => Err(GwmError::Config(format!(
      "issue labels [{}] do not separate the branch types they match ({}): pass --type <TYPE> to choose",
      seen,
      many.join(", ")
    ))),
  }
}

/// The `<desc>` an issue title yields: the prefix `gwm new` writes taken back
/// off, then the same kebab-case normalisation a hand-typed `<desc>` gets
/// (#617).
///
/// A title is arbitrary text from the forge, so this is a sanitisation
/// boundary: it goes *through* [`kebab`] rather than around it, which is what
/// leaves the bidi and control-character work of #500 / #502 / #503 covering
/// this path too — `kebab` keeps ASCII alphanumerics and nothing else.
pub fn desc_from_title(title: &str, title_prefix: &str) -> Result<String> {
  let stripped = if title_prefix.is_empty() {
    title
  } else {
    title.strip_prefix(title_prefix).unwrap_or(title)
  };
  let slug = truncate_at_word(&kebab(stripped), DERIVED_DESC_MAX);
  if slug.is_empty() {
    return Err(GwmError::Other(format!(
      "issue title '{}' has no characters a branch description can be built from: pass <TYPE> <ISSUE> <DESC> instead",
      sanitise_diagnostic_for_terminal(title)
    )));
  }
  Ok(slug)
}

/// Cut `slug` back to `max` characters on a word boundary.
///
/// `kebab` output is ASCII, so byte and character indices coincide and the
/// slicing below cannot split a code point. Cutting back to the last dash
/// keeps whole words; a first word longer than the cap has no dash to cut
/// back to and is hard-cut rather than yielding an empty slug.
fn truncate_at_word(slug: &str, max: usize) -> String {
  if slug.len() <= max {
    return slug.to_string();
  }
  let cut = slug[..max].rfind('-').unwrap_or(max);
  slug[..cut].to_string()
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
