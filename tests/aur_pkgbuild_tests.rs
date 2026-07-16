//! Integration tests for the AUR packaging (issue #379).
//!
//! `.github/scripts/render-aur-pkgbuild.sh` substitutes placeholders in
//! `packaging/aur/PKGBUILD.template` to produce the `PKGBUILD` that
//! `release.yml > aur-publish` pushes to the `gwm-cli-bin` AUR package after
//! every stable release. A malformed PKGBUILD silently breaks `yay -S
//! gwm-cli-bin` for every Arch user, so the contract — the right version, both
//! per-arch checksums, the `gwm`/`gwm-cli` provides/conflicts, and the
//! completion/license install lines — is exercised here. The second half pins
//! the `release.yml` wiring so a deleted step or an unpinned third-party
//! action can't slip through green (mirrors `linux_packaging_metadata_tests.rs`
//! + `scoop_manifest_tests.rs`).

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn project_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn script_path() -> PathBuf {
  project_root().join(".github/scripts/render-aur-pkgbuild.sh")
}

fn template_path() -> PathBuf {
  project_root().join("packaging/aur/PKGBUILD.template")
}

fn release_yml() -> String {
  let path = project_root().join(".github/workflows/release.yml");
  std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn run_script(args: &[&str]) -> Output {
  Command::new("sh")
    .arg(script_path())
    .args(args)
    .output()
    .expect("script ran")
}

/// A valid render: version + x86_64 sha + aarch64 sha + template.
fn render(version: &str, sha_x86: &str, sha_arm: &str) -> String {
  let out = run_script(&[version, sha_x86, sha_arm, template_path().to_str().unwrap()]);
  assert!(
    out.status.success(),
    "render failed: stderr={}",
    String::from_utf8_lossy(&out.stderr)
  );
  String::from_utf8_lossy(&out.stdout).into_owned()
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
  for ph in ["__VERSION__", "__SHA256_X86_64__", "__SHA256_ARM64__"] {
    assert!(body.contains(ph), "template missing placeholder {ph}: {}", t.display());
  }
}

#[test]
fn renders_a_complete_pkgbuild() {
  let sha_x86 = "a".repeat(64);
  let sha_arm = "b".repeat(64);
  let out = render("1.2.3", &sha_x86, &sha_arm);

  // Version + both per-arch checksums are substituted.
  assert!(out.contains("pkgver=1.2.3"), "pkgver not substituted: {out}");
  assert!(
    out.contains(&format!("sha256sums_x86_64=('{sha_x86}')")),
    "x86_64 sha not substituted: {out}"
  );
  assert!(
    out.contains(&format!("sha256sums_aarch64=('{sha_arm}')")),
    "aarch64 sha not substituted: {out}"
  );

  // No placeholder survives the render.
  for ph in ["__VERSION__", "__SHA256_X86_64__", "__SHA256_ARM64__"] {
    assert!(!out.contains(ph), "leftover placeholder {ph} after render: {out}");
  }

  // Package identity + collision guards.
  assert!(
    out.contains("pkgname=gwm-cli-bin"),
    "pkgname must be gwm-cli-bin: {out}"
  );
  assert!(
    out.contains("provides=('gwm-cli' 'gwm')"),
    "must provide both the base name and the binary name: {out}"
  );
  assert!(
    out.contains("conflicts=('gwm-cli' 'gwm')"),
    "must conflict on both the base name and the binary name (owns /usr/bin/gwm): {out}"
  );

  // Both architectures are packaged from the versioned linux-gnu tarballs.
  assert!(
    out.contains("arch=('x86_64' 'aarch64')"),
    "must target both arches: {out}"
  );
  assert!(
    out.contains("gwm-v$pkgver-x86_64-unknown-linux-gnu.tar.gz"),
    "x86_64 source must point at the versioned linux-gnu tarball: {out}"
  );
  assert!(
    out.contains("gwm-v$pkgver-aarch64-unknown-linux-gnu.tar.gz"),
    "aarch64 source must point at the versioned linux-gnu tarball: {out}"
  );

  // The prebuilt binary is already stripped and carries no debug symbols.
  assert!(
    out.contains("'!strip'"),
    "must skip re-stripping the prebuilt binary: {out}"
  );
  assert!(out.contains("'!debug'"), "must skip the empty debug package: {out}");

  // Binary, license, and all three shell completions land in the right Arch
  // paths (acceptance criteria: installs gwm + license + completions).
  for line in [
    "install -Dm755 gwm \"$pkgdir/usr/bin/gwm\"",
    "usr/share/licenses/$pkgname/LICENSE.md",
    "usr/share/bash-completion/completions/gwm",
    "usr/share/zsh/site-functions/_gwm",
    "usr/share/fish/vendor_completions.d/gwm.fish",
  ] {
    assert!(out.contains(line), "PKGBUILD missing install target `{line}`: {out}");
  }
}

#[test]
fn fails_when_required_args_missing() {
  let out = run_script(&["1.2.3"]);
  assert!(!out.status.success(), "should fail with too few args");
  let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
  assert!(
    stderr.contains("usage") || stderr.contains("missing"),
    "stderr should explain usage/missing arg: {stderr}"
  );
}

#[test]
fn fails_when_template_path_does_not_exist() {
  let sha = "a".repeat(64);
  let out = run_script(&["1.2.3", &sha, &sha, "/nonexistent/path/PKGBUILD.template"]);
  assert!(!out.status.success(), "should fail when template not found");
  let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
  assert!(
    stderr.contains("not found") || stderr.contains("no such file") || stderr.contains("/nonexistent/"),
    "stderr should mention the missing template: {stderr}"
  );
}

#[test]
fn rejects_invalid_sha256_in_either_position() {
  let good = "a".repeat(64);
  let tmpl = template_path();
  let tmpl = tmpl.to_str().unwrap();

  // x86_64 sha too short.
  let out = run_script(&["1.2.3", "tooshort", &good, tmpl]);
  assert!(!out.status.success(), "should reject a short x86_64 sha256");

  // aarch64 sha not hex.
  let bad_hex = "z".repeat(64);
  let out = run_script(&["1.2.3", &good, &bad_hex, tmpl]);
  assert!(!out.status.success(), "should reject a non-hex aarch64 sha256");
  let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
  assert!(
    stderr.contains("sha256") || stderr.contains("64") || stderr.contains("hex"),
    "stderr should explain the sha rejection: {stderr}"
  );
}

// ---- release.yml wiring (#379) ------------------------------------------

#[test]
fn release_workflow_publishes_to_the_aur() {
  let yml = release_yml();
  assert!(
    yml.contains("aur-publish:"),
    "release.yml must define the aur-publish job"
  );
  assert!(
    yml.contains("pkgname: gwm-cli-bin"),
    "aur-publish must push the gwm-cli-bin package"
  );
  assert!(
    yml.contains(".github/scripts/render-aur-pkgbuild.sh"),
    "aur-publish must render the PKGBUILD via the render script"
  );
  assert!(
    yml.contains("packaging/aur/PKGBUILD.template"),
    "aur-publish must render from the AUR template"
  );
}

#[test]
fn deploy_aur_action_is_pinned_to_a_commit_sha() {
  let yml = release_yml();
  // The third-party action is trusted with AUR_SSH_PRIVATE_KEY — it must be
  // pinned to a 40-char commit SHA, never a mutable @vX / @branch ref.
  let needle = "KSXGitHub/github-actions-deploy-aur@";
  let idx = yml.find(needle).expect("release.yml must use the deploy-aur action");
  let rest = &yml[idx + needle.len()..];
  let sha: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
  assert_eq!(
    sha.len(),
    40,
    "deploy-aur must be pinned to a 40-char commit SHA, got `{sha}`"
  );
}

#[test]
fn aur_publish_is_stable_only_and_advisory() {
  let yml = release_yml();
  let job = yml.split_once("aur-publish:").expect("aur-publish job present").1;
  // Stable-only gate: pre-release suffixes must be excluded so a `-rc.` tag
  // never lands on `yay -S gwm-cli-bin`.
  assert!(job.contains("-rc."), "aur-publish must gate out -rc. pre-releases");
  // Advisory until the SSH key secret is provisioned (flip after first sync).
  assert!(
    job.contains("continue-on-error: true"),
    "aur-publish must be advisory while AUR_SSH_PRIVATE_KEY is being provisioned"
  );
}
