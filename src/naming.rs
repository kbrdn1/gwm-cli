use crate::config::{expand_placeholders, BranchType, WorktreeConfig};
use crate::error::{GwmError, Result};
use regex::Regex;
use std::sync::LazyLock;

/// Compile-time literal regexes lifted to module statics so each branch
/// validation / parse runs at ~50ns instead of recompiling the pattern
/// per call (issue #97). `LazyLock::new` defers the work until the
/// first access; `expect` is acceptable here because the input is a
/// hard-coded literal — a regex-compile failure would be a developer
/// bug caught by the test suite at first use, not a user-facing error
/// path the CLAUDE.md "no unwrap on user paths" rule targets.
///
/// `ISSUE_RE` pins the digits-only contract on issue numbers (no
/// scientific notation, no hex, no leading zeros stripped). `DESC_RE`
/// matches the post-`kebab` shape — leading alphanumeric, then a tail
/// of alphanumeric / dash. `BRANCH_RE` captures the three segments of
/// a gwm-style branch (`<type>/#<issue>-<desc>`) in one pass.
static ISSUE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+$").expect("static ISSUE_RE compiles"));
static DESC_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]*$").expect("static DESC_RE compiles"));
static BRANCH_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^([a-z]+)/#(\d+)-([a-z0-9-]+)$").expect("static BRANCH_RE compiles"));

/// Built-in branch types — the fallback when `.gwm.toml` carries no
/// `[[branch_types]]` block. Kept as a `&[(&str, &str)]` const so the
/// static string table stays compile-time and zero-alloc; the runtime
/// view is materialised on demand via [`default_branch_types`].
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

/// Runtime view of [`BRANCH_TYPES`] as a `Vec<BranchType>`. Used by
/// [`crate::config::Config::resolved_branch_types`] when no override
/// is configured, and by [`BranchSpec::validate`] / [`BranchSpec::new`]
/// to keep the legacy "no config = built-in defaults" contract.
pub fn default_branch_types() -> Vec<BranchType> {
  BRANCH_TYPES
    .iter()
    .map(|(name, desc)| BranchType {
      name: (*name).into(),
      description: (*desc).into(),
    })
    .collect()
}

#[derive(Debug, Clone)]
pub struct BranchSpec {
  pub type_: String,
  pub issue: String,
  pub desc: String,
}

impl BranchSpec {
  /// Construct a [`BranchSpec`] validated against the built-in branch
  /// types. Kept for callers (tests, internal helpers) that don't have
  /// a [`crate::config::Config`] in scope; production code paths
  /// (`gwm create`, TUI create) should use [`Self::new_with_types`]
  /// with the resolved list so per-repo overrides are honoured.
  pub fn new(type_: impl Into<String>, issue: impl Into<String>, desc: impl Into<String>) -> Result<Self> {
    Self::new_with_types(type_, issue, desc, &default_branch_types())
  }

  /// Construct a [`BranchSpec`] validated against the supplied list of
  /// allowed branch types — typically the output of
  /// [`crate::config::Config::resolved_branch_types`].
  pub fn new_with_types(
    type_: impl Into<String>,
    issue: impl Into<String>,
    desc: impl Into<String>,
    allowed: &[BranchType],
  ) -> Result<Self> {
    let s = Self {
      type_: type_.into(),
      issue: issue.into(),
      desc: kebab(&desc.into()),
    };
    s.validate_against(allowed)?;
    Ok(s)
  }

  /// Validate against the built-in branch types. Convenience wrapper
  /// around [`Self::validate_against`] for legacy call sites.
  pub fn validate(&self) -> Result<()> {
    self.validate_against(&default_branch_types())
  }

  /// Validate against the supplied list of allowed branch types. The
  /// error message produced when the type is rejected enumerates the
  /// allowed names so the TUI status bar / CLI stderr always shows the
  /// repo-local truth (built-in or `.gwm.toml`-driven).
  pub fn validate_against(&self, allowed: &[BranchType]) -> Result<()> {
    if !allowed.iter().any(|t| t.name == self.type_) {
      let names = allowed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ");
      return Err(GwmError::InvalidBranchType {
        got: self.type_.clone(),
        allowed: names,
      });
    }
    if !ISSUE_RE.is_match(&self.issue) {
      return Err(GwmError::InvalidIssue(self.issue.clone()));
    }
    if !DESC_RE.is_match(&self.desc) {
      return Err(GwmError::InvalidDescription(self.desc.clone()));
    }
    Ok(())
  }

  pub fn branch_name(&self, cfg: &WorktreeConfig, repo: &str) -> Result<String> {
    expand_placeholders(
      &cfg.branch_pattern,
      repo,
      Some(&self.type_),
      Some(&self.issue),
      Some(&self.desc),
      None,
    )
  }

  pub fn worktree_dirname(&self, cfg: &WorktreeConfig, repo: &str) -> Result<String> {
    expand_placeholders(
      &cfg.path_pattern,
      repo,
      Some(&self.type_),
      Some(&self.issue),
      Some(&self.desc),
      None,
    )
  }

  /// Resolve the absolute worktree path for this spec. `repo_path` is the
  /// main repo's working directory on disk — it feeds the `{repo_path}` /
  /// `{repo_parent}` placeholders so a `base` like `{repo_parent}/worktrees`
  /// can be expressed relative to the repo (matching, e.g., an editor's
  /// `../worktrees` convention).
  pub fn worktree_path(
    &self,
    cfg: &WorktreeConfig,
    repo: &str,
    repo_path: &std::path::Path,
  ) -> Result<std::path::PathBuf> {
    let base = expand_placeholders(
      &cfg.base,
      repo,
      Some(&self.type_),
      Some(&self.issue),
      Some(&self.desc),
      Some(repo_path),
    )?;
    let dir = self.worktree_dirname(cfg, repo)?;
    Ok(std::path::PathBuf::from(base).join(dir))
  }
}

/// Try to recover a BranchSpec from a free-form branch name like `feat/#123-my-desc`.
pub fn parse_branch(branch: &str) -> Option<BranchSpec> {
  let cap = BRANCH_RE.captures(branch)?;
  Some(BranchSpec {
    type_: cap.get(1)?.as_str().to_string(),
    issue: cap.get(2)?.as_str().to_string(),
    desc: cap.get(3)?.as_str().to_string(),
  })
}

/// Issue #415 — `worktree.branch_pattern` drives [`BranchSpec::branch_name`]
/// (formatting) but not [`parse_branch`], which matches the hardcoded
/// `BRANCH_RE`. When the two disagree, every feature keyed on the re-parsed
/// segments quietly reads the wrong thing: gitmoji selection off `type`
/// (`cli.rs` commit prefix), issue/PR auto-linking off `issue`, lifecycle
/// hook placeholders and the TUI rename off `desc`, plus the `doctor`
/// branch-convention check which only asks whether the name parses at all.
///
/// The check is an actual round-trip probe, not a comparison against the
/// default string: "differs from the default" and "breaks the parser" are
/// not the same set. `{type}/#{issue}-prefix-{desc}` is customised yet
/// still yields `feat/#42-…`, so `type` and `issue` survive and only
/// `desc` comes back wrong — claiming auto-linking is dead there would be
/// false, and a warning whose whole value is accuracy cannot afford that.
///
/// Returns the user-facing warning naming what actually breaks, or `None`
/// when the pattern round-trips. This is the single predicate both
/// `gwm doctor` and `gwm config validate` consume; issue #417 derives the
/// parser from the pattern and replaces this body without moving either
/// call site.
/// `repo` must be the real repo name ([`crate::worktree::repo_name`]), not a
/// placeholder: `{repo}` is a supported token, and a name carrying a `-`
/// (`gwm-cli`) is rejected by `BRANCH_RE`'s `[a-z]+` type charset while a
/// dummy `repo` sails through. The verdict is genuinely repo-dependent.
pub fn branch_pattern_warning(pattern: &str, repo: &str) -> Option<String> {
  // Two probes, no value shared between them. One is not enough: a pattern
  // that hardcodes a segment — `feat/#{issue}-{desc}` — round-trips
  // perfectly against a probe that happens to use `feat`, while
  // `gwm create fix …` writes a `feat/` branch and reads back the wrong
  // type. Distinct values on every segment collapse that false negative.
  // Each desc carries the `-` that `DESC_RE` allows, so a greedy-adjacency
  // split shows up here rather than in the user's branch names.
  const PROBES: [(&str, &str, &str); 2] = [("feat", "42", "probe-desc"), ("chore", "7", "other-slug")];

  let mut unparseable: Option<String> = None;
  let (mut bad_type, mut bad_issue, mut bad_desc) = (false, false, false);

  for (type_, issue, desc) in PROBES {
    // A pattern that does not expand at all is a different, *loud* failure:
    // `gwm create` errors on it outright. Not this warning's business.
    let formatted = expand_placeholders(pattern, repo, Some(type_), Some(issue), Some(desc), None).ok()?;
    match parse_branch(&formatted) {
      None => unparseable.get_or_insert(formatted),
      Some(back) => {
        bad_type |= back.type_ != type_;
        bad_issue |= back.issue != issue;
        bad_desc |= back.desc != desc;
        continue;
      }
    };
  }

  if let Some(formatted) = unparseable {
    return Some(format!(
      "worktree.branch_pattern `{}` produces branch names gwm cannot parse back (`{}` matches nothing); issue/PR auto-linking, gitmoji, lifecycle hook placeholders, the TUI rename and the branch-convention check will be inactive on branches created with this pattern",
      pattern, formatted
    ));
  }

  // Every segment feeds `HookContext::for_worktree` (hook placeholders) and
  // the TUI rename, on top of its own headline consumer — naming only the
  // headline would under-report exactly what this warning promises to name.
  let mut broken: Vec<&str> = Vec::new();
  if bad_type {
    broken
      .push("`type`, so gitmoji selection, lifecycle hook placeholders and the TUI rename read the wrong branch type");
  }
  if bad_issue {
    broken
      .push("`issue`, so issue/PR auto-linking, lifecycle hook placeholders and the TUI rename target the wrong issue");
  }
  if bad_desc {
    broken.push("`desc`, so lifecycle hook placeholders and the TUI rename see the wrong description");
  }
  if broken.is_empty() {
    return None;
  }
  Some(format!(
    "worktree.branch_pattern `{}` does not round-trip: gwm reads back {}",
    pattern,
    broken.join("; ")
  ))
}

pub fn kebab(input: &str) -> String {
  // Lowercase, then collapse every non-alphanumeric run into a single `-`.
  let lower = input.to_lowercase();
  let mut out = String::with_capacity(lower.len());
  let mut prev_dash = false;
  for c in lower.chars() {
    if c.is_ascii_alphanumeric() {
      out.push(c);
      prev_dash = false;
    } else if !prev_dash && !out.is_empty() {
      out.push('-');
      prev_dash = true;
    }
  }
  out.trim_matches('-').to_string()
}
