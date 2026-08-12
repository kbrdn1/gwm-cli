//! The in-TUI note editor's buffer, cursor and viewport (issue #515).
//!
//! Three invariants carry the rest and are pinned first:
//!
//! 1. **`lines` is never empty.** Every method indexes `lines[cursor_line]`
//!    directly; an empty vec would panic the TUI on the first keystroke.
//! 2. **The column is a `char` index and the string is indexed by bytes.**
//!    Confusing the two panics mid-codepoint the first time a note holds an
//!    accent, which is the first thing a French note holds.
//! 3. **Blank is nothing, not an empty line.** `crate::notes` reads a blank
//!    file as "no note", so the editor must hand back an empty string for a
//!    buffer that has been cleared, and the caller must remove the file
//!    rather than write one byte.

use gwm::tui::state::note_editor::NoteEditor;
use std::path::PathBuf;

fn editor(text: &str) -> NoteEditor {
  NoteEditor::open("feat/#515-worktree-notes".into(), PathBuf::from("/tmp/n.md"), text)
}

/// Cursor as `(line, col)`, which is what every movement assertion reads.
fn at(e: &NoteEditor) -> (usize, usize) {
  (e.cursor_line, e.cursor_col)
}

// ---------------------------------------------------------------------------
// 1. The buffer is never empty
// ---------------------------------------------------------------------------

#[test]
fn an_empty_note_opens_on_one_empty_line() {
  let e = editor("");
  assert_eq!(e.lines, vec![""], "an empty buffer is one empty line, not zero lines");
  assert_eq!(at(&e), (0, 0));
  assert!(!e.dirty, "opening a note has not changed it");
}

#[test]
fn deleting_everything_leaves_one_line_standing() {
  // The path that would empty the vec if `backspace` popped lines blindly:
  // walk back over a two-line buffer, one keystroke at a time.
  let mut e = editor("ab\ncd");
  for _ in 0..10 {
    e.backspace();
  }
  assert_eq!(e.lines, vec![""], "the buffer bottoms out at one empty line");
  assert_eq!(at(&e), (0, 0));
}

#[test]
fn the_cursor_opens_at_the_end_because_reopening_a_note_is_appending() {
  let e = editor("first\nsecond");
  assert_eq!(at(&e), (1, 6));
}

#[test]
fn a_trailing_newline_opens_on_the_blank_line_it_makes() {
  // What every note on disk looks like: `text()` writes a trailing `\n`, so
  // the round trip has to land the cursor on the empty line after it rather
  // than at the end of the last written line.
  let e = editor("one\n");
  assert_eq!(e.lines, vec!["one", ""]);
  assert_eq!(at(&e), (1, 0));
}

// ---------------------------------------------------------------------------
// 2. The column is a char index
// ---------------------------------------------------------------------------

#[test]
fn editing_a_line_with_accents_never_slices_mid_codepoint() {
  // `é` is two bytes. A column used as a byte offset puts `String::insert`
  // inside it and panics — taking the whole TUI down on a French note.
  let mut e = editor("héllo");
  e.home();
  e.right();
  e.right();
  e.insert_char('X');
  assert_eq!(e.lines[0], "héXllo");
  assert_eq!(at(&e), (0, 3));

  e.backspace();
  assert_eq!(e.lines[0], "héllo");
  e.backspace();
  assert_eq!(
    e.lines[0], "hllo",
    "backspace removed the whole `é`, not one of its bytes"
  );
}

#[test]
fn delete_forwards_also_counts_chars() {
  let mut e = editor("héllo");
  e.home();
  e.right();
  e.delete();
  assert_eq!(e.lines[0], "hllo");
  assert_eq!(at(&e), (0, 1), "delete does not move the cursor");
}

#[test]
fn the_end_of_a_line_is_measured_in_chars_too() {
  let mut e = editor("héllo");
  e.home();
  e.end();
  assert_eq!(at(&e), (0, 5), "5 chars, not 6 bytes");
}

// ---------------------------------------------------------------------------
// 3. Blank is nothing
// ---------------------------------------------------------------------------

#[test]
fn a_cleared_buffer_hands_back_nothing_at_all() {
  // Not "\n": `crate::notes` reads a one-byte file as absent, so writing one
  // would leave a file the rest of gwm reports as no note. The caller has to
  // be able to tell "remove it" from "write this".
  let mut e = editor("something\nhere");
  for _ in 0..20 {
    e.backspace();
  }
  assert_eq!(e.text(), "", "a cleared buffer is nothing to write");
}

#[test]
fn a_buffer_of_only_whitespace_is_also_nothing() {
  let mut e = editor("");
  for c in ["  ", "\t"].concat().chars() {
    e.insert_char(c);
  }
  e.newline();
  assert_eq!(
    e.text(),
    "",
    "whitespace-only is blank, the same predicate `notes::read` uses"
  );
}

#[test]
fn text_ends_in_exactly_one_newline() {
  let mut e = editor("a line");
  assert_eq!(e.text(), "a line\n");
  e.newline();
  e.newline();
  assert_eq!(
    e.text(),
    "a line\n",
    "trailing blank lines do not accumulate newlines on disk"
  );
}

#[test]
fn a_note_round_trips_through_open_and_text() {
  for body in ["one\n", "one\ntwo\n", "accentué\nsecond ligne\n", "a\n\nb\n"] {
    assert_eq!(editor(body).text(), body, "{body:?} did not survive the round trip");
  }
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

#[test]
fn enter_splits_the_line_at_the_cursor() {
  let mut e = editor("abcd");
  e.home();
  e.right();
  e.right();
  e.newline();
  assert_eq!(e.lines, vec!["ab", "cd"]);
  assert_eq!(at(&e), (1, 0));
}

#[test]
fn backspace_at_column_zero_joins_onto_the_previous_line() {
  let mut e = editor("ab\ncd");
  e.home();
  e.backspace();
  assert_eq!(e.lines, vec!["abcd"]);
  assert_eq!(
    at(&e),
    (0, 2),
    "the cursor lands on the seam, where the text the user watched now is"
  );
}

#[test]
fn delete_at_end_of_line_pulls_the_next_one_up() {
  let mut e = editor("ab\ncd");
  e.cursor_line = 0;
  e.end();
  e.delete();
  assert_eq!(e.lines, vec!["abcd"]);
  assert_eq!(at(&e), (0, 2));
}

#[test]
fn backspace_at_the_very_start_does_nothing() {
  let mut e = editor("ab");
  e.cursor_line = 0;
  e.home();
  e.backspace();
  assert_eq!(e.lines, vec!["ab"]);
  assert!(!e.dirty, "a no-op keystroke must not mark the note as changed");
}

#[test]
fn delete_at_the_very_end_does_nothing() {
  let mut e = editor("ab");
  e.delete();
  assert_eq!(e.lines, vec!["ab"]);
  assert!(!e.dirty);
}

#[test]
fn only_a_real_edit_sets_dirty() {
  // The write on close is skipped when the buffer is clean, so opening a
  // note to read it does not touch its mtime. Movement must not fake an
  // edit, or that guarantee is worthless.
  let mut e = editor("text\nhere");
  e.up();
  e.down();
  e.left();
  e.right();
  e.home();
  e.end();
  e.page_up(5);
  e.page_down(5);
  assert!(!e.dirty, "moving is not editing");

  e.insert_char('!');
  assert!(e.dirty);
}

// ---------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------

#[test]
fn left_and_right_wrap_across_lines() {
  let mut e = editor("ab\ncd");
  e.cursor_line = 1;
  e.home();
  e.left();
  assert_eq!(at(&e), (0, 2), "left at column 0 lands at the end of the line above");
  e.right();
  assert_eq!(
    at(&e),
    (1, 0),
    "right at end of line lands at the start of the line below"
  );
}

#[test]
fn left_at_the_very_start_and_right_at_the_very_end_stay_put() {
  let mut e = editor("ab");
  e.home();
  e.left();
  assert_eq!(at(&e), (0, 0));
  e.end();
  e.right();
  assert_eq!(at(&e), (0, 2));
}

#[test]
fn moving_onto_a_shorter_line_clamps_the_column() {
  let mut e = editor("a long line\nshort\nanother long line");
  e.cursor_line = 0;
  e.end();
  assert_eq!(at(&e), (0, 11));
  e.down();
  assert_eq!(at(&e), (1, 5), "the column clamps onto the shorter line");
  e.down();
  assert_eq!(
    at(&e),
    (2, 5),
    "and does not spring back — no remembered virtual column"
  );
}

#[test]
fn paging_stops_at_the_ends_rather_than_running_off() {
  // The line, not the column: paging is vertical, and the column keeps its
  // clamped value the same way `up` / `down` leave it.
  let mut e = editor("1\n2\n3\n4\n5");
  e.page_up(100);
  assert_eq!(e.cursor_line, 0, "page up stops on the first line");
  e.page_down(100);
  assert_eq!(e.cursor_line, 4, "page down stops on the last line");
}

#[test]
fn a_zero_height_page_still_moves_one_line() {
  // The renderer can hand over a 0-row viewport mid-resize; a page that
  // moves by 0 would be a key that silently does nothing.
  let mut e = editor("1\n2\n3");
  e.cursor_line = 0;
  e.page_down(0);
  assert_eq!(e.cursor_line, 1);
  e.page_up(0);
  assert_eq!(e.cursor_line, 0);
}

// ---------------------------------------------------------------------------
// Viewport
// ---------------------------------------------------------------------------

#[test]
fn the_viewport_follows_the_cursor_both_ways() {
  let mut e = editor(&(0..50).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"));
  e.cursor_line = 0;
  e.scroll = 0;

  e.cursor_line = 20;
  e.clamp_scroll(10);
  assert_eq!(e.scroll, 11, "scrolling down shows the cursor on the last visible row");

  e.cursor_line = 5;
  e.clamp_scroll(10);
  assert_eq!(e.scroll, 5, "scrolling up shows it on the first");
}

#[test]
fn a_cursor_already_in_view_does_not_move_the_viewport() {
  let mut e = editor(&(0..50).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"));
  e.scroll = 10;
  e.cursor_line = 15;
  e.clamp_scroll(10);
  assert_eq!(e.scroll, 10, "no scroll when nothing needs to move");
}

#[test]
fn a_zero_height_viewport_does_not_divide_the_scroll_by_zero() {
  let mut e = editor("1\n2\n3");
  e.cursor_line = 2;
  e.clamp_scroll(0);
  assert_eq!(e.scroll, 2, "a 0-row viewport is treated as 1 row, not as a panic");
}
