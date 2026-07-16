//! Pin the winget publishing wiring in `release.yml` (issue #381).
//!
//! The `winget-publish` job runs the third-party `winget-releaser` action to
//! open a manifest PR to `microsoft/winget-pkgs` for each stable release. There
//! is no manifest file in this repo (the action generates it via komac), so the
//! contract that can regress is purely the workflow wiring: the job exists, the
//! action is pinned to a commit SHA (it is trusted with `WINGET_TOKEN`), it
//! targets the right package identifier, and it matches our nested-zip asset
//! rather than the action's exe/msi default. Mirrors the release-wiring half of
//! `aur_pkgbuild_tests.rs` / `linux_packaging_metadata_tests.rs`.

use std::path::Path;

fn release_yml() -> String {
  let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml");
  std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn release_workflow_publishes_to_winget() {
  let yml = release_yml();
  assert!(
    yml.contains("winget-publish:"),
    "release.yml must define the winget-publish job"
  );
  assert!(
    yml.contains("identifier: kbrdn1.gwm"),
    "winget-publish must target the kbrdn1.gwm package identifier"
  );
}

#[test]
fn winget_installers_regex_matches_the_nested_zip() {
  let yml = release_yml();
  // The action's default installers-regex only matches exe/msi/msix; our
  // Windows artifact is a nested-portable .zip, so the override is load-bearing
  // — without it the action finds no installer and fails.
  assert!(
    yml.contains(r"gwm-.*-x86_64-pc-windows-msvc\.zip$"),
    "winget-publish must override installers-regex to match the windows .zip asset"
  );
}

#[test]
fn winget_releaser_action_is_pinned_to_a_commit_sha() {
  let yml = release_yml();
  // The third-party action is trusted with WINGET_TOKEN — it must be pinned to
  // a 40-char commit SHA, never a mutable @v2 / @branch ref.
  let needle = "vedantmgoyal9/winget-releaser@";
  let idx = yml
    .find(needle)
    .expect("release.yml must use the winget-releaser action");
  let rest = &yml[idx + needle.len()..];
  let sha: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
  assert_eq!(
    sha.len(),
    40,
    "winget-releaser must be pinned to a 40-char commit SHA, got `{sha}`"
  );
}

#[test]
fn winget_publish_is_stable_only_and_advisory() {
  let yml = release_yml();
  let job = yml.split_once("winget-publish:").expect("winget-publish job present").1;
  // Stable-only gate so a `-rc.` tag never reaches `winget install`.
  assert!(job.contains("-rc."), "winget-publish must gate out -rc. pre-releases");
  // Advisory until WINGET_TOKEN is provisioned and the first manifest is merged.
  assert!(
    job.contains("continue-on-error: true"),
    "winget-publish must be advisory while WINGET_TOKEN / the first manifest are pending"
  );
  // The token flows from the secret, never hardcoded.
  assert!(
    job.contains("${{ secrets.WINGET_TOKEN }}"),
    "winget-publish must read the token from the WINGET_TOKEN secret"
  );
}
