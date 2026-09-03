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

/// `1`-based line numbers holding a container directive, outside code fences.
///
/// Fenced blocks are skipped because a page is allowed to *show* the syntax it
/// does not use. Line endings are normalised first: Windows runners check out
/// with `core.autocrlf=true` and the fence tracking would otherwise see
/// `` ```\r ``.
fn container_lines(page: &Path) -> Vec<(usize, String)> {
  let text = fs::read_to_string(page)
    .unwrap_or_else(|err| panic!("{} must be readable: {err}", page.display()))
    .replace("\r\n", "\n");
  let mut out = Vec::new();
  let mut in_fence = false;
  for (index, line) in text.lines().enumerate() {
    if line.trim_start().starts_with("```") {
      in_fence = !in_fence;
      continue;
    }
    if in_fence {
      continue;
    }
    if line.trim_start().starts_with(":::") {
      out.push((index + 1, line.trim().to_string()));
    }
  }
  out
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
  for page in &pages {
    let relative = page.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(page);
    for (line, text) in container_lines(page) {
      offenders.push(format!("{}:{line}: {text}", relative.display()));
    }
  }

  assert!(
    offenders.is_empty(),
    "{} container directive(s) found across {} pages; `:::` renders on neither \
     GitHub nor the docs site — use a GFM alert (`> [!TIP]`, `> [!WARNING]`):\n{}",
    offenders.len(),
    pages.len(),
    offenders.join("\n")
  );
}
