//! Pin the winget publishing wiring in `release.yml` (issue #381).
//!
//! The `winget-publish` job submits a manifest update to
//! `microsoft/winget-pkgs` for each stable release by running `komac` directly
//! from a pinned, checksum-verified release binary — deliberately NOT the
//! `winget-releaser` action, which pulls `cargo-bins/cargo-binstall@main` and
//! installs the latest komac at runtime (both mutable, and both would run with
//! `WINGET_TOKEN` in scope). There is no manifest file in this repo (komac
//! generates it), so the contract that can regress is the workflow wiring: the
//! job exists, komac is pinned + checksum-verified, it targets the right
//! package, strips the tag's `v`, and stays advisory. Mirrors the
//! release-wiring half of `aur_pkgbuild_tests.rs`.

use std::path::Path;

fn release_yml() -> String {
  let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml");
  std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn winget_job() -> String {
  release_yml()
    .split_once("winget-publish:")
    .expect("release.yml must define the winget-publish job")
    .1
    .to_string()
}

#[test]
fn release_workflow_publishes_to_winget() {
  let job = winget_job();
  assert!(
    job.contains("komac update kbrdn1.gwm"),
    "winget-publish must run `komac update` for the kbrdn1.gwm package"
  );
  assert!(
    job.contains("gwm-${TAG}-x86_64-pc-windows-msvc.zip"),
    "winget-publish must submit the windows .zip release asset"
  );
  // The winget PackageVersion has no `v`, so the tag prefix must be stripped.
  assert!(
    job.contains("${TAG#v}"),
    "winget-publish must strip the tag's v prefix to match the winget version"
  );
}

#[test]
fn komac_is_pinned_and_checksum_verified() {
  let job = winget_job();
  // The tool that runs with WINGET_TOKEN must be a fixed version, not "latest".
  assert!(
    job.contains("KOMAC_VERSION: 2.16.0"),
    "komac must be pinned to an explicit version"
  );
  // ...and its download must be checksum-verified against the release SHA256SUMS.
  assert!(
    job.contains("SHA256SUMS") && job.contains("sha256sum -c"),
    "the pinned komac download must be checksum-verified"
  );
}

#[test]
fn winget_publish_does_not_use_the_unpinned_releaser_action() {
  // Regression guard: `winget-releaser` transitively pulls cargo-binstall@main +
  // the latest komac, defeating a SHA pin for a secret-trusted step. The job
  // must never `uses:` it.
  let yml = release_yml();
  assert!(
    !yml.contains("uses: vedantmgoyal9/winget-releaser"),
    "winget-publish must run komac directly, not the winget-releaser action"
  );
}

#[test]
fn winget_publish_is_stable_only_and_advisory() {
  let job = winget_job();
  // Stable-only gate so a `-rc.` tag never reaches `winget install`.
  assert!(job.contains("-rc."), "winget-publish must gate out -rc. pre-releases");
  // Advisory until WINGET_TOKEN is provisioned and the first manifest is merged.
  assert!(
    job.contains("continue-on-error: true"),
    "winget-publish must be advisory while WINGET_TOKEN / the first manifest are pending"
  );
  // The token flows from the secret into komac's GITHUB_TOKEN, never hardcoded.
  assert!(
    job.contains("${{ secrets.WINGET_TOKEN }}"),
    "winget-publish must read the token from the WINGET_TOKEN secret"
  );
}
