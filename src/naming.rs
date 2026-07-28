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

/// How a worktree got its name (issue #416).
///
/// [`Self::Structured`] is the canonical `<type>/#<issue>-<desc>` triple that
/// `branch_pattern` / `path_pattern` expand. [`Self::Freeform`] is a name the
/// user simply chose — `gwm create --name spike-redis` — and it deliberately
/// escapes the convention: no branch type, no issue, no `DESC_RE`.
///
/// The patterns do not apply to a free-form name, because they are written in
/// terms of `{type}` / `{issue}` / `{desc}` and it has none of them. `base`
/// still applies, so a free-form worktree lands beside the structured ones
/// rather than somewhere else — but only for the placeholders it documents
/// (`{home}` / `{repo}` / `{repo_path}` / `{repo_parent}`). The structured
/// path also feeds `{type}` / `{issue}` / `{desc}` through `base`; a `base`
/// written with one of those is refused here rather than expanded literally.
/// Max bytes in a single path component. `NAME_MAX` is 255 on every
/// filesystem gwm targets; git's own ref check is silent about length,
/// so the worktree directory is the binding constraint.
///
/// Public because the TUI create form has to stop typing at exactly this
/// number: a form that stopped short would silently truncate a name the
/// validator would have accepted, and submit a different branch than the
/// one that was typed.
pub const MAX_DIR_COMPONENT_BYTES: usize = 255;

/// Bytes git tacks onto the ref's **final** component while creating it:
/// `refs/heads/<name>.lock` has to exist before `refs/heads/<name>` does.
/// Earlier components are plain directories and carry no suffix.
const GIT_REF_LOCK_SUFFIX_BYTES: usize = ".lock".len();

/// The `base` placeholders only the structured triple can supply. A
/// free-form name has no value for any of them, and `expand_placeholders`
/// leaves an unfed placeholder literal, so a `base` written with one of
/// these has to be refused rather than turned into a directory called
/// `{type}`.
const STRUCTURED_BASE_PLACEHOLDERS: [&str; 3] = ["{type}", "{issue}", "{desc}"];

#[derive(Debug, Clone)]
pub enum WorktreeName {
  Structured(BranchSpec),
  /// Already validated by [`WorktreeName::freeform`] — constructing this
  /// variant directly bypasses the ref/path checks.
  Freeform(String),
}

impl WorktreeName {
  /// Validate a user-supplied free-form name.
  ///
  /// The bar is deliberately low. `Spike_Redis`, `2026.07.27` and
  /// `réécriture` are all fine — refusing them would defeat the point of the
  /// flag. The rules are enumerated from the three things the name has to be
  /// at once, rather than accreted one reviewer example at a time:
  ///
  /// 1. **A git branch.** Delegated to libgit2's own
  ///    [`git2::Branch::name_is_valid`] — the branch-level oracle, not the
  ///    reference-level one, because they disagree: `refs/heads/HEAD` is a
  ///    syntactically valid *reference* name while `HEAD` is not a usable
  ///    *branch* name. The authority on what git accepts is git.
  /// 2. **A single filesystem path component** — the worktree directory. A
  ///    branch name is a *path* of components, so the two have different
  ///    limits: bounded at [`MAX_DIR_COMPONENT_BYTES`], and no `.` / `..`.
  /// 3. **A literal value in placeholder expansion.**
  ///    `lifecycle::expand_placeholders` substitutes sequentially, so a name
  ///    that itself contains a token gets rewritten inside its own
  ///    substituted value.
  ///
  /// Plus one rule that belongs to none of them: no leading `-`, which is a
  /// CLI-ergonomics rule (git accepts it).
  ///
  /// What is deliberately **not** checked is the Windows-specific character
  /// and reserved-device-name set (`< > " |`, `CON`, `NUL`, `COM1`…). That
  /// gap is real and tracked separately — it cannot be verified from a Unix
  /// machine, and a rule that only runs on one platform's CI is a rule
  /// nobody can pre-validate.
  ///
  /// The name is validated exactly as typed. Trimming would accept
  /// `--name " spike"` and create `spike` instead — a different branch from
  /// the one that was asked for, which the "the name becomes the branch"
  /// contract does not allow.
  pub fn freeform(input: &str) -> Result<Self> {
    let name = input;
    let reject = |reason: &str| {
      Err(GwmError::InvalidWorktreeName {
        name: input.to_string(),
        reason: reason.to_string(),
      })
    };

    if name.is_empty() {
      return reject("empty");
    }
    // Checked before the ref oracle so the message points at the real
    // problem: git happily accepts `..` inside a longer component, but a
    // worktree directory named `..` would escape the base directory.
    if name.split('/').any(|part| part == "." || part == "..") {
      return reject("`.` and `..` are not usable as a directory name");
    }
    if name.contains('\0') {
      return reject("contains a NUL byte");
    }
    // Not a git rule: libgit2 accepts `-x` as a branch name quite happily
    // (verified — the oracle below lets it through). It is an *ergonomics*
    // rule. A leading `-` makes the name unusable as an argument to every
    // command that would take it back: `gwm remove -x`, `git branch -d -x`,
    // `cd -x` all read it as a flag.
    if name.starts_with('-') {
      return reject("a leading `-` makes the name unusable as a command argument");
    }
    // Consumer (3): `lifecycle::expand_placeholders` replaces `{branch}`
    // first and `{type}` / `{issue}` / `{desc}` / `{repo}` after, so a hook
    // asking for `{branch}` on `spike-{issue}` receives `spike-` — the
    // second pass rewrites the value the first one just substituted.
    // `DESC_RE` kept structured names out of this; free-form names reach it,
    // so the boundary is where it gets closed.
    if name.contains('{') || name.contains('}') {
      return reject("`{` and `}` would be re-substituted when a lifecycle hook expands its placeholders");
    }
    // A ref is a *path* of components, a worktree directory is a single one,
    // so `a×130/b×130` is a legal ref and an illegal directory name. Without
    // this check `worktree::add` creates the branch, then fails on the
    // directory and leaves the branch orphaned. `/` → `-` is 1:1, so the
    // flattened dirname has exactly the name's byte length.
    if name.len() > MAX_DIR_COMPONENT_BYTES {
      return reject(&format!(
        "{} bytes long — a worktree directory is a single path component, capped at {}",
        name.len(),
        MAX_DIR_COMPONENT_BYTES
      ));
    }
    // The ref side of the same limit, five bytes tighter on the final
    // component only: git creates `refs/heads/<name>.lock` first, and
    // `Branch::name_is_valid` checks syntax, never length. Measured: a
    // 250-byte final segment creates, 251 fails — after `pre_create` hooks
    // have run. Earlier segments are plain directories, so they keep the
    // full budget; capping them too would refuse names git accepts.
    let last = name.rsplit('/').next().unwrap_or(name);
    if last.len() + GIT_REF_LOCK_SUFFIX_BYTES > MAX_DIR_COMPONENT_BYTES {
      return reject(&format!(
        "its last segment is {} bytes — git writes `refs/heads/<name>.lock` first, leaving {} for it",
        last.len(),
        MAX_DIR_COMPONENT_BYTES - GIT_REF_LOCK_SUFFIX_BYTES
      ));
    }

    // Consumer (1). The branch-level oracle, not `Reference::is_valid_name`
    // on `refs/heads/<name>`: that one accepts `HEAD`, which `git branch`
    // refuses and which would collide with the HEAD pseudo-ref.
    if !git2::Branch::name_is_valid(name).unwrap_or(false) {
      return reject(
        "not a valid git branch name — git rejects spaces, `~ ^ : ? * [ \\`, `@{`, `..`, \
         leading/trailing `/`, a trailing `.`, a `.lock` suffix and `HEAD`",
      );
    }

    Ok(Self::Freeform(name.to_string()))
  }

  /// The branch this worktree gets. Structured names expand
  /// `branch_pattern`; free-form names are the branch.
  pub fn branch_name(&self, cfg: &WorktreeConfig, repo: &str) -> Result<String> {
    match self {
      Self::Structured(spec) => spec.branch_name(cfg, repo),
      Self::Freeform(name) => Ok(name.clone()),
    }
  }

  /// The worktree directory name. A branch may carry `/`; a directory is a
  /// single path component, so it flattens to `-` — the same relationship
  /// the default `branch_pattern` / `path_pattern` pair already has
  /// (`feat/#42-x` on disk is `feat-42-x`).
  pub fn worktree_dirname(&self, cfg: &WorktreeConfig, repo: &str) -> Result<String> {
    match self {
      Self::Structured(spec) => spec.worktree_dirname(cfg, repo),
      Self::Freeform(name) => Ok(name.replace('/', "-")),
    }
  }

  /// Absolute worktree path: `base` (expanded) joined with the directory
  /// name. `base` applies in both modes.
  pub fn worktree_path(
    &self,
    cfg: &WorktreeConfig,
    repo: &str,
    repo_path: &std::path::Path,
  ) -> Result<std::path::PathBuf> {
    match self {
      Self::Structured(spec) => spec.worktree_path(cfg, repo, repo_path),
      Self::Freeform(_) => {
        if let Some(ph) = STRUCTURED_BASE_PLACEHOLDERS.iter().find(|ph| cfg.base.contains(**ph)) {
          return Err(GwmError::Config(format!(
            "worktree.base `{}` uses `{}`, which a free-form name has no value for \
             (it would be left literal in the path) — drop it from base, or create \
             the worktree with <type> <issue> <desc>",
            cfg.base, ph
          )));
        }
        let base = expand_placeholders(&cfg.base, repo, None, None, None, Some(repo_path))?;
        Ok(std::path::PathBuf::from(base).join(self.worktree_dirname(cfg, repo)?))
      }
    }
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
/// `types` must be the repo's [`crate::config::Config::resolved_branch_types`]
/// for the same reason: a type gwm would refuse to create must not produce a
/// warning about branches that cannot exist.
///
/// **Invariant: this function reports what it observed, and never
/// generalises.** Whether a pattern round-trips depends on the values put
/// through it, and which values matter depends on the pattern — the
/// separator ambiguity of `{desc}-{issue}` is not the ambiguity of
/// `{issue}{desc}`. That space cannot be closed by any fixed value set, only
/// by deriving the parser from the pattern (#417). So the probe set is
/// deliberately described as *classes worth probing*, not as exhaustive, and
/// every message is phrased over "the N branch shapes probed". A class this
/// set happens to miss can only make the counts smaller — it can never make
/// the statement false.
///
/// - `type` — every configured branch type. Finite, so this one *is*
///   exhaustive, and a type gwm would refuse to create is excluded.
/// - `issue` — `ISSUE_RE` is `\d+`: single-digit and multi-digit.
/// - `desc` — `DESC_RE` is `[a-z0-9][a-z0-9-]*`: a plain word, one carrying
///   the `-` it allows, one all-digits, and one that starts with digits and
///   then carries a `-`. The dash collides with a literal separator; leading
///   digits get swallowed by `BRANCH_RE`'s `\d+` issue group
///   (`{type}/#{issue}{desc}` parses only for a desc starting with digits).
pub fn branch_pattern_warning(pattern: &str, repo: &str, types: &[BranchType]) -> Option<String> {
  const ISSUES: [&str; 2] = ["7", "42"];
  const DESCS: [&str; 4] = ["probe", "probe-desc", "123", "123-probe"];

  let (mut unparseable, mut parsed, mut lossy) = (None::<String>, 0usize, 0usize);
  let (mut bad_type, mut bad_issue, mut bad_desc) = (false, false, false);
  let mut probes = 0usize;

  // Probe only types `gwm create` would actually accept. `merge_layered`
  // deserialises `[[branch_types]]` without running `validate_branch_types`,
  // so the effective list can carry a name like `Feat` that the config
  // validator rejects outright — probing it would report the *default*
  // pattern as broken, because `BRANCH_RE`'s `[a-z]+` cannot match it. The
  // invalid config is reported by its own check; this one stays quiet
  // rather than blaming the pattern for it.
  let usable = types
    .iter()
    .map(|t| t.name.as_str())
    .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase()));

  for type_ in usable {
    for issue in ISSUES {
      for desc in DESCS {
        // A pattern that does not expand at all is a different, *loud*
        // failure: `gwm create` errors on it outright. Not our business.
        let formatted = expand_placeholders(pattern, repo, Some(type_), Some(issue), Some(desc), None).ok()?;
        probes += 1;
        match parse_branch(&formatted) {
          None => {
            unparseable.get_or_insert(formatted);
          }
          Some(back) => {
            parsed += 1;
            let (t, i, d) = (back.type_ != type_, back.issue != issue, back.desc != desc);
            // `lossy` counts probes, the flags accumulate across them. The
            // distinction matters: the flags say *which* segments can come
            // back wrong somewhere, `lossy` says *how many* shapes they came
            // back wrong on. Reporting the flags as if they held for every
            // parsed probe is the over-claim this counter exists to stop —
            // `{desc}/#{issue}-{type}` has probes that round-trip perfectly
            // alongside probes that swap two segments.
            lossy += usize::from(t || i || d);
            bad_type |= t;
            bad_issue |= i;
            bad_desc |= d;
          }
        }
      }
    }
  }

  // No configured types at all would leave the verdict unprobed — say
  // nothing rather than guess. `resolved_branch_types` never yields this
  // (it falls back to the built-ins), so it is a guard, not a path.
  if probes == 0 {
    return None;
  }

  let unparsed = probes - parsed;
  if unparsed == 0 && lossy == 0 {
    return None;
  }

  // One message shape, always a count over the probed shapes. There is no
  // branch that says "every branch created with this pattern", because that
  // is precisely the claim the probe set cannot support.
  let mut parts: Vec<String> = Vec::new();
  if let Some(example) = unparseable {
    parts.push(format!(
      "{} match nothing at all (e.g. `{}`), so issue auto-linking from the branch name, gitmoji / `gwm commit-prefix`, `gwm pr` template selection and placeholders, remove/bootstrap hook placeholders, the TUI rename and the branch-convention check are inactive on those (PR/MR detection is unaffected — it queries the forge with the full branch name)",
      unparsed,
      sanitise_for_terminal(&example)
    ));
  }
  if lossy > 0 {
    // Every segment feeds the TUI rename and `cmd_pr`'s template context on
    // top of its own headline consumer — naming only the headline would
    // under-report exactly what this warning promises to name. Two
    // boundaries keep the claim honest:
    //
    // - `issue` is specifically *issue* linking. PR/MR detection goes through
    //   `Forge::find_pr_for_branch`, which takes the whole branch name and
    //   never parses it, so it keeps working whatever the pattern.
    // - hook placeholders break on the **remove / bootstrap** paths only.
    //   Those rebuild the context with `HookContext::for_worktree`, which
    //   re-parses the branch. `gwm create` uses `HookContext::for_create` and
    //   passes the original `BranchSpec` straight through, so its own hooks
    //   keep the right `type` / `issue` / `desc` however unreadable the
    //   pattern is.
    let mut broken: Vec<&str> = Vec::new();
    if bad_type {
      broken.push(
        "`type`, so gitmoji / `gwm commit-prefix`, `[pr_template.by_type]` selection, remove/bootstrap hook placeholders and the TUI rename read the wrong branch type",
      );
    }
    if bad_issue {
      broken.push(
        "`issue`, so issue auto-linking from the branch name, `gwm pr` body placeholders, remove/bootstrap hook placeholders and the TUI rename target the wrong issue",
      );
    }
    if bad_desc {
      broken.push(
        "`desc`, so `gwm pr` body placeholders, remove/bootstrap hook placeholders and the TUI rename see the wrong description",
      );
    }
    parts.push(format!("{} parse but read back {}", lossy, broken.join("; ")));
  }

  Some(format!(
    "worktree.branch_pattern `{}` does not round-trip: of the {} branch shapes probed, {}",
    sanitise_for_terminal(pattern),
    probes,
    parts.join("; and ")
  ))
}

/// Neutralise control characters before echoing a config-supplied value.
///
/// `branch_pattern` comes from a repo's `.gwm.toml`, and neither `gwm doctor`
/// nor `gwm config validate` goes through the TOFU trust gate — running
/// either inside a repo you have not vetted is meant to be safe. Echoing the
/// raw value would hand an untrusted `.gwm.toml` a terminal escape channel
/// (an OSC 52 clipboard write, a title rewrite, cursor games). Same idiom as
/// [`crate::tui::wt_tree::sanitize_name`]: replace, don't strip, so the value
/// stays recognisable and its length is not silently altered.
fn sanitise_for_terminal(s: &str) -> String {
  s.chars().map(|c| if c.is_control() { '?' } else { c }).collect()
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
