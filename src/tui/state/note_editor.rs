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

/// How a line opens, in Markdown terms (issue #557).
///
/// A note becomes a checklist after a day, and the three shapes below are
/// the whole vocabulary of one: a bullet, a box, a ticked box. Anything
/// else is prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListPrefix {
  /// `- `
  Bullet,
  /// `- [ ] `
  Unchecked,
  /// `- [x] `, and `- [X] ` reads the same: notes are plain Markdown other
  /// editors have written into.
  Checked,
}

impl ListPrefix {
  /// The marker a continued item is born with. A box is never born ticked:
  /// ticking is an act.
  fn continued(self) -> &'static str {
    match self {
      ListPrefix::Bullet => "- ",
      ListPrefix::Unchecked | ListPrefix::Checked => "- [ ] ",
    }
  }
}

/// The list marker a line carries, measured in `char`s so the cursor (which
/// counts `char`s) can be moved by the same amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListLine {
  /// Leading whitespace, in `char`s. Hand-written nesting lives here, so
  /// every rewrite goes *after* it and a continued item copies it.
  indent: usize,
  kind: ListPrefix,
  /// Chars from the start of the line to the item text: `indent` plus the
  /// marker. Measured rather than derived from `kind`, because a line can
  /// end right after `]` with no trailing space.
  width: usize,
}

impl ListLine {
  /// Read the marker off `line`, or `None` when the line is prose.
  ///
  /// A marker needs its space: `-foo` is prose and `--flag` is a flag, and
  /// silently treating either as an item would rewrite text the user meant
  /// literally.
  fn parse(line: &str) -> Option<Self> {
    let body = line.trim_start();
    let indent = line[..line.len() - body.len()].chars().count();
    let rest = body.strip_prefix('-')?;
    let (kind, marker) = if let Some(after) = rest.strip_prefix(" [") {
      let mut chars = after.chars();
      let kind = match chars.next()? {
        ' ' => ListPrefix::Unchecked,
        'x' | 'X' => ListPrefix::Checked,
        _ => return None,
      };
      if chars.next() != Some(']') {
        return None;
      }
      match chars.next() {
        Some(' ') => (kind, 6),
        None => (kind, 5),
        _ => return None,
      }
    } else if rest.is_empty() {
      (ListPrefix::Bullet, 1)
    } else if rest.starts_with(' ') {
      (ListPrefix::Bullet, 2)
    } else {
      return None;
    };
    Some(Self {
      indent,
      kind,
      width: indent + marker,
    })
  }
}

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

  /// Split the current line at the cursor, continuing the list the line is
  /// part of (issue #557).
  ///
  /// An item whose text is empty ends the list instead: that second `Enter`
  /// after the last item is how every Markdown editor breaks out, and
  /// without it the only way out of a list is to backspace the bullet the
  /// editor just wrote.
  pub fn newline(&mut self) {
    let list = ListLine::parse(&self.lines[self.cursor_line]);
    if let Some(l) = list {
      let line = &self.lines[self.cursor_line];
      let text_at = Self::byte_at(line, l.width);
      if line[text_at..].trim().is_empty() {
        self.lines[self.cursor_line].clear();
        self.cursor_col = 0;
        self.dirty = true;
        return;
      }
    }
    let prefix = match list {
      Some(l) => {
        let indent: String = self.lines[self.cursor_line].chars().take(l.indent).collect();
        format!("{indent}{}", l.kind.continued())
      }
      None => String::new(),
    };
    let at = Self::byte_at(&self.lines[self.cursor_line], self.cursor_col);
    let tail = self.lines[self.cursor_line].split_off(at);
    self.cursor_col = prefix.chars().count();
    self.lines.insert(self.cursor_line + 1, format!("{prefix}{tail}"));
    self.cursor_line += 1;
    self.dirty = true;
  }

  // -------------------------------------------------------------------
  // Lists (issue #557)
  // -------------------------------------------------------------------

  /// Make the current line a list item, or take the marker back off it.
  ///
  /// `- [ ] ` *is* a bullet, so turning a checkbox line off removes the
  /// whole marker rather than leaving a widowed `[ ]`.
  pub fn toggle_bullet(&mut self) {
    match ListLine::parse(&self.lines[self.cursor_line]) {
      Some(l) => self.rewrite_marker(l.indent, l.width, ""),
      None => {
        let indent = self.indent_of(self.cursor_line);
        self.rewrite_marker(indent, indent, "- ");
      }
    }
  }

  /// Tick the box under the caret, from anywhere on the line, spawning one
  /// first if the line does not have it yet. One chord covers writing the
  /// item and ticking it, which is the gesture a checklist exists for.
  pub fn toggle_checkbox(&mut self) {
    match ListLine::parse(&self.lines[self.cursor_line]) {
      Some(l) => {
        // A bullet gains an empty box rather than a ticked one: the first
        // press writes the item, the second one ticks it.
        let marker = if l.kind == ListPrefix::Unchecked {
          "- [x] "
        } else {
          "- [ ] "
        };
        self.rewrite_marker(l.indent, l.width, marker);
      }
      None => {
        let indent = self.indent_of(self.cursor_line);
        self.rewrite_marker(indent, indent, "- [ ] ");
      }
    }
  }

  /// Leading whitespace of `index`, in `char`s.
  fn indent_of(&self, index: usize) -> usize {
    let line = &self.lines[index];
    line.chars().take_while(|c| c.is_whitespace()).count()
  }

  /// Replace the chars in `indent..old` on the cursor line with `marker`,
  /// carrying the caret along so it stays on the character it was on.
  ///
  /// A same-width rewrite (ticking a box) leaves the caret exactly where it
  /// is; a caret standing inside a marker that shrinks lands on the item
  /// text, since the column it pointed at no longer exists.
  fn rewrite_marker(&mut self, indent: usize, old: usize, marker: &str) {
    let line = &mut self.lines[self.cursor_line];
    let from = Self::byte_at(line, indent);
    let to = Self::byte_at(line, old);
    line.replace_range(from..to, marker);
    let new = indent + marker.chars().count();
    self.cursor_col = if new == old {
      self.cursor_col
    } else if self.cursor_col >= old {
      self.cursor_col - old + new
    } else {
      new
    };
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
