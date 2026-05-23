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
//! - **Self-contained script** — the generated POSIX `sh` script
//!   uses only `grep` + `sed`, both POSIX-mandated. The shell-out
//!   to `gwm commit-prefix` degrades gracefully when `gwm` is not
//!   on `$PATH` (silent no-op rather than blocking the commit).
//!
//! The script is materialised by [`commit_msg_script`] and
//! installed by [`install_commit_msg`]. Tests against the rendered
//! script live in `tests/hooks_tests.rs` so a regression on the
//! detection clause (the "is the message already prefixed?" guard)
//! surfaces immediately rather than at the user's next `git commit`.

use crate::error::{GwmError, Result};
use std::path::{Path, PathBuf};

/// Install `.git/hooks/commit-msg` under `repo_root`. Returns the
/// written path on success. When `force` is `false` (default) and a
/// `commit-msg` already exists, returns
/// `GwmError::Other("commit-msg hook already exists at … — pass
/// --force to overwrite")` without touching the file.
///
/// Requires `repo_root` to point at a workdir whose `.git` directory
/// exists. Bare repos and non-git dirs are rejected with an error;
/// the goal is "fail loudly, leak nothing into the filesystem".
pub fn install_commit_msg(repo_root: &Path, force: bool) -> Result<PathBuf> {
  let git_dir = repo_root.join(".git");
  if !git_dir.exists() {
    return Err(GwmError::Other(format!(
      "no .git directory found at {} — `gwm hooks install` must be run from inside a git repo",
      repo_root.display()
    )));
  }
  // Bare repos have a directory at `.git`-the-path but no `hooks/`
  // subdir hierarchy in the same way; even if it existed, hooks on
  // a bare repo are server-side, not commit-time. We let the
  // existence check above pass, but the subsequent `hooks_dir`
  // creation handles the rest uniformly.
  let hooks_dir = git_dir.join("hooks");
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

/// Return the body of the generated `commit-msg` hook. Exposed as a
/// pure function so tests can assert on the rendered script without
/// touching the filesystem, and so future hook variants (commit-msg,
/// pre-push, …) can share helpers from this module.
///
/// The script:
/// 1. Reads the in-progress commit message (path passed by git as `$1`).
/// 2. Bails out (exit 0) if the first non-empty line already starts
///    with an emoji-ish prefix (`grep -E` against a small set of
///    unicode codepoints + the `:shortcode:` form).
/// 3. Shells out to `gwm commit-prefix --unicode`. If `gwm` is
///    missing from `$PATH`, the hook exits 0 cleanly — the goal is
///    "never block a commit because the hook itself broke".
/// 4. Prepends the resolved prefix + a space to the message and
///    writes the result back to the same file.
pub fn commit_msg_script() -> String {
  // The raw string literal keeps escaping minimal. `\\$` would turn
  // into `\$` (shell var literal); we use a regular `$` so the
  // generated file is hand-readable. `grep -E` is mandated by POSIX
  // and present everywhere git is.
  r#"#!/bin/sh
# gwm commit-msg hook — auto-prepends the gitmoji + type prefix when
# the commit message doesn't already start with one. Installed by
# `gwm hooks install commit-msg` (issue #85). Re-running the installer
# with --force overwrites this file.

set -eu

# Skip when `gwm` isn't on $PATH — never block a commit because of us.
if ! command -v gwm >/dev/null 2>&1; then
  exit 0
fi

msg_file="$1"
[ -f "$msg_file" ] || exit 0

first_line="$(sed -n '1p' "$msg_file")"

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

# Prepend `<prefix> ` + the original body. Using printf so a missing
# trailing newline doesn't fold the user's first line into the prefix.
tmp_file="$(mktemp "${TMPDIR:-/tmp}/gwm-commit-msg.XXXXXX")"
printf '%s ' "$prefix" > "$tmp_file"
cat "$msg_file" >> "$tmp_file"
mv "$tmp_file" "$msg_file"
"#
  .to_string()
}
