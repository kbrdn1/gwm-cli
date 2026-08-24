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
