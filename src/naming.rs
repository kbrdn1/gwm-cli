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
/// of alphanumeric / dash.
///
/// There is deliberately no `BRANCH_RE` here any more (issue #417): the
/// parser is compiled from `worktree.branch_pattern` by [`BranchParser`],
/// so the shape gwm reads is the shape gwm writes, by construction.
static ISSUE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+$").expect("static ISSUE_RE compiles"));
static DESC_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]*$").expect("static DESC_RE compiles"));

/// Charset each capturing token contributes to the compiled regex.
///
/// These mirror the validators the *formatter* side enforces, so the parser
/// accepts exactly the strings [`BranchSpec::branch_name`] can emit and
/// nothing more: `{issue}` is `ISSUE_RE`, `{desc}` is `DESC_RE`, and `{type}`
/// is the `^[a-z]+$` that `validate_branch_types` pins on every configured
/// name.
///
/// `{type}` is deliberately *not* an alternation of the repo's configured
/// types, which issue #417 proposed. It would narrow the parser to branches
/// the repo can create *today*, so a branch created before a type was retired
/// from `.gwm.toml` would stop being recognised as gwm's — a regression on a
/// name the previous release read fine. Nothing needs the narrowing either:
/// once adjacent placeholders are refused, `[a-z]+` splits every pattern the
/// alternation splits (measured across the whole documented pattern table),
/// and the one consumer that genuinely requires a *configured* type — the TUI
/// rename — checks the resolved list itself and says so precisely.
const TYPE_GROUP: &str = r"(?P<type>[a-z]+)";
const ISSUE_GROUP: &str = r"(?P<issue>\d+)";
const DESC_GROUP: &str = r"(?P<desc>[a-z0-9][a-z0-9-]*)";

/// Every token [`crate::config::expand_placeholders`] substitutes on the branch
/// path, paired with the capture group it compiles to — `None` for the two the
/// formatter resolves to fixed text before a branch name exists.
///
/// `{repo_path}` / `{repo_parent}` are absent on purpose: `BranchSpec::branch_name`
/// passes no `repo_path`, so the formatter leaves them verbatim and so does the
/// compiler. `tests/naming_tests.rs` reads the token list back out of
/// `expand_placeholders` and fails if the two drift apart.
type Token = (&'static str, Option<(&'static str, &'static str)>);
const TOKENS: [Token; 5] = [
  ("{type}", Some(("type", TYPE_GROUP))),
  ("{issue}", Some(("issue", ISSUE_GROUP))),
  ("{desc}", Some(("desc", DESC_GROUP))),
  ("{repo}", None),
  ("{home}", None),
];

/// The three segments a branch name carries, in the order constants are
/// resolved: strictest oracle first, so a token that could serve two segments
/// goes to the one that can be sure about it.
const SEGMENTS: [&str; 3] = ["type", "issue", "desc"];

/// Marks where a placeholder stood in the literal text [`literal_constants`]
/// reads. The two literals in `1{type}2-{desc}` are not one token: fused into
/// `12`, the recovery froze an issue number that no branch the pattern writes
/// ever contains, and auto-linking then pointed at #12. Any character outside
/// `[a-z0-9-]` would do; NUL is the one no branch name can carry.
const SEGMENT_BREAK: char = '\0';

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

/// The reader half of `worktree.branch_pattern` (issue #417).
///
/// [`BranchSpec::branch_name`] writes a branch by expanding the pattern;
/// this compiles the *same* pattern into the regex that reads it back. One
/// source of truth, so a repo that customises `branch_pattern` keeps issue
/// auto-linking, gitmoji, `gwm pr` placeholders, lifecycle hook placeholders
/// and the TUI rename instead of losing them silently.
///
/// The compiler mirrors the exact [`expand_placeholders`] call the formatter
/// makes — `(pattern, repo, Some(type), Some(issue), Some(desc), None)`:
///
/// - `{type}` / `{issue}` / `{desc}` become named capture groups.
/// - `{repo}` and `{home}` become escaped literals, because the formatter
///   substitutes them too and their values are fixed at parse time.
/// - **Everything else is an escaped literal**, including `{repo_path}` /
///   `{repo_parent}`. That is not an oversight: the formatter passes `None`
///   for `repo_path`, so those tokens survive into the branch name verbatim
///   and the parser has to expect them verbatim to round-trip.
///
/// The `shellexpand::tilde` pass the formatter ends with is *not* mirrored, so
/// a pattern starting with `~` does not round-trip. That is reported rather
/// than hidden: [`branch_pattern_warning`] probes the compiled parser and
/// names the loss.
/// A pattern may also **freeze** a segment instead of writing it from a
/// placeholder: `feat/#{issue}-{desc}` hardcodes the type, `{type}/#1-{desc}`
/// hardcodes the issue number. Those are recovered as constants (see
/// [`BranchParser::constants`]), because the previous release read them out of
/// the branch name and dropping them would take gitmoji or auto-linking away
/// from a repo that had them working.
#[derive(Debug, Clone)]
pub struct BranchParser {
  re: Regex,
  /// Segments the pattern freezes as literal text, in [`SEGMENTS`] order.
  /// Never overlaps the regex's capture groups: a segment is either written
  /// by a placeholder or frozen by a literal, never both.
  constants: Vec<(&'static str, String)>,
}

impl BranchParser {
  /// Compile `pattern` into a parser.
  ///
  /// `repo` must be the real repo name ([`crate::worktree::repo_name`]) and
  /// `types` the repo's [`crate::config::Config::resolved_branch_types`] —
  /// both feed the compiled regex, so a stand-in produces a parser that reads
  /// a different repo's branches.
  ///
  /// Two patterns are refused rather than compiled into a parser that reads
  /// back the wrong thing:
  ///
  /// 1. **A boundary between two capturing tokens that can move.** The
  ///    question is never "is there a separator" but "can the split between
  ///    these two land in more than one place", and [`boundary_can_shift`]
  ///    answers it from the charsets. Adjacent placeholders are only refused
  ///    when their alphabets overlap: `{issue}{desc}` reads `42` + `123-x`
  ///    back as `4212` + `3-x`, but `{type}{issue}` is safe, because `[a-z]+`
  ///    stops at the first digit and `\d+` at the first letter. A separator is
  ///    not required to sit outside the left token's charset either —
  ///    `{desc}-{issue}` round-trips, since `\d+` can never contain the `-`
  ///    that would have to reappear after the shift.
  /// 2. **The same capturing token twice.** The formatter's `str::replace`
  ///    substitutes every occurrence, so `{desc}-{desc}` writes `foo-foo`;
  ///    reading that back needs a backreference, which this engine has not
  ///    got, and a second group of the same name will not compile.
  ///
  /// The rule as a whole is pinned by
  /// `tests/naming_tests.rs::the_ambiguity_rule_accepts_exactly_the_patterns_that_round_trip`,
  /// which enumerates the pattern space rather than sampling it: three
  /// separate review findings landed here, and every one of them was a pattern
  /// the hand-picked examples did not cover.
  pub fn compile(pattern: &str, repo: &str, types: &[BranchType]) -> Result<Self> {
    let mut re = String::from("^");
    let mut seen: Vec<&str> = Vec::new();
    // Literal text the *pattern author* wrote, kept for constant recovery.
    // Deliberately not fed by `{repo}` / `{home}`: the user wrote `feat` in
    // `feat/#{issue}-{desc}` and meant it, but nobody chose the repo's name
    // for this purpose, and a repo that happens to be called `docs` must not
    // turn `{repo}/{issue}-{desc}` into a docs-typed branch.
    let mut authored = String::new();
    // The last capture emitted, paired with every literal character emitted
    // since — the separator the next capture would sit behind. An empty string
    // means the two are adjacent, which a `{repo}` expanding to nothing
    // achieves just as surely as writing them side by side, so the state
    // follows the emitted *text* and not the token kind.
    let mut pending: Option<(&'static str, String)> = None;
    let mut rest = pattern;

    while !rest.is_empty() {
      // Find tokens the way `expand_placeholders` does — by searching for each
      // one, not by scanning for `{`. The two disagree the moment a brace sits
      // next to a placeholder: `str::replace` sees `{type}` at offset 1 of
      // `{{type}` and writes `{feat`, whereas a `{`-scanner takes `{{type}` for
      // one unknown token and demands that text back verbatim. Everything the
      // formatter leaves alone — an unknown `{foo}`, an unbalanced brace — is
      // literal here for the same reason.
      let Some((at, token, group)) = TOKENS
        .iter()
        .filter_map(|(token, group)| rest.find(token).map(|at| (at, *token, *group)))
        .min_by_key(|(at, ..)| *at)
      else {
        authored.push_str(rest);
        push_literal(&mut re, rest, &mut pending);
        break;
      };
      if at > 0 {
        authored.push_str(&rest[..at]);
        push_literal(&mut re, &rest[..at], &mut pending);
      }
      rest = &rest[at + token.len()..];

      match group {
        Some((name, group)) => {
          // Before the boundary check, so `{desc}-{desc}` is diagnosed as the
          // repeat it is rather than as a separator it could swallow.
          if seen.contains(&name) {
            return Err(GwmError::Config(format!(
              "worktree.branch_pattern `{}` uses `{{{}}}` more than once; every occurrence expands \
               to the same value, which cannot be read back",
              sanitise_for_terminal(pattern),
              name
            )));
          }
          // The split between the previous capture and this one has to be the
          // only one a branch name admits. Refusing here rather than reporting
          // it later is the point: a mis-split is deterministic, so every probe
          // agrees with it and the pattern would be declared valid while
          // auto-linking targeted the wrong issue.
          if let Some((left, sep)) = pending.as_ref() {
            if boundary_can_shift(left, sep, name) {
              return Err(GwmError::Config(if sep.is_empty() {
                format!(
                  "worktree.branch_pattern `{}` puts `{{{}}}` straight after `{{{}}}` with nothing \
                   between them and both can hold the same characters, so a branch it writes cannot \
                   be read back unambiguously — separate them with a literal (`-`, `_`, `/`, …)",
                  sanitise_for_terminal(pattern),
                  name,
                  left
                )
              } else {
                format!(
                  "worktree.branch_pattern `{}` separates `{{{}}}` from `{{{}}}` with `{}`, which \
                   could be read as part of either, so a branch it writes splits at the wrong place \
                   — separate them with a character neither can contain (`/`, `_`, `#`, `.`, …)",
                  sanitise_for_terminal(pattern),
                  left,
                  name,
                  sanitise_for_terminal(sep)
                )
              }));
            }
          }
          seen.push(name);
          re.push_str(group);
          authored.push(SEGMENT_BREAK);
          pending = Some((name, String::new()));
        }
        // `{repo}` / `{home}`: resolved by the formatter, so fixed text by the
        // time a branch name exists. `{home}` is looked up lazily, since a
        // pattern that does not use it must not fail to compile on a machine
        // with no resolvable `$HOME`.
        //
        // Both break the literal run they sit in: they put real text between
        // the literals either side, so those literals are not one token.
        None => {
          let text = if token == "{home}" {
            dirs::home_dir()
              .ok_or_else(|| GwmError::Config("cannot resolve $HOME".into()))?
              .to_string_lossy()
              .to_string()
          } else {
            repo.to_string()
          };
          authored.push(SEGMENT_BREAK);
          push_literal(&mut re, &text, &mut pending);
        }
      }
    }

    re.push('$');
    let re = Regex::new(&re).map_err(|e| {
      GwmError::Config(format!(
        "worktree.branch_pattern `{}` does not compile into a parser: {}",
        sanitise_for_terminal(pattern),
        e
      ))
    })?;
    let constants = literal_constants(&authored, &seen, types);
    Ok(Self { re, constants })
  }

  /// The parser for a repo's effective config. The single lookup site that
  /// pairs `worktree.branch_pattern` with `resolved_branch_types`, so no
  /// caller has to remember they belong together.
  ///
  /// A pattern that cannot be compiled yields a parser that reads nothing
  /// rather than one that reads the *default* shape: falling back to the
  /// default is exactly the format/parse divergence this issue removes, and
  /// it would put the wrong issue number on a branch instead of none. The
  /// loud report belongs to `gwm doctor` / `gwm config validate`, which call
  /// [`branch_pattern_warning`].
  pub fn from_config(config: &crate::config::Config, repo: &str) -> Self {
    let types = config.resolved_branch_types().types;
    Self::compile(&config.worktree.branch_pattern, repo, &types).unwrap_or_else(|_| Self::inert())
  }

  /// The parser for whatever repo `repo` points at, loading its effective
  /// config (repo layer over global, same as every other runtime read).
  ///
  /// For call sites that hold a repo handle but no [`crate::config::Config`].
  /// Prefer [`Self::from_config`] when a config is already in hand, and hoist
  /// this out of loops — it reads `.gwm.toml` and compiles a regex, so once
  /// per listing rather than once per branch.
  ///
  /// A config that fails to load falls back to the built-in pattern. That is
  /// the pre-#417 behaviour and it is the right one here: a `.gwm.toml` gwm
  /// cannot read is a `.gwm.toml` gwm could not have created a worktree from
  /// either, and it has its own diagnostic in `gwm doctor`.
  pub fn for_repo(repo: &git2::Repository) -> Self {
    let config = repo
      .workdir()
      .and_then(|wd| crate::config::Config::load_for_repo(wd).ok())
      .unwrap_or_default();
    Self::from_config(&config, &crate::worktree::repo_name(repo))
  }

  /// The built-in `{type}/#{issue}-{desc}` parser, for the entry points that
  /// genuinely have no repo to read a config from — `gwm commit-prefix
  /// --branch <name>` run outside a checkout is the only one.
  pub fn builtin() -> &'static Self {
    static BUILTIN: LazyLock<BranchParser> = LazyLock::new(|| {
      BranchParser::compile(&crate::config::default_branch_pattern(), "", &default_branch_types())
        .expect("the default branch_pattern compiles")
    });
    &BUILTIN
  }

  /// Does this parser recover `segment` (`type` / `issue` / `desc`) at all?
  ///
  /// True when the pattern writes it from a placeholder *or* freezes it as a
  /// literal. False means no branch name the pattern produces can carry it: a
  /// permanent absence rather than a parse that goes wrong.
  pub fn reads_segment(&self, segment: &str) -> bool {
    self.re.capture_names().flatten().any(|name| name == segment)
      || self.constants.iter().any(|(name, _)| *name == segment)
  }

  /// The segments this pattern freezes as literal text, `(segment, value)`.
  ///
  /// Disclosure, not decoration: `gwm doctor` names these on its OK line, so a
  /// pattern that quietly pins every branch to one issue number says so rather
  /// than looking like it read one out of the branch.
  pub fn constants(&self) -> &[(&'static str, String)] {
    &self.constants
  }

  /// A parser that matches nothing. `\z\A` can never match: it demands the
  /// end of the haystack before its start.
  fn inert() -> Self {
    Self {
      re: Regex::new(r"\z\A").expect("static inert regex compiles"),
      constants: Vec::new(),
    }
  }

  /// Recover the segments from a branch name, or `None` when the name was not
  /// written by this pattern.
  ///
  /// A segment the pattern neither writes nor freezes comes back empty rather
  /// than blocking the parse: `{type}/{desc}` carries no issue number, and
  /// reporting the type and desc it *does* carry beats reporting nothing.
  /// Callers that need a segment check it — see `gwm commit-prefix`, which is
  /// defined in terms of the type and the issue and says so when either is
  /// missing.
  pub fn parse(&self, branch: &str) -> Option<BranchSpec> {
    let cap = self.re.captures(branch)?;
    let seg = |name: &str| {
      cap
        .name(name)
        .map(|m| m.as_str().to_string())
        .or_else(|| {
          self
            .constants
            .iter()
            .find(|(seg, _)| *seg == name)
            .map(|(_, value)| value.clone())
        })
        .unwrap_or_default()
    };
    Some(BranchSpec {
      type_: seg("type"),
      issue: seg("issue"),
      desc: seg("desc"),
    })
  }
}

/// Append fixed text to the regex under construction, escaping it, and record
/// it as part of the separator behind the capture still open — but only when
/// the text is actually non-empty. `{repo}` in a repo whose name failed to
/// resolve contributes nothing, and pretending it separated two groups would
/// let an ambiguous pattern through.
fn push_literal(re: &mut String, text: &str, pending: &mut Option<(&'static str, String)>) {
  if text.is_empty() {
    return;
  }
  // A separator may arrive in several chunks (`{issue}` `-` `{repo}` `-`
  // `{desc}`), so it accumulates rather than being decided by the first one.
  if let Some((_, sep)) = pending.as_mut() {
    sep.push_str(text);
  }
  re.push_str(&regex::escape(text));
}

/// Can the split between two consecutive captures land in more than one place?
///
/// `left` and `right` are the captures and `sep` the literal text the pattern
/// puts between them, empty when they are adjacent. The formatter writes
/// `left · sep · right`; this asks whether some branch name it produces also
/// reads as a *different* pair.
///
/// With no separator the answer is yes as soon as one character can both end
/// `left` and start `right`: that character crosses the split. `{type}{issue}`
/// is therefore safe — `[a-z]+` stops at the first digit and `\d+` at the first
/// letter — while `{issue}{desc}` is not.
///
/// With a separator, moving the split `d` characters to the right means `left`
/// swallows `sep[..d]`, the separator then has to match `d` characters further
/// along — which requires `sep` to repeat with period `d` — and `right` has to
/// supply the `d` characters of separator that are no longer covered. All three
/// have to hold at once, which is what keeps `{type}-{issue}9-{desc}` legal
/// (`\d+` can eat the `9` but never the `-` that would have to follow it) while
/// `{type}-{issue}9{desc}` is refused.
///
/// Known ceiling: the last test approximates "what `right` can supply" by its
/// charset, which is exact only while the displaced separator is no longer than
/// the value beside it. No combination of the three segment charsets makes a
/// longer one reachable, and the enumeration in `tests/naming_tests.rs` is what
/// checks that claim rather than this comment.
fn boundary_can_shift(left: &str, sep: &str, right: &str) -> bool {
  let sep: Vec<char> = sep.chars().collect();
  if sep.is_empty() {
    return WITNESSES
      .iter()
      .any(|&c| segment_accepts(left, c) && segment_starts(right, c));
  }
  (1..=sep.len()).any(|d| {
    sep[..d].iter().all(|&c| segment_accepts(left, c))
      && sep[..sep.len() - d] == sep[d..]
      && segment_can_start_with(right, &sep[sep.len() - d..])
  })
}

/// One character per class the three capture groups are built from. Every
/// charset is a union of these, so testing them decides any "can both sides
/// hold the same character" question without walking Unicode.
const WITNESSES: [char; 3] = ['a', '0', '-'];

/// Can `segment` *begin* with `c`? Only `desc` differs from
/// [`segment_accepts`]: its tail allows the `-` its first character cannot be.
fn segment_starts(segment: &str, c: char) -> bool {
  match segment {
    "desc" => c.is_ascii_lowercase() || c.is_ascii_digit(),
    _ => segment_accepts(segment, c),
  }
}

/// Can `segment` begin with the whole of `prefix`?
fn segment_can_start_with(segment: &str, prefix: &[char]) -> bool {
  match prefix.split_first() {
    None => true,
    Some((first, rest)) => segment_starts(segment, *first) && rest.iter().all(|&c| segment_accepts(segment, c)),
  }
}

/// Can `segment` contain `c`? Mirrors the charsets the capture groups use, so
/// the boundary check asks about the same characters the regex would match.
fn segment_accepts(segment: &str, c: char) -> bool {
  match segment {
    "type" => c.is_ascii_lowercase(),
    "issue" => c.is_ascii_digit(),
    _ => c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
  }
}

/// Recover the segments a pattern freezes as literal text instead of writing
/// from a placeholder.
///
/// `feat/#{issue}-{desc}` has no `{type}`, yet every branch it writes *is* a
/// `feat` branch, and the release before this one read that back because its
/// hardcoded regex happened to have a group in that position. Dropping it here
/// would take gitmoji, `[pr_template.by_type]` and `gwm commit-prefix` away
/// from a repo where they work today, so the literal is recovered on purpose.
///
/// `authored` is the literal text **the user wrote in the pattern**, never the
/// expansion of `{repo}` / `{home}`: nobody picked the repo's name for this,
/// and a repo that happens to be called `docs` must not turn
/// `{repo}/{issue}-{desc}` into a docs-typed branch.
///
/// Each segment gets its own oracle, applied strictest first so a token that
/// could serve two goes to the one that can be sure of it:
///
/// - **`type`** — an exact match against a configured branch type. Finite
///   list, so `feature/{issue}-{desc}` recovers nothing (the pattern names a
///   namespace, not a type) while `feat/#{issue}-{desc}` recovers `feat`.
/// - **`issue`** — an all-digits token, which nothing else in a branch name
///   is in isolation.
/// - **`desc`** — anything `DESC_RE` accepts, which is a superset of both
///   above, so it runs last on what they left.
///
/// A segment is only recovered when **exactly one** candidate survives. Two
/// candidates mean the pattern is genuinely ambiguous about which literal is
/// the value, and inventing one would be worse than reporting none.
///
/// The `desc` oracle is the weak one, and knowingly so: `wt/{type}/#{issue}`
/// has no description, yet `wt` is the one `DESC_RE`-shaped token in it and is
/// recovered as one. That is why every constant is disclosed by `gwm doctor`
/// rather than applied silently.
fn literal_constants(authored: &str, captured: &[&str], types: &[BranchType]) -> Vec<(&'static str, String)> {
  let tokens = literal_tokens(authored);
  let mut claimed: Vec<usize> = Vec::new();
  let mut out: Vec<(&'static str, String)> = Vec::new();

  for segment in SEGMENTS {
    if captured.contains(&segment) {
      continue;
    }
    // `desc` is the only segment whose charset spans the `-`, so it is the
    // only one that looks at dash-joined runs; `type` and `issue` see single
    // tokens. Running it last means those runs are built from what the
    // stricter oracles left, which is what keeps `feat/#1-fixed` from reading
    // its description as `1-fixed`.
    let candidates: Vec<(Vec<usize>, String)> = if segment == "desc" {
      dash_joined_runs(&tokens, &claimed)
    } else {
      tokens
        .iter()
        .enumerate()
        .filter(|(index, _)| !claimed.contains(index))
        .map(|(index, (text, _))| (vec![index], (*text).to_string()))
        .collect()
    };

    let mut hits = candidates.into_iter().filter(|(_, value)| match segment {
      "type" => types.iter().any(|t| t.name == *value),
      "issue" => ISSUE_RE.is_match(value),
      _ => DESC_RE.is_match(value),
    });
    let (Some((indices, value)), None) = (hits.next(), hits.next()) else {
      continue;
    };
    claimed.extend(indices);
    out.push((segment, value));
  }
  out
}

/// Maximal `[a-z0-9]` runs in the literal text, each paired with whether a
/// single `-` joins it to the run that follows. Everything else in the pattern
/// is a separator.
fn literal_tokens(text: &str) -> Vec<(&str, bool)> {
  let mut out: Vec<(&str, bool)> = Vec::new();
  let mut chars = text.char_indices().peekable();
  while let Some((start, c)) = chars.next() {
    if !(c.is_ascii_lowercase() || c.is_ascii_digit()) {
      continue;
    }
    let mut end = start + c.len_utf8();
    while let Some((index, next)) = chars.peek().copied() {
      if next.is_ascii_lowercase() || next.is_ascii_digit() {
        end = index + next.len_utf8();
        chars.next();
      } else {
        break;
      }
    }
    // A trailing `-`, or a doubled one, does not join anything: it has to be
    // followed by another token for the two to be one value.
    let joined = text[end..].starts_with('-')
      && text[end + 1..]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    out.push((&text[start..end], joined));
  }
  out
}

/// Every maximal run of consecutive unclaimed tokens joined by a single `-`,
/// as `(indices, joined value)`. `feat/#1-fixed` with `feat` and `1` already
/// claimed yields just `fixed`; `{type}/#{issue}-my-fix` yields `my-fix`.
fn dash_joined_runs(tokens: &[(&str, bool)], claimed: &[usize]) -> Vec<(Vec<usize>, String)> {
  let mut runs: Vec<(Vec<usize>, String)> = Vec::new();
  let mut index = 0;
  while index < tokens.len() {
    if claimed.contains(&index) {
      index += 1;
      continue;
    }
    let mut indices = vec![index];
    let mut value = tokens[index].0.to_string();
    while tokens[index].1 && index + 1 < tokens.len() && !claimed.contains(&(index + 1)) {
      index += 1;
      value.push('-');
      value.push_str(tokens[index].0);
      indices.push(index);
    }
    runs.push((indices, value));
    index += 1;
  }
  runs
}

/// What each branch segment feeds, shared by both shapes of the warning.
///
/// Every segment feeds the TUI rename and `cmd_pr`'s template context on top
/// of its own headline consumer — naming only the headline would under-report
/// exactly what this warning promises to name. Two boundaries keep the claim
/// honest:
///
/// - `issue` is specifically *issue* linking. PR/MR detection goes through
///   `Forge::find_pr_for_branch`, which takes the whole branch name and never
///   parses it, so it keeps working whatever the pattern.
/// - hook placeholders break on the **remove / bootstrap** paths only. Those
///   rebuild the context with `HookContext::for_worktree`, which re-parses the
///   branch. `gwm create` uses `HookContext::for_create` and passes the
///   original `BranchSpec` straight through, so its own hooks keep the right
///   `type` / `issue` / `desc` however unreadable the pattern is.
fn segment_consumers(segment: &str) -> &'static str {
  match segment {
    "type" => "gitmoji / `gwm commit-prefix`, `[pr_template.by_type]` selection, remove/bootstrap hook placeholders and the TUI rename",
    "issue" => "issue auto-linking from the branch name, `gwm pr` body placeholders, remove/bootstrap hook placeholders and the TUI rename",
    _ => "`gwm pr` body placeholders, remove/bootstrap hook placeholders and the TUI rename",
  }
}

/// How a segment's consumers fail when the pattern never writes it. Distinct
/// from the mis-parse verbs: nothing here reads the *wrong* value, there is
/// simply no value to read.
fn segment_absent_verb(segment: &str) -> &'static str {
  match segment {
    "type" => "have no branch type to work from",
    "issue" => "have no issue number to work from",
    _ => "have no description to work from",
  }
}

/// Does `worktree.branch_pattern` survive a format/parse round-trip?
///
/// Issue #415 introduced this as damage assessment: the parser was hardcoded,
/// so most customised patterns broke, and the warning's job was to name which
/// features died. Issue #417 derived the parser from the pattern, which fixes
/// the cause. What is left for this predicate is the residue — the two things
/// a derived parser still cannot recover:
///
/// 1. **A pattern that cannot be compiled at all** (adjacent tokens, a
///    repeated token). [`BranchParser::from_config`] falls back to reading
///    nothing rather than to the built-in shape, deliberately; this is the
///    loud half of that silence.
/// 2. **A segment the pattern does not carry.** `{type}/{desc}` writes no
///    issue number, so nothing can read one back, and issue auto-linking is
///    genuinely inactive on that repo. Same for a pattern that hardcodes the
///    type: it is a legitimate convention, and stating what it costs is the
///    whole point of #415.
///
/// The check is an actual probe, never a comparison against the default
/// string: "differs from the default" and "breaks the parser" were never the
/// same set, and since #417 they barely overlap. `{type}-{issue}-{desc}`,
/// `{type}_{issue}_{desc}` and `{desc}-{issue}` are all customised and all
/// round-trip.
///
/// Returns the user-facing warning naming what actually breaks, or `None`
/// when the pattern round-trips. This is the single predicate both
/// `gwm doctor` and `gwm config validate` consume.
///
/// `repo` must be the real repo name ([`crate::worktree::repo_name`]) and
/// `types` the repo's [`crate::config::Config::resolved_branch_types`]: both
/// feed the compiled parser, so a stand-in returns a verdict about a
/// different repo's branches.
///
/// **Invariant: this function reports what it observed, and never
/// generalises.** Every message is phrased over "the N branch shapes probed".
/// The probe set is *classes worth probing*, not an exhaustive space; a class
/// it misses can only make the counts smaller, never make the statement
/// false. Since the parser is now derived, the probe's remaining job is to
/// catch what the compiler does not mirror — the `shellexpand::tilde` pass
/// the formatter ends with is the known one.
///
/// - `type` — every configured branch type. Finite, so this one *is*
///   exhaustive, and a type gwm would refuse to create is excluded.
/// - `issue` — `ISSUE_RE` is `\d+`: single-digit and multi-digit.
/// - `desc` — `DESC_RE` is `[a-z0-9][a-z0-9-]*`: a plain word, one carrying
///   the `-` it allows, one all-digits, and one that starts with digits and
///   then carries a `-`. Those last two are what tell an ambiguous adjacency
///   apart from a merely unusual separator.
pub fn branch_pattern_warning(pattern: &str, repo: &str, types: &[BranchType]) -> Option<String> {
  const ISSUES: [&str; 2] = ["7", "42"];
  const DESCS: [&str; 4] = ["probe", "probe-desc", "123", "123-probe"];

  // A pattern that cannot be compiled reads *nothing* (see
  // `BranchParser::from_config`), so this is the loud half of that silence
  // rather than a separate class of problem. The compile error already names
  // the pattern and the reason.
  let parser = match BranchParser::compile(pattern, repo, types) {
    Ok(p) => p,
    Err(e) => return Some(format!("{}", e)),
  };

  // A segment the pattern cannot supply at all is a different report from a
  // segment that reads back wrong, and conflating them was misleading in both
  // directions: saying "N of the shapes probed read back the wrong type" both
  // over-quantifies a permanent absence and hides the one-line fix. Ask the
  // compiled parser rather than re-scanning the pattern string, so the verdict
  // comes from the artefact that does the reading — and so a segment the
  // pattern *freezes* as a literal counts as supplied, because it is.
  let missing: Vec<&str> = SEGMENTS.into_iter().filter(|seg| !parser.reads_segment(seg)).collect();

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
        match parser.parse(&formatted) {
          None => {
            unparseable.get_or_insert(formatted);
          }
          Some(back) => {
            parsed += 1;
            // A segment the pattern omits is reported by `missing`; counting
            // it here too would tally the same loss twice, in the shape that
            // describes it least well.
            let (t, i, d) = (
              !missing.contains(&"type") && back.type_ != type_,
              !missing.contains(&"issue") && back.issue != issue,
              !missing.contains(&"desc") && back.desc != desc,
            );
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
  if missing.is_empty() && unparsed == 0 && lossy == 0 {
    return None;
  }

  // Counts are always scoped to the shapes probed. There is no branch that
  // says "every branch created with this pattern", because that is precisely
  // the claim the probe set cannot support. The `missing` part is the one
  // exception, and it earns it: a placeholder the pattern does not contain
  // is absent from every name it will ever write, no probing required.
  let mut parts: Vec<String> = Vec::new();
  if !missing.is_empty() {
    let tokens = missing.iter().map(|s| format!("`{{{}}}`", s)).collect::<Vec<_>>();
    let losses = missing
      .iter()
      .map(|seg| format!("{} {}", segment_consumers(seg), segment_absent_verb(seg)))
      .collect::<Vec<_>>();
    parts.push(format!(
      "it carries no {}, so {} — write {} into the pattern to get them back",
      tokens.join(" and "),
      losses.join("; "),
      tokens.join(" and ")
    ));
  }
  if let Some(example) = unparseable {
    parts.push(format!(
      "{} of the {} branch shapes probed match nothing at all (e.g. `{}`), so issue auto-linking from the branch name, gitmoji / `gwm commit-prefix`, `gwm pr` template selection and placeholders, remove/bootstrap hook placeholders, the TUI rename and the branch-convention check are inactive on those (PR/MR detection is unaffected — it queries the forge with the full branch name)",
      unparsed,
      probes,
      sanitise_for_terminal(&example)
    ));
  }
  if lossy > 0 {
    let mut broken: Vec<String> = Vec::new();
    for (flag, seg, verb) in [
      (bad_type, "type", "read the wrong branch type"),
      (bad_issue, "issue", "target the wrong issue"),
      (bad_desc, "desc", "see the wrong description"),
    ] {
      if flag {
        broken.push(format!("`{}`, so {} {}", seg, segment_consumers(seg), verb));
      }
    }
    parts.push(format!(
      "{} of the {} branch shapes probed parse but read back {}",
      lossy,
      probes,
      broken.join("; ")
    ));
  }

  Some(format!(
    "worktree.branch_pattern `{}` does not round-trip: {}",
    sanitise_for_terminal(pattern),
    parts.join("; and ")
  ))
}

/// Neutralise control characters before echoing a config-supplied value.
///
/// `branch_pattern` comes from a repo's `.gwm.toml`, and none of the commands
/// that quote it — `gwm doctor`, `gwm config validate`, `gwm commit-prefix` —
/// goes through the TOFU trust gate, because running them inside a repo you
/// have not vetted is meant to be safe. Echoing the raw value would hand an
/// untrusted `.gwm.toml` a terminal escape channel (an OSC 52 clipboard write,
/// a title rewrite, cursor games). Same idiom as
/// [`crate::tui::wt_tree::sanitize_name`]: replace, don't strip, so the value
/// stays recognisable and its length is not silently altered.
///
/// `pub(crate)` rather than private because every site that quotes a
/// config-supplied pattern has to use it; a second copy would be a second
/// thing to forget.
pub(crate) fn sanitise_for_terminal(s: &str) -> String {
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
