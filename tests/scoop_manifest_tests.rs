//! Integration tests for `.github/scripts/render-scoop-manifest.sh`.
//!
//! The script substitutes placeholders in `packaging/scoop/gwm.json.template`
//! to produce the final `bucket/gwm.json` that `release.yml >
//! scoop-bucket-update` pushes to `kbrdn1/scoop-gwm` after every stable
//! release. A malformed manifest would silently break `scoop install gwm` for
//! every Windows user, so the contract — valid JSON, right URL/hash, and the
//! Scoop autoupdate variables left intact — is exercised in tests (issue
//! #376). Mirrors `homebrew_formula_tests.rs`.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn project_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn script_path() -> PathBuf {
  project_root().join(".github/scripts/render-scoop-manifest.sh")
}

fn template_path() -> PathBuf {
  project_root().join("packaging/scoop/gwm.json.template")
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
  for ph in ["__TAG__", "__VERSION__", "__SHA256_X86_64__"] {
    assert!(
      body.contains(ph),
      "template missing placeholder {}: {}",
      ph,
      t.display()
    );
  }
  // The Scoop autoupdate variables must live in the template verbatim so
  // `scoop update` can bump versions on its own — they must NOT be rendered.
  assert!(
    body.contains("$version"),
    "template missing the Scoop $version autoupdate var"
  );
  assert!(body.contains("autoupdate"), "template missing the autoupdate block");
}

#[test]
fn renders_a_complete_valid_json_manifest() {
  let x86 = "b".repeat(64);
  let out = run_script(&["v0.5.0", "0.5.0", &x86, template_path().to_str().unwrap()]);
  assert!(
    out.status.success(),
    "render failed: stderr={}",
    String::from_utf8_lossy(&out.stderr)
  );
  let stdout = String::from_utf8_lossy(&out.stdout);

  // A broken manifest is worse than no manifest — the rendered output must be
  // valid JSON.
  let v: serde_json::Value =
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("rendered manifest is not valid JSON ({e}): {stdout}"));

  assert_eq!(v["version"], "0.5.0", "version not substituted: {stdout}");
  assert_eq!(
    v["bin"], "gwm.exe",
    "bin must point at the Windows executable: {stdout}"
  );
  let arch = &v["architecture"]["64bit"];
  assert_eq!(
    arch["url"], "https://github.com/kbrdn1/gwm-cli/releases/download/v0.5.0/gwm-v0.5.0-x86_64-pc-windows-msvc.zip",
    "x86_64 URL missing or mis-tagged: {stdout}"
  );
  assert_eq!(arch["hash"], x86, "x86_64 sha not substituted: {stdout}");
  assert_eq!(
    arch["extract_dir"], "gwm-v0.5.0-x86_64-pc-windows-msvc",
    "extract_dir must match the versioned folder inside the zip: {stdout}"
  );

  // Autoupdate vars survive rendering so `scoop update` keeps working.
  assert!(
    stdout.contains("v$version"),
    "autoupdate $version var was clobbered: {stdout}"
  );
  assert!(
    stdout.contains("$url.sha256"),
    "autoupdate hash-from-sidecar var was clobbered: {stdout}"
  );

  for ph in ["__TAG__", "__VERSION__", "__SHA256_X86_64__"] {
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
  let x86 = "b".repeat(64);
  let out = run_script(&["v0.5.0", "0.5.0", &x86, "/nonexistent/path/gwm.json.template"]);
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
  let out = run_script(&["v0.5.0", "0.5.0", "tooshort", template_path().to_str().unwrap()]);
  assert!(!out.status.success(), "should reject sha256 that is not 64 hex chars");
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.to_lowercase().contains("sha256")
      || stderr.to_lowercase().contains("64")
      || stderr.to_lowercase().contains("invalid"),
    "stderr should explain the sha rejection: {stderr}"
  );
}
