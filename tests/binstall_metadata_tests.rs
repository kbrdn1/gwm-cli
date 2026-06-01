//! Pin the `[package.metadata.binstall]` contract (issue #27).
//!
//! `cargo binstall gwm` reads this block to locate the prebuilt archive
//! on the GitHub Release instead of compiling from source. The block
//! must stay in lock-step with the release workflows' artefact naming
//! (`gwm-v{version}-{target}.{tar.gz|zip}`) and the in-archive layout
//! (`gwm-v{version}-{target}/{gwm|gwm.exe}`). A drift here ships a
//! `binstall` that 404s or extracts the wrong path, so the contract is
//! asserted structurally against the parsed manifest.

use std::path::{Path, PathBuf};

fn manifest() -> toml::Value {
  let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
  let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
  toml::from_str(&raw).expect("Cargo.toml is valid TOML")
}

fn binstall() -> toml::Value {
  manifest()
    .get("package")
    .and_then(|p| p.get("metadata"))
    .and_then(|m| m.get("binstall"))
    .cloned()
    .expect("[package.metadata.binstall] block is present")
}

#[test]
fn binstall_pkg_url_points_at_the_release_tarball() {
  let b = binstall();
  let pkg_url = b.get("pkg-url").and_then(|v| v.as_str()).expect("pkg-url is a string");
  assert_eq!(
    pkg_url, "{ repo }/releases/download/v{ version }/gwm-v{ version }-{ target }.tar.gz",
    "pkg-url must match the release workflow's tarball naming"
  );
}

#[test]
fn binstall_pkg_fmt_is_tgz() {
  let b = binstall();
  let fmt = b.get("pkg-fmt").and_then(|v| v.as_str()).expect("pkg-fmt is a string");
  assert_eq!(fmt, "tgz", "the non-windows artefacts are gzipped tarballs");
}

#[test]
fn binstall_bin_dir_matches_in_archive_layout() {
  let b = binstall();
  let bin_dir = b.get("bin-dir").and_then(|v| v.as_str()).expect("bin-dir is a string");
  // The packaging step stages the binary under
  // `gwm-v{version}-{target}/` inside the archive; `{ bin }` resolves
  // to `gwm` (or `gwm.exe` on windows).
  assert_eq!(bin_dir, "gwm-v{ version }-{ target }/{ bin }");
}

#[test]
fn binstall_windows_override_uses_zip_artefact() {
  let b = binstall();
  let win = b
    .get("overrides")
    .and_then(|o| o.get("x86_64-pc-windows-msvc"))
    .expect("windows MSVC override is present");

  let fmt = win
    .get("pkg-fmt")
    .and_then(|v| v.as_str())
    .expect("windows pkg-fmt is a string");
  assert_eq!(fmt, "zip", "the windows artefact is a .zip, not a tarball");

  let pkg_url = win
    .get("pkg-url")
    .and_then(|v| v.as_str())
    .expect("windows pkg-url is a string");
  assert!(
    pkg_url.ends_with("gwm-v{ version }-{ target }.zip"),
    "windows pkg-url must target the .zip artefact, got: {pkg_url}"
  );
}
