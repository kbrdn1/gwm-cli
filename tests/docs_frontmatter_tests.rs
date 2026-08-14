//! Guards on the `description` frontmatter of `docs/**` (issue #579).
//!
//! The frontmatter of this tree is the source of the published `<title>` and
//! `<meta name="description">`: the `kbrdn-docs` bridge carries it through
//! verbatim, it does not rewrite the copy. So a metadata defect here is only
//! fixable here, and it ships green — nothing in the build looks at a
//! description, and the page renders identically either way.
//!
//! Two failure modes are mechanical enough to pin, and both were live when
//! this file was written:
//!
//! 1. **A description too long to survive truncation.** Engines cut and
//!    rewrite anyway, so length alone is not a fault; past a point though no
//!    variant of the sentence comes out intact and the useful half sits
//!    behind the cut. Both roadmap pages read 297 (EN) / 307 (FR).
//! 2. **Two pages sharing a description.** `3.cli/index.md` and
//!    `3.cli/1.reference.md` said the same sentence to a word, in both
//!    locales: two separately crawled URLs, one of which gets dropped or
//!    rewritten, when the two pages have distinct jobs (the section landing
//!    against the exhaustive index).
//!
//! Title casing is *not* guarded. Separating `Getting Started` from
//! `GitHub issue / PR linking` or `Open dispatch (o and O)` needs a
//! hand-written allow-list of proper nouns, which is a census that goes stale
//! the first time a page is named after something not on it.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Past this many characters, no variant of the sentence survives the cut.
///
/// A ceiling on egregiousness, not a style rule: the longest page that is
/// *not* a defect (`fr/4.configuration/index.md`) already sits at 242, so
/// there is little room under this line — a description that trips it is
/// long enough that the fix is to say less, not to raise the cap.
const MAX_DESCRIPTION_CHARS: usize = 250;

/// Word-overlap past which two descriptions read as the same sentence.
///
/// Measured on the tree, same locale, every pair: the two defective pairs
/// scored 0.667 (EN) and 0.632 (FR), the next pair 0.312, and once #579 is
/// applied the highest score anywhere is 0.312. The threshold sits in that
/// gap rather than on either edge of it.
const MAX_DESCRIPTION_OVERLAP: f64 = 0.5;

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

/// The `description` of a page, or `None` when it carries no frontmatter.
///
/// `docs/schema/README.md` and `docs/_capture/README.md` are working notes
/// with no frontmatter at all; they are skipped rather than excluded by name.
///
/// Line endings are normalised because Windows runners check out with
/// `core.autocrlf=true`, and a trailing `\r` would otherwise count as a
/// character and as a word.
fn description(page: &Path) -> Option<String> {
  let text = fs::read_to_string(page)
    .unwrap_or_else(|err| panic!("{} must be readable: {err}", page.display()))
    .replace("\r\n", "\n");
  let body = text.strip_prefix("---\n")?;
  let front = body.split("\n---").next()?;
  let value = front.lines().find_map(|line| line.strip_prefix("description:"))?;
  Some(unquote(value.trim()).to_string())
}

/// Strip the one layer of YAML quoting a description may carry, which the
/// pages starting on an inline code span need (`` `gwm tmux` / … ``).
fn unquote(value: &str) -> &str {
  for quote in ['"', '\''] {
    if let Some(inner) = value.strip_prefix(quote).and_then(|v| v.strip_suffix(quote)) {
      return inner;
    }
  }
  value
}

/// Locale of a page: the two are crawled as separate sites, so a shared
/// description across locales is a translation, not a duplicate.
fn locale(page: &Path) -> &'static str {
  if page.starts_with(docs_root().join("fr")) {
    "fr"
  } else {
    "en"
  }
}

/// The words of a description, lowercased, punctuation and backticks dropped.
fn words(description: &str) -> BTreeSet<String> {
  description
    .to_lowercase()
    .replace('`', "")
    .split(|c: char| !c.is_alphanumeric())
    .filter(|w| !w.is_empty())
    .map(str::to_string)
    .collect()
}

/// Jaccard index of two word sets: shared words over words used at all.
fn overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
  let union = left.union(right).count();
  if union == 0 {
    return 0.0;
  }
  left.intersection(right).count() as f64 / union as f64
}

/// Every page description stays short enough to be served whole.
#[test]
fn descriptions_survive_search_result_truncation() {
  let root = docs_root();
  let mut long = Vec::new();
  for page in markdown_pages() {
    let Some(description) = description(&page) else {
      continue;
    };
    let len = description.chars().count();
    if len > MAX_DESCRIPTION_CHARS {
      let rel = page.strip_prefix(&root).unwrap_or(&page);
      long.push(format!("{} ({len} chars)", rel.display()));
    }
  }
  assert!(
    long.is_empty(),
    "these descriptions run past {MAX_DESCRIPTION_CHARS} characters, so what a search engine \
     shows of them is whatever fits before the cut; say the current state first and drop the \
     history (the longest page that is not a defect already sits at 242, so this is a wide \
     line to cross):\n  {}",
    long.join("\n  ")
  );
}

/// No two pages of the same locale describe themselves the same way.
#[test]
fn descriptions_are_distinct_within_a_locale() {
  let root = docs_root();
  let pages: Vec<(PathBuf, BTreeSet<String>)> = markdown_pages()
    .into_iter()
    .filter_map(|page| description(&page).map(|d| (page, words(&d))))
    .collect();
  let mut clashes = Vec::new();
  for (i, (left, left_words)) in pages.iter().enumerate() {
    for (right, right_words) in pages.iter().skip(i + 1) {
      if locale(left) != locale(right) {
        continue;
      }
      let score = overlap(left_words, right_words);
      if score >= MAX_DESCRIPTION_OVERLAP {
        clashes.push(format!(
          "{} ↔ {} ({score:.3} word overlap)",
          left.strip_prefix(&root).unwrap_or(left).display(),
          right.strip_prefix(&root).unwrap_or(right).display()
        ));
      }
    }
  }
  assert!(
    clashes.is_empty(),
    "these pages are crawled as separate URLs and say the same sentence, so an engine drops \
     or rewrites one of the two; give each the description of its own job:\n  {}",
    clashes.join("\n  ")
  );
}
