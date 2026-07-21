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

// ---- release.yml wiring (#377/#378) -------------------------------------
//
// The metadata blocks above are inert without the release job that invokes
// `cargo deb` / `cargo generate-rpm` and attaches the results. Deleting or
// mistyping those lines would break the next stable release while leaving the
// metadata tests green — so the critical wiring is pinned here too.

fn release_yml() -> String {
  let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml");
  std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn release_workflow_builds_both_linux_packages() {
  let yml = release_yml();
  // `--no-strip` is load-bearing: without it cargo-deb runs the host strip on
  // the cross-built aarch64 ELF and the arm64 job fails (see #385 review).
  assert!(
    yml.contains("cargo deb --no-build --no-strip --target"),
    "release.yml must build the .deb with --no-build --no-strip --target"
  );
  assert!(
    yml.contains("cargo generate-rpm --target"),
    "release.yml must build the .rpm per target"
  );
}

#[test]
fn release_workflow_publishes_both_linux_packages() {
  let yml = release_yml();
  for glob in ["dist/*.deb", "dist/*.deb.sha256", "dist/*.rpm", "dist/*.rpm.sha256"] {
    assert!(yml.contains(glob), "the release upload step must publish {glob}");
  }
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
fn deb_declares_glibc_with_a_version_floor() {
  let deb = metadata("deb");
  let depends = deb
    .get("depends")
    .and_then(|v| v.as_str())
    .expect("depends is a string");
  // Explicit (not `$auto`) so cargo-deb never runs dpkg-shlibdeps — required
  // to package the cross-built aarch64 binary from an x86_64 runner — AND
  // carries a glibc floor so dpkg refuses cleanly on too-old distros instead
  // of installing then crashing at load.
  assert!(depends.starts_with("libc6"), "libc6 leads the depends list: {depends}");
  assert!(depends.contains(">= 2.34"), "depends must pin a glibc floor: {depends}");
}

#[test]
fn deb_and_rpm_declare_git_as_a_runtime_dep() {
  // gwm shells out to the `git` binary (sync, worktree rename, clean, TUI
  // previews) beyond the vendored libgit2, so the distro packages must pull
  // git — shlibdeps / rpm auto-req would never catch an exec dependency (#388).
  let deb = metadata("deb");
  let depends = deb
    .get("depends")
    .and_then(|v| v.as_str())
    .expect("depends is a string");
  assert!(
    depends.split(',').any(|d| d.trim() == "git"),
    "deb depends must include git: {depends}"
  );

  let rpm = metadata("generate-rpm");
  let requires = rpm.get("requires").expect("rpm requires table is present");
  assert!(
    requires.get("git").is_some(),
    "rpm requires must include git: {requires:?}"
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
  let glibc = requires
    .get("glibc")
    .and_then(|v| v.as_str())
    .expect("glibc requirement present");
  assert!(
    glibc.contains(">= 2.34"),
    "rpm glibc requirement must pin a floor, got: {glibc}"
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
