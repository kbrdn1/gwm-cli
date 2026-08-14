//! Pin the `MIT OR Apache-2.0` dual license across every surface that
//! declares it (#573).
//!
//! A license is declared in five different syntaxes here: an SPDX expression
//! in `Cargo.toml` and the AUR `PKGBUILD`, `any_of:` in the Homebrew formula,
//! a `|` separator in the Scoop manifest, a list of nixpkgs attributes in
//! `flake.nix`, and none of them is a substring of another. A relicense that
//! updates four of the five is not a compile error anywhere: it ships a
//! package whose metadata contradicts the repo.
//!
//! So the surfaces are **walked**, not listed: everything under `packaging/`
//! plus the two manifests at the root has to carry a declaration, and every
//! declaration has to name both halves. A sixth channel added under
//! `packaging/` without a license line fails here rather than shipping mute.
//!
//! The second half of the file guards the other way a rename goes wrong. The
//! AUR package installs its license out of the release tarball, and the
//! tarball is staged by a `cp` line in the release workflows. Those two live
//! in different files, so renaming one leaves the other pointing at a path
//! that no longer exists, and `aur_pkgbuild_tests.rs` cannot catch it,
//! because it asserts what the PKGBUILD *says*, never that the tarball
//! *carries* it. That mismatch stays green until the next release, when
//! `makepkg` fails on a user's machine. Here the two sets are compared.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Read a repo file with line endings normalised.
///
/// Git may hand these back with CRLF on `windows-latest`; every assertion
/// below is about content, never about the checkout's line-ending policy.
fn read(rel: &str) -> String {
  let path = root().join(rel);
  fs::read_to_string(&path)
    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    .replace("\r\n", "\n")
}

// ---------------------------------------------------------------------------
// The two license texts
// ---------------------------------------------------------------------------

#[test]
fn both_license_texts_sit_at_the_repo_root() {
  for name in ["LICENSE-MIT", "LICENSE-APACHE"] {
    let path = root().join(name);
    assert!(
      path.is_file(),
      "{name} must exist at the repo root: a dual license that ships only one text grants only one license"
    );
  }
  assert!(
    !root().join("LICENSE.md").exists(),
    "LICENSE.md must be gone: it was renamed to LICENSE-MIT, and leaving both means two files claim to be the license"
  );
}

#[test]
fn the_apache_text_is_the_upstream_one_verbatim() {
  let body = read("LICENSE-APACHE");

  // The upstream text at https://www.apache.org/licenses/LICENSE-2.0.txt is
  // frozen: 11358 bytes with LF endings. Byte length is the actual verbatim
  // check: a reflowed paragraph or a "helpfully" filled-in appendix changes
  // it. The anchors below only make the failure legible.
  assert_eq!(
    body.len(),
    11358,
    "LICENSE-APACHE must be the upstream Apache-2.0 text byte for byte (expected 11358 bytes, got {})",
    body.len()
  );

  for anchor in [
    "                                 Apache License\n",
    "                           Version 2.0, January 2004\n",
    "   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION\n",
    "   APPENDIX: How to apply the Apache License to your work.\n",
    // The appendix boilerplate is part of the license text and stays
    // unfilled: substituting a real name here edits the license.
    "   Copyright [yyyy] [name of copyright owner]\n",
  ] {
    assert!(
      body.contains(anchor),
      "LICENSE-APACHE is missing the upstream anchor {anchor:?}"
    );
  }
}

#[test]
fn the_mit_text_still_carries_the_copyright_line() {
  let body = read("LICENSE-MIT");
  assert!(body.contains("MIT License"), "LICENSE-MIT must be the MIT text");
  assert!(
    body.contains("Copyright (c) 2026 Kylian Bardini"),
    "the rename to LICENSE-MIT must preserve the copyright line verbatim"
  );
}

// ---------------------------------------------------------------------------
// Every channel that declares a license
// ---------------------------------------------------------------------------

/// Does this line *assign* a license, as opposed to merely naming a license
/// file or the `usr/share/licenses/` install directory?
///
/// The distinction is the character right after the key: a declaration is
/// `license` followed by an assignment token (`=`, `:`, `(`, `"`, `any_of`),
/// whereas `LICENSE-MIT` and `licenses/$pkgname` are followed by `-` and `s`.
/// Without it every `install -Dm644 LICENSE-MIT …` row reads as a declaration
/// that names only one half.
fn is_declaration(line: &str) -> bool {
  let lower = line.to_lowercase();
  let mut from = 0;
  while let Some(at) = lower[from..].find("licen") {
    let start = from + at;
    let rest = &lower[start + "licen".len()..];
    if let Some(tail) = rest.strip_prefix("se").or_else(|| rest.strip_prefix("ce")) {
      let tail = tail.trim_start();
      for token in ["=", ":", "(", "\"", "any_of", "all_of"] {
        if tail.starts_with(token) {
          return true;
        }
      }
    }
    from = start + "licen".len();
  }
  false
}

/// Each channel spells the same two licenses its own way, so the spellings,
/// and only the spellings, are tabulated. The *set of files* checked is
/// walked, which is the half that goes stale.
fn names_mit(line: &str) -> bool {
  line.contains("MIT") || line.contains("licenses.mit")
}

fn names_apache(line: &str) -> bool {
  // `asl20` is the nixpkgs attribute; its `spdxId` is `Apache-2.0`.
  line.contains("Apache-2.0") || line.contains("asl20")
}

/// Every file that declares a license for a distribution channel: the two
/// manifests at the root, plus everything under `packaging/`, discovered by
/// walking rather than by a list that a new channel would not appear in.
fn declaring_surfaces() -> Vec<String> {
  let mut surfaces = vec!["Cargo.toml".to_string(), "flake.nix".to_string()];

  fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
      .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
      .map(|e| e.expect("dir entry").path())
      .collect();
    entries.sort();
    for path in entries {
      let name = path.file_name().expect("named entry").to_string_lossy().to_string();
      // Join with `/`: these are repo-relative keys, and `read()` joins them
      // back onto the root, which accepts forward slashes on every platform.
      let rel = format!("{prefix}/{name}");
      if path.is_dir() {
        walk(&path, &rel, out);
      } else {
        out.push(rel);
      }
    }
  }
  walk(&root().join("packaging"), "packaging", &mut surfaces);

  surfaces
}

#[test]
fn every_packaging_surface_declares_both_licenses() {
  let surfaces = declaring_surfaces();

  // Non-vacuity: an empty or truncated walk would let the loop below pass
  // without reading a single channel. Five is the count at the time of
  // writing (two manifests + three templates); the floor only has to prove
  // the walk found the tree.
  assert!(
    surfaces.len() >= 5,
    "the surface walk found only {} file(s): it is not reaching packaging/: {surfaces:?}",
    surfaces.len()
  );

  for rel in &surfaces {
    let body = read(rel);
    let declarations: Vec<&str> = body.lines().filter(|l| is_declaration(l)).collect();

    assert!(
      !declarations.is_empty(),
      "{rel} declares no license at all: a distribution channel that ships without one is the silent failure this guard exists for"
    );

    for line in declarations {
      assert!(
        names_mit(line) && names_apache(line),
        "{rel} declares a license that is not the MIT OR Apache-2.0 disjunction: {}",
        line.trim()
      );
    }
  }
}

#[test]
fn the_crate_declares_the_spdx_disjunction() {
  let manifest: toml::Value = toml::from_str(&read("Cargo.toml")).expect("Cargo.toml is valid TOML");
  let package = manifest.get("package").expect("[package] block");

  assert_eq!(
    package.get("license").and_then(|v| v.as_str()),
    Some("MIT OR Apache-2.0"),
    "crates.io reads this field; it must be the SPDX expression, not one half of it"
  );
  assert!(
    package.get("license-file").is_none(),
    "`license-file` names a single file and would contradict the disjunction above"
  );
}

// ---------------------------------------------------------------------------
// The license texts have to reach every artefact
// ---------------------------------------------------------------------------

fn deb() -> toml::Value {
  let manifest: toml::Value = toml::from_str(&read("Cargo.toml")).expect("Cargo.toml is valid TOML");
  manifest["package"]["metadata"]["deb"].clone()
}

fn rpm() -> toml::Value {
  let manifest: toml::Value = toml::from_str(&read("Cargo.toml")).expect("Cargo.toml is valid TOML");
  manifest["package"]["metadata"]["generate-rpm"].clone()
}

/// The `LICENSE*` basenames in a set of packaged file paths.
fn license_files<'a>(paths: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
  paths
    .filter_map(|p| p.rsplit('/').next())
    .filter(|name| name.starts_with("LICENSE"))
    .map(str::to_string)
    .collect()
}

fn expected_license_files() -> BTreeSet<String> {
  ["LICENSE-APACHE".to_string(), "LICENSE-MIT".to_string()]
    .into_iter()
    .collect()
}

#[test]
fn the_deb_ships_both_license_texts() {
  let assets = deb();
  let rows = assets["assets"].as_array().expect("deb assets is an array");
  // cargo-deb rows are `[source, dest, mode]`; the destination is what the
  // installed package actually carries.
  let dests: Vec<String> = rows
    .iter()
    .filter_map(|r| r.get(1).and_then(|v| v.as_str()).map(str::to_string))
    .collect();

  assert_eq!(
    license_files(dests.iter().map(String::as_str)),
    expected_license_files(),
    "the .deb must carry both license texts: dropping `license-file` moved them from the generated copyright file into the package's doc dir, so they have to be listed as assets"
  );
}

#[test]
fn the_rpm_ships_both_license_texts() {
  let assets = rpm();
  let rows = assets["assets"].as_array().expect("rpm assets is an array");
  let dests: Vec<String> = rows
    .iter()
    .filter_map(|r| r.get("dest").and_then(|v| v.as_str()).map(str::to_string))
    .collect();

  assert_eq!(
    license_files(dests.iter().map(String::as_str)),
    expected_license_files(),
    "the .rpm must carry both license texts"
  );
}

/// The extra files a release workflow stages into the archive alongside the
/// binary, read off the `cp` / `Copy-Item` line rather than assumed.
///
/// The binary's own copy line is skipped by its `/release/` source prefix:
/// it is the one staging command whose source is a build output rather than a
/// repo file, and its `${{ matrix.target }}` expansion does not tokenise into
/// anything a file-set comparison can use.
fn staged_docs(workflow: &str, marker: &str) -> BTreeSet<String> {
  let body = read(workflow);
  let lines: Vec<&str> = body
    .lines()
    .map(str::trim)
    .filter(|l| l.starts_with(marker) && !l.contains("/release/"))
    .collect();
  assert!(
    !lines.is_empty(),
    "{workflow} has no `{marker}` line staging repo files into the archive"
  );

  lines
    .iter()
    .flat_map(|line| {
      line
        .trim_start_matches(marker)
        .split(|c: char| c.is_whitespace() || c == ',')
    })
    .map(|t| t.trim().trim_matches('"'))
    // The destination is the quoted `dist/…` argument; everything before it
    // is a repo file being copied in.
    .filter(|t| !t.is_empty() && !t.contains("dist/"))
    .map(str::to_string)
    .collect()
}

#[test]
fn every_release_archive_stages_the_same_docs() {
  let sets = [
    (
      "release.yml (linux/macos)",
      staged_docs(".github/workflows/release.yml", "cp "),
    ),
    (
      "release.yml (windows)",
      staged_docs(".github/workflows/release.yml", "Copy-Item "),
    ),
    (
      "pre-release.yml (linux/macos)",
      staged_docs(".github/workflows/pre-release.yml", "cp "),
    ),
    (
      "pre-release.yml (windows)",
      staged_docs(".github/workflows/pre-release.yml", "Copy-Item "),
    ),
  ];

  let (_, reference) = &sets[0];
  assert_eq!(
    license_files(reference.iter().map(String::as_str)),
    expected_license_files(),
    "the release tarball must carry both license texts"
  );

  for (label, set) in &sets[1..] {
    assert_eq!(
      set, reference,
      "{label} stages a different file set than release.yml (linux/macos): the four archives must agree or a platform ships without its license"
    );
  }
}

#[test]
fn the_aur_package_installs_only_files_the_release_archive_carries() {
  let template = read("packaging/aur/PKGBUILD.template");
  let staged = staged_docs(".github/workflows/release.yml", "cp ");

  // Sources of `install -Dm… <src> "$pkgdir/…"` rows. Anything the
  // `package()` body writes itself (`./gwm completions bash >gwm.bash`) comes
  // from the binary, not from the archive, and is derived here rather than
  // spelled out so a fourth completion shell does not need a test edit.
  let installed: BTreeSet<String> = template
    .lines()
    .map(str::trim)
    .filter(|l| l.starts_with("install -Dm"))
    .filter_map(|l| l.split_whitespace().nth(2).map(str::to_string))
    .filter(|src| !template.contains(&format!(">{src}")))
    .collect();

  assert!(
    !installed.is_empty(),
    "no `install -Dm… <src>` row found in the PKGBUILD template: the extractor is not reading the package() body"
  );

  for src in &installed {
    // The binary is the archive's reason to exist and is not on the `cp` line.
    assert!(
      src == "gwm" || staged.contains(src),
      "the AUR PKGBUILD installs `{src}` out of the release archive, but the release workflow never stages it: makepkg would fail at the next release, and nothing else here would notice"
    );
  }

  assert_eq!(
    license_files(installed.iter().map(String::as_str)),
    expected_license_files(),
    "the AUR package must install both license texts, not just the one the single-license PKGBUILD installed"
  );
}
