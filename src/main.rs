use clap::Parser;
use gwm::{aliases, cli};
use std::ffi::OsString;

fn main() {
  // Issue #86: expand CLI aliases BEFORE clap parses argv. We need
  // the repo workdir to read `.gwm.toml`'s `[aliases]` block, so the
  // discovery happens here in `main` rather than inside `cli::run` —
  // by the time `run()` is reached, clap has already rejected an
  // unknown subcommand and the expansion is moot.
  //
  // Graceful degradation: any error reading the alias config is
  // surfaced on stderr but does not abort the process — we fall
  // back to the raw argv. This is the same conservative shape used
  // by `Config::load_for_repo` in `cmd_doctor` (`.unwrap_or_default()`),
  // and it matches the issue's "no breaking change" promise:
  // absence of `[aliases]` ⇒ aliasing disabled, full stop.
  //
  // argv is read as `OsString` via `std::env::args_os()` — `args()`
  // panics on any non-UTF-8 argv entry, which is a regression vs.
  // clap's default `args_os` handling and could abort the binary on
  // a perfectly valid OS argv. The alias expansion path only attempts
  // UTF-8 decoding on the alias-slot token (alias keys are `String`
  // by construction); non-UTF-8 tokens flow through untouched and
  // clap reports the unknown subcommand verbatim.
  let argv: Vec<OsString> = std::env::args_os().collect();
  let expanded = match expand_aliases(argv.clone()) {
    Ok(v) => v,
    Err(e) => {
      // Print the error but keep going with the original argv so a
      // typo in `[aliases]` doesn't lock the user out of `gwm doctor`
      // or `gwm aliases list` (which would otherwise be the only
      // way to surface the typo). The dispatcher will re-surface
      // the error if the user runs a subcommand that touches
      // config.
      eprintln!("warning: failed to load aliases — using raw argv: {}", e);
      argv
    }
  };

  let args = cli::Cli::parse_from(expanded);
  if let Err(e) = cli::run(args) {
    eprintln!("error: {}", e);
    std::process::exit(1);
  }
}

/// Resolve the alias chain (built-in + repo + user) and run the
/// single-pass expansion on `argv`. Returns `Ok(argv)` unchanged when
/// the first non-flag token doesn't match an alias.
///
/// Repo discovery uses the current working directory — same gate as
/// every other repo-bound subcommand. Outside a repo we still load
/// the user-level file so `gwm wip` works even when the user is
/// not in a git tree (matching git's own `[alias]` behaviour).
fn expand_aliases(argv: Vec<OsString>) -> Result<Vec<OsString>, gwm::error::GwmError> {
  let repo_workdir = gwm::worktree::discover_repo(None)
    .ok()
    .and_then(|r| r.workdir().map(|w| w.to_path_buf()));
  let resolved = aliases::load(repo_workdir.as_deref(), None)?;
  Ok(aliases::expand_argv_os(argv, &resolved))
}
