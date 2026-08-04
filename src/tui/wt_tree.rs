//! Working Tree file-explorer model (issue #300).
//!
//! A **pure, ratatui-free** transform of `git status --short` porcelain
//! output (`XY PATH` lines) into a nested directory tree, so the Status
//! pane's Working Tree section can render like a real file explorer
//! (nerd-font folder / file-type icons + a per-file change badge) instead
//! of a flat list. The renderer in [`super::ui`] walks the [`WtNode`] tree
//! and paints it with [`Theme`](super::theme::Theme) colours; everything in
//! this module is deterministic and theme-free so it can be unit-tested
//! without a terminal.
//!
//! Layout rules baked into [`build_tree`]:
//!
//! - **Nesting** by path segment (`src/tui/ui.rs` → `src` → `tui` → `ui.rs`).
//! - **Directories before files**, alphabetical within each level.
//! - **Single-child directory chains collapse** for compactness
//!   (`src` → `tui` → `ui.rs` renders as `src/tui/` then `ui.rs`), matching
//!   the way file explorers fold empty intermediate folders.

use std::collections::BTreeMap;
use std::path::Path;

/// Nerd-font glyph for a closed directory (`nf-fa-folder`). Kept public so
/// a future expand/collapse affordance (issue #300, deferred) can pick the
/// closed variant; the MVP renders the full tree with [`WT_DIR_OPEN_ICON`].
pub const WT_DIR_ICON: &str = "\u{f07b}";
/// Nerd-font glyph for an open directory (`nf-fa-folder_open`). The MVP
/// always shows children, so every directory row uses this.
pub const WT_DIR_OPEN_ICON: &str = "\u{f07c}";
/// Generic file glyph (`nf-fa-file`) — the fallback when no extension in
/// [`file_icon`]'s table matches.
pub const WT_FILE_ICON: &str = "\u{f15b}";

/// `.rs` (`nf-dev-rust`).
pub const WT_RUST_ICON: &str = "\u{e7a8}";
/// `.md` / `.markdown` (`nf-oct-markdown`).
pub const WT_MARKDOWN_ICON: &str = "\u{f48a}";
/// `.toml` (`nf-seti-config`).
pub const WT_TOML_ICON: &str = "\u{e615}";
/// `.json` (`nf-seti-json`).
pub const WT_JSON_ICON: &str = "\u{e60b}";
/// `.js` / `.cjs` / `.mjs` (`nf-seti-javascript`).
pub const WT_JS_ICON: &str = "\u{e74e}";
/// `.ts` / `.tsx` (`nf-seti-typescript`).
pub const WT_TS_ICON: &str = "\u{e628}";
/// `.lock` (`nf-fa-lock`).
pub const WT_LOCK_ICON: &str = "\u{f023}";
/// `.yml` / `.yaml` (`nf-seti-yml`).
pub const WT_YAML_ICON: &str = "\u{e6a8}";
/// `.sh` / `.bash` / `.zsh` (`nf-oct-terminal`).
pub const WT_SHELL_ICON: &str = "\u{f489}";
/// `.txt` (`nf-fa-file_text`).
pub const WT_TEXT_ICON: &str = "\u{f15c}";

/// The single change-category a `git status --short` `XY` pair falls into
/// (issue #287, relocated here in #300 to be the shared, ratatui-free
/// source of truth). Drives both the Working-Tree footer counts and the
/// per-row / per-badge colouring so a file's colour always equals the
/// footer segment it's counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WtCategory {
  Created,
  Modified,
  Deleted,
}

/// Classify a porcelain `XY` status pair into its dominant
/// [`WtCategory`], with a deterministic precedence (created > deleted >
/// modified) so each file maps to exactly one bucket:
///
/// - `??` (untracked) or an `A` in either column → **created**,
/// - else a `D` in either column → **deleted**,
/// - else anything changed (`M`, `R`, `C`, `T`, `U`, …) → **modified**.
pub fn working_tree_category(x: char, y: char) -> WtCategory {
  if (x == '?' && y == '?') || x == 'A' || y == 'A' {
    WtCategory::Created
  } else if x == 'D' || y == 'D' {
    WtCategory::Deleted
  } else {
    WtCategory::Modified
  }
}

/// Representative single-character status badge for a porcelain `XY` pair,
/// shown at the start of a file row in the same colour as its
/// [`WtCategory`]:
///
/// - `??` → `?` (untracked, created colour),
/// - `A` in either column → `A` (added, created colour),
/// - `D` in either column → `D` (deleted colour),
/// - anything else → `M` (modified colour).
///
/// The precedence mirrors [`working_tree_category`] so badge and row colour
/// never disagree.
pub fn status_badge(x: char, y: char) -> char {
  if x == '?' && y == '?' {
    '?'
  } else if x == 'A' || y == 'A' {
    'A'
  } else if x == 'D' || y == 'D' {
    'D'
  } else {
    'M'
  }
}

/// Pick a nerd-font glyph for a file by its extension, falling back to the
/// generic [`WT_FILE_ICON`] for unknown or extension-less names (including
/// dotfiles like `.gitignore`). The match is a single table so new types
/// are a one-line addition.
pub fn file_icon(name: &str) -> &'static str {
  let ext = Path::new(name)
    .extension()
    .and_then(|e| e.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();
  match ext.as_str() {
    "rs" => WT_RUST_ICON,
    "md" | "markdown" => WT_MARKDOWN_ICON,
    "toml" => WT_TOML_ICON,
    "json" => WT_JSON_ICON,
    "js" | "cjs" | "mjs" => WT_JS_ICON,
    "ts" | "tsx" => WT_TS_ICON,
    "lock" => WT_LOCK_ICON,
    "yml" | "yaml" => WT_YAML_ICON,
    "sh" | "bash" | "zsh" => WT_SHELL_ICON,
    "txt" => WT_TEXT_ICON,
    _ => WT_FILE_ICON,
  }
}

/// Make a path segment safe to render in the TUI: every control character
/// (newline, tab, carriage return, ANSI escape, …) becomes `?`. `-z` emits
/// filenames verbatim, so a name carrying embedded control bytes could
/// otherwise break the sidebar layout or inject terminal escape sequences;
/// the real bytes still live in git, this only guards what reaches the
/// screen.
///
/// Delegates rather than keeping its own copy of the rule (issue #506): this
/// had the same body as [`crate::naming::sanitise_for_terminal`] until that
/// one grew the `Bidi_Control` characters, and a filename carrying one
/// reorders a sidebar row exactly as it reorders a config value. A second copy
/// is a second thing to forget.
pub fn sanitize_name(name: &str) -> String {
  crate::naming::sanitise_for_terminal(name)
}

/// A node in the Working Tree file-explorer model. A `Dir` carries its
/// (possibly collapsed) display name, ordered children, and the aggregate
/// change-category of its subtree (issue #300: `Some(c)` when every
/// descendant shares category `c`, `None` when the subtree mixes
/// categories) so the directory row can be coloured by what it contains. A
/// `File` carries its leaf name plus the precomputed icon, badge glyph, and
/// change category the renderer needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WtNode {
  Dir {
    name: String,
    children: Vec<WtNode>,
    category: Option<WtCategory>,
  },
  File {
    name: String,
    icon: &'static str,
    badge: char,
    category: WtCategory,
  },
}

/// Aggregate change-category of a directory subtree (issue #300): `Some(c)`
/// when every categorised descendant shares category `c`, `None` when they
/// mix (or the subtree is empty). A child directory that is itself mixed
/// (`None`) makes its parent mixed too. Drives the retroactive directory
/// colouring — a folder of only-modified files reads yellow, only-new
/// green, only-deleted red, and a mixed folder a neutral accent.
fn aggregate_category(children: &[WtNode]) -> Option<WtCategory> {
  let mut found: Option<WtCategory> = None;
  for child in children {
    let cat = match child {
      WtNode::File { category, .. } => *category,
      WtNode::Dir { category: Some(c), .. } => *c,
      // A child subtree that is already mixed makes this directory mixed.
      WtNode::Dir { category: None, .. } => return None,
    };
    match found {
      None => found = Some(cat),
      Some(f) if f == cat => {}
      Some(_) => return None,
    }
  }
  found
}

/// One parsed `git status --porcelain -z` record: the two status columns
/// and the working-tree path. For a rename/copy this is the **destination**
/// (the source token is consumed and dropped during parsing), so every
/// record is a live entry the tree can nest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRecord {
  pub x: char,
  pub y: char,
  pub path: String,
}

/// Parse `git status --porcelain -z` output into [`StatusRecord`]s.
///
/// The `-z` format is a run of **NUL-terminated** tokens, each `XY<space>PATH`;
/// a rename/copy entry (`R`/`C` in either status column) is immediately
/// *followed* by a second NUL-terminated token carrying its original path,
/// which is skipped. Crucially, `-z` emits paths **verbatim** — no double-
/// quoting, no C-escapes — and delimits on NUL, so a filename containing a
/// space, a literal ` -> `, a quote, or non-ASCII bytes is unambiguous.
/// This is why the file-explorer reads `-z` rather than the human `--short`
/// format: it removes every textual-parsing edge case at the source.
///
/// Tokens too short to carry an `XY` pair, or with an empty path, are
/// skipped; the trailing NUL's empty token is ignored. The helper is total
/// for non-git callers.
pub fn parse_status_z(raw: &str) -> Vec<StatusRecord> {
  let mut records = Vec::new();
  let mut tokens = raw.split('\0');
  while let Some(tok) = tokens.next() {
    if tok.is_empty() {
      continue;
    }
    let mut chars = tok.chars();
    let x = match chars.next() {
      Some(c) => c,
      None => continue,
    };
    let y = match chars.next() {
      Some(c) => c,
      None => continue,
    };
    // Skip the single separator space between the `XY` pair and the path.
    if chars.next().is_none() {
      continue;
    }
    let path: String = chars.collect();
    if path.is_empty() {
      continue;
    }
    // A rename/copy entry is trailed by its source-path token — drop it so
    // the source dir doesn't show up as a phantom entry.
    if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
      tokens.next();
    }
    records.push(StatusRecord { x, y, path });
  }
  records
}

/// Build the nested Working Tree model from `git status --porcelain -z`
/// output (via [`parse_status_z`]). Each record's path becomes a leaf at
/// the end of its `/`-separated segments; intermediate segments are
/// directories. The result is dir-first alphabetical at every level with
/// single-child directory chains collapsed.
pub fn build_tree(status_z: &str) -> Vec<WtNode> {
  build_capped_tree(&parse_status_z(status_z), usize::MAX).0
}

/// Maximum file leaves the Working Tree explorer builds in one pass (issue
/// #300). `--untracked-files=all` makes git enumerate every file inside an
/// unignored generated/vendor directory; without a cap the sidebar would
/// build and cache one `Line` per file and size its non-scrollable section
/// from that full length, so selecting such a worktree could flood the TUI.
/// Past the cap, [`build_capped_tree`] stops and reports the remainder for
/// a single `… N more` row.
pub const WT_TREE_MAX_FILES: usize = 500;

/// Build the nested model from at most `max` of `records`, returning the
/// node list plus the number of records dropped past the cap (`0` when
/// nothing was capped). The kept records are the first `max` in the order
/// git emitted them.
pub fn build_capped_tree(records: &[StatusRecord], max: usize) -> (Vec<WtNode>, usize) {
  let shown = records.len().min(max);
  let mut root = DirBuilder::default();
  for rec in &records[..shown] {
    root.insert(&rec.path, rec.x, rec.y);
  }
  (root.into_nodes(), records.len() - shown)
}

// Intermediate mutable builder kept private; `BTreeMap` gives the
// alphabetical ordering for free, and emitting dirs before files yields the
// dir-first rule.
#[derive(Default)]
struct DirBuilder {
  dirs: BTreeMap<String, DirBuilder>,
  files: BTreeMap<String, FileLeaf>,
}

struct FileLeaf {
  icon: &'static str,
  badge: char,
  category: WtCategory,
}

impl DirBuilder {
  /// Insert one `XY PATH` entry, splitting `path` on `/` into directory
  /// segments plus a final file leaf. Empty segments (a trailing slash or
  /// `//`) are ignored so a stray separator can't spawn a blank node.
  fn insert(&mut self, path: &str, x: char, y: char) {
    let mut segments = path.split('/').filter(|s| !s.is_empty()).peekable();
    let mut node = self;
    while let Some(seg) = segments.next() {
      if segments.peek().is_none() {
        // Last segment → the file leaf.
        node.files.insert(
          seg.to_string(),
          FileLeaf {
            icon: file_icon(seg),
            badge: status_badge(x, y),
            category: working_tree_category(x, y),
          },
        );
        return;
      }
      node = node.dirs.entry(seg.to_string()).or_default();
    }
  }

  /// Lower the mutable builder into the public [`WtNode`] tree: directories
  /// first (alphabetical, courtesy of `BTreeMap`), then files. Single-child
  /// directory chains (a dir holding exactly one subdir and no files) are
  /// folded into a single `a/b/c` row for compactness.
  fn into_nodes(self) -> Vec<WtNode> {
    let mut out = Vec::with_capacity(self.dirs.len() + self.files.len());
    for (mut name, mut dir) in self.dirs {
      while dir.files.is_empty() && dir.dirs.len() == 1 {
        let (child_name, child_dir) = dir.dirs.into_iter().next().unwrap();
        name.push('/');
        name.push_str(&child_name);
        dir = child_dir;
      }
      let children = dir.into_nodes();
      let category = aggregate_category(&children);
      out.push(WtNode::Dir {
        name,
        children,
        category,
      });
    }
    for (name, leaf) in self.files {
      out.push(WtNode::File {
        name,
        icon: leaf.icon,
        badge: leaf.badge,
        category: leaf.category,
      });
    }
    out
  }
}
