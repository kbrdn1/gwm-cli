use std::fs;

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

  assert!(
    !workflow.contains("uses: softprops/action-gh-release"),
    "release.yml must not use softprops/action-gh-release for the stable GitHub Release publish step"
  );
  assert!(
    workflow.contains("GH_TOKEN: ${{ github.token }}"),
    "release.yml must pass the workflow token to gh via GH_TOKEN"
  );
  assert!(
    workflow.contains("gh release create \"$TAG\""),
    "release.yml must create the stable GitHub Release with gh release create"
  );
  assert!(
    workflow.contains("--notes-file \"${{ steps.changelog.outputs.path }}\""),
    "stable release notes must still come from changelogs/<version>.md"
  );
  assert!(
    workflow.contains("gh release upload \"$TAG\"") && workflow.contains("--clobber"),
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
