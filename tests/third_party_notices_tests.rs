//! The statically-linked third-party libraries ship their notices (#577).
//!
//! `git2` is built with `vendored-libgit2` and `libz-sys` with `static`, so
//! every `gwm` binary this project distributes *contains* libgit2 and zlib.
//! libgit2 is GPLv2 with a linking exception, and that exception is
//! conditional: it permits linking into a work under any license provided the
//! libgit2 notice travels with the distribution. zlib's own terms say its
//! notice may not be removed or altered. Neither was shipping, under the
//! previous MIT-only license as much as under the current dual one, which is
//! why this is its own change rather than part of #573.
//!
//! Two halves are guarded here, and the second is the one that keeps this from
//! going stale. Shipping the files is easy to get right once; keeping them
//! matched to the versions actually compiled in is not, because a dependabot
//! bump moves the vendored library without touching anything a reviewer reads.
//! So the recorded versions are checked against `Cargo.lock`: bump the crate
//! and this reddens, which is the prompt to refresh the notice.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
  let path = root().join(rel);
  fs::read_to_string(&path)
    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    .replace("\r\n", "\n")
}

/// Every notice this repo vendors, with the crate whose build produces it.
///
/// `libgit2-sys` and `libz-sys` both carry the upstream version in their own
/// version's build metadata or, for zlib, in the `README` of the crate; the
/// provenance file records what was vendored and is checked against the
/// lockfile below.
const NOTICES: [(&str, &str); 2] = [
  ("third-party/libgit2-COPYING", "libgit2-sys"),
  ("third-party/zlib-LICENSE", "libz-sys"),
];

#[test]
fn both_notices_are_committed() {
  for (rel, _) in NOTICES {
    assert!(
      root().join(rel).is_file(),
      "{rel} is missing: the binary statically contains this library, so its notice has to travel with every artefact"
    );
  }
}

#[test]
fn the_libgit2_notice_is_the_upstream_one() {
  let body = read("third-party/libgit2-COPYING");

  // The linking exception is the whole reason this file is a condition rather
  // than a courtesy: without it, statically linking libgit2 would pull the
  // whole binary under the GPL.
  for anchor in [
    "LINKING EXCEPTION",
    "the authors give you unlimited permission to link the compiled",
    "GNU GENERAL PUBLIC LICENSE",
    "Version 2, June 1991",
  ] {
    assert!(
      body.contains(anchor),
      "third-party/libgit2-COPYING is missing the upstream anchor {anchor:?}"
    );
  }

  // Hashing the whole file is the verbatim check; anchors only make a failure
  // legible. Checkable in one line: `git hash-object third-party/libgit2-COPYING`.
  let oid = git2::Oid::hash_object(git2::ObjectType::Blob, body.as_bytes()).expect("hash the notice");
  assert_eq!(
    oid.to_string(),
    "80788a3ed790689b5b30918d17ec67ccd24e7a20",
    "third-party/libgit2-COPYING is not libgit2 1.9.6's COPYING verbatim (it is {} bytes)",
    body.len()
  );
}

#[test]
fn the_zlib_notice_is_the_upstream_one() {
  let body = read("third-party/zlib-LICENSE");

  for anchor in [
    "Jean-loup Gailly and Mark Adler",
    "This software is provided 'as-is', without any express or implied",
    "Altered source versions must be plainly marked as such",
  ] {
    assert!(
      body.contains(anchor),
      "third-party/zlib-LICENSE is missing the upstream anchor {anchor:?}"
    );
  }

  let oid = git2::Oid::hash_object(git2::ObjectType::Blob, body.as_bytes()).expect("hash the notice");
  assert_eq!(
    oid.to_string(),
    "b7a69d058e616651eae27b3f90c0b7fd36c099b2",
    "third-party/zlib-LICENSE is not the vendored zlib's LICENSE verbatim (it is {} bytes)",
    body.len()
  );
}

/// The version of a crate as the committed lockfile pins it.
fn locked_version(krate: &str) -> String {
  let lock = read("Cargo.lock");
  let needle = format!("name = \"{krate}\"\n");
  let after = lock
    .split(&needle)
    .nth(1)
    .unwrap_or_else(|| panic!("{krate} is not in Cargo.lock"));
  after
    .lines()
    .find_map(|l| l.strip_prefix("version = \""))
    .and_then(|v| v.split('"').next())
    .unwrap_or_else(|| panic!("no version line after {krate} in Cargo.lock"))
    .to_string()
}

#[test]
fn the_recorded_provenance_matches_the_lockfile() {
  // This is the guard that keeps the notices honest over time. A dependabot
  // bump of `libgit2-sys` moves the vendored libgit2 without touching any file
  // a reviewer reads, so without this the committed COPYING would silently
  // describe a version the binary no longer contains.
  let provenance = read("third-party/README.md");

  for (rel, krate) in NOTICES {
    let version = locked_version(krate);
    assert!(
      provenance.contains(&format!("{krate} {version}")),
      "third-party/README.md does not record `{krate} {version}`, which is what Cargo.lock pins today. Refresh {rel} from that version's crate source and update the record."
    );
  }

  // Non-vacuity: an empty or truncated provenance file would satisfy nothing
  // above if the loop ever stopped running.
  assert!(
    provenance.contains("libgit2-sys") && provenance.contains("libz-sys"),
    "third-party/README.md names neither crate: it is not the provenance record this guard reads"
  );
}

// ---------------------------------------------------------------------------
// The notices have to reach every artefact
// ---------------------------------------------------------------------------

fn manifest_meta(key: &str) -> toml::Value {
  let manifest: toml::Value = toml::from_str(&read("Cargo.toml")).expect("Cargo.toml is valid TOML");
  manifest["package"]["metadata"][key].clone()
}

fn notice_basenames() -> BTreeSet<String> {
  NOTICES
    .iter()
    .map(|(rel, _)| rel.rsplit('/').next().expect("basename").to_string())
    .collect()
}

/// The notice basenames present in a set of packaged paths.
fn notices_in<'a>(paths: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
  let wanted = notice_basenames();
  paths
    .filter_map(|p| p.rsplit('/').next())
    .filter(|n| wanted.contains(*n))
    .map(str::to_string)
    .collect()
}

#[test]
fn the_deb_ships_both_notices() {
  let rows = manifest_meta("deb")["assets"].as_array().expect("deb assets").clone();
  let dests: Vec<String> = rows
    .iter()
    .filter_map(|r| r.get(1).and_then(|v| v.as_str()).map(str::to_string))
    .collect();
  assert_eq!(
    notices_in(dests.iter().map(String::as_str)),
    notice_basenames(),
    "the .deb must carry both third-party notices"
  );
}

#[test]
fn the_rpm_ships_both_notices() {
  let rows = manifest_meta("generate-rpm")["assets"]
    .as_array()
    .expect("rpm assets")
    .clone();
  let dests: Vec<String> = rows
    .iter()
    .filter_map(|r| r.get("dest").and_then(|v| v.as_str()).map(str::to_string))
    .collect();
  assert_eq!(
    notices_in(dests.iter().map(String::as_str)),
    notice_basenames(),
    "the .rpm must carry both third-party notices"
  );
}

/// The repo files a release workflow stages into the archive, read off the
/// `cp` / `Copy-Item` line. The binary's own copy line is skipped by its
/// `/release/` source prefix.
fn staged(workflow: &str, marker: &str) -> BTreeSet<String> {
  let body = read(workflow);
  let lines: Vec<&str> = body
    .lines()
    .map(str::trim)
    .filter(|l| l.starts_with(marker) && !l.contains("/release/"))
    .collect();
  assert!(!lines.is_empty(), "{workflow} has no `{marker}` staging line");

  lines
    .iter()
    .flat_map(|l| {
      l.trim_start_matches(marker)
        .split(|c: char| c.is_whitespace() || c == ',')
    })
    .map(|t| t.trim().trim_matches('"'))
    .filter(|t| !t.is_empty() && !t.contains("dist/"))
    .map(str::to_string)
    .collect()
}

#[test]
fn every_release_archive_carries_both_notices() {
  for (workflow, marker) in [
    (".github/workflows/release.yml", "cp "),
    (".github/workflows/release.yml", "Copy-Item "),
    (".github/workflows/pre-release.yml", "cp "),
    (".github/workflows/pre-release.yml", "Copy-Item "),
  ] {
    let set = staged(workflow, marker);
    assert_eq!(
      notices_in(set.iter().map(String::as_str)),
      notice_basenames(),
      "{workflow} ({marker}) stages an archive without both third-party notices: {set:?}"
    );
  }
}

#[test]
fn the_aur_package_installs_both_notices() {
  let template = read("packaging/aur/PKGBUILD.template");
  let installed: BTreeSet<String> = template
    .lines()
    .map(str::trim)
    .filter(|l| l.starts_with("install -Dm"))
    .filter_map(|l| l.split_whitespace().nth(2).map(str::to_string))
    .collect();

  assert_eq!(
    notices_in(installed.iter().map(String::as_str)),
    notice_basenames(),
    "the AUR package must install both third-party notices out of the archive: {installed:?}"
  );
}
