//! Guard on the callout syntax used across `docs/**` (issue #641).
//!
//! A `:::` container is markdown-it / Nuxt Content syntax, and this tree is
//! read by neither. It ships green and renders **nowhere**:
//!
//! - **GitHub** does GFM alerts, not containers. The blob page for
//!   `docs/5.integrations/5.gitlab.md` output `<p>::: warning A bare
//!   <code>forge</code> key authorises nothing …</p>` — raw punctuation where
//!   a security callout was meant to be, on the page that travels with the
//!   code.
//! - **The docs site** (Starlight, via the `kbrdn-docs` sync) wants
//!   `:::caution[title]`, colons glued to the name, and only knows `note`,
//!   `tip`, `caution`, `danger`. `::: warning` matches neither half of that
//!   and is not parsed at all.
//!
//! So the guard rejects the **whole** `:::` prefix, not just the spaced form
//! the tree happened to carry: `:::note` is valid Starlight and would render
//! on the site while staying raw text on GitHub, which is the same defect
//! seen from the other side. After #641 the repo has one answer, GFM alerts
//! (`> [!TIP]`, `> [!WARNING]`, …), and the site half of the mapping lives in
//! the sync script (kbrdn1/kbrdn-docs#81) rather than in this tree.

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

/// The number of lines read outside code fences, and the `1`-based line
/// numbers among them holding a container directive.
///
/// The count is returned rather than the offenders alone (#649) because
/// `pages.is_empty()` proves a page was found, not that a line of it was
/// inspected: a fence tracker stuck on, which is what an unclosed fence does,
/// skips every line of a page and reports no offender exactly like a clean
/// page.
///
/// Fenced blocks are skipped because a page is allowed to *show* the syntax it
/// does not use. Line endings are normalised first: Windows runners check out
/// with `core.autocrlf=true` and the fence tracking would otherwise see
/// `` ```\r ``.
fn container_lines(page: &Path) -> (usize, Vec<(usize, String)>) {
  let text = fs::read_to_string(page)
    .unwrap_or_else(|err| panic!("{} must be readable: {err}", page.display()))
    .replace("\r\n", "\n");
  let mut out = Vec::new();
  let mut inspected = 0usize;
  let mut in_fence = false;
  for (index, line) in text.lines().enumerate() {
    if line.trim_start().starts_with("```") {
      in_fence = !in_fence;
      continue;
    }
    if in_fence {
      continue;
    }
    inspected += 1;
    if line.trim_start().starts_with(":::") {
      out.push((index + 1, line.trim().to_string()));
    }
  }
  (inspected, out)
}

/// Issue #649. Every page closes the code fences it opens.
///
/// Not a style rule: two guards follow fences to decide what to skip, this
/// file's `container_lines` and `docs_assets_tests::image_targets`, and both
/// toggle on a line starting with ` ``` `. An unclosed fence leaves the
/// toggle on for the rest of the file, so everything after it is read as
/// fenced and skipped. A `:::` directive or a broken image reference below an
/// orphan fence is invisible to the guard that exists to catch it, and the
/// guard stays green.
///
/// `docs/fr/6.development/1.testing.md` carried one at its last line when this
/// was written, so nothing was blinded, which is exactly how such a line
/// survives: at end of file it costs nothing until someone appends to the page.
///
/// The rule here is copied from those two followers on purpose, ` ``` ` after
/// `trim_start`, CRLF normalised first. A guard that recognised fences
/// differently could pass while they are blind, which is the one thing it must
/// not do. `docs/` holds no `~~~` fence and no four-backtick fence today, and
/// neither follower would see one either, so parity counting is what matches
/// what they do.
#[test]
fn every_docs_page_closes_its_code_fences() {
  let root = docs_root();
  let mut unclosed = Vec::new();
  let mut inspected = 0usize;
  for page in markdown_pages() {
    let text = fs::read_to_string(&page)
      .unwrap_or_else(|err| panic!("{} must be readable: {err}", page.display()))
      .replace("\r\n", "\n");
    let mut opened_at = None;
    for (index, line) in text.lines().enumerate() {
      inspected += 1;
      if line.trim_start().starts_with("```") {
        opened_at = match opened_at {
          None => Some(index + 1),
          Some(_) => None,
        };
      }
    }
    if let Some(line) = opened_at {
      unclosed.push(format!(
        "{}:{line}",
        page.strip_prefix(&root).unwrap_or(&page).display()
      ));
    }
  }
  // Deliberately under, the policy for a high-churn corpus (#649): a markdown
  // line is not an owned unit, every rewrite moves the count, and an exact
  // floor would redden a paragraph deletion. ~80% of what the tree reads
  // today, which still catches the failure this exists for: a fence tracker
  // stuck on, swallowing whole pages.
  assert!(
    inspected >= 11500,
    "expected the walk to read the lines of the pages under docs/, found {inspected}"
  );
  assert!(
    unclosed.is_empty(),
    "these pages open a code fence they never close, so everything below it reads as fenced \
     and both `container_lines` and `docs_assets_tests::image_targets` skip it:\n  {}",
    unclosed.join("\n  ")
  );
}

#[test]
fn docs_carry_no_container_directives() {
  let pages = markdown_pages();
  assert!(
    !pages.is_empty(),
    "no markdown found under {} — the guard would pass on an empty tree",
    docs_root().display()
  );

  let mut offenders = Vec::new();
  let mut inspected = 0usize;
  for page in &pages {
    let relative = page.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(page);
    let (read, found) = container_lines(page);
    inspected += read;
    for (line, text) in found {
      offenders.push(format!("{}:{line}: {text}", relative.display()));
    }
  }

  // Deliberately under, the policy for a high-churn corpus (#649): a markdown
  // line is not an owned unit, every rewrite moves the count, and an exact
  // floor would redden a paragraph deletion. ~80% of what the tree reads
  // today, which still catches the failure this exists for: a fence tracker
  // stuck on, swallowing whole pages.
  assert!(
    inspected >= 8800,
    "expected the walk to read the unfenced lines of the {} pages under docs/, found {inspected}",
    pages.len()
  );
  assert!(
    offenders.is_empty(),
    "{} container directive(s) found across {} pages; `:::` renders on neither \
     GitHub nor the docs site — use a GFM alert (`> [!TIP]`, `> [!WARNING]`):\n{}",
    offenders.len(),
    pages.len(),
    offenders.join("\n")
  );
}
