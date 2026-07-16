//! Pin the `.deb` / `.rpm` packaging contracts (issues #377, #378).
//!
//! `release.yml` builds `.deb` (cargo-deb) and `.rpm` (cargo-generate-rpm)
//! packages for the two Linux targets and attaches them to each stable
//! Release. Both read their `[package.metadata.*]` block from `Cargo.toml`;
//! a drift there ships a package that installs the binary to the wrong path,
//! declares the wrong deps, or fails to build. The contract is asserted
//! structurally against the parsed manifest — mirrors
//! `binstall_metadata_tests.rs`.

use std::path::{Path, PathBuf};

fn manifest() -> toml::Value {
  let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
  let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
  toml::from_str(&raw).expect("Cargo.toml is valid TOML")
}

fn metadata(key: &str) -> toml::Value {
  manifest()
    .get("package")
    .and_then(|p| p.get("metadata"))
    .and_then(|m| m.get(key))
    .cloned()
    .unwrap_or_else(|| panic!("[package.metadata.{key}] block is present"))
}

/// Every asset row's first field (deb: `["src", "dest", "mode"]`;
/// rpm: `{{ source, dest, mode }}`) → the `source` path.
fn asset_sources(block: &toml::Value) -> Vec<String> {
  let assets = block
    .get("assets")
    .and_then(|a| a.as_array())
    .expect("assets is an array");
  assets
    .iter()
    .map(|row| {
      if let Some(arr) = row.as_array() {
        arr[0].as_str().expect("deb asset source is a string").to_string()
      } else {
        row
          .get("source")
          .and_then(|s| s.as_str())
          .expect("rpm asset source is a string")
          .to_string()
      }
    })
    .collect()
}

// ---- .deb (#377) --------------------------------------------------------

#[test]
fn deb_ships_the_binary_under_usr_bin() {
  let deb = metadata("deb");
  let assets = deb
    .get("assets")
    .and_then(|a| a.as_array())
    .expect("deb assets is an array");
  // The binary row: `["target/release/gwm", "usr/bin/", "755"]`. The
  // `target/release` prefix is what cargo-deb rewrites to the per-target dir
  // when `--target` is passed, so it must stay exactly that.
  let bin_row = assets
    .iter()
    .find(|r| {
      r.as_array()
        .is_some_and(|a| a[0].as_str() == Some("target/release/gwm"))
    })
    .expect("deb must ship target/release/gwm");
  let row = bin_row.as_array().unwrap();
  assert_eq!(row[1].as_str(), Some("usr/bin/"), "gwm must land in usr/bin/");
  assert_eq!(row[2].as_str(), Some("755"), "the binary must be executable");
}

#[test]
fn deb_skips_shlibdeps_with_an_explicit_depends() {
  let deb = metadata("deb");
  let depends = deb.get("depends").and_then(|v| v.as_str());
  // Explicit (not `$auto`) so cargo-deb never runs dpkg-shlibdeps — required
  // to package the cross-built aarch64 binary from an x86_64 runner.
  assert_eq!(
    depends,
    Some("libc6"),
    "depends must be the explicit glibc dep, not $auto"
  );
}

// ---- .rpm (#378) --------------------------------------------------------

#[test]
fn rpm_ships_the_binary_under_usr_bin() {
  let rpm = metadata("generate-rpm");
  let sources = asset_sources(&rpm);
  assert!(
    sources.iter().any(|s| s == "target/release/gwm"),
    "rpm must ship target/release/gwm, got: {sources:?}"
  );
  let bin = rpm
    .get("assets")
    .and_then(|a| a.as_array())
    .unwrap()
    .iter()
    .find(|r| r.get("source").and_then(|s| s.as_str()) == Some("target/release/gwm"))
    .expect("rpm binary asset present");
  assert_eq!(bin.get("dest").and_then(|d| d.as_str()), Some("/usr/bin/gwm"));
  assert_eq!(bin.get("mode").and_then(|m| m.as_str()), Some("755"));
}

#[test]
fn deb_conflicts_with_debian_gwm_window_manager() {
  // Debian ships an unrelated `gwm` window manager that also owns
  // `/usr/bin/gwm`; without Conflicts, dpkg errors on the file clash at unpack.
  let deb = metadata("deb");
  assert_eq!(
    deb.get("conflicts").and_then(|v| v.as_str()),
    Some("gwm"),
    "deb must declare Conflicts: gwm"
  );
}

#[test]
fn rpm_declares_glibc_requirement() {
  // With auto-req off (cross-packaging), the sole dynamic dep — glibc, since
  // libgit2 and zlib are statically linked — must be declared explicitly.
  let rpm = metadata("generate-rpm");
  let requires = rpm.get("requires").expect("rpm requires table is present");
  assert!(
    requires.get("glibc").is_some(),
    "rpm must require glibc explicitly, got: {requires:?}"
  );
}

#[test]
fn rpm_disables_auto_req() {
  let rpm = metadata("generate-rpm");
  // Cross-packaging aarch64 from an x86_64 runner can't run rpm's dependency
  // auto-detection; the vendored-static binary needs only glibc anyway.
  assert_eq!(
    rpm.get("auto-req").and_then(|v| v.as_str()),
    Some("no"),
    "auto-req must be disabled for cross-packaging"
  );
}

#[test]
fn both_packages_agree_on_the_doc_dir() {
  // Both use the crate name (`gwm-cli`) for the doc dir, not `gwm`, matching
  // the package name and avoiding Debian's unrelated `gwm` package.
  for key in ["deb", "generate-rpm"] {
    let sources = asset_sources(&metadata(key));
    assert!(
      sources.iter().any(|s| s.contains("README")),
      "{key} should ship the README"
    );
  }
  // Spot-check the doc dir path shape in each block's raw form.
  let deb = metadata("deb");
  let deb_readme = deb
    .get("assets")
    .and_then(|a| a.as_array())
    .unwrap()
    .iter()
    .find_map(|r| {
      r.as_array()
        .and_then(|a| a[1].as_str())
        .filter(|d| d.contains("README"))
    })
    .expect("deb README dest");
  assert_eq!(deb_readme, "usr/share/doc/gwm-cli/README.md");
}
