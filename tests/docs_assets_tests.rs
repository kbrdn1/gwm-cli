//! Guards for the doc captures referenced from `docs/**` (issue #524).
//!
//! Three failure modes, all of which ship green and break at publish time,
//! which is why they are pinned here rather than left to review:
//!
//! 1. **A reference resolves to nothing.** A renamed or never-generated
//!    capture leaves a broken image on the published site: the markdown is
//!    valid, the build is green, and only a human looking at the page sees
//!    the gap.
//! 2. **The French mirror drifts.** The sync generates both locales from this
//!    tree, and a capture added on the English side alone leaves the French
//!    page text-only forever. That divergence is exactly the class of gap
//!    #524 exists to close, so it is closed by a test and not by discipline.
//! 3. **Two captures share a basename.** `sync-gwm-docs.mjs` flattens every
//!    `_assets` directory into one `src/assets/captures/` and **throws** on a
//!    duplicate name, so a second `hero.png` in another section fails the
//!    docs deploy, far from the PR that introduced it.
//!
//! Parsing note: image references are read outside fenced code blocks only.
//! `docs/6.development/3.stability.md` documents `#![doc(hidden)]`, which a
//! naive `![` sweep reads as an image reference.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn docs_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs")
}

/// Every `.md` under `docs/`, repo-relative, sorted for stable failures.
fn markdown_pages() -> Vec<PathBuf> {
  let mut out = Vec::new();
  collect_markdown(&docs_root(), &mut out);
  out.sort();
  out
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
  let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("{} must be readable: {err}", dir.display()));
  for entry in entries.flatten() {
    let path = entry.path();
    if path.is_dir() {
      collect_markdown(&path, out);
    } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
      out.push(path);
    }
  }
}

/// The link targets of every markdown image on a page, in source order.
///
/// Line endings are normalised because Windows runners check out with
/// `core.autocrlf=true`; the fence tracking would otherwise see `` ```\r ``.
fn image_targets(page: &Path) -> Vec<String> {
  let text = fs::read_to_string(page)
    .unwrap_or_else(|err| panic!("{} must be readable: {err}", page.display()))
    .replace("\r\n", "\n");
  let mut out = Vec::new();
  let mut in_fence = false;
  for line in text.lines() {
    if line.trim_start().starts_with("```") {
      in_fence = !in_fence;
      continue;
    }
    if in_fence {
      continue;
    }
    out.extend(images_in_line(line));
  }
  out
}

/// `![alt](target)` occurrences in one line.
///
/// Two things that look like an image reference and are not: the Rust inner
/// attribute `#![…]`, and a reference written inside an inline code span,
/// which is prose *about* the syntax (`docs/README.md`'s authoring note shows
/// `![keymap](./_assets/tui-keymap.png)`, a file that is not meant to exist).
/// The span test counts backticks to the left rather than splitting the line,
/// because an alt text may legitimately contain code: splitting would drop
/// `![`gwm list`: …](./_assets/cli-list.png)` and leave the guard vacuous.
fn images_in_line(line: &str) -> Vec<String> {
  let bytes = line.as_bytes();
  let mut out = Vec::new();
  let mut i = 0;
  while let Some(bang) = line[i..].find("![") {
    let at = i + bang;
    i = at + 2;
    if at > 0 && bytes[at - 1] == b'#' {
      continue; // `#![doc(hidden)]`, not an image
    }
    if line[..at].matches('`').count() % 2 == 1 {
      continue; // inside an inline code span
    }
    let Some(close) = line[i..].find(']') else {
      continue;
    };
    let after_alt = i + close + 1;
    if line.as_bytes().get(after_alt) != Some(&b'(') {
      continue; // reference-style or plain text, no inline target
    }
    let Some(end) = line[after_alt + 1..].find(')') else {
      continue;
    };
    out.push(line[after_alt + 1..after_alt + 1 + end].to_string());
    i = after_alt + 1 + end;
  }
  out
}

fn is_local_asset(target: &str) -> bool {
  !target.starts_with("http://") && !target.starts_with("https://") && !target.starts_with("data:")
}

fn basename(target: &str) -> &str {
  target.rsplit('/').next().unwrap_or(target)
}

/// The set of capture file names a page shows, locale-independent.
fn referenced_basenames(page: &Path) -> BTreeSet<String> {
  image_targets(page)
    .iter()
    .filter(|t| is_local_asset(t))
    .map(|t| basename(t).to_string())
    .collect()
}

/// Every image reference resolves to a file that exists on disk.
#[test]
fn every_referenced_capture_exists() {
  let mut missing = Vec::new();
  for page in markdown_pages() {
    let dir = page.parent().expect("a page has a parent directory");
    for target in image_targets(&page) {
      if !is_local_asset(&target) {
        continue;
      }
      // Strip a `#fragment` / `?query`, which markdown allows on an image.
      let clean = target.split(['#', '?']).next().unwrap_or(&target);
      if !dir.join(clean).exists() {
        missing.push(format!("{} → {target}", page.display()));
      }
    }
  }
  assert!(
    missing.is_empty(),
    "these image references resolve to nothing (regenerate with docs/_capture/generate.sh, \
     or fix the path):\n  {}",
    missing.join("\n  ")
  );
}

/// An English page and its French mirror show the same captures.
///
/// Compared by basename, not by path: the two locales reach the same file
/// through different relative prefixes (`./_assets/x.png` against
/// `../../<section>/_assets/x.png`), which is the shape the sync rewrites.
#[test]
fn french_mirrors_reference_the_same_captures() {
  let root = docs_root();
  let fr_root = root.join("fr");
  let mut divergent = Vec::new();
  for page in markdown_pages() {
    if page.starts_with(&fr_root) {
      continue;
    }
    let Ok(rel) = page.strip_prefix(&root) else {
      continue;
    };
    let mirror = fr_root.join(rel);
    if !mirror.exists() {
      continue; // untranslated pages are a separate concern
    }
    let en = referenced_basenames(&page);
    let fr = referenced_basenames(&mirror);
    if en != fr {
      divergent.push(format!(
        "{}\n    EN: {:?}\n    FR: {:?}",
        rel.display(),
        en.iter().collect::<Vec<_>>(),
        fr.iter().collect::<Vec<_>>()
      ));
    }
  }
  assert!(
    divergent.is_empty(),
    "these pages show different captures in English and in French; the sync builds both \
     locales from this tree, so a capture added on one side has to be referenced from the \
     other:\n  {}",
    divergent.join("\n  ")
  );
}

/// No two captures share a basename across the whole tree.
#[test]
fn capture_basenames_are_unique_across_sections() {
  let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
  collect_assets(&docs_root(), &mut by_name);
  let clashes: Vec<String> = by_name
    .iter()
    .filter(|(_, paths)| paths.len() > 1)
    .map(|(name, paths)| {
      let places: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
      format!("{name}: {}", places.join(", "))
    })
    .collect();
  assert!(
    clashes.is_empty(),
    "the docs sync flattens every `_assets` directory into one and throws on a duplicate \
     name, so these clashes would fail the site deploy:\n  {}",
    clashes.join("\n  ")
  );
}

fn collect_assets(dir: &Path, out: &mut BTreeMap<String, Vec<PathBuf>>) {
  let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("{} must be readable: {err}", dir.display()));
  for entry in entries.flatten() {
    let path = entry.path();
    if path.is_dir() {
      collect_assets(&path, out);
    } else if path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) == Some("_assets") {
      let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .expect("an asset has a UTF-8 file name")
        .to_string();
      out.entry(name).or_default().push(path);
    }
  }
}
