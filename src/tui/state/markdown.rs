//! Markdown for the rich PR / issue view (issue #551).
//!
//! A PR description, a review and a comment are all Markdown, and until now
//! they reached the terminal as source: `## Description`, `**bold**` and
//! `<!-- an auto-generated comment -->` were painted literally. This module
//! turns a body into styled lines that read the way the forge renders them.
//!
//! **What it is not.** Not a CommonMark implementation. It covers what shows
//! up in a PR body on a forge, and nothing else: headings, emphasis, inline
//! code, fenced blocks, lists, task lists, block quotes, GitHub alerts,
//! links, rules and HTML comments. Tables, footnotes, reference links and
//! nested block structures are rendered as the plain text they already were,
//! which is exactly what happened before this module existed.
//!
//! **Why it is ratatui-free.** [`Emphasis`] is a semantic role, not a colour,
//! for the same reason `DetailRole` is: the mapping to theme colours belongs
//! to the renderer, so this stays a pure function that a test can call
//! without standing up a terminal.
//!
//! **Why wrapping happens here and not on the source text.** `**Lists, for
//! everyone.**` is twenty-four characters of source and twenty columns on
//! screen, so wrapping the source both under-fills every line and can cut
//! between a marker and the text it opens. The order is therefore: parse to
//! segments, then wrap on the *rendered* widths, carrying the emphasis
//! across the break.

use crate::naming::sanitise_block_for_terminal;

/// The semantic role of a run of text, mapped to theme colours at render
/// time. Deliberately a small set: each variant has to earn a distinct
/// treatment in the renderer, and a role nothing paints differently is a
/// role that does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
  Plain,
  Bold,
  Italic,
  /// `**bold _and_ italic**`, which the forge does render distinctly.
  BoldItalic,
  /// Inline `` `code` `` and the body of a fenced block.
  Code,
  Strike,
  /// The text of a `[link](url)`, and a bare URL.
  Link,
  /// A `#` heading, whatever its level. The level is not carried: a terminal
  /// has one weight above bold, so rendering `##` differently from `###`
  /// would mean inventing a distinction the medium cannot show.
  Heading,
  /// The `▎` rule down the left of a block quote, and the quoted text.
  Quote,
  /// A list bullet, a task checkbox, a horizontal rule: the structural
  /// glyphs this module inserts, which are not part of the author's text.
  Marker,

  // The roles below carry no Markdown meaning. They are here because this
  // enum is what styles a RUN of text, and the rich view's metadata block
  // needs two outcomes on one line (`+1198 −12`), which a role-per-row
  // cannot express. Their names and their theme colours are the Status
  // pane's, so the same fact reads the same in both places (issue #551).
  /// A good outcome: an open PR, a passing check. Theme `clean`, which is
  /// where `pr_badge_color` sends an open PR.
  Success,
  /// A bad outcome: a closed PR, a failing check. Theme `prunable`.
  Failure,
  /// An in-flight outcome: a running check. Theme `dirty`.
  Running,
  /// A resolved-not-failed outcome: a merged PR, a closed issue. Theme
  /// `locked`, again following `pr_badge_color` / `issue_badge_color`.
  Notice,
  /// De-emphasised: a draft PR. Theme `muted`.
  Muted,
}

/// A run of text sharing one role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
  pub text: String,
  pub emphasis: Emphasis,
  /// Paint this run as a **badge** rather than as coloured text: reverse
  /// video plus bold, which is what `ui::chip_style` does and what the
  /// Status pane uses for a PR state (validation feedback on issue #551).
  ///
  /// Orthogonal to [`Emphasis`] on purpose, because the two answer
  /// different questions: the role says which colour, this says whether the
  /// colour is the ink or the ground. Splitting them is also what lets the
  /// renderer reach `chip_style` itself, so a badge here IS the pane's
  /// badge rather than a second thing that resembles it.
  pub chip: bool,
}

impl Segment {
  pub fn new(text: impl Into<String>, emphasis: Emphasis) -> Self {
    Self {
      text: text.into(),
      emphasis,
      chip: false,
    }
  }

  /// A [`Segment::new`] painted as a badge. The caller pads the text: the
  /// pane's badges read ` open `, and the padding is what gives the reverse
  /// video its shape.
  pub fn chip(text: impl Into<String>, emphasis: Emphasis) -> Self {
    Self {
      chip: true,
      ..Self::new(text, emphasis)
    }
  }
}

/// One rendered line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdLine {
  pub segments: Vec<Segment>,
  /// True for a line that came out of a fenced block. The renderer paints it
  /// as code, and it is the line kind that must not be reflowed for meaning
  /// rather than for looks: in YAML or Python the indentation *is* the
  /// program.
  pub preformatted: bool,
}

impl MdLine {
  /// The line as the reader sees it, markers included. This is what the
  /// overlay stores in `DetailRow::value`, so every consumer that measures,
  /// filters or asserts on a row keeps working against one plain string.
  pub fn plain(&self) -> String {
    self.segments.iter().map(|s| s.text.as_str()).collect()
  }

  fn blank() -> Self {
    Self {
      segments: Vec::new(),
      preformatted: false,
    }
  }
}

/// Columns a line of segments occupies.
fn width(segments: &[Segment]) -> usize {
  segments.iter().map(|s| s.text.chars().count()).sum()
}

/// Render `body` into lines that fit `budget` columns.
///
/// The whole pipeline: sanitise (remote text, issue #502), split into block
/// constructs, parse the inline markers inside each, then wrap.
pub fn render(body: &str, budget: usize) -> Vec<MdLine> {
  let budget = budget.max(1);
  let clean = sanitise_block_for_terminal(body).replace('\t', "    ");
  let mut out = Vec::new();
  let mut fence: Option<(char, usize)> = None;
  let mut in_comment = false;

  for raw in clean.lines() {
    let trimmed = raw.trim_start();

    // A fenced block runs until its own closing fence, and nothing inside it
    // is markdown. Checked before everything else for that reason.
    if let Some((marker, len)) = fence {
      if fence_closes(trimmed, marker, len) {
        fence = None;
        continue;
      }
      // NOT wrapped, and past the budget when it has to be (issue #551).
      // A wrapped `+` line's continuation carries no sigil and reads as
      // context; a wrapped YAML line lands at the wrong indent. In code the
      // column is the meaning, which is the argument `rich_view::hunk_rows`
      // already makes for a diff hunk. The horizontal offset is what
      // reaches the tail of one of these.
      out.push(MdLine {
        segments: vec![Segment::new(raw, Emphasis::Code)],
        preformatted: true,
      });
      continue;
    }

    // `<!-- … -->`. Bot reviews open with a couple of these (CodeRabbit's
    // "summarize by coderabbit.ai"), and the forge shows none of them.
    //
    // Stripped wherever the delimiters sit on the line, not only at column
    // zero (Codex review, pass 1): a claim that comments are never shown is
    // not a claim that holds only when one opens the line.
    let (visible, still_open) = strip_comments(raw, in_comment);
    in_comment = still_open;
    if visible.trim().is_empty() {
      // A line that was ONLY a comment leaves nothing, and must not leave a
      // blank row either — the forge closes the gap.
      if !raw.trim().is_empty() {
        continue;
      }
    }
    let raw = visible.as_str();
    let trimmed = raw.trim_start();

    if let Some(opener) = fence_opens(trimmed) {
      fence = Some(opener);
      continue;
    }

    out.extend(block(raw, budget));
  }
  out
}

/// `Some((char, len))` when the line opens a fenced block, carrying the
/// fence character AND its length: a closer has to be at least as long as
/// its opener (Codex review, pass 2).
///
/// Four backticks are how you show a three-backtick block, which is what a
/// body about Markdown contains. Matching on the characters alone let the
/// inner fence close the outer block, so the inner line vanished and the
/// rest of the code was read as prose.
fn fence_opens(trimmed: &str) -> Option<(char, usize)> {
  for marker in ['`', '~'] {
    let len = trimmed.chars().take_while(|c| *c == marker).count();
    if len >= 3 {
      return Some((marker, len));
    }
  }
  None
}

/// Whether `trimmed` closes a fence opened with `len` copies of `marker`.
///
/// CommonMark: at least as long as the opener, the same character, and
/// nothing but whitespace after it. The trailing rule is what stops
/// ` ```rust ` inside a block from closing it.
fn fence_closes(trimmed: &str, marker: char, len: usize) -> bool {
  let run = trimmed.chars().take_while(|c| *c == marker).count();
  run >= len && trimmed[run..].trim().is_empty()
}

/// Remove every HTML comment from one line, returning what is left to show
/// and whether a comment is still open at the end of it.
///
/// `open` says a comment was still running when the previous line ended, so
/// this one starts inside it. Comments do not nest, which is why a plain
/// scan is enough.
fn strip_comments(line: &str, mut open: bool) -> (String, bool) {
  let mut out = String::new();
  let mut rest = line;
  loop {
    if open {
      match rest.find("-->") {
        Some(at) => {
          rest = &rest[at + 3..];
          open = false;
        }
        None => return (out, true),
      }
    } else {
      // A backtick span is literal, delimiters included (Codex review,
      // pass 2): a body writing `` `<!-- marker -->` `` is documenting the
      // delimiter, and the forge shows it. Skipped whole, before looking
      // for an opener, since the strip runs before the inline parse and is
      // the only place that knows to leave it alone.
      let opener = rest.find("<!--");
      let code = code_span(rest);
      match (opener, code) {
        (Some(at), Some((start, end))) if start < at => {
          out.push_str(&rest[..end]);
          rest = &rest[end..];
        }
        (Some(at), _) => {
          out.push_str(&rest[..at]);
          rest = &rest[at + 4..];
          open = true;
        }
        (None, _) => {
          out.push_str(rest);
          return (out, false);
        }
      }
    }
  }
}

/// The byte range of the first complete `` `…` `` span in `s`.
fn code_span(s: &str) -> Option<(usize, usize)> {
  let start = s.find('`')?;
  let end = s[start + 1..].find('`')? + start + 2;
  Some((start, end))
}

/// One source line as a block construct.
fn block(raw: &str, budget: usize) -> Vec<MdLine> {
  let trimmed = raw.trim_start();
  if trimmed.is_empty() {
    return vec![MdLine::blank()];
  }
  let indent: String = raw.chars().take_while(|c| c.is_whitespace()).collect();

  if is_rule(trimmed) {
    return vec![MdLine {
      segments: vec![Segment::new("─".repeat(budget.min(40)), Emphasis::Marker)],
      preformatted: false,
    }];
  }

  if let Some((hashes, text)) = heading(trimmed) {
    let segments = vec![Segment::new(text.to_string(), Emphasis::Heading)];
    let rule = width(&segments).min(budget);
    let mut lines = wrap(segments, budget, false, "", "");
    // A level-1 or level-2 heading is a section break on the forge; the
    // underline is how a terminal says the same thing without a font.
    if hashes <= 2 {
      lines.push(MdLine {
        segments: vec![Segment::new("─".repeat(rule), Emphasis::Marker)],
        preformatted: false,
      });
    }
    return lines;
  }

  if let Some(rest) = quote(trimmed) {
    let segments = match github_alert(rest) {
      Some(alert) => vec![Segment::new(alert, Emphasis::Heading)],
      None => inline(rest, Emphasis::Quote),
    };
    // The rule repeats down every wrapped row, which is what makes the quote
    // read as one block instead of a first line and some orphans.
    let rule = format!("{indent}▎ ");
    return wrap(segments, budget, false, &rule, &rule);
  }

  if let Some((marker, rest)) = list_item(trimmed) {
    // Continuations line up under the text, not under the bullet.
    let hang = format!("{indent}{}", " ".repeat(marker.chars().count()));
    return wrap(
      inline(rest, Emphasis::Plain),
      budget,
      false,
      &format!("{indent}{marker}"),
      &hang,
    );
  }

  // Plain prose. A line that already fits is not reflowed, which preserves
  // runs of spaces and hand-made alignment (Codex review #529), and a
  // continuation belongs under the original indent rather than at column
  // zero.
  wrap(inline(trimmed, Emphasis::Plain), budget, false, &indent, &indent)
}

fn is_rule(trimmed: &str) -> bool {
  let t = trimmed.trim_end();
  t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

/// `(level, text)` for an ATX heading.
fn heading(trimmed: &str) -> Option<(usize, &str)> {
  let hashes = trimmed.chars().take_while(|c| *c == '#').count();
  if hashes == 0 || hashes > 6 {
    return None;
  }
  let rest = &trimmed[hashes..];
  // `#tag` is not a heading; the space is what makes it one.
  let text = rest.strip_prefix(' ')?;
  Some((hashes, text.trim()))
}

/// The quoted text of a block quote line, without its `>`.
fn quote(trimmed: &str) -> Option<&str> {
  let rest = trimmed.strip_prefix('>')?;
  Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// The label of a GitHub alert (`> [!IMPORTANT]`), which the forge renders as
/// a coloured callout header.
fn github_alert(rest: &str) -> Option<String> {
  let inner = rest.trim().strip_prefix("[!")?.strip_suffix(']')?;
  let known = ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"];
  known
    .contains(&inner.to_ascii_uppercase().as_str())
    .then(|| inner.to_ascii_uppercase())
}

/// `(marker, rest)` for a list item, task lists included.
fn list_item(trimmed: &str) -> Option<(String, &str)> {
  let rest = bullet(trimmed).or_else(|| ordered(trimmed))?;
  let (glyph, rest) = match task_box(rest.1) {
    Some((glyph, text)) => (format!("{} {glyph} ", rest.0), text),
    None => (format!("{} ", rest.0), rest.1),
  };
  Some((glyph, rest))
}

/// `("•", rest)` for `-`, `*` or `+`.
fn bullet(trimmed: &str) -> Option<(&'static str, &str)> {
  for marker in ['-', '*', '+'] {
    if let Some(rest) = trimmed.strip_prefix(marker).and_then(|r| r.strip_prefix(' ')) {
      return Some(("•", rest));
    }
  }
  None
}

/// `("1.", rest)` for an ordered item, keeping the author's own number.
fn ordered(trimmed: &str) -> Option<(&'static str, &str)> {
  let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
  if digits == 0 {
    return None;
  }
  let rest = &trimmed[digits..];
  let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
  // The number is dropped rather than kept: preserving it would need the
  // list's own counter to renumber a wrapped item, and a bullet reads the
  // same. `ordered` exists to stop `1. text` being painted literally.
  rest.strip_prefix(' ').map(|r| ("•", r))
}

/// `("☑", rest)` for `[x] `, `("☐", rest)` for `[ ] `.
fn task_box(rest: &str) -> Option<(&'static str, &str)> {
  for (open, glyph) in [("[x] ", "☑"), ("[X] ", "☑"), ("[ ] ", "☐")] {
    if let Some(text) = rest.strip_prefix(open) {
      return Some((glyph, text));
    }
  }
  None
}

/// Parse the inline markers of one line into styled runs.
///
/// `base` is the role plain text takes, which is `Plain` in prose and `Quote`
/// inside a block quote, so a quoted paragraph stays visually one block even
/// where it carries its own emphasis.
fn inline(line: &str, base: Emphasis) -> Vec<Segment> {
  let chars: Vec<char> = line.chars().collect();
  let mut out: Vec<Segment> = Vec::new();
  let mut plain = String::new();
  let mut i = 0;

  while i < chars.len() {
    // A backslash escape shows the character and eats the markup meaning
    // (Codex review, pass 3). Checked before anything else, since its whole
    // job is to stop what follows from being read as a delimiter.
    if chars[i] == '\\' {
      if let Some(next) = chars.get(i + 1).filter(|c| is_escapable(**c)) {
        plain.push(*next);
        i += 2;
        continue;
      }
    }
    match marker_at(&chars, i) {
      Some((Hit::Styled(segment), next)) => {
        flush(&mut out, &mut plain, base);
        // Emphasis nests: `**bold _and_ italic**` is one bold run with an
        // italic inside it, and reading the inner slice as flat text left
        // the underscores on screen. Re-read, with the outer role folded
        // into whatever the inner one turns out to be.
        flush_nested(&mut out, segment);
        i = next;
      }
      // A delimiter run that opened nothing is literal text, and it is
      // consumed WHOLE. Falling through one character at a time would let
      // the second asterisk of an unmatched `**` open an italic of its own:
      // `**bold **` came out as `*bold *`, which is not what the forge does
      // with it either.
      Some((Hit::Literal(text), next)) => {
        plain.push_str(&text);
        i = next;
      }
      None => {
        plain.push(chars[i]);
        i += 1;
      }
    }
  }
  flush(&mut out, &mut plain, base);
  if out.is_empty() {
    out.push(Segment::new(String::new(), base));
  }
  out
}

/// What [`marker_at`] found: a styled run, or a stretch of literal text that
/// must be consumed whole rather than re-examined character by character.
enum Hit {
  Styled(Segment),
  Literal(String),
}

/// The run starting at `at`, and the index just past it.
///
/// Ordered by precedence, which is the forge's: nothing inside a backtick
/// span is a marker, and a link's text is parsed as a unit rather than for
/// the underscores a URL slug is full of.
fn marker_at(chars: &[char], at: usize) -> Option<(Hit, usize)> {
  if chars[at] == '`' {
    if let Some(end) = find(chars, at + 1, '`') {
      // An empty span is two literal backticks.
      if end > at + 1 {
        return Some((
          Hit::Styled(Segment::new(slice(chars, at + 1, end), Emphasis::Code)),
          end + 1,
        ));
      }
    }
  }
  if let Some((text, next)) = link(chars, at) {
    return Some((Hit::Styled(Segment::new(text, Emphasis::Link)), next));
  }

  // Emphasis is decided on the whole DELIMITER RUN, not on one character:
  // `***` is one run of three, and reading it as `*` opening an italic is
  // how `***loud***` ends up showing its own markers.
  let c = chars[at];
  if !matches!(c, '*' | '_' | '~') {
    return None;
  }
  let run = chars[at..].iter().take_while(|x| **x == c).count();
  let literal = || Some((Hit::Literal(slice(chars, at, at + run)), at + run));

  let Some(emphasis) = emphasis_for(c, run) else {
    return literal();
  };
  let marker: Vec<char> = vec![c; run];
  if !opens(chars, at, &marker) {
    return literal();
  }
  let Some(end) = closes(chars, at + run, &marker) else {
    return literal();
  };
  // `**` with nothing between it is two literal asterisks.
  if end == at + run {
    return literal();
  }
  Some((
    Hit::Styled(Segment::new(slice(chars, at + run, end), emphasis)),
    end + run,
  ))
}

/// The role a delimiter run of `len` copies of `c` carries, if any.
fn emphasis_for(c: char, len: usize) -> Option<Emphasis> {
  match (c, len) {
    // Strikethrough is `~~` and only `~~`; a lone `~` is a home directory.
    ('~', 2) => Some(Emphasis::Strike),
    ('~', _) => None,
    (_, 1) => Some(Emphasis::Italic),
    (_, 2) => Some(Emphasis::Bold),
    (_, 3) => Some(Emphasis::BoldItalic),
    _ => None,
  }
}

fn slice(chars: &[char], from: usize, to: usize) -> String {
  chars[from..to].iter().collect()
}

/// Whether the marker at `at` can OPEN a run.
///
/// CommonMark's flanking rules, cut down to the two that matter in a PR
/// body. Without them prose falls apart: `2 * 3 and file_name` used to have
/// its lone `*` open an italic that ran to the next asterisk anywhere on the
/// line, swallowing the text between into a role nobody asked for.
///
/// - **An opener is not followed by whitespace.** `a * b` is arithmetic.
/// - **`_` does not delimit inside a word.** `file_name` and `snake_case`
///   are identifiers, and a body about code is full of them. Asterisks keep
///   working intraword, which is what CommonMark says too.
fn opens(chars: &[char], at: usize, marker: &[char]) -> bool {
  let after = chars.get(at + marker.len());
  if after.is_none_or(|c| c.is_whitespace()) {
    return false;
  }
  if marker[0] == '_' {
    if let Some(before) = at.checked_sub(1).and_then(|i| chars.get(i)) {
      if before.is_alphanumeric() {
        return false;
      }
    }
  }
  true
}

/// The index of the marker that CLOSES the run opened at `from`, if any.
///
/// Mirror of [`opens`]: a closer is not preceded by whitespace (`a *b * c`
/// has no italic in it), and `_` again refuses to close inside a word.
fn closes(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
  let mut i = from;
  while let Some(at) = find_seq(chars, i, marker) {
    let before = at.checked_sub(1).and_then(|i| chars.get(i));
    let after = chars.get(at + marker.len());
    let flanks = before.is_some_and(|c| !c.is_whitespace());
    let intraword = marker[0] == '_' && after.is_some_and(|c| c.is_alphanumeric());
    // An escaped delimiter is a character, not a closer (Codex review,
    // pass 4). Without this, `*foo \* bar*` closed on the escaped
    // asterisk: the run ended early, and the text came out carrying the
    // backslash it was escaping with.
    if flanks && !intraword && !escaped(chars, at) {
      return Some(at);
    }
    i = at + 1;
  }
  None
}

/// Whether the character at `at` is backslash-escaped.
///
/// Counted rather than peeked: `\\*` is an escaped backslash followed by a
/// live asterisk, so it is the PARITY of the run that decides.
fn escaped(chars: &[char], at: usize) -> bool {
  let mut n = 0usize;
  let mut i = at;
  while i > 0 && chars[i - 1] == '\\' {
    n += 1;
    i -= 1;
  }
  n % 2 == 1
}

fn find(chars: &[char], from: usize, c: char) -> Option<usize> {
  (from..chars.len()).find(|i| chars[*i] == c)
}

fn find_seq(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
  (from..chars.len().saturating_sub(marker.len() - 1)).find(|i| chars[*i..*i + marker.len()] == *marker)
}

/// `[text](url)` starting at `at`, as `(text, index just past the link)`.
fn link(chars: &[char], at: usize) -> Option<(String, usize)> {
  if chars.get(at) != Some(&'[') {
    return None;
  }
  let close = find(chars, at + 1, ']')?;
  if chars.get(close + 1) != Some(&'(') {
    return None;
  }
  let end = find(chars, close + 2, ')')?;
  let text: String = chars[at + 1..close].iter().collect();
  // `[]()` carries nothing to show; leave it as literal text.
  (!text.is_empty()).then_some((text, end + 1))
}

/// Push a styled run, re-reading its text for markers of its own.
///
/// Only [`Emphasis::Bold`], [`Emphasis::Italic`] and their pair can carry
/// anything nested. A code span is literal by definition and a link's text
/// is taken as a unit, so both are pushed as they are — re-reading either
/// would undo the reason it has its own role.
fn flush_nested(out: &mut Vec<Segment>, segment: Segment) {
  if !matches!(
    segment.emphasis,
    Emphasis::Bold | Emphasis::Italic | Emphasis::BoldItalic
  ) {
    out.push(segment);
    return;
  }
  for inner in inline(&segment.text, segment.emphasis) {
    out.push(Segment::new(inner.text, combine(segment.emphasis, inner.emphasis)));
  }
}

/// The role a run carries when `inner` sits inside `outer`.
///
/// Bold inside italic (or the reverse) is the pair; anything more specific
/// than emphasis — code, a link — keeps its own role, because that is the
/// role that says how to read the text rather than how loud it is.
fn combine(outer: Emphasis, inner: Emphasis) -> Emphasis {
  use Emphasis::{Bold, BoldItalic, Italic};
  match (outer, inner) {
    (o, i) if o == i => o,
    (Bold, Italic) | (Italic, Bold) | (BoldItalic, _) | (_, BoldItalic) => BoldItalic,
    // The inner run said something the outer one did not.
    (_, i) => i,
  }
}

/// Characters a backslash may escape. CommonMark's ASCII punctuation set,
/// which is deliberately wide: outside it, a backslash is a backslash, and
/// a Windows path in a body must survive being written down.
fn is_escapable(c: char) -> bool {
  c.is_ascii_punctuation()
}

fn flush(out: &mut Vec<Segment>, plain: &mut String, base: Emphasis) {
  if !plain.is_empty() {
    out.push(Segment::new(std::mem::take(plain), base));
  }
}

/// Wrap `segments` to `budget`.
///
/// `first` prefixes row zero and `hang` prefixes every row after it. Both are
/// structure this module inserts rather than the author's text, so they are
/// applied HERE rather than pushed into the segments: a prefix baked into the
/// segments is leading whitespace, and leading whitespace is exactly what the
/// wrap has to drop when it breaks a line. That is how an indented paragraph
/// lost its indent on every continuation.
fn wrap(segments: Vec<Segment>, budget: usize, preformatted: bool, first: &str, hang: &str) -> Vec<MdLine> {
  wrap_rows(segments, budget, first, hang)
    .into_iter()
    .map(|segments| MdLine { segments, preformatted })
    .collect()
}

/// The wrap itself, over a run of segments rather than a string.
///
/// Preserves what the string version guaranteed and the tests pin: a row that
/// already fits is emitted untouched (so runs of spaces and hand-made
/// alignment survive), a word wider than the budget is hard split rather than
/// left to be ellipsised, and the loop always advances.
fn wrap_rows(segments: Vec<Segment>, budget: usize, first: &str, hang: &str) -> Vec<Vec<Segment>> {
  let budget = budget.max(1);
  // A prefix wider than the budget would leave no room to make progress.
  let too_wide = |p: &str| p.chars().count() + 8 > budget;
  let (first, hang) = if too_wide(first) || too_wide(hang) {
    ("", "")
  } else {
    (first, hang)
  };
  let prefix = |row: usize| if row == 0 { first } else { hang };
  let lead = |row: usize| -> Vec<Segment> {
    match prefix(row) {
      "" => Vec::new(),
      p => vec![Segment::new(p.to_string(), Emphasis::Marker)],
    }
  };

  if first.chars().count() + width(&segments) <= budget {
    let mut row = lead(0);
    row.extend(segments);
    return vec![row];
  }

  let mut rows: Vec<Vec<Segment>> = Vec::new();
  let mut row: Vec<Segment> = Vec::new();
  let mut cols = 0usize;

  for segment in segments {
    for word in words(&segment.text) {
      let room = budget.saturating_sub(prefix(rows.len()).chars().count()).max(1);
      let word_cols = word.chars().count();
      // Whitespace never opens a row: a break eats the space it broke on.
      if word.trim().is_empty() {
        if cols > 0 && cols + word_cols <= room {
          cols += word_cols;
          push_text(&mut row, &word, segment.emphasis);
        }
        continue;
      }
      if cols + word_cols > room {
        if cols > 0 {
          rows.push(std::mem::take(&mut row));
          cols = 0;
        }
        // A single word wider than a whole row: hard split it, since it has
        // no break opportunity and would otherwise be ellipsised away.
        let mut rest = word.as_str();
        loop {
          let room = budget.saturating_sub(prefix(rows.len()).chars().count()).max(1);
          if rest.chars().count() <= room {
            break;
          }
          let cut = rest.char_indices().nth(room).map(|(i, _)| i).unwrap_or(rest.len());
          push_text(&mut row, &rest[..cut], segment.emphasis);
          rows.push(std::mem::take(&mut row));
          rest = &rest[cut..];
        }
        if !rest.is_empty() {
          cols = rest.chars().count();
          push_text(&mut row, rest, segment.emphasis);
        }
        continue;
      }
      cols += word_cols;
      push_text(&mut row, &word, segment.emphasis);
    }
  }
  if !row.is_empty() {
    rows.push(row);
  }
  if rows.is_empty() {
    rows.push(Vec::new());
  }
  rows
    .into_iter()
    .enumerate()
    .map(|(i, content)| {
      let mut out = lead(i);
      out.extend(content);
      out
    })
    .collect()
}

/// Append to the row, merging into the last segment when the role matches so
/// a wrapped line does not become one segment per word.
fn push_text(row: &mut Vec<Segment>, text: &str, emphasis: Emphasis) {
  match row.last_mut() {
    Some(last) if last.emphasis == emphasis => last.text.push_str(text),
    _ => row.push(Segment::new(text.to_string(), emphasis)),
  }
}

/// Split into words AND the whitespace runs between them, so a break can drop
/// the space it broke on while an un-broken run keeps its exact spacing.
fn words(text: &str) -> Vec<String> {
  let mut out = Vec::new();
  let mut cur = String::new();
  let mut in_space = false;
  for c in text.chars() {
    let is_space = c.is_whitespace();
    if !cur.is_empty() && is_space != in_space {
      out.push(std::mem::take(&mut cur));
    }
    in_space = is_space;
    cur.push(c);
  }
  if !cur.is_empty() {
    out.push(cur);
  }
  out
}
