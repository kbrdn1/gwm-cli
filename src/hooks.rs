//! Git hook installer (issue #85).
//!
//! Currently exposes a single hook: `commit-msg`. The installed script
//! shells out to `gwm commit-prefix --unicode` and auto-prepends the
//! resolved Gitmoji + Conventional Commits prefix when the user's
//! commit message doesn't already start with one.
//!
//! Design:
//! - **Opt-in** — `gwm` never installs hooks implicitly. The
//!   `gwm hooks install` subcommand is the only entry point.
//! - **Non-destructive by default** — refuses to overwrite a
//!   pre-existing `commit-msg` (husky, commitlint, pre-commit, …)
//!   unless `--force` is passed.
//! - **Linked-worktree aware** — `gwm`'s primary use case is linked
//!   worktrees whose `.git` is a file pointing at the real admin dir
//!   (`<main>/.git/worktrees/<name>/`). The installer resolves the
//!   effective gitdir via `git2::Repository::discover` rather than
//!   blindly joining `.git/hooks/` to the workdir path.
//! - **`core.hooksPath` aware** — when the repo config sets a custom
//!   hooks directory (this project even recommends
//!   `git config core.hooksPath .githooks`), the installer writes
//!   into that directory. Writing into `.git/hooks/` while
//!   `core.hooksPath` is set is dead code: git never invokes hooks
//!   from the default location once the override is in play.
//! - **Self-contained, best-effort script** — the generated POSIX
//!   `sh` script uses only widely-available tools (`grep`, `sed`,
//!   `printf`, `cat`, `mv`, `mktemp`, `command -v`). It deliberately
//!   does NOT enable `set -e`: a transient filesystem failure must
//!   not abort the user's commit. The shell-out to
//!   `gwm commit-prefix` degrades gracefully when `gwm` is not on
//!   `$PATH` (silent no-op rather than blocking the commit).
//!
//! The script is materialised by [`commit_msg_script`] and
//! installed by [`install_commit_msg`]. Tests against the rendered
//! script live in `tests/hooks_tests.rs` so a regression on the
//! detection clause (the "is the message already prefixed?" guard)
//! surfaces immediately rather than at the user's next `git commit`.

use crate::error::{GwmError, Result};
use std::path::{Path, PathBuf};

/// Install `commit-msg` for the repository rooted at `repo_root`. The
/// effective target directory is resolved as follows:
///
/// 1. Open the repository via `git2::Repository::discover` so a
///    linked worktree's `.git` file pointer is followed transparently.
/// 2. If `core.hooksPath` is set in the repo config (the project's
///    recommended setup is `core.hooksPath = .githooks`), resolve it
///    against the workdir and install there. The directory is created
///    if missing.
/// 3. Otherwise fall back to `<gitdir>/hooks/` (where `<gitdir>` is
///    the worktree's admin dir — e.g.
///    `<main>/.git/worktrees/<name>/` for a linked worktree, or
///    `<root>/.git/` for the main worktree).
///
/// Returns the written path on success. When `force` is `false`
/// (default) and the target file already exists, returns
/// `GwmError::Other("commit-msg hook already exists at … — pass
/// --force to overwrite")` without touching the file. Bare repos and
/// non-git dirs are rejected with an error; the goal is "fail
/// loudly, leak nothing into the filesystem".
pub fn install_commit_msg(repo_root: &Path, force: bool) -> Result<PathBuf> {
  let repo = git2::Repository::discover(repo_root).map_err(|_| {
    GwmError::Other(format!(
      "no git repository discovered from {} — `gwm hooks install` must be run from inside a git repo",
      repo_root.display()
    ))
  })?;

  let hooks_dir = resolve_hooks_dir(&repo)?;
  std::fs::create_dir_all(&hooks_dir)?;

  let hook_path = hooks_dir.join("commit-msg");
  if hook_path.exists() && !force {
    return Err(GwmError::Other(format!(
      "commit-msg hook already exists at {} — pass --force to overwrite",
      hook_path.display()
    )));
  }

  std::fs::write(&hook_path, commit_msg_script())?;

  // Mark the hook executable. Git refuses to run a non-executable
  // hook silently — which would be the worst failure mode for a
  // "commit-msg" hook: the user thinks their commits are getting
  // auto-prefixed, but git is just skipping the hook entirely.
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&hook_path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&hook_path, perms)?;
  }

  Ok(hook_path)
}

/// Resolve the directory git will actually load hooks from. Priority
/// order matches git itself: `core.hooksPath` (if set) > `<gitdir>/hooks`.
///
/// `core.hooksPath` is resolved against the workdir when it's
/// relative (matches git's own behaviour — `git config
/// core.hooksPath .githooks` puts hooks at `<repo>/.githooks/`, not
/// `<cwd>/.githooks/`). Absolute paths are honoured verbatim.
fn resolve_hooks_dir(repo: &git2::Repository) -> Result<PathBuf> {
  // `Repository::config` opens a snapshot view that includes the
  // global / system / repo layers in priority order — exactly what
  // git uses when invoking hooks at commit time.
  if let Ok(cfg) = repo.config() {
    if let Ok(custom) = cfg.get_string("core.hooksPath") {
      let trimmed = custom.trim();
      if !trimmed.is_empty() {
        let candidate = Path::new(trimmed);
        let resolved = if candidate.is_absolute() {
          candidate.to_path_buf()
        } else if let Some(wd) = repo.workdir() {
          wd.join(candidate)
        } else {
          // Bare repo + relative `core.hooksPath` — exceedingly rare
          // and ill-defined in git itself. Fall through to the
          // default gitdir-based location so we at least produce a
          // deterministic path.
          repo.path().join("hooks")
        };
        return Ok(resolved);
      }
    }
  }
  // `repo.path()` returns the gitdir — `<main>/.git/` for the main
  // worktree, `<main>/.git/worktrees/<name>/` for a linked one. This
  // is the path git itself uses to locate the default hooks dir.
  Ok(repo.path().join("hooks"))
}

/// Return the body of the generated `commit-msg` hook. Exposed as a
/// pure function so tests can assert on the rendered script without
/// touching the filesystem, and so future hook variants (commit-msg,
/// pre-push, …) can share helpers from this module.
///
/// The script:
/// 1. Reads the in-progress commit message (path passed by git as `$1`).
/// 2. Locates the *first non-empty non-comment* line — git's own
///    commit template puts `# Please enter the commit message…`
///    above the user's first real line, and `git commit -v` appends
///    a diff dump prefixed with `#`. Both are stripped before the
///    "is the message already prefixed?" check.
/// 3. Bails out (exit 0) if that line already starts with an
///    emoji-ish prefix (a `case` match against the `:shortcode:`
///    form + a `grep -E` for unicode emoji codepoints).
/// 4. Shells out to `gwm commit-prefix --unicode`. If `gwm` is
///    missing from `$PATH`, or the shell-out fails for any reason,
///    the hook exits 0 cleanly — the goal is "never block a commit
///    because the hook itself broke".
/// 5. Prepends the resolved prefix + a space to the message and
///    writes the result back to the same file. Every filesystem
///    step here is guarded by `|| exit 0` so a transient failure
///    (full /tmp, noexec mount, read-only fs, …) lets the original
///    commit through unmodified rather than aborting it.
pub fn commit_msg_script() -> String {
  // The raw string literal keeps escaping minimal. We deliberately do
  // NOT enable `set -e`: the script is best-effort, and any failure
  // path must return 0 so `git commit` proceeds with the user's
  // original message. `set -u` is kept so referencing an unset
  // variable surfaces as a real bug (rather than silently producing
  // empty output).
  r#"#!/bin/sh
# gwm commit-msg hook — auto-prepends the gitmoji + type prefix when
# the commit message doesn't already start with one. Installed by
# `gwm hooks install commit-msg` (issue #85). Re-running the installer
# with --force overwrites this file.
#
# Best-effort by design: every filesystem step is guarded so a
# transient failure (full /tmp, noexec mount, …) never blocks the
# commit — the user just loses the auto-prefix for that one commit.

set -u

# Skip when `gwm` isn't on $PATH — never block a commit because of us.
if ! command -v gwm >/dev/null 2>&1; then
  exit 0
fi

msg_file="${1:-}"
[ -n "$msg_file" ] || exit 0
[ -f "$msg_file" ] || exit 0

# Find the first non-empty / non-comment line. `grep -nvE` returns
# `<lineno>:<text>` for every line that is NOT pure whitespace AND NOT
# a `#`-prefixed comment; we take the first match. Falling back to an
# empty string lets the downstream check no-op cleanly when the buffer
# only contains the git template (i.e. the user aborted with an empty
# message — git itself will reject that commit, no need for us to).
first_line="$(grep -nvE '^([[:space:]]*#|[[:space:]]*$)' "$msg_file" 2>/dev/null | sed -n '1s/^[0-9]*://p')"
[ -n "$first_line" ] || exit 0

# Already-prefixed messages are passed through untouched. We detect
# both the shortcode form (`:sparkles: feat(…)`) and the unicode form
# (✨ feat(…)) — any non-space first byte that looks like an emoji
# fence covers the latter. The shortcode check is tighter so common
# `:foo:` mentions in PR / issue subjects don't false-positive.
case "$first_line" in
  :[a-z_]*:\ *) exit 0 ;;
esac

# Unicode emoji check via grep: the leading character is well into
# the BMP, so a `[^[:alnum:][:space:][:punct:]]` heuristic catches it
# without needing a full emoji table.
if printf '%s' "$first_line" | grep -qE '^[^[:alnum:][:space:][:punct:]]'; then
  exit 0
fi

prefix="$(gwm commit-prefix --unicode 2>/dev/null)" || exit 0
[ -n "$prefix" ] || exit 0

# Prepend `<prefix> ` + the original body. Every fs step below is
# guarded with `|| exit 0` so a failure (read-only mount, full /tmp,
# noexec /tmp, …) leaves the user's commit message intact rather than
# aborting the commit.
tmp_file="$(mktemp "${TMPDIR:-/tmp}/gwm-commit-msg.XXXXXX" 2>/dev/null)" || exit 0
{ printf '%s ' "$prefix" > "$tmp_file"; } || { rm -f "$tmp_file"; exit 0; }
cat "$msg_file" >> "$tmp_file" 2>/dev/null || { rm -f "$tmp_file"; exit 0; }
mv "$tmp_file" "$msg_file" 2>/dev/null || { rm -f "$tmp_file"; exit 0; }
"#
  .to_string()
}
