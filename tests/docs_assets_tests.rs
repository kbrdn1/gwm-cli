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
//! 4. **A capture ships at 1x and blurs on the site** (issue #581). The site
//!    paints a capture wider than its own pixels (a 1000px `hero.png` measured
//!    at 1230 CSS px in a 2560px viewport), and a HiDPI display doubles that
//!    again, so anything rendered at terminal scale is upscaled before a reader
//!    ever sees it. Nothing downstream fails: the image resolves, the build is
//!    green, the text is simply soft.
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

/// The captures stay out of the published crate.
///
/// `cargo package` on 1.8.0 measured **9.8 MiB compressed of the 10 MiB
/// crates.io limit** (442 files, 14.7 MiB uncompressed), and `docs/` was
/// 9.2 MiB of that tree with no `exclude` in sight. Doubling the captures
/// pushes the tarball past the limit, so the release fails at `cargo publish`,
/// long after this PR merged and with nothing in the diff pointing at it.
///
/// crates.io rewrites relative image links in a README against `repository`,
/// so dropping `docs/` costs the published page nothing.
#[test]
fn the_published_crate_excludes_the_docs_tree() {
  let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
  let raw = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
  let manifest: toml::Value = toml::from_str(&raw).expect("Cargo.toml is valid TOML");
  let excluded: Vec<String> = manifest
    .get("package")
    .and_then(|p| p.get("exclude"))
    .and_then(|e| e.as_array())
    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
    .unwrap_or_default();
  assert!(
    excluded
      .iter()
      .any(|pattern| pattern == "docs/" || pattern == "/docs/" || pattern == "docs"),
    "[package] exclude must drop `docs/` from the published crate, or the 2x captures take the \
     tarball over the crates.io size limit; found {excluded:?}"
  );
}

/// The narrowest terminal any tape photographs is `narrow.tape`, and it ships
/// at 1580 px rather than the 1600 a straight doubling would suggest.
///
/// The 20 px are the point. Doubling `Set Width` exactly moves the terminal
/// grid: measured with a `tput cols` tape, 800 px at `FontSize 15` gives
/// 81x31 and 1600 px at `FontSize 30` gives one column more, which reframes
/// the capture instead of merely sharpening it. 1580 px lands back on 81x31.
/// The same trim applies to every tape (`hero` is 1980, not 2000, and holds
/// its 103x31).
///
/// A floor rather than an exact width: widths are per-tape (1580 to 2760) and
/// `promo.png` is not a vhs capture at all. What keeps a *new* tape honest is
/// [`every_tape_renders_at_retina_density`]; this one catches an asset that
/// was regenerated from somewhere other than its tape.
const MIN_CAPTURE_WIDTH: u32 = 1580;

/// The `Set FontSize` every tape must declare: 15 was the 1x size, so 2x is
/// 30. vhs 0.11 has no `Set Scale` (`Unknown setting: Scale`), and font size
/// is what fixes the pixels-per-cell: doubling it together with `Set Width`,
/// `Set Height` and `Set Padding` keeps the same terminal grid (measured
/// 103x31 columns at 1x against 104x32 at 2x) and doubles the output.
const RETINA_FONT_SIZE: &str = "Set FontSize 30";

/// Every capture ships at 2x so the site has pixels to paint with.
///
/// Reads the width out of the file header rather than trusting the tape: the
/// two tapes `generate.sh` deliberately skips (`demo`, `github-linking`) are
/// regenerated by hand, and a hand step is exactly where a 1x asset survives a
/// pass that doubled every tape.
#[test]
fn every_capture_ships_at_retina_density() {
  let mut thin = Vec::new();
  for asset in capture_files() {
    let Some((width, _)) = image_size(&asset) else {
      panic!("{} is neither a PNG nor a GIF", asset.display());
    };
    if width < MIN_CAPTURE_WIDTH {
      thin.push(format!("{} is {width}px wide", asset.display()));
    }
  }
  assert!(
    thin.is_empty(),
    "these captures are below {MIN_CAPTURE_WIDTH}px and the site will upscale them; regenerate \
     with docs/_capture/generate.sh (and by hand for demo.tape / github-linking.tape):\n  {}",
    thin.join("\n  ")
  );
}

/// Every tape declares the 2x font size.
///
/// The companion to the check above, and the one that holds: a new tape copied
/// from an older one arrives at `Set FontSize 15`, produces a capture whose
/// width happens to clear the floor, and reintroduces #581 one asset at a time.
#[test]
fn every_tape_renders_at_retina_density() {
  let mut stale = Vec::new();
  for tape in tape_files() {
    let text = fs::read_to_string(&tape)
      .unwrap_or_else(|err| panic!("{} must be readable: {err}", tape.display()))
      .replace("\r\n", "\n");
    let declared: Vec<&str> = text
      .lines()
      .map(str::trim_end)
      .filter(|l| l.starts_with("Set FontSize"))
      .collect();
    if declared != [RETINA_FONT_SIZE] {
      stale.push(format!("{}: {declared:?}", tape.display()));
    }
  }
  assert!(
    stale.is_empty(),
    "every tape must declare `{RETINA_FONT_SIZE}` exactly once, with `Set Width`, `Set Height` \
     and `Set Padding` doubled to match, or its capture ships at terminal scale and blurs on the \
     site (#581):\n  {}",
    stale.join("\n  ")
  );
}

/// Every `.png` / `.gif` under `docs/`, repo-relative, sorted.
///
/// Not restricted to `_assets/`: `demo.gif` lives in `docs/_capture/` and is
/// the capture the README shows first.
fn capture_files() -> Vec<PathBuf> {
  let mut out = Vec::new();
  collect_by_extension(&docs_root(), &["png", "gif"], &mut out);
  out.sort();
  out
}

fn tape_files() -> Vec<PathBuf> {
  let mut out = Vec::new();
  collect_by_extension(&docs_root().join("_capture"), &["tape"], &mut out);
  out.sort();
  out
}

fn collect_by_extension(dir: &Path, wanted: &[&str], out: &mut Vec<PathBuf>) {
  let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("{} must be readable: {err}", dir.display()));
  for entry in entries.flatten() {
    let path = entry.path();
    if path.is_dir() {
      collect_by_extension(&path, wanted, out);
    } else if path
      .extension()
      .and_then(|e| e.to_str())
      .is_some_and(|e| wanted.contains(&e))
    {
      out.push(path);
    }
  }
}

/// `(width, height)` from a PNG or GIF header, or `None` for anything else.
///
/// Hand-parsed to keep the guard dependency-free: a PNG's IHDR is the first
/// chunk and carries two big-endian u32 at offset 16; a GIF's logical screen
/// descriptor follows the 6-byte signature with two little-endian u16.
fn image_size(path: &Path) -> Option<(u32, u32)> {
  let bytes = fs::read(path).unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
  if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 && &bytes[12..16] == b"IHDR" {
    let be = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    return Some((be(16), be(20)));
  }
  if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len() >= 10 {
    let le = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as u32;
    return Some((le(6), le(8)));
  }
  None
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
