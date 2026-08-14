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
  // frozen, so its hash is a constant, and hashing the whole content is the
  // only honest verbatim check: length plus the anchors below would pass a
  // same-length edit that avoids them, and what is being pinned is a license
  // text this project redistributes.
  //
  // The git blob id is the digest of choice here rather than a raw SHA-256:
  // `git2` is already a direct dependency (a `sha2` dev-dependency does not
  // even link, the graph carries both 0.10 and 0.11 and rustc cannot pick),
  // the header git prepends folds the length into the hash, and the constant
  // is checkable by anyone in one line: `git hash-object LICENSE-APACHE`.
  let oid = git2::Oid::hash_object(git2::ObjectType::Blob, body.as_bytes()).expect("hash LICENSE-APACHE");
  assert_eq!(
    oid.to_string(),
    "d645695673349e3947e8e5ae42332d0ac3164cd7",
    "LICENSE-APACHE is not the upstream Apache-2.0 text (it is {} bytes; upstream is 11358)",
    body.len()
  );

  // Kept for the failure message: the digest says "wrong", the anchors say
  // which part went missing.
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
/// The distinction is what follows the key: a declaration is `license`
/// followed by an assignment token (`=`, `:`, `(`, `":`, `any_of`), whereas
/// `LICENSE-MIT` and `licenses/$pkgname` are followed by `-` and `s`. Without
/// it every `install -Dm644 LICENSE-MIT …` row reads as a declaration that
/// names only one half.
///
/// Comment lines are excluded, and that exclusion is what keeps the guard from
/// answering itself. The comments explaining *why* each channel spells the
/// disjunction the way it does necessarily quote both halves, so a file whose
/// real declaration is deleted would still satisfy "declares both licenses"
/// out of its own prose while the formula declares nothing at all.
fn is_declaration(line: &str) -> bool {
  let trimmed = line.trim_start();
  if trimmed.starts_with('#') || trimmed.starts_with("//") {
    return false;
  }

  let lower = line.to_lowercase();
  let mut from = 0;
  while let Some(at) = lower[from..].find("licen") {
    let start = from + at;
    let rest = &lower[start + "licen".len()..];
    if let Some(tail) = rest.strip_prefix("se").or_else(|| rest.strip_prefix("ce")) {
      let tail = tail.trim_start();
      // `":` and not a bare `"`: Scoop's key is quoted (`"license": "..."`),
      // so the closing quote is followed by the colon. A bare `"` also matched
      // the *end* of any packaged path ending in the word, which is how
      // `["third-party/zlib-LICENSE", ...]` read as a declaration naming one
      // half (#577).
      for token in ["=", ":", "(", "\":", "any_of", "all_of"] {
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

/// Does the declaration grant a *choice* between the two, rather than imposing
/// both at once?
///
/// Naming both halves is not enough, and the difference is not cosmetic: every
/// one of these syntaxes also has a conjunction form, one character or one word
/// away. `MIT AND Apache-2.0`, `all_of:`, and Scoop's `,` all say the user must
/// comply with both licenses simultaneously, which is a materially different
/// and strictly narrower grant than the one this project offers. A guard that
/// only looks for the two names passes every one of them.
fn grants_a_choice(line: &str) -> bool {
  // Conjunctions are rejected outright rather than merely not matched, so a
  // line carrying both an `OR` and an `AND` cannot squeak through.
  if line.contains(" AND ") || line.contains("all_of") {
    return false;
  }
  // Scoop separates alternatives with `|` and co-applying licenses with `,`.
  if line.contains("\"license\"") {
    return line.contains('|');
  }
  // nixpkgs has no operator: dual licensing is spelled as a list of attributes.
  if line.contains("licenses.") {
    return line.contains('[');
  }
  line.contains(" OR ") || line.contains("any_of")
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
        continue;
      }
      // A license *text* is not a channel declaring its license, and reading
      // one as such is nonsense: DEP-5 gives every license its own
      // `License: <name>` paragraph header, so a correct copyright file is
      // full of lines naming exactly one half. These are pinned by
      // `the_deb_copyright_describes_both_licenses` instead. The rule is
      // structural rather than a path list, so a new channel still cannot slip
      // past by being added here.
      if name == "copyright" || name.starts_with("LICENSE") {
        continue;
      }
      out.push(rel);
    }
  }
  walk(&root().join("packaging"), "packaging", &mut surfaces);

  surfaces
}

#[test]
fn a_comment_quoting_a_license_is_not_a_declaration() {
  // Every line here is lifted from a comment this repo actually carries. Each
  // one names both halves, so without the comment filter each would satisfy
  // the walk on its own and let the file's real declaration be deleted
  // unnoticed.
  for comment in [
    r#"  # `license "MIT OR Apache-2.0"` string is not an SPDX expression to brew,"#,
    "# inlining one half under a `License: MIT OR Apache-2.0` header would state",
    "// license = \"MIT OR Apache-2.0\"",
  ] {
    assert!(
      !is_declaration(comment),
      "a comment must not count as a declaration: {comment}"
    );
  }

  // Nor does a packaged *path* that happens to end in the word. These are the
  // asset rows #577 added; before the token was tightened from `"` to `":`,
  // the closing quote of the path read as Scoop's quoted key and the row was
  // judged a declaration naming a single half.
  for path_row in [
    r#"  ["third-party/zlib-LICENSE", "usr/share/doc/gwm-cli/zlib-LICENSE", "644"],"#,
    r#"  { source = "third-party/zlib-LICENSE", dest = "/usr/share/doc/gwm-cli/zlib-LICENSE", mode = "644" },"#,
  ] {
    assert!(
      !is_declaration(path_row),
      "a packaged path is not a declaration: {path_row}"
    );
  }

  // The real declarations still register, in every syntax the repo uses.
  for real in [
    r#"license = "MIT OR Apache-2.0""#,
    r#"  "license": "MIT|Apache-2.0","#,
    "license=('MIT OR Apache-2.0')",
    r#"  license any_of: ["MIT", "Apache-2.0"]"#,
    "            license = [ licenses.asl20 licenses.mit ];",
  ] {
    assert!(is_declaration(real), "a real declaration must register: {real}");
  }
}

#[test]
fn a_conjunction_is_not_the_grant_this_project_offers() {
  // Each of these is one word or one character away from the real declaration
  // it shadows, names both halves, and says the opposite thing: comply with
  // both licenses at once rather than pick one. Naming-only checks pass them
  // all, which is why the operator is asserted separately.
  for conjunction in [
    r#"license = "MIT AND Apache-2.0""#,
    r#"  "license": "MIT,Apache-2.0","#,
    "license=('MIT AND Apache-2.0')",
    r#"  license all_of: ["MIT", "Apache-2.0"]"#,
    "            license = licenses.asl20;",
  ] {
    assert!(
      names_mit(conjunction) || names_apache(conjunction),
      "fixture must still name a license, else it proves nothing: {conjunction}"
    );
    assert!(
      !grants_a_choice(conjunction),
      "a conjunction must not read as the dual-license grant: {conjunction}"
    );
  }

  for disjunction in [
    r#"license = "MIT OR Apache-2.0""#,
    r#"  "license": "MIT|Apache-2.0","#,
    "license=('MIT OR Apache-2.0')",
    r#"  license any_of: ["MIT", "Apache-2.0"]"#,
    "            license = [ licenses.asl20 licenses.mit ];",
  ] {
    assert!(
      grants_a_choice(disjunction),
      "the real declaration must read as a choice: {disjunction}"
    );
  }
}

#[test]
fn no_packaging_surface_publishes_an_em_dash() {
  // #567 swept the em dash out of `src/`, but its guard is scoped to `src/`
  // and these files are not there, so the same one-line tagline kept one in
  // eight published fields: the crates.io description, the deb
  // `extended-description`, the rpm `summary`, both `flake.nix` descriptions,
  // and the Homebrew / Scoop / AUR blurbs. A ninth sat in the flake's
  // `shellHook`, which prints to whoever runs `nix develop`. Every one of them
  // is read by a user in a package listing, which is exactly the surface #567
  // argues is as published as the README.
  //
  // Comments are excluded, on the same reasoning #567 used: a comment is not
  // published, and sweeping those belongs to that campaign, not to this file.
  let surfaces = declaring_surfaces();
  let mut published = 0usize;

  for rel in &surfaces {
    for (n, line) in read(rel).lines().enumerate() {
      let trimmed = line.trim_start();
      if trimmed.starts_with('#') || trimmed.starts_with("//") {
        continue;
      }
      published += 1;
      assert!(
        !line.contains('\u{2014}'),
        "{rel}:{} publishes an em dash: {}",
        n + 1,
        line.trim()
      );
    }
  }

  // Non-vacuity: a walk that read nothing, or a comment filter that swallowed
  // every line, would pass the loop above without inspecting a single
  // published string.
  assert!(
    published >= 100,
    "only {published} non-comment line(s) inspected across {} surface(s): the walk or the comment filter is eating the file",
    surfaces.len()
  );
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
        "{rel} declares a license that does not name both halves: {}",
        line.trim()
      );
      assert!(
        grants_a_choice(line),
        "{rel} names both halves but does not grant a choice between them, which is a narrower license than this project offers: {}",
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
    "`license-file` at the [package] level names a single file and would contradict the disjunction above; the deb block carries its own, for a different reason"
  );
}

/// One DEP-5 paragraph body, un-indented back to plain text.
///
/// DEP-5 indents a license body by one space and writes blank lines as ` .`,
/// so the inverse of that transform is what has to match `LICENSE-MIT`.
fn dep5_paragraph(copyright: &str, header: &str) -> Option<String> {
  let after = copyright.split(&format!("\n{header}\n")).nth(1)?;
  let body: Vec<&str> = after
    .lines()
    .take_while(|l| l.starts_with(' '))
    .map(|l| if l.trim() == "." { "" } else { &l[1..] })
    .collect();
  Some(body.join("\n").trim_end().to_string())
}

#[test]
fn the_deb_copyright_describes_both_licenses() {
  // cargo-deb generates this file when it can, and neither shape it generates
  // is right here: without `license-file` the `License:` header ships with no
  // text under it, and with it, one file is pasted verbatim and the other half
  // goes undescribed. An asset landing on the copyright path wins over the
  // generator, so the file below is the one that ships.
  let dests: Vec<String> = deb()["assets"]
    .as_array()
    .expect("deb assets is an array")
    .iter()
    .filter_map(|r| r.get(1).and_then(|v| v.as_str()).map(str::to_string))
    .collect();
  assert!(
    dests.iter().any(|d| d == "usr/share/doc/gwm-cli/copyright"),
    "the deb must ship its own copyright file, else cargo-deb generates one that describes a single half: {dests:?}"
  );
  assert!(
    deb().get("license-file").is_none(),
    "`license-file` would have cargo-deb generate a copyright that the asset then has to fight over"
  );

  let copyright = read("packaging/debian/copyright");

  // The package-level grant, in the `Files: *` paragraph.
  assert!(
    copyright.contains("\nLicense: MIT or Apache-2.0\n"),
    "the Files paragraph must offer the choice, not one half"
  );

  // MIT is inlined because it is not in `/usr/share/common-licenses`. Deriving
  // the expected body from `LICENSE-MIT` rather than restating it is what
  // makes the duplication safe: an edit to the license that is not carried
  // across reddens here.
  let mit = read("LICENSE-MIT");
  let expected = mit.replacen("# MIT License\n\n", "", 1).trim().to_string();
  let got = dep5_paragraph(&copyright, "License: MIT").expect("copyright carries a `License: MIT` paragraph");
  assert_eq!(
    got, expected,
    "the MIT paragraph has drifted from LICENSE-MIT: the copyright file must be regenerated from it"
  );

  // Apache-2.0 is a Debian common license, so it is referenced rather than
  // inlined, and the reference has to name the path a Debian system uses.
  let apache =
    dep5_paragraph(&copyright, "License: Apache-2.0").expect("copyright carries a `License: Apache-2.0` paragraph");
  assert!(
    apache.contains("/usr/share/common-licenses/Apache-2.0"),
    "the Apache paragraph must point at the common-licenses copy: {apache}"
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
    // The trailing newline is load-bearing: without it `>gwm` matches inside
    // `>gwm.bash`, so the binary reads as generated and its install line is
    // never compared against anything. `read()` normalises CRLF, so `\n` is
    // the end of a redirect on every runner.
    .filter(|src| !template.contains(&format!(">{src}\n")))
    .collect();

  assert!(
    !installed.is_empty(),
    "no `install -Dm… <src>` row found in the PKGBUILD template: the extractor is not reading the package() body"
  );

  // `cp a/b/c dist/STAGE/` writes `dist/STAGE/c`, so the archive is flat and
  // the PKGBUILD names basenames. A staged entry carrying a directory (#577
  // stages out of `third-party/`) has to be compared the same way, or the
  // guard reports a mismatch the archive does not have.
  let staged: BTreeSet<&str> = staged.iter().map(|p| p.rsplit('/').next().expect("basename")).collect();

  for src in &installed {
    // The binary is the archive's reason to exist and is not on the `cp` line.
    assert!(
      src == "gwm" || staged.contains(src.as_str()),
      "the AUR PKGBUILD installs `{src}` out of the release archive, but the release workflow never stages it: makepkg would fail at the next release, and nothing else here would notice"
    );
  }

  assert_eq!(
    license_files(installed.iter().map(String::as_str)),
    expected_license_files(),
    "the AUR package must install both license texts, not just the one the single-license PKGBUILD installed"
  );
}
