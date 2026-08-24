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
}

/// A run of text sharing one role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
  pub text: String,
  pub emphasis: Emphasis,
}

impl Segment {
  pub fn new(text: impl Into<String>, emphasis: Emphasis) -> Self {
    Self {
      text: text.into(),
      emphasis,
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
  let mut fence: Option<String> = None;
  let mut in_comment = false;

  for raw in clean.lines() {
    let trimmed = raw.trim_start();

    // A fenced block runs until its own closing fence, and nothing inside it
    // is markdown. Checked before everything else for that reason.
    if let Some(marker) = &fence {
      if trimmed.starts_with(marker.as_str()) {
        fence = None;
        continue;
      }
      out.extend(wrap(vec![Segment::new(raw, Emphasis::Code)], budget, true, "", ""));
      continue;
    }

    // `<!-- … -->`. Bot reviews open with a couple of these (CodeRabbit's
    // "summarize by coderabbit.ai"), and the forge shows none of them.
    if in_comment {
      if let Some(rest) = trimmed.split_once("-->") {
        in_comment = false;
        if !rest.1.trim().is_empty() {
          out.extend(block(rest.1, budget));
        }
      }
      continue;
    }
    if let Some(rest) = comment_opens(trimmed) {
      in_comment = true;
      if let Some(tail) = rest {
        in_comment = false;
        if !tail.trim().is_empty() {
          out.extend(block(&tail, budget));
        }
      }
      continue;
    }

    if let Some(marker) = fence_opens(trimmed) {
      fence = Some(marker);
      continue;
    }

    out.extend(block(raw, budget));
  }
  out
}

/// `Some(marker)` when the line opens a fenced block, carrying the fence
/// characters so the closer has to match the opener's kind.
fn fence_opens(trimmed: &str) -> Option<String> {
  for marker in ["```", "~~~"] {
    if trimmed.starts_with(marker) {
      return Some(marker.to_string());
    }
  }
  None
}

/// `Some(None)` when the line opens an HTML comment that stays open,
/// `Some(Some(tail))` when it also closes on the same line.
fn comment_opens(trimmed: &str) -> Option<Option<String>> {
  let rest = trimmed.strip_prefix("<!--")?;
  match rest.split_once("-->") {
    Some((_, tail)) => Some(Some(tail.to_string())),
    None => Some(None),
  }
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
    match marker_at(&chars, i) {
      Some((Hit::Styled(segment), next)) => {
        flush(&mut out, &mut plain, base);
        out.push(segment);
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
    if flanks && !intraword {
      return Some(at);
    }
    i = at + 1;
  }
  None
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
