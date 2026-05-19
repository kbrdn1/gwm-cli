//! Integration tests for `.github/scripts/render-tap-formula.sh`.
//!
//! The script substitutes placeholders in `packaging/homebrew/gwm.rb.template`
//! to produce the final `Formula/gwm.rb` that `release.yml >
//! homebrew-tap-update` pushes to `kbrdn1/homebrew-tap` after every stable
//! release. Failures here would silently ship a broken `brew install`
//! experience to every macOS user, so the contract is exercised in tests.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn project_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn script_path() -> PathBuf {
  project_root().join(".github/scripts/render-tap-formula.sh")
}

fn template_path() -> PathBuf {
  project_root().join("packaging/homebrew/gwm.rb.template")
}

fn run_script(args: &[&str]) -> Output {
  Command::new("sh")
    .arg(script_path())
    .args(args)
    .output()
    .expect("script ran")
}

#[test]
fn script_exists_and_is_executable() {
  let p = script_path();
  assert!(p.exists(), "render script missing at {}", p.display());
  let mode = std::fs::metadata(&p).unwrap().permissions().mode();
  assert!(mode & 0o111 != 0, "render script not executable: mode={:o}", mode);
}

#[test]
fn template_exists_and_carries_all_placeholders() {
  let t = template_path();
  let body = std::fs::read_to_string(&t).expect("template should exist");
  for ph in ["__TAG__", "__VERSION__", "__SHA256_ARM64__", "__SHA256_X86_64__"] {
    assert!(
      body.contains(ph),
      "template missing placeholder {}: {}",
      ph,
      t.display()
    );
  }
  // Sanity-check it's a Homebrew formula, not a random scratch file.
  assert!(body.contains("class Gwm < Formula"), "template not a Homebrew formula");
}

#[test]
fn renders_a_complete_formula_with_all_substitutions() {
  let arm = "a".repeat(64);
  let x86 = "b".repeat(64);
  let out = run_script(&["v0.5.0", "0.5.0", &arm, &x86, template_path().to_str().unwrap()]);
  assert!(
    out.status.success(),
    "render failed: stderr={}",
    String::from_utf8_lossy(&out.stderr)
  );
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    stdout.contains("class Gwm < Formula"),
    "missing formula class: {stdout}"
  );
  assert!(
    stdout.contains("version \"0.5.0\""),
    "version not substituted: {stdout}"
  );
  assert!(
    stdout.contains("v0.5.0/gwm-v0.5.0-aarch64-apple-darwin.tar.gz"),
    "arm64 URL missing or mis-tagged: {stdout}"
  );
  assert!(
    stdout.contains("v0.5.0/gwm-v0.5.0-x86_64-apple-darwin.tar.gz"),
    "x86_64 URL missing or mis-tagged: {stdout}"
  );
  assert!(stdout.contains(&arm), "arm64 sha not substituted: {stdout}");
  assert!(stdout.contains(&x86), "x86_64 sha not substituted: {stdout}");
  for ph in ["__TAG__", "__VERSION__", "__SHA256_ARM64__", "__SHA256_X86_64__"] {
    assert!(!stdout.contains(ph), "leftover placeholder {ph} after render: {stdout}");
  }
}

#[test]
fn fails_when_required_args_missing() {
  let out = run_script(&["v0.5.0"]);
  assert!(!out.status.success(), "should fail with too few args");
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.to_lowercase().contains("usage") || stderr.to_lowercase().contains("missing"),
    "stderr should explain usage/missing arg: {stderr}"
  );
}

#[test]
fn fails_when_template_path_does_not_exist() {
  let arm = "a".repeat(64);
  let x86 = "b".repeat(64);
  let out = run_script(&["v0.5.0", "0.5.0", &arm, &x86, "/nonexistent/path/template.rb.template"]);
  assert!(!out.status.success(), "should fail when template not found");
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.to_lowercase().contains("not found")
      || stderr.to_lowercase().contains("no such file")
      || stderr.contains("/nonexistent/"),
    "stderr should mention missing template: {stderr}"
  );
}

#[test]
fn rejects_obviously_invalid_sha256_lengths() {
  // sha256 hex is 64 chars. A 10-char "sha" should be rejected so the
  // tap doesn't ship a syntactically valid but semantically wrong formula.
  let out = run_script(&[
    "v0.5.0",
    "0.5.0",
    "tooshort",
    &"b".repeat(64),
    template_path().to_str().unwrap(),
  ]);
  assert!(!out.status.success(), "should reject sha256 that is not 64 hex chars");
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.to_lowercase().contains("sha256")
      || stderr.to_lowercase().contains("64")
      || stderr.to_lowercase().contains("invalid"),
    "stderr should explain the sha rejection: {stderr}"
  );
}
