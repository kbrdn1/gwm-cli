//! The in-TUI note editor (issue #515).
//!
//! `N` opens the selected worktree's note here rather than handing it to
//! `$EDITOR`: a note is usually three lines written in the ten seconds
//! between two thoughts, and suspending the TUI to spawn `vi` for that is
//! a heavier gesture than the note itself. `$EDITOR` stays one keystroke
//! away (`Ctrl+e`) for anything longer.
//!
//! ## Saving
//!
//! **`Esc` writes and closes.** There is no "quit without saving", because
//! the reflex on leaving a note is to keep it, and the alternative makes
//! `Esc` destroy prose nothing can regenerate. Discarding is spelled by
//! emptying the buffer: blank is already "no note" everywhere
//! ([`crate::notes`]), so clearing the text and leaving removes it.
//!
//! ## The cursor counts `char`s, not graphemes
//!
//! A `char` is a Unicode scalar, so `é` written as `e` + U+0301 takes two
//! left-arrows to walk past and can be split by `Backspace`. Fixing that
//! needs a segmentation table, and no crate in this tree carries one —
//! adding a dependency to move a cursor over combining marks in a scratch
//! note is not the trade this feature is worth. Precomposed accents (what
//! every keyboard on this machine emits) are one `char` and behave.
//! `Ctrl+e` is the answer for text that needs more than this.

use std::path::PathBuf;

/// Buffer, cursor and viewport for one note.
///
/// `lines` is never empty: an empty note is `[""]`, one empty line, so the
/// cursor always has a line to sit on and every method can index `[0]`.
#[derive(Debug, Clone)]
pub struct NoteEditor {
  /// Branch the note is keyed on — the modal title, and what the status
  /// bar names when a write fails.
  pub branch: String,
  /// Where [`text`](Self::text) is written on close.
  pub path: PathBuf,
  pub lines: Vec<String>,
  pub cursor_line: usize,
  /// Column in `char`s, not bytes: `lines[cursor_line]` is indexed through
  /// [`byte_at`], never sliced directly.
  pub cursor_col: usize,
  /// First visible line. Owned here rather than recomputed at render time,
  /// so a resize cannot scroll the cursor off screen (#343 keeps work off
  /// the render path).
  pub scroll: usize,
  /// Rows the text area got the last time it was drawn, which is what a
  /// page key moves by. Learned from the renderer the same way `scroll` is,
  /// rather than plumbed onto `App`: the modal's height is a layout fact
  /// nothing else needs. 10 until the first frame.
  pub viewport: usize,
  /// Whether the buffer differs from what was read off disk. The write on
  /// close is skipped when it does not, so opening a note to read it does
  /// not touch its mtime.
  pub dirty: bool,
}

impl NoteEditor {
  /// Open `text` for `branch`. The cursor lands at the end, which is where
  /// an append goes, and re-opening a note is nearly always appending.
  pub fn open(branch: String, path: PathBuf, text: &str) -> Self {
    // `split('\n')` on "" yields [""], which is exactly the empty buffer.
    // A trailing newline yields a trailing "" — the blank line the cursor
    // should land on, so it is kept rather than trimmed.
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let cursor_line = lines.len() - 1;
    let cursor_col = lines[cursor_line].chars().count();
    Self {
      branch,
      path,
      lines,
      cursor_line,
      cursor_col,
      scroll: 0,
      viewport: 10,
      dirty: false,
    }
  }

  /// Byte offset of `col` (a `char` index) into `line`, saturating at its
  /// end. Every mutation goes through this: `String::insert` and
  /// `remove` take byte offsets, and handing them a `char` index panics
  /// mid-codepoint the first time a note holds an accent.
  fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(at, _)| at)
  }

  fn line_len(&self, index: usize) -> usize {
    self.lines.get(index).map_or(0, |l| l.chars().count())
  }

  /// The buffer as it goes to disk: lines re-joined with `\n`, plus a
  /// trailing newline unless the buffer is blank. POSIX text ends in one,
  /// and a note is meant to be `cat`-ed and `grep`-ed.
  pub fn text(&self) -> String {
    let joined = self.lines.join("\n");
    if joined.trim().is_empty() {
      // Blank is "no note" (`crate::notes`), and a lone newline would be a
      // one-byte file that reads as absent anyway. Hand back nothing, so
      // the caller writes nothing rather than an empty file.
      return String::new();
    }
    format!("{}\n", joined.trim_end_matches('\n'))
  }

  // -------------------------------------------------------------------
  // Editing
  // -------------------------------------------------------------------

  pub fn insert_char(&mut self, c: char) {
    let at = Self::byte_at(&self.lines[self.cursor_line], self.cursor_col);
    self.lines[self.cursor_line].insert(at, c);
    self.cursor_col += 1;
    self.dirty = true;
  }

  /// Split the current line at the cursor.
  pub fn newline(&mut self) {
    let at = Self::byte_at(&self.lines[self.cursor_line], self.cursor_col);
    let tail = self.lines[self.cursor_line].split_off(at);
    self.lines.insert(self.cursor_line + 1, tail);
    self.cursor_line += 1;
    self.cursor_col = 0;
    self.dirty = true;
  }

  /// Delete backwards. At column 0 this joins the line onto the previous
  /// one and the cursor lands on the seam, which is where the text the
  /// user was looking at now is.
  pub fn backspace(&mut self) {
    if self.cursor_col > 0 {
      let at = Self::byte_at(&self.lines[self.cursor_line], self.cursor_col - 1);
      self.lines[self.cursor_line].remove(at);
      self.cursor_col -= 1;
      self.dirty = true;
    } else if self.cursor_line > 0 {
      let tail = self.lines.remove(self.cursor_line);
      self.cursor_line -= 1;
      self.cursor_col = self.line_len(self.cursor_line);
      self.lines[self.cursor_line].push_str(&tail);
      self.dirty = true;
    }
  }

  /// Delete forwards, joining the next line in when at end of line.
  pub fn delete(&mut self) {
    let len = self.line_len(self.cursor_line);
    if self.cursor_col < len {
      let at = Self::byte_at(&self.lines[self.cursor_line], self.cursor_col);
      self.lines[self.cursor_line].remove(at);
      self.dirty = true;
    } else if self.cursor_line + 1 < self.lines.len() {
      let tail = self.lines.remove(self.cursor_line + 1);
      self.lines[self.cursor_line].push_str(&tail);
      self.dirty = true;
    }
  }

  // -------------------------------------------------------------------
  // Movement
  // -------------------------------------------------------------------

  /// Left, wrapping onto the end of the previous line.
  pub fn left(&mut self) {
    if self.cursor_col > 0 {
      self.cursor_col -= 1;
    } else if self.cursor_line > 0 {
      self.cursor_line -= 1;
      self.cursor_col = self.line_len(self.cursor_line);
    }
  }

  /// Right, wrapping onto the start of the next line.
  pub fn right(&mut self) {
    if self.cursor_col < self.line_len(self.cursor_line) {
      self.cursor_col += 1;
    } else if self.cursor_line + 1 < self.lines.len() {
      self.cursor_line += 1;
      self.cursor_col = 0;
    }
  }

  /// Up, clamping the column onto the shorter line rather than remembering
  /// a "virtual" column. Column memory is state that has to be invalidated
  /// by every edit, and this editor is not where that earns its keep.
  pub fn up(&mut self) {
    if self.cursor_line > 0 {
      self.cursor_line -= 1;
      self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
    }
  }

  pub fn down(&mut self) {
    if self.cursor_line + 1 < self.lines.len() {
      self.cursor_line += 1;
      self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
    }
  }

  pub fn home(&mut self) {
    self.cursor_col = 0;
  }

  pub fn end(&mut self) {
    self.cursor_col = self.line_len(self.cursor_line);
  }

  pub fn page_up(&mut self, height: usize) {
    self.cursor_line = self.cursor_line.saturating_sub(height.max(1));
    self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
  }

  pub fn page_down(&mut self, height: usize) {
    let last = self.lines.len().saturating_sub(1);
    self.cursor_line = (self.cursor_line + height.max(1)).min(last);
    self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
  }

  /// Pull `scroll` just far enough that the cursor is inside a `height`-row
  /// viewport. Called once per frame with the height the renderer actually
  /// got, which is why the editor does not try to know it in advance.
  pub fn clamp_scroll(&mut self, height: usize) {
    let height = height.max(1);
    self.viewport = height;
    if self.cursor_line < self.scroll {
      self.scroll = self.cursor_line;
    } else if self.cursor_line >= self.scroll + height {
      self.scroll = self.cursor_line + 1 - height;
    }
  }
}
