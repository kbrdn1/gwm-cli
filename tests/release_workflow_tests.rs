use std::{fs, path::Path, process::Command};

const CHECK_RC_DUPES: &str = ".github/scripts/check-rc-changelog-dupes.sh";

#[test]
fn stable_release_workflow_skips_prerelease_tags() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();

  for suffix in ["-rc.", "-alpha.", "-beta."] {
    let guard = format!("!contains(github.event.inputs.tag || github.ref_name, '{suffix}')");
    assert!(
      workflow.contains(&guard),
      "release.yml must guard stable release jobs against {suffix} tags"
    );
  }
}

#[test]
fn stable_release_publish_uses_github_cli_with_workflow_token() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();
  let publish_step = workflow
    .split("      - name: publish release")
    .nth(1)
    .and_then(|tail| tail.split("\n  homebrew-tap-update:").next())
    .expect("release.yml must contain a publish release step before homebrew-tap-update");

  assert!(
    !workflow.contains("uses: softprops/action-gh-release"),
    "release.yml must not use softprops/action-gh-release for the stable GitHub Release publish step"
  );
  assert!(
    publish_step.contains("GH_TOKEN: ${{ github.token }}"),
    "release.yml must pass the workflow token to gh via GH_TOKEN in the publish release step"
  );
  assert!(
    publish_step.contains("gh release create \"$TAG\""),
    "release.yml must create the stable GitHub Release with gh release create"
  );
  assert!(
    publish_step.contains("--notes-file \"${{ steps.changelog.outputs.path }}\""),
    "stable release notes must still come from changelogs/<version>.md"
  );
  assert!(
    publish_step.contains("gh release upload \"$TAG\"") && publish_step.contains("--clobber"),
    "release.yml must upload artifacts with gh release upload --clobber so recovery reruns can replace assets"
  );
}

#[test]
fn prerelease_workflow_does_not_match_stable_tags() {
  let workflow = fs::read_to_string(".github/workflows/pre-release.yml").unwrap();

  assert!(
    workflow.contains("\"v*.*.*-rc.*\""),
    "pre-release.yml must trigger on rc tags"
  );
  assert!(
    workflow.contains("\"v*.*.*-alpha.*\""),
    "pre-release.yml must trigger on alpha tags"
  );
  assert!(
    workflow.contains("\"v*.*.*-beta.*\""),
    "pre-release.yml must trigger on beta tags"
  );
  assert!(
    !workflow.contains("\n      - \"v*.*.*\""),
    "pre-release.yml must not trigger on stable tags"
  );
}

#[test]
fn ci_test_matrix_runs_on_windows_latest() {
  let workflow = fs::read_to_string(".github/workflows/ci.yml").unwrap();
  let test_job = workflow
    .split("  test:")
    .nth(1)
    .and_then(|tail| tail.split("\n  hook-smoke:").next())
    .expect("ci.yml must contain a test job before hook-smoke");

  for os in ["ubuntu-latest", "macos-latest", "windows-latest"] {
    assert!(test_job.contains(os), "ci.yml test matrix must include {os}");
  }
  assert!(
    test_job.contains("run: cargo build --verbose"),
    "windows-latest must run the same cargo build step as the other test matrix rows"
  );
  assert!(
    test_job.contains("run: cargo test --verbose"),
    "windows-latest must run the same cargo test step as the other test matrix rows"
  );
}

#[test]
fn prerelease_workflow_checks_unreleased_against_previous_rc_before_publish() {
  let workflow = fs::read_to_string(".github/workflows/pre-release.yml").unwrap();
  let check_pos = workflow
    .find("check unreleased changelog against previous rc")
    .expect("pre-release.yml must run the duplicate changelog guard");
  let publish_pos = workflow
    .find("publish pre-release")
    .expect("pre-release.yml must still publish the pre-release");

  assert!(
    check_pos < publish_pos,
    "duplicate changelog guard must run before publishing the pre-release"
  );
  assert!(
    workflow.contains("./.github/scripts/check-rc-changelog-dupes.sh \"${{ steps.tag.outputs.name }}\""),
    "pre-release.yml must call the duplicate changelog guard with the resolved tag"
  );
}

#[test]
fn rc_changelog_dupe_check_fails_on_repeated_bullet() {
  let tmp = tempfile::tempdir().unwrap();
  write_release_files(
    tmp.path(),
    r#"
# Changelog

## [Unreleased]

### Fixed

- Release workflow publishes with the workflow token. (#146)
- Fresh post-rc delta. (#147)

## Past releases
"#,
    r#"
# [0.7.0-rc.2] - 2026-05-23

### Fixed

- Release workflow publishes with the workflow token. (#146)
"#,
  );

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.3");

  assert!(!output.status.success(), "duplicate bullet must fail the check");
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("#146"),
    "failure should name the duplicated issue ref: {stderr}"
  );
  assert!(
    stderr.contains("Release workflow publishes with the workflow token"),
    "failure should name the duplicated changelog bullet: {stderr}"
  );
}

#[test]
fn rc_changelog_dupe_check_fails_on_repeated_issue_ref() {
  let tmp = tempfile::tempdir().unwrap();
  write_release_files(
    tmp.path(),
    r#"
# Changelog

## [Unreleased]

### Changed

- Tighten release workflow token handling. (#146)

## Past releases
"#,
    r#"
# [0.7.0-rc.2] - 2026-05-23

### Fixed

- Release workflow publishes with the workflow token. (#146)
"#,
  );

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.3");

  assert!(!output.status.success(), "repeated issue ref must fail the check");
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("#146"),
    "failure should name the duplicated issue ref: {stderr}"
  );
}

#[test]
fn rc_changelog_dupe_check_allows_new_post_rc_delta() {
  let tmp = tempfile::tempdir().unwrap();
  write_release_files(
    tmp.path(),
    r#"
# Changelog

## [Unreleased]

### Fixed

- Fresh post-rc delta. (#147)

## Past releases
"#,
    r#"
# [0.7.0-rc.2] - 2026-05-23

### Fixed

- Release workflow publishes with the workflow token. (#146)
"#,
  );

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.3");

  assert!(
    output.status.success(),
    "new post-rc deltas must pass: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}

#[test]
fn rc_changelog_dupe_check_skips_first_rc_without_previous_notes() {
  let tmp = tempfile::tempdir().unwrap();
  fs::create_dir_all(tmp.path().join("changelogs/pre-releases")).unwrap();
  fs::write(
    tmp.path().join("CHANGELOG.md"),
    r#"
# Changelog

## [Unreleased]

### Fixed

- First rc entry. (#147)

## Past releases
"#,
  )
  .unwrap();

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.1");

  assert!(
    output.status.success(),
    "rc.1 has no previous rc to compare: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}

fn write_release_files(root: &Path, changelog: &str, previous_rc: &str) {
  fs::create_dir_all(root.join("changelogs/pre-releases")).unwrap();
  fs::write(root.join("CHANGELOG.md"), changelog).unwrap();
  fs::write(root.join("changelogs/pre-releases/0.7.0-rc.2.md"), previous_rc).unwrap();
}

fn run_dupe_check(root: &Path, tag: &str) -> std::process::Output {
  let script = std::env::current_dir().unwrap().join(CHECK_RC_DUPES);
  let test_script = root.join(CHECK_RC_DUPES);
  fs::create_dir_all(test_script.parent().unwrap()).unwrap();
  fs::copy(script, &test_script).unwrap();

  Command::new("bash")
    .arg(CHECK_RC_DUPES)
    .arg(tag)
    .current_dir(root)
    .output()
    .unwrap()
}
