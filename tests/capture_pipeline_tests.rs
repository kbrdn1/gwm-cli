//! Integration tests for the preflight half of `docs/_capture/generate.sh`
//! (issue #631).
//!
//! The script grew three gates that a release depends on, and all three fail
//! in the direction that looks like success: a stale binary produces captures
//! that exist and are correctly sized, a dirty trunk produces a capture of the
//! release in progress, a missing PR produces an empty pane. None of it is
//! visible downstream, which is why the gates exist and why they are exercised
//! here rather than read.
//!
//! `tests/docs_assets_tests.rs::the_capture_run_keeps_its_order` pins the
//! *order* of the phases by reading the script. This suite pins what they
//! *do*, by running it: same split as `precommit_hook_tests.rs`, which drives
//! `.githooks/pre-commit` against stubbed tools.
//!
//! Scope: the script stops at `GWM_CAPTURE_PREFLIGHT_ONLY=1`, right after the
//! gates. The tapes past that point need a real terminal, a Nerd Font and the
//! demo fixture, so they stay out of the suite and are checked by looking at
//! the pixels they produce.

#![cfg(unix)]

mod common;

use common::git_only_bin;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn script() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/_capture/generate.sh")
}

fn write_exec(path: &Path, body: &str) {
  fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
  fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// The fixture's own git calls, run in isolation from the machine's config.
///
/// `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_NOSYSTEM`: a `commit.gpgsign = true` or a
/// global `core.hooksPath` would otherwise reach into this repo and ask for a
/// signature, or run somebody's hook, before the case under test starts.
/// Setting the identity locally covers neither.
fn git(repo: &Path, args: &[&str]) {
  let status = Command::new(git_only_bin().join("git"))
    .args(args)
    .current_dir(repo)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_NOSYSTEM", "1")
    .status()
    .expect("git ran");
  assert!(status.success(), "git {args:?} failed");
}

/// What the fixture lets a case vary.
struct Fixture {
  /// The version `Cargo.toml` declares.
  manifest_version: &'static str,
  /// What the stubbed `vhs` writes as `gwm --version`, or `None` to write
  /// nothing at all (the tape that exits 0 having produced no file).
  stamped_version: Option<&'static str>,
  /// The path the stubbed `vhs` reports `gwm` resolved to, relative to the
  /// repo. `None` means the one cargo just built, which is the happy path.
  resolved_relative: Option<&'static str>,
  /// Leave an untracked file behind, so the trunk is dirty.
  dirty: bool,
  /// What the stubbed `gh` answers: a PR number, or nothing (no open PR).
  open_pr: Option<&'static str>,
}

impl Default for Fixture {
  fn default() -> Self {
    Self {
      manifest_version: "9.9.9",
      stamped_version: Some("9.9.9"),
      resolved_relative: None,
      dirty: false,
      open_pr: Some("42"),
    }
  }
}

/// A throwaway repo shaped like this one, with `cargo`, `vhs` and `gh` stubbed,
/// and the real `generate.sh` copied in. Returns the run's output.
///
/// `git` is deliberately *not* stubbed: the gates ask it which checkout is the
/// trunk and whether that trunk is clean, and a stub would only restate the
/// answer the test wants.
fn run_preflight(fx: Fixture) -> (TempDir, Output) {
  let dir = TempDir::new().unwrap();
  let repo = dir.path();

  let bin = repo.join("stub-bin");
  fs::create_dir_all(&bin).unwrap();
  fs::create_dir_all(repo.join("docs/_capture")).unwrap();

  // The binary `cargo` claims to have produced, under a `target/` of its own
  // rather than next to the stubs: the script prepends its directory to PATH,
  // which is the whole point of the phase, and a name other than `gwm` would
  // not be one cargo could report.
  let built = repo.join("target/release/gwm");
  fs::create_dir_all(built.parent().unwrap()).unwrap();
  write_exec(&built, "echo stub");
  // A second gwm at the same version, standing in for the one a ~/.bashrc puts
  // ahead of the build. Only the path tells the two apart.
  let decoy = repo.join("elsewhere/gwm");
  fs::create_dir_all(decoy.parent().unwrap()).unwrap();
  write_exec(&decoy, "echo stub");

  // `cargo build --message-format=json` is how the script learns where the
  // binary landed; the plain build is a no-op.
  write_exec(
    &bin.join("cargo"),
    &format!(
      r#"case "$*" in
  *--message-format=json*) echo '{{"reason":"compiler-artifact","executable":"{}"}}' ;;
esac
exit 0"#,
      built.display()
    ),
  );

  // `vhs` stands in for the version stamp: the one thing the preflight reads
  // out of it is the file the tape leaves behind.
  let resolved = fx.resolved_relative.unwrap_or("target/release/gwm");
  let vhs = match fx.stamped_version {
    Some(v) => format!(
      "echo \"$@\" >> \"$PWD/docs/_capture/.tmp/vhs.log\"\nprintf 'gwm {v}\\n' > \"$PWD/docs/_capture/.tmp/version.txt\"\nprintf '%s\\n' \"$PWD/{resolved}\" > \"$PWD/docs/_capture/.tmp/which.txt\"\nexit 0"
    ),
    // vhs exits 0 whether or not it wrote what the tape asked for, which is
    // the failure the `-s` check exists for.
    None => "echo \"$@\" >> \"$PWD/docs/_capture/.tmp/vhs.log\"\nexit 0".to_string(),
  };
  write_exec(&bin.join("vhs"), &vhs);

  write_exec(
    &bin.join("gh"),
    match fx.open_pr {
      Some(n) => format!("echo {n}"),
      None => "exit 1".to_string(),
    }
    .as_str(),
  );

  fs::write(
    repo.join("Cargo.toml"),
    format!("[package]\nname = \"gwm-cli\"\nversion = \"{}\"\n", fx.manifest_version),
  )
  .unwrap();
  fs::copy(script(), repo.join("docs/_capture/generate.sh")).unwrap();
  // Named by the script; the stubbed vhs never reads them.
  fs::write(repo.join("docs/_capture/version-stamp.tape"), "").unwrap();
  fs::write(repo.join("docs/_capture/github-linking.tape"), "").unwrap();
  // As in the real tree, and load-bearing here: without them the stamp under
  // `.tmp/` and the build output under `target/` are untracked files, every
  // case reads the trunk as dirty, and the happy path would assert nothing.
  fs::write(repo.join("docs/_capture/.gitignore"), ".tmp/\n").unwrap();
  fs::write(repo.join(".gitignore"), "/target/\n/stub-bin/\n/elsewhere/\n").unwrap();

  git(repo, &["init", "-q", "."]);
  git(repo, &["config", "user.email", "gwm@test"]);
  git(repo, &["config", "user.name", "gwm-test"]);
  git(repo, &["add", "-A"]);
  git(repo, &["commit", "-qm", "fixture"]);
  if fx.dirty {
    fs::write(repo.join("uncommitted.txt"), "in the shot\n").unwrap();
  }

  // The stubs first, then a git that has been checked to work, then the system
  // pair: the script resolves `cargo`, `vhs`, `gh` and `git` through this and
  // nothing else, so no case can be decided by what happens to be installed.
  let path = format!("{}:{}:/usr/bin:/bin", bin.display(), git_only_bin().display());
  let out = Command::new("bash")
    .arg("docs/_capture/generate.sh")
    .current_dir(repo)
    .env_clear()
    .env("PATH", path)
    .env("HOME", repo)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_NOSYSTEM", "1")
    .env("GWM_CAPTURE_PREFLIGHT_ONLY", "1")
    .output()
    .expect("generate.sh ran");
  (dir, out)
}

fn stdout(out: &Output) -> String {
  String::from_utf8_lossy(&out.stdout).to_string()
}

/// The happy path reaches the end of the preflight and shoots github-linking.
#[test]
fn a_clean_trunk_with_an_open_pr_captures_github_linking() {
  let (dir, out) = run_preflight(Fixture::default());
  let text = stdout(&out);
  assert!(
    out.status.success(),
    "preflight should pass: {text}\n{}",
    String::from_utf8_lossy(&out.stderr)
  );
  assert!(
    text.contains("captured by gwm 9.9.9") && text.contains("target/release/gwm"),
    "the stamp should report the version and the file it came from: {text}"
  );
  assert!(
    text.contains("PR #42"),
    "the PR it captures against should be named: {text}"
  );
  let log = fs::read_to_string(dir.path().join("docs/_capture/.tmp/vhs.log")).unwrap();
  assert!(
    log.contains("github-linking.tape"),
    "github-linking.tape should have run: {log}"
  );
}

/// A binary that reports another version stops the run before it captures.
///
/// The trap this closes is the release cut in the wrong order: regenerate
/// before the bump and every TUI capture paints the previous version, with
/// nothing downstream failing.
#[test]
fn a_version_mismatch_stops_the_run() {
  let (_dir, out) = run_preflight(Fixture {
    stamped_version: Some("1.2.3"),
    ..Default::default()
  });
  let text = stdout(&out);
  assert!(!out.status.success(), "a mismatch must fail the run: {text}");
  assert!(
    text.contains("vhs resolves 'gwm 1.2.3'") && text.contains("gwm 9.9.9"),
    "both versions should be named: {text}"
  );
}

/// `vhs` exiting 0 without writing the stamp is not an empty answer.
#[test]
fn a_stamp_that_was_never_written_stops_the_run() {
  let (_dir, out) = run_preflight(Fixture {
    stamped_version: None,
    ..Default::default()
  });
  let text = stdout(&out);
  assert!(!out.status.success(), "a missing stamp must fail the run: {text}");
  assert!(
    text.contains("version-stamp.tape produced nothing"),
    "the run should say the tape produced nothing: {text}"
  );
}

/// The same version out of a different binary is still the wrong binary.
///
/// This is the shape the v1.10.0 near miss had: a build 175 commits behind
/// carrying the manifest's version number, which answers the version check
/// perfectly and paints a UI the tree never had. Only the path separates them.
#[test]
fn a_gwm_that_is_not_the_build_stops_the_run() {
  let (_dir, out) = run_preflight(Fixture {
    resolved_relative: Some("elsewhere/gwm"),
    ..Default::default()
  });
  let text = stdout(&out);
  assert!(!out.status.success(), "a shadowed build must fail the run: {text}");
  assert!(
    text.contains("elsewhere/gwm") && text.contains("Same version is not the same binary"),
    "the run should name both files: {text}"
  );
}

/// A dirty trunk skips the capture, out loud, and finishes.
#[test]
fn a_dirty_trunk_skips_github_linking_and_says_so() {
  let (dir, out) = run_preflight(Fixture {
    dirty: true,
    ..Default::default()
  });
  let text = stdout(&out);
  assert!(out.status.success(), "a skip is not a failure: {text}");
  assert!(
    text.contains("is not clean") && text.contains("Working Tree pane is the shot"),
    "the skip should name its reason: {text}"
  );
  let log = fs::read_to_string(dir.path().join("docs/_capture/.tmp/vhs.log")).unwrap();
  assert!(
    !log.contains("github-linking.tape"),
    "github-linking.tape must not run against a dirty trunk: {log}"
  );
}

/// So does a branch with no open PR: the pane would come back empty, and vhs
/// would exit 0 over it.
#[test]
fn no_open_pr_skips_github_linking_and_says_so() {
  let (dir, out) = run_preflight(Fixture {
    open_pr: None,
    ..Default::default()
  });
  let text = stdout(&out);
  assert!(out.status.success(), "a skip is not a failure: {text}");
  assert!(text.contains("no open PR"), "the skip should name its reason: {text}");
  let log = fs::read_to_string(dir.path().join("docs/_capture/.tmp/vhs.log")).unwrap();
  assert!(
    !log.contains("github-linking.tape"),
    "github-linking.tape must not run without a PR to show: {log}"
  );
}
