//! Pure-state tests for the Working Tree file-explorer model (issue #300).
//!
//! Ratatui-free and Theme-free: they pin the flat `XY PATH` → nested tree
//! transform — nesting, dir-first alphabetical sort, single-child directory
//! collapse, the extension→icon table, and the per-file status badge /
//! category — without ever rasterizing a buffer. The render-level
//! counterpart lives in `tui_sidebar_render_tests.rs`.

use gwm::tui::wt_tree::{
  build_tree, file_icon, parse_status_z, status_badge, working_tree_category, WtCategory, WtNode, WT_FILE_ICON,
  WT_JSON_ICON, WT_MARKDOWN_ICON, WT_RUST_ICON, WT_TOML_ICON,
};

/// Find a direct `Dir` child by name in a node slice.
fn dir<'a>(nodes: &'a [WtNode], name: &str) -> &'a WtNode {
  nodes
    .iter()
    .find(|n| matches!(n, WtNode::Dir { name: n, .. } if n == name))
    .unwrap_or_else(|| panic!("no Dir named {name:?} in {nodes:?}"))
}

/// Borrow a `Dir`'s children.
fn children(node: &WtNode) -> &[WtNode] {
  match node {
    WtNode::Dir { children, .. } => children,
    other => panic!("expected a Dir, got {other:?}"),
  }
}

#[test]
fn build_tree_empty_status_is_empty() {
  assert!(build_tree("").is_empty());
  assert!(build_tree("\0").is_empty(), "a lone trailing NUL yields no nodes");
}

#[test]
fn build_tree_collapses_single_child_directory_chain() {
  // `src` → `tui` → `ui.rs`: the two single-child dirs fold into one
  // `src/tui` row, and `ui.rs` stays its own child (issue #300 example).
  let tree = build_tree("?? src/tui/ui.rs\0");
  assert_eq!(tree.len(), 1, "one top-level node: {tree:?}");
  let top = dir(&tree, "src/tui");
  let kids = children(top);
  assert_eq!(kids.len(), 1, "the collapsed dir holds just the file: {kids:?}");
  assert!(
    matches!(&kids[0], WtNode::File { name, .. } if name == "ui.rs"),
    "leaf file kept separate from the collapsed chain: {:?}",
    kids[0]
  );
}

#[test]
fn build_tree_does_not_collapse_a_dir_with_a_file_sibling() {
  // `src` has both a file (`lib.rs`) and a subdir (`tui`), so it must NOT
  // fold into its child — only single-*child* dir chains collapse.
  let tree = build_tree("?? src/lib.rs\0?? src/tui/ui.rs\0");
  let src = dir(&tree, "src");
  let kids = children(src);
  assert_eq!(kids.len(), 2, "src keeps both children: {kids:?}");
  // Directory sorts before the file.
  assert!(matches!(&kids[0], WtNode::Dir { name, .. } if name == "tui"));
  assert!(matches!(&kids[1], WtNode::File { name, .. } if name == "lib.rs"));
}

#[test]
fn build_tree_sorts_dirs_before_files_alphabetically() {
  let tree = build_tree("?? zebra.txt\0?? alpha.txt\0?? lib/mod.rs\0?? abc/x.rs\0");
  let names: Vec<&str> = tree
    .iter()
    .map(|n| match n {
      WtNode::Dir { name, .. } => name.as_str(),
      WtNode::File { name, .. } => name.as_str(),
    })
    .collect();
  // dirs (alphabetical) before files (alphabetical).
  assert_eq!(names, vec!["abc", "lib", "alpha.txt", "zebra.txt"], "got {names:?}");
}

#[test]
fn build_tree_file_carries_extension_icon_badge_and_category() {
  let tree = build_tree(" M src/main.rs\0");
  let kids = children(dir(&tree, "src"));
  match &kids[0] {
    WtNode::File {
      name,
      icon,
      badge,
      category,
    } => {
      assert_eq!(name, "main.rs");
      assert_eq!(*icon, WT_RUST_ICON, "extension drives the icon");
      assert_eq!(*badge, 'M', "tracked-modified badge");
      assert_eq!(*category, WtCategory::Modified);
    }
    other => panic!("expected a File, got {other:?}"),
  }
}

#[test]
fn build_tree_takes_destination_of_a_rename() {
  // In `-z` porcelain a rename is `XY <dest>\0<source>\0`: the destination
  // comes first, the source in the trailing NUL field. The tree keys off the
  // destination and drops the source.
  let tree = build_tree("R  new/b.rs\0old/a.rs\0");
  assert_eq!(tree.len(), 1, "only the destination dir survives: {tree:?}");
  let kids = children(dir(&tree, "new"));
  assert!(
    matches!(&kids[0], WtNode::File { name, badge, .. } if name == "b.rs" && *badge == 'M'),
    "rename keyed to destination, badge M (modified): {:?}",
    kids[0]
  );
}

#[test]
fn build_tree_keeps_special_chars_in_names_verbatim() {
  // `-z` emits paths verbatim — no quoting. A filename with a space, a
  // literal ` -> `, and non-ASCII bytes nests with its real name intact (the
  // edge cases that textual `--short` parsing would mangle).
  let tree = build_tree("?? a -> café b.txt\0");
  assert!(
    matches!(&tree[0], WtNode::File { name, .. } if name == "a -> café b.txt"),
    "special chars preserved verbatim: {:?}",
    tree[0]
  );
}

#[test]
fn parse_status_z_drops_rename_source_and_preserves_paths() {
  // NUL-delimited records; the rename destination (`dst.rs`) is kept and its
  // trailing source field (`src.rs`) dropped, while spaces in plain names
  // survive untouched.
  let recs = parse_status_z("?? a b.rs\0R  dst.rs\0src.rs\0 M c.rs\0");
  let got: Vec<(char, char, &str)> = recs.iter().map(|r| (r.x, r.y, r.path.as_str())).collect();
  assert_eq!(
    got,
    vec![('?', '?', "a b.rs"), ('R', ' ', "dst.rs"), (' ', 'M', "c.rs")],
    "rename source dropped, paths verbatim: {got:?}"
  );
}

#[test]
fn status_badge_maps_each_porcelain_family() {
  assert_eq!(status_badge('?', '?'), '?', "untracked");
  assert_eq!(status_badge('A', ' '), 'A', "staged add");
  assert_eq!(status_badge(' ', 'A'), 'A', "add in worktree column");
  assert_eq!(status_badge('D', ' '), 'D', "deleted");
  assert_eq!(status_badge(' ', 'M'), 'M', "worktree-modified");
  assert_eq!(status_badge('R', ' '), 'M', "rename → modified");
  // Created precedence: an add wins over a delete in the other column.
  assert_eq!(status_badge('A', 'D'), 'A');
}

#[test]
fn working_tree_category_matches_badge_families() {
  assert_eq!(working_tree_category('?', '?'), WtCategory::Created);
  assert_eq!(working_tree_category('A', ' '), WtCategory::Created);
  assert_eq!(working_tree_category('D', ' '), WtCategory::Deleted);
  assert_eq!(working_tree_category(' ', 'M'), WtCategory::Modified);
  assert_eq!(working_tree_category('R', ' '), WtCategory::Modified);
}

#[test]
fn file_icon_maps_known_extensions_and_falls_back() {
  assert_eq!(file_icon("main.rs"), WT_RUST_ICON);
  assert_eq!(file_icon("README.md"), WT_MARKDOWN_ICON);
  assert_eq!(file_icon("Cargo.toml"), WT_TOML_ICON);
  assert_eq!(file_icon("pkg.json"), WT_JSON_ICON);
  // Case-insensitive on the extension.
  assert_eq!(file_icon("MAIN.RS"), WT_RUST_ICON);
  // No extension, and dotfiles, fall back to the generic glyph.
  assert_eq!(file_icon("Makefile"), WT_FILE_ICON);
  assert_eq!(file_icon(".gitignore"), WT_FILE_ICON);
}
