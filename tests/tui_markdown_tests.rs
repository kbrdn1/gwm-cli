//! Markdown rendering for the rich PR / issue view (issue #551).
//!
//! Pure state, no ratatui: [`Emphasis`] is a semantic role and the mapping to
//! theme colours belongs to the renderer. Which means these assertions cover
//! HALF the surface. The other half — that the renderer paints each role
//! distinctly — is in `tests/tui_modal_render_tests.rs`, because a parse that
//! produces perfect segments nobody colours is a feature that is dead on
//! screen with the suite green.

use gwm::tui::state::markdown::{render, Emphasis, MdLine};

/// Wide enough that nothing below wraps unless the test is about wrapping.
const W: usize = 80;

fn lines(body: &str) -> Vec<MdLine> {
  render(body, W)
}

/// Every line as the reader sees it.
fn plain(body: &str) -> Vec<String> {
  lines(body).iter().map(MdLine::plain).collect()
}

/// The roles used on the first line that carries text.
fn roles(body: &str) -> Vec<(String, Emphasis)> {
  lines(body)
    .into_iter()
    .find(|l| !l.plain().trim().is_empty())
    .map(|l| l.segments.into_iter().map(|s| (s.text, s.emphasis)).collect())
    .unwrap_or_default()
}

#[test]
fn bold_loses_its_markers_and_gains_a_role() {
  assert_eq!(
    roles("**Lists, for everyone.**"),
    vec![("Lists, for everyone.".to_string(), Emphasis::Bold)]
  );
}

#[test]
fn the_marker_table_is_ordered_longest_first() {
  // `***x***` must not be read as `*` opening an italic that ends at the
  // next `*`, which would leave the reader with `**x**` painted literally.
  assert_eq!(roles("***loud***"), vec![("loud".to_string(), Emphasis::BoldItalic)]);
  assert_eq!(roles("~~gone~~"), vec![("gone".to_string(), Emphasis::Strike)]);
  assert_eq!(roles("_soft_"), vec![("soft".to_string(), Emphasis::Italic)]);
}

#[test]
fn inline_code_wins_over_every_other_marker() {
  // A backtick span is literal: `*` inside it is an asterisk, not emphasis.
  assert_eq!(
    roles("run `a * b` now"),
    vec![
      ("run ".to_string(), Emphasis::Plain),
      ("a * b".to_string(), Emphasis::Code),
      (" now".to_string(), Emphasis::Plain),
    ]
  );
}

#[test]
fn an_unclosed_marker_stays_literal() {
  // Prose is full of lone asterisks and underscores (`snake_case`, a `*` in
  // a glob). Treating an opener with no closer as emphasis would swallow the
  // rest of the line into a role it never asked for.
  assert_eq!(
    plain("2 * 3 and file_name and **open"),
    vec!["2 * 3 and file_name and **open"]
  );
}

#[test]
fn a_heading_drops_its_hashes_and_a_section_break_gets_a_rule() {
  let got = plain("## Description\n\nprose");
  assert_eq!(got[0], "Description");
  assert!(
    got[1].starts_with('─'),
    "a level-2 heading is a section break on the forge; the rule is how a terminal says so: {got:?}"
  );
  // `#tag` is not a heading — the space is what makes it one.
  assert_eq!(plain("#551 is the issue"), vec!["#551 is the issue"]);
}

#[test]
fn a_list_item_gets_a_bullet_and_a_task_gets_its_box() {
  assert_eq!(plain("- first\n- second"), vec!["• first", "• second"]);
  assert_eq!(plain("1. first"), vec!["• first"]);
  assert_eq!(plain("- [x] Feature\n- [ ] Docs"), vec!["• ☑ Feature", "• ☐ Docs"]);
}

#[test]
fn a_github_alert_reads_as_a_callout_not_as_brackets() {
  // `> [!IMPORTANT]` is a coloured callout header on the forge and was
  // painted as literal brackets here.
  let got = roles("> [!IMPORTANT]");
  assert_eq!(
    got,
    vec![
      // The rail is structure this module inserts, not the author's text,
      // which is what `Marker` means. It repeats down every wrapped row.
      ("▎ ".to_string(), Emphasis::Marker),
      ("IMPORTANT".to_string(), Emphasis::Heading),
    ]
  );
  // An unknown label is not an alert, so it stays the text it is.
  assert_eq!(plain("> [!SHOUTY]"), vec!["▎ [!SHOUTY]"]);
}

#[test]
fn an_html_comment_is_not_shown_at_all() {
  // Bot reviews open with a couple of these ("summarize by coderabbit.ai"),
  // and the forge shows none of them.
  assert_eq!(
    plain("<!-- This is an auto-generated comment: summarize -->\nreal text"),
    vec!["real text"]
  );
  // Multi-line, which is how they actually arrive.
  assert_eq!(plain("<!--\nskip review\n-->\nreal text"), vec!["real text"]);
}

#[test]
fn a_link_shows_its_text_not_its_url() {
  assert_eq!(
    roles("see [the issue](https://github.com/kbrdn1/gwm-cli/issues/551) now"),
    vec![
      ("see ".to_string(), Emphasis::Plain),
      ("the issue".to_string(), Emphasis::Link),
      (" now".to_string(), Emphasis::Plain),
    ]
  );
}

#[test]
fn a_fenced_block_is_code_and_nothing_inside_it_is_a_marker() {
  let got = lines("```yaml\njobs:\n  build:\n    runs-on: ubuntu # *not* italic\n```");
  let texts: Vec<String> = got.iter().map(MdLine::plain).collect();
  assert_eq!(
    texts,
    vec!["jobs:", "  build:", "    runs-on: ubuntu # *not* italic"],
    "the fence markers are structure, the lines between them are verbatim"
  );
  assert!(
    got.iter().all(|l| l.preformatted),
    "a fenced line must be flagged so the renderer paints it as code"
  );
  assert!(
    got
      .iter()
      .all(|l| l.segments.iter().all(|s| s.emphasis == Emphasis::Code)),
    "an asterisk inside a fence is an asterisk: {got:?}"
  );
}

#[test]
fn indentation_inside_a_fence_survives() {
  // In YAML or Python the indentation IS the meaning (Codex review #529),
  // and this is the class of body that carries one: almost every PR on this
  // repo has a fenced block in its description.
  let got = plain("```\n  two\n    four\n```");
  assert_eq!(got, vec!["  two", "    four"]);
}

#[test]
fn wrapping_carries_the_emphasis_across_the_break() {
  // The reason the parse happens before the wrap. Wrapping the SOURCE would
  // cut between `**` and the text it opens, and the second row would lose
  // the role entirely.
  // Trimmed on purpose: a closer preceded by whitespace does not close a
  // run, on the forge either. `**bold **` is literal asterisks there too.
  let body = format!("**{}**", "bold ".repeat(40).trim());
  let got = render(&body, 30);
  assert!(got.len() > 1, "the body must actually wrap");
  for line in &got {
    for segment in &line.segments {
      assert_eq!(
        segment.emphasis,
        Emphasis::Bold,
        "every row of a wrapped bold run stays bold: {line:?}"
      );
    }
    assert!(!line.plain().contains('*'), "no row may show the marker: {line:?}");
  }
}

#[test]
fn no_rendered_line_is_wider_than_its_budget() {
  // The renderer ellipsises what overflows, so an over-wide row loses
  // exactly the tail the reader was after.
  let body = "# A heading long enough to need more than one row of a narrow modal\n\n\
              - a list item that also runs past the budget it was given here\n\n\
              > a quoted paragraph that runs past the budget as well, twice over\n\n\
              a plain paragraph with a https://example.test/an/unbreakably/long/url/in/it";
  for budget in [12usize, 24, 40, 80] {
    for line in render(body, budget) {
      let cols = line.plain().chars().count();
      assert!(
        cols <= budget,
        "a {cols}-column row broke a {budget}-column budget: {:?}",
        line.plain()
      );
    }
  }
}

#[test]
fn a_zero_budget_terminates() {
  // A wrap loop that never advances hangs the render thread, which is not a
  // failure mode worth leaving reachable.
  let got = render("some prose that cannot possibly fit\n\n- and a list", 0);
  assert!(!got.is_empty());
}

#[test]
fn a_line_that_fits_is_not_reflowed() {
  // `split_whitespace` collapses runs of spaces, and hand-made alignment in
  // a body is meaning the reader put there (Codex review #529).
  assert_eq!(plain("A    |    B"), vec!["A    |    B"]);
  assert_eq!(plain("    indented prose"), vec!["    indented prose"]);
}

#[test]
fn remote_text_is_still_sanitised() {
  // Same boundary as before this module existed (issue #502): a body comes
  // from a remote forge and can carry a bidi override that makes the
  // terminal paint an order the bytes do not have.
  let got = plain("safe \u{202E}txet desrever\u{202C} end");
  assert!(
    !got.iter().any(|l| l.contains('\u{202E}') || l.contains('\u{202C}')),
    "a bidi override must not reach the terminal: {got:?}"
  );
}

#[test]
fn a_blank_line_between_paragraphs_survives() {
  // The view had no concept of a paragraph before this: everything was a
  // label/value pair, so prose ran together.
  assert_eq!(plain("one\n\ntwo"), vec!["one", "", "two"]);
}

#[test]
fn a_marker_flanked_by_whitespace_does_not_delimit() {
  // CommonMark's flanking rules, and the reason prose survives contact with
  // this parser: an opener is not followed by whitespace, a closer is not
  // preceded by it. Without them a lone `*` opened a run that swallowed
  // everything up to the next asterisk anywhere on the line.
  assert_eq!(plain("a * b * c"), vec!["a * b * c"]);
  assert_eq!(plain("**bold **"), vec!["**bold **"]);
  // And `_` does not delimit inside a word, because a body about code is
  // full of identifiers.
  assert_eq!(plain("call file_name_here once"), vec!["call file_name_here once"]);
  // The rules must not break the nominal case.
  assert_eq!(roles("**yes**"), vec![("yes".to_string(), Emphasis::Bold)]);
}

#[test]
fn a_fenced_line_is_not_wrapped_it_is_kept_whole() {
  // Issue #551. A wrapped `+` line's continuation carries no sigil and reads
  // as context; a wrapped YAML line lands at the wrong indent. In code the
  // column IS the meaning, which is the argument `hunk_rows` already makes
  // for diff hunks (#528). So a fenced line is emitted WHOLE, past the
  // budget if it has to be, and the horizontal offset is what reaches its
  // tail.
  let long = "x".repeat(200);
  let got = render(&format!("```\n{long}\n```"), 40);
  assert_eq!(got.len(), 1, "one source line, one rendered line: {got:?}");
  assert_eq!(got[0].plain(), long, "kept whole rather than reflowed");
  assert!(got[0].preformatted);
}

#[test]
fn prose_still_wraps_next_to_a_fence_that_does_not() {
  // The exemption is for preformatted lines only. A paragraph in the same
  // body keeps wrapping, or the change would trade one unreadable view for
  // another.
  let got = render(&format!("{}\n\n```\n{}\n```", "word ".repeat(40), "y".repeat(90)), 30);
  let prose: Vec<&MdLine> = got.iter().filter(|l| !l.preformatted).collect();
  assert!(prose.len() > 1, "the paragraph wrapped");
  for line in prose {
    assert!(line.plain().chars().count() <= 30, "{:?}", line.plain());
  }
  assert!(
    got.iter().any(|l| l.preformatted && l.plain().chars().count() == 90),
    "and the fenced line did not: {got:?}"
  );
}

#[test]
fn an_html_comment_is_hidden_wherever_it_starts_on_the_line() {
  // Codex review, pass 1 (P2): `comment_opens` used `strip_prefix`, so a
  // comment opening after visible text was painted with its markers. The
  // module claims HTML comments are not shown at all, and a claim that only
  // holds at column zero is a claim that does not hold.
  assert_eq!(
    plain("visible <!-- hidden --> tail"),
    vec!["visible  tail"],
    "the comment goes, the text around it stays"
  );
  // Opening mid-line and closing on a later one. The closing line keeps the
  // space that followed its `-->`: a line that fits is rendered verbatim,
  // which is the property that preserves hand-made alignment, and it does
  // not get to make an exception for whitespace it happens to dislike.
  assert_eq!(
    plain("visible <!-- start\nstill hidden\n--> tail"),
    vec!["visible ", " tail"]
  );
  // A lone `-->` with no opener is not a comment; it is text.
  assert_eq!(plain("a --> b"), vec!["a --> b"]);
}

#[test]
fn a_longer_fence_is_not_closed_by_a_shorter_one_inside_it() {
  // Codex review, pass 2 (P2). Four backticks are how you show a
  // three-backtick block, which is exactly what a PR about Markdown
  // rendering contains. Matching on the opener's characters alone let the
  // inner fence close the outer block, so the inner line vanished and the
  // rest of the code was read as prose.
  let got = plain("````\n```\ncode\n```\n````\nafter");
  assert_eq!(
    got,
    vec!["```", "code", "```", "after"],
    "the inner fence is content, only a fence at least as long closes"
  );
}

#[test]
fn an_html_comment_inside_inline_code_is_kept() {
  // Codex review, pass 2 (P2), and the mirror of the pass-1 fix: the strip
  // ran before the backtick scan, so a body documenting the delimiter lost
  // it. The forge shows it as literal code, which is the whole point of
  // putting it in backticks.
  let got = roles("Use `<!-- marker -->` here");
  assert_eq!(
    got,
    vec![
      ("Use ".to_string(), Emphasis::Plain),
      ("<!-- marker -->".to_string(), Emphasis::Code),
      (" here".to_string(), Emphasis::Plain),
    ]
  );
  // An unbacked comment on the same line still goes.
  assert_eq!(plain("`<!-- kept -->` <!-- gone -->"), vec!["<!-- kept --> "]);
}

#[test]
fn emphasis_nested_inside_emphasis_is_parsed_not_shown() {
  // Codex review, pass 3 (P2). `Emphasis::BoldItalic` documents this exact
  // body as the reason it exists, and the parser put the whole inner slice
  // in one `Bold` run without re-reading it — so the underscores stayed on
  // screen and the nested half never got the role the doc promised.
  assert_eq!(
    roles("**bold _and_ italic**"),
    vec![
      ("bold ".to_string(), Emphasis::Bold),
      ("and".to_string(), Emphasis::BoldItalic),
      (" italic".to_string(), Emphasis::Bold),
    ]
  );
  // Inline code nested in emphasis keeps being code: it is the more
  // specific role, and its content is literal.
  assert_eq!(
    roles("**run `x` now**"),
    vec![
      ("run ".to_string(), Emphasis::Bold),
      ("x".to_string(), Emphasis::Code),
      (" now".to_string(), Emphasis::Bold),
    ]
  );
}

#[test]
fn a_backslash_escape_shows_the_character_not_the_markup() {
  // Codex review, pass 3 (P2). `\*literal\*` is ordinary Markdown for
  // showing an asterisk. Unescaped, the delimiters matched and the body
  // rendered as `\literal\` in italics: the markup was obeyed AND the
  // backslashes were shown, which is both halves wrong at once.
  assert_eq!(plain(r"\*literal\*"), vec!["*literal*"]);
  assert_eq!(plain(r"a \_b\_ c"), vec!["a _b_ c"]);
  // A backslash before an ordinary character is just a backslash.
  assert_eq!(plain(r"C:\path\to"), vec![r"C:\path\to"]);
  // And an escape does not break the nominal case next to it.
  assert_eq!(
    roles(r"\*not\* **yes**"),
    vec![
      ("*not* ".to_string(), Emphasis::Plain),
      ("yes".to_string(), Emphasis::Bold),
    ]
  );
}

#[test]
fn deeply_nested_emphasis_terminates() {
  // The nesting fix re-enters the parser on the inner slice. That slice is
  // strictly shorter — the delimiters are gone — so it cannot recurse
  // forever, but a parser that hangs takes the render thread with it and
  // the property is cheap to hold down.
  let deep = format!("{}x{}", "**_".repeat(20), "_**".repeat(20));
  assert!(!render(&deep, 40).is_empty());
  assert!(!render("***_a_***", 40).is_empty());
  assert!(!render("**", 40).is_empty());
}

#[test]
fn an_escaped_delimiter_inside_emphasis_does_not_close_it() {
  // Codex review, pass 4 (P2), on the escape support added in pass 3: the
  // opener honoured escapes but the CLOSER did not, so `*foo \* bar*` ended
  // at the escaped asterisk and rendered carrying the backslash it was
  // escaping with.
  assert_eq!(
    roles(r"*foo \* bar*"),
    vec![("foo * bar".to_string(), Emphasis::Italic)]
  );
  // Parity, not presence: `\\` is an escaped backslash, so the asterisk
  // after it is live and does close the run.
  assert_eq!(
    roles(r"*a \\* b"),
    vec![
      (r"a \".to_string(), Emphasis::Italic),
      (" b".to_string(), Emphasis::Plain),
    ]
  );
}

#[test]
fn a_multi_backtick_code_span_holds_a_backtick() {
  // Codex review, pass 5 (P2). Two backticks are how you show a span that
  // contains one, and the scan stopped at the next single backtick instead
  // of a run of the same length — so a literal backtick survived on each
  // side, and a `<!-- … -->` written inside such a span was exposed to the
  // comment strip and deleted.
  assert_eq!(
    roles("a ``x ` y`` b"),
    vec![
      ("a ".to_string(), Emphasis::Plain),
      ("x ` y".to_string(), Emphasis::Code),
      (" b".to_string(), Emphasis::Plain),
    ]
  );
  assert_eq!(plain("``<!-- kept -->``"), vec!["<!-- kept -->"]);
}

#[test]
fn a_line_of_unclosed_openers_does_not_take_quadratic_time() {
  // Codex review, pass 5 (P2), and the only finding of this loop that
  // describes the TUI locking up rather than looking wrong. Each opener
  // rescanned the whole suffix before failing, so a body of `*a *a *a …` —
  // remote text, and a forge accepts 65 536 characters of it — went
  // quadratic on the render thread, which re-wraps on every resize.
  //
  // Timed with a wide margin rather than counted: linear is milliseconds
  // here, quadratic is minutes, and no plausible CI runner sits between.
  let body = "*a ".repeat(20_000);
  let start = std::time::Instant::now();
  let got = render(&body, 80);
  assert!(!got.is_empty());
  assert!(
    start.elapsed().as_secs() < 5,
    "took {:?}; a rescan per opener is the shape that gets here",
    start.elapsed()
  );
}

#[test]
fn emphasis_sharing_a_delimiter_run_is_a_documented_limit() {
  // Codex review, pass 5 (P2), NOT fixed and deliberately so. Splitting the
  // trailing run of three between the two levels is CommonMark's delimiter
  // stack, and implementing it here means reimplementing a Markdown parser
  // to render a PR body in a terminal.
  //
  // Pinned rather than left unsaid: the day it does get fixed, this test
  // fails and says where the decision was made. Nesting with DIFFERENT
  // markers, which is the form that reads naturally, does work.
  let shared = plain("**bold and *italic***");
  assert!(
    shared[0].contains('*'),
    "the markers stay visible, which is what every body did before this module: {shared:?}"
  );
  assert_eq!(
    roles("**bold _and_ italic**"),
    vec![
      ("bold ".to_string(), Emphasis::Bold),
      ("and".to_string(), Emphasis::BoldItalic),
      (" italic".to_string(), Emphasis::Bold),
    ],
    "the form that does work must keep working"
  );
}
