//! Row builders for the rich PR / issue view (issue #420).
//!
//! The overlay shell renders one *line* per [`DetailRow`] and truncates a
//! long `value` with an ellipsis — there is no wrapping in the render path
//! and adding one would change how the agent and CI consumers lay out. So
//! the wrapping happens here, in pure state: a body becomes N rows, and
//! every assertion below is a pure function call with no ratatui involved.

use gwm::forge::{ForgeComment, ForgeReview, IssueDetail, PrDetail, ReviewState};
use gwm::forge::{ReviewThread, ReviewThreads};
use gwm::github::{CheckOutcome, CiState, IssueState, IssueStatus, PrCheck, PrState, PrStatus};
use gwm::tui::state::detail_overlay::DetailRole;
use gwm::tui::state::markdown::Emphasis;
use gwm::tui::state::rich_view::{rich_issue_rows, rich_pr_rows, LABEL_W};
use gwm::tui::GitHubFetchState;

/// The inner width the list-mode overlay hands the builder on a typical
/// terminal: `overlay_modal_width(120) - 6`.
const W: usize = 68;

/// The threads state for the tests that predate the section (issue
/// #528): nobody asked for the fetch, so the section is absent.
const NO_THREADS: GitHubFetchState<ReviewThreads> = GitHubFetchState::Idle;

fn sample_pr() -> PrStatus {
  PrStatus {
    number: 519,
    title: "feat(config): Symfony preset".into(),
    state: PrState::Open,
    url: "https://github.com/kbrdn1/gwm-cli/pull/519".into(),
    updated_at: "2026-08-04T13:00:00Z".into(),
    checks_passed: 7,
    checks_total: 7,
    ci: CiState::Passing,
    checks: vec![PrCheck {
      name: "ci".into(),
      outcome: CheckOutcome::Passing,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    }],
    detail: PrDetail {
      body: "A seventh preset.".into(),
      author: "kbrdn1".into(),
      additions: 1198,
      deletions: 12,
      base_ref: "dev".into(),
      head_ref: "feat/#392-symfony-preset".into(),
      reviews: vec![
        ForgeReview {
          author: "Copilot".into(),
          state: ReviewState::Approved,
          body: String::new(),
          submitted_at: "2026-08-04T14:00:00Z".into(),
        },
        ForgeReview {
          author: "coderabbitai".into(),
          state: ReviewState::ChangesRequested,
          body: "Actionable comments posted: 2".into(),
          submitted_at: "2026-08-04T13:40:21Z".into(),
        },
      ],
      comments: vec![ForgeComment {
        author: "kbrdn1".into(),
        body: "rebased".into(),
        created_at: "2026-08-04T15:00:00Z".into(),
        url: Some("https://github.com/kbrdn1/gwm-cli/pull/519#issuecomment-3".into()),
      }],
    },
  }
}

fn sample_issue() -> IssueStatus {
  IssueStatus {
    number: 420,
    title: "rich PR/Issue view".into(),
    state: IssueState::Open,
    url: "https://github.com/kbrdn1/gwm-cli/issues/420".into(),
    labels: vec!["feature".into(), "tui".into()],
    updated_at: "2026-08-01T10:00:00Z".into(),
    detail: IssueDetail {
      body: "The Status pane cannot show what the issue contains.".into(),
      author: "kbrdn1".into(),
      comments: vec![ForgeComment {
        author: "sassman".into(),
        body: "Sounds good.".into(),
        created_at: "2026-08-02T09:00:00Z".into(),
        url: Some("https://github.com/kbrdn1/gwm-cli/issues/420#issuecomment-1".into()),
      }],
    },
  }
}

/// The shell's own layout, mirrored: a LABELLED row pays the label column
/// and its two padding columns, a label-less row spans the whole inner
/// width (issue #551). Getting this wrong in either direction makes every
/// width assertion below meaningless, so it is the one thing
/// `tests/tui_modal_render_tests.rs` asserts against the real renderer.
fn row_width(r: &gwm::tui::state::detail_overlay::DetailRow) -> usize {
  if r.label.is_empty() {
    r.value.chars().count()
  } else {
    LABEL_W + 2 + r.value.chars().count()
  }
}

fn values(rows: &[gwm::tui::state::detail_overlay::DetailRow]) -> Vec<String> {
  rows.iter().map(|r| r.value.clone()).collect()
}

fn value_for(rows: &[gwm::tui::state::detail_overlay::DetailRow], label: &str) -> Option<String> {
  rows.iter().find(|r| r.label == label).map(|r| r.value.clone())
}

#[test]
fn every_emitted_label_fits_the_reserved_column() {
  // `LABEL_W` is an ASSUMPTION here and a DERIVED value in the renderer,
  // which sizes its label column from the widest label it is handed. A
  // longer label added later would push every row past the modal's inner
  // width — and `row_width` below, which measures against the constant,
  // would keep passing. This is the assertion that makes the width checks
  // mean something.
  let rows = [
    rich_pr_rows(&sample_pr(), &NO_THREADS, W),
    rich_issue_rows(&sample_issue(), W),
  ]
  .concat();
  for r in &rows {
    assert!(
      r.label.chars().count() <= LABEL_W,
      "label {:?} is wider than the {LABEL_W}-column budget the wrap assumes",
      r.label
    );
  }
}

#[test]
fn pr_rows_carry_the_metadata_block() {
  let rows = rich_pr_rows(&sample_pr(), &NO_THREADS, W);

  assert_eq!(value_for(&rows, "state").as_deref(), Some("open"));
  assert_eq!(value_for(&rows, "author").as_deref(), Some("kbrdn1"));
  assert_eq!(
    value_for(&rows, "branch").as_deref(),
    Some("feat/#392-symfony-preset → dev"),
    "head → base, the direction the merge actually goes"
  );
  assert_eq!(value_for(&rows, "diff").as_deref(), Some("+1198 −12"));
  assert_eq!(value_for(&rows, "checks").as_deref(), Some("passing 7/7"));
  assert_eq!(
    value_for(&rows, "updated").as_deref(),
    Some("2026-08-04"),
    "the date alone: the RFC 3339 timestamp is noise at a glance"
  );
}

#[test]
fn the_url_row_is_the_actionable_one() {
  let rows = rich_pr_rows(&sample_pr(), &NO_THREADS, W);
  let url = rows.iter().find(|r| r.label == "url").expect("a url row");

  assert_eq!(url.value, "https://github.com/kbrdn1/gwm-cli/pull/519");
  assert_eq!(
    url.meta.as_deref(),
    Some("https://github.com/kbrdn1/gwm-cli/pull/519"),
    "meta is what Enter opens in the browser"
  );
}

#[test]
fn no_row_overflows_the_width_it_was_built_for() {
  let mut pr = sample_pr();
  pr.detail.body = "A very long paragraph that has to be broken across several rows \
because the overlay renderer truncates a value instead of wrapping it, so the wrapping \
is this builder's job and nothing else's."
    .into();

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);

  for r in &rows {
    assert!(
      row_width(r) <= W,
      "row {:?} is {} cols wide, budget is {W}",
      r.value,
      row_width(r)
    );
  }
  assert!(
    rows.iter().filter(|r| r.value.contains("wrapping")).count() >= 1,
    "the paragraph survives the wrap"
  );
}

#[test]
fn a_word_longer_than_the_budget_is_hard_split() {
  let mut pr = sample_pr();
  // A URL in a body has no break opportunity: a word-only wrap would emit
  // one over-wide row and the renderer would ellipsise the tail away.
  pr.detail.body = format!("see {}", "x".repeat(200));

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);

  for r in &rows {
    assert!(row_width(r) <= W, "unbreakable word must be hard split");
  }
}

#[test]
fn a_long_body_is_rendered_whole_because_the_scroll_is_the_budget() {
  // Issue #551, replacing `a_long_body_is_capped_and_says_so`. The 40-line
  // cap was honest about what it dropped, but it was spending a budget the
  // overlay does not actually have to ration: the view scrolls, so the
  // window is the terminal height and the row count costs nothing but the
  // rows themselves. A description cut at `… 85 more lines` was the single
  // loudest complaint against this view.
  let mut pr = sample_pr();
  pr.detail.body = (0..500).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);
  let vals = values(&rows);

  assert_eq!(
    vals.iter().filter(|v| v.starts_with("line ")).count(),
    500,
    "every line of the description must be there"
  );
  assert!(
    !vals.iter().any(|v| v.contains("more lines")),
    "nothing was dropped, so nothing may claim it was: {:?}",
    vals.iter().filter(|v| v.contains("more")).collect::<Vec<_>>()
  );
}

#[test]
fn every_comment_of_the_conversation_is_rendered() {
  // The other half of the same call (issue #551): the comment LIST was
  // capped at 20 and each comment's body at 12 lines, so a busy thread read
  // as a wall of `… N more`. `gwm` is where the user is; sending them to the
  // browser for the rest of a conversation it already fetched is the thing
  // the view exists to avoid.
  let mut pr = sample_pr();
  pr.detail.comments = (0..40)
    .map(|i| ForgeComment {
      author: format!("commenter{i}"),
      body: (0..30).map(|l| format!("body line {l}")).collect::<Vec<_>>().join("\n"),
      created_at: "2026-08-04T15:00:00Z".into(),
      url: None,
    })
    .collect();

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);
  let vals = values(&rows);

  assert!(
    vals.iter().any(|v| v.contains("commenter39")),
    "the last comment must be reachable, not elided behind a marker"
  );
  assert!(
    !vals.iter().any(|v| v.contains("more comments")),
    "no comment was dropped, so nothing may claim it was"
  );
  assert_eq!(
    vals.iter().filter(|v| v.trim() == "body line 29").count(),
    40,
    "every comment must render its whole body"
  );
}

#[test]
fn control_and_bidi_characters_are_neutralised() {
  let mut pr = sample_pr();
  // A body is remote text: it can carry a right-to-left override that makes
  // the terminal render an order the bytes do not have (issue #502), or a
  // lone CR that overwrites the line already painted.
  pr.detail.body = "safe \u{202E}txet desrever\u{202C} and \u{000D}overwrite".into();
  pr.detail.author = "al\u{202E}ice".into();

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);

  for r in &rows {
    assert!(
      !r.value
        .chars()
        .any(|c| c.is_control() || ('\u{202A}'..='\u{202E}').contains(&c)),
      "row {:?} still carries a control or bidi character",
      r.value
    );
  }
  assert_eq!(value_for(&rows, "author").as_deref(), Some("al?ice"));
}

#[test]
fn reviews_are_listed_with_their_verdict() {
  use gwm::tui::state::detail_overlay::DetailRole;
  let rows = rich_pr_rows(&sample_pr(), &NO_THREADS, W);
  let vals = values(&rows);

  assert!(
    vals.iter().any(|v| v.contains("reviews")),
    "a section header naming the count"
  );
  let approved = rows
    .iter()
    .find(|r| r.value.contains("approved") && r.value.contains("Copilot"))
    .expect("the approval row");
  assert_eq!(approved.role, DetailRole::Success, "green for an approval");

  let changes = rows
    .iter()
    .find(|r| r.value.contains("changes requested"))
    .expect("the changes-requested row");
  assert_eq!(changes.role, DetailRole::Failure, "red for changes requested");
  assert!(
    vals.iter().any(|v| v.contains("Actionable comments posted: 2")),
    "the review body rides along: {vals:?}"
  );
}

#[test]
fn comments_carry_their_permalink_as_meta() {
  let rows = rich_pr_rows(&sample_pr(), &NO_THREADS, W);

  let head = rows
    .iter()
    .find(|r| r.value.contains("kbrdn1") && r.value.contains("2026-08-04"))
    .expect("the comment header row");
  assert_eq!(
    head.meta.as_deref(),
    Some("https://github.com/kbrdn1/gwm-cli/pull/519#issuecomment-3")
  );
  assert!(values(&rows).iter().any(|v| v.contains("rebased")));
}

#[test]
fn a_summary_only_pr_renders_no_empty_sections() {
  let mut pr = sample_pr();
  pr.detail = PrDetail::default();

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);
  let vals = values(&rows);

  assert!(
    !vals.iter().any(|v| v.starts_with("description")),
    "no description header without a body: {vals:?}"
  );
  assert!(!vals.iter().any(|v| v.starts_with("reviews")));
  assert!(!vals.iter().any(|v| v.starts_with("comments")));
  // The summary tier still renders — a GitLab MR lands exactly here.
  assert_eq!(value_for(&rows, "state").as_deref(), Some("open"));
  assert!(
    value_for(&rows, "diff").is_none(),
    "a 0/0 diff is a missing measurement, not an empty one"
  );
  assert!(value_for(&rows, "branch").is_none(), "no branch pair without the refs");
}

#[test]
fn issue_rows_drop_what_an_issue_does_not_have() {
  let rows = rich_issue_rows(&sample_issue(), W);
  let vals = values(&rows);

  assert_eq!(value_for(&rows, "state").as_deref(), Some("open"));
  assert_eq!(value_for(&rows, "author").as_deref(), Some("kbrdn1"));
  assert_eq!(value_for(&rows, "labels").as_deref(), Some("feature, tui"));
  assert!(value_for(&rows, "checks").is_none(), "an issue has no CI");
  assert!(value_for(&rows, "diff").is_none(), "an issue has no diff");
  assert!(
    !vals.iter().any(|v| v.starts_with("reviews")),
    "an issue has no reviews"
  );
  assert!(vals.iter().any(|v| v.contains("Sounds good.")), "the comment renders");
}

#[test]
fn a_draft_pr_says_draft() {
  let mut pr = sample_pr();
  pr.state = PrState::Draft;
  assert_eq!(
    value_for(&rich_pr_rows(&pr, &NO_THREADS, W), "state").as_deref(),
    Some("draft")
  );
}

#[test]
fn a_zero_width_budget_does_not_panic_or_loop() {
  // `overlay_modal_width` clamps at 48, so this cannot happen through the
  // TUI — but a wrap loop that never advances hangs the whole render
  // thread, and that is not a failure mode worth leaving reachable.
  let rows = rich_pr_rows(&sample_pr(), &NO_THREADS, 0);
  assert!(!rows.is_empty());
}

#[test]
fn a_preformatted_block_keeps_its_indentation() {
  // Codex review #529: the wrap ran every line through `split_whitespace`,
  // which drops the leading indent and collapses runs of spaces. That is
  // the nominal case, not an edge one: a PR description on this repo
  // almost always carries a fenced block, and for YAML or Python the
  // indentation IS the meaning.
  let mut pr = sample_pr();
  pr.detail.body = "Config:\n\n```yaml\njobs:\n  build:\n    runs-on: ubuntu\n```\n\nA | B\n--- | ---\n1 | 2".into();

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);
  let vals = values(&rows);

  assert!(
    vals.iter().any(|v| v == "  build:"),
    "the two-space indent must survive: {vals:?}"
  );
  assert!(
    vals.iter().any(|v| v == "    runs-on: ubuntu"),
    "and so must the four-space one: {vals:?}"
  );
  assert!(
    vals.iter().any(|v| v == "--- | ---"),
    "aligned table separators must not be re-spaced: {vals:?}"
  );
}

#[test]
fn a_wrapped_continuation_keeps_the_line_indent() {
  // A preformatted line too long for the modal still has to wrap, but its
  // continuations belong under the original indent, not at column zero.
  let mut pr = sample_pr();
  pr.detail.body = format!("    {}", "alpha ".repeat(40));

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);
  let body: Vec<&String> = rows.iter().map(|r| &r.value).filter(|v| v.contains("alpha")).collect();

  assert!(body.len() > 1, "precondition: the line had to wrap");
  for line in &body {
    assert!(line.starts_with("    "), "continuation lost the indent: {line:?}");
    assert!(row_width_of(line) <= W, "indent must be inside the budget: {line:?}");
  }
}

/// `row_width` for a bare value string. Every caller measures a BODY line,
/// which carries no label and therefore no gutter.
fn row_width_of(v: &str) -> usize {
  v.chars().count()
}

#[test]
fn indented_comment_and_review_bodies_stay_inside_the_budget() {
  // The width checks above only ever measured PR *description* rows, where
  // the body indent is empty. Comment and review bodies are pushed with a
  // two-space indent, and `wrap_line`'s "already fits" early return
  // measures against the budget it was handed, not against the indent the
  // caller prepends afterwards. So that whole class of row was unmeasured.
  let mut pr = sample_pr();
  // Lines walking across the boundary from both sides.
  let body = (50..70).map(|n| "y".repeat(n)).collect::<Vec<_>>().join("\n");
  pr.detail.comments[0].body = body.clone();
  pr.detail.reviews[1].body = body;

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);

  for r in &rows {
    assert!(
      row_width(r) <= W,
      "row {:?} is {} cols, budget {W}",
      r.value,
      row_width(r)
    );
  }
  assert!(
    rows.iter().any(|r| r.value.starts_with("  y")),
    "precondition: the indented bodies rendered"
  );
}

// ---- Inline review threads (issue #528) ---------------------------------

fn thread(path: &str, line: Option<u32>, start: Option<u32>, hunk: &str, bodies: &[&str]) -> ReviewThread {
  ReviewThread {
    path: path.into(),
    line,
    start_line: start,
    is_resolved: false,
    is_outdated: false,
    diff_hunk: hunk.into(),
    total_comments: bodies.len() as u32,
    comments: bodies
      .iter()
      .map(|b| ForgeComment {
        author: "coderabbitai".into(),
        body: (*b).into(),
        created_at: "2026-08-04T13:40:21Z".into(),
        url: Some("https://github.com/kbrdn1/gwm-cli/pull/514#discussion_r1".into()),
      })
      .collect(),
  }
}

fn loaded(threads: Vec<ReviewThread>, total: u32) -> GitHubFetchState<ReviewThreads> {
  GitHubFetchState::Loaded(ReviewThreads::Threads { threads, total })
}

#[test]
fn a_thread_renders_its_anchor_its_hunk_and_its_chain() {
  let state = loaded(
    vec![thread(
      "src/tui/app.rs",
      Some(11),
      Some(7),
      "@@ -4,10 +4,11 @@\n context\n-old line\n+new line",
      &["This drops the guard.", "Fixed."],
    )],
    1,
  );

  let rows = rich_pr_rows(&sample_pr(), &state, W);
  let text = values(&rows).join("\n");

  // The anchor is what tells the reader which code is under discussion.
  assert!(text.contains("src/tui/app.rs:7-11"), "no anchor row in:\n{text}");
  // The hunk is the context, sigils included.
  assert!(text.contains("+new line"), "no hunk in:\n{text}");
  assert!(text.contains("-old line"), "no hunk in:\n{text}");
  // The chain stays a chain.
  assert!(text.contains("This drops the guard."));
  assert!(text.contains("Fixed."));
}

#[test]
fn a_single_line_anchor_renders_one_number_not_a_range() {
  let state = loaded(
    vec![thread("src/lib.rs", Some(3), None, "@@ -1,1 +1,1 @@\n+x", &["nit"])],
    1,
  );

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n");

  assert!(text.contains("src/lib.rs:3"), "in:\n{text}");
  assert!(!text.contains("src/lib.rs:3-3"), "a null startLine is not a range");
}

#[test]
fn hunk_lines_keep_their_sigil_and_are_never_re_wrapped() {
  // The wrap path splits on whitespace, so a wrapped `+` line's
  // continuation would read as context — in a diff the sigil *is* the
  // meaning. A long hunk line is kept whole instead, which keeps the row
  // count equal to the line count. It USED to be truncated here; issue #551
  // moved that decision to the renderer, which clips against the view's
  // horizontal offset so the tail can still be reached.
  let long_add = format!("+{}", "x ".repeat(80));
  let hunk = format!("@@ -1,2 +1,2 @@\n context\n{long_add}");
  let state = loaded(vec![thread("a.rs", Some(2), None, &hunk, &["see above"])], 1);

  let rows = rich_pr_rows(&sample_pr(), &state, W);
  // Identified by role, not by their first character: a filter that looks
  // for a sigil cannot see a row that LOST one, which is the whole bug.
  // The metadata block's `diff: +1198 −12` carries a label, and the
  // `… N more` tails start with an ellipsis.
  let hunk_rows: Vec<&String> = rows
    .iter()
    .filter(|r| r.label.is_empty() && r.role == DetailRole::Muted)
    .map(|r| &r.value)
    .filter(|v| v.starts_with("    ") && !v.trim_start().starts_with('…'))
    .collect();

  assert_eq!(
    hunk_rows.len(),
    3,
    "three hunk lines in, three rows out — a wrap emits more: {hunk_rows:?}"
  );
  for v in &hunk_rows {
    let sigil = v
      .strip_prefix("    ")
      .and_then(|rest| rest.chars().next())
      .expect("a hunk row is indented and non-empty");
    assert!(
      matches!(sigil, '+' | '-' | ' ' | '@'),
      "a hunk row lost its sigil and now reads as context: {v:?}"
    );
  }
  assert!(
    hunk_rows.iter().any(|v| v.chars().count() > W),
    "the long line is kept whole for the offset to scroll: {hunk_rows:?}"
  );
  assert!(
    !hunk_rows.iter().any(|v| v.contains('…')),
    "and nothing is thrown away before it can be scrolled to: {hunk_rows:?}"
  );
}

#[test]
fn a_long_hunk_keeps_its_tail_because_the_anchor_is_the_last_line() {
  let body: String = (1..=20).map(|i| format!(" context {i}\n")).collect();
  let hunk = format!("@@ -1,20 +1,21 @@\n{body}+the anchored line");
  let state = loaded(vec![thread("a.rs", Some(21), None, &hunk, &["here"])], 1);

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n");

  assert!(
    text.contains("+the anchored line"),
    "the last hunk line is the one the thread is about:\n{text}"
  );
  assert!(
    !text.contains("context 1\n"),
    "the head of a long hunk is the part to drop"
  );
}

#[test]
fn an_unsupported_forge_says_so_instead_of_reporting_none() {
  let state = GitHubFetchState::Loaded(ReviewThreads::Unsupported);

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n").to_lowercase();

  assert!(
    text.contains("not available") || text.contains("github only"),
    "a GitLab MR must not read as having no inline comments:\n{text}"
  );
  assert!(!text.contains("no inline comments"), "that is a claim gwm cannot make");
}

#[test]
fn zero_threads_reads_as_zero_not_as_a_missing_section() {
  let state = loaded(vec![], 0);

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n").to_lowercase();

  assert!(text.contains("no inline comments"), "a clean PR says so:\n{text}");
}

#[test]
fn an_inflight_fetch_and_a_failed_one_are_both_visible() {
  let loading = values(&rich_pr_rows(&sample_pr(), &GitHubFetchState::Loading, W)).join("\n");
  assert!(loading.to_lowercase().contains("loading"), "in:\n{loading}");

  let failed = values(&rich_pr_rows(
    &sample_pr(),
    &GitHubFetchState::Error("gh: HTTP 403".into()),
    W,
  ))
  .join("\n");
  assert!(failed.contains("gh: HTTP 403"), "in:\n{failed}");
}

#[test]
fn an_idle_fetch_renders_no_threads_section_at_all() {
  let text = values(&rich_pr_rows(&sample_pr(), &NO_THREADS, W))
    .join("\n")
    .to_lowercase();

  assert!(!text.contains("inline comments"), "nobody asked yet:\n{text}");
}

#[test]
fn a_capped_thread_list_states_what_it_dropped() {
  // `total` is the forge's own count, so the number is true rather than
  // "more".
  let threads: Vec<ReviewThread> = (1..=3)
    .map(|i| thread(&format!("f{i}.rs"), Some(i), None, "@@ -1 +1 @@\n+x", &["c"]))
    .collect();
  let state = loaded(threads, 9);

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n");

  assert!(text.contains("6 more"), "9 reported, 3 rendered:\n{text}");
}

#[test]
fn a_capped_comment_chain_states_what_it_dropped() {
  let mut t = thread("a.rs", Some(1), None, "@@ -1 +1 @@\n+x", &["only one kept"]);
  t.total_comments = 4;
  let state = loaded(vec![t], 1);

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n");

  assert!(text.contains("3 more"), "4 reported, 1 rendered:\n{text}");
}

#[test]
fn a_resolved_or_outdated_thread_is_labelled() {
  let mut resolved = thread("a.rs", Some(1), None, "@@ -1 +1 @@\n+x", &["done"]);
  resolved.is_resolved = true;
  let mut outdated = thread("b.rs", Some(2), None, "@@ -1 +1 @@\n+y", &["stale"]);
  outdated.is_outdated = true;
  let state = loaded(vec![resolved, outdated], 2);

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n").to_lowercase();

  assert!(text.contains("resolved"), "in:\n{text}");
  assert!(text.contains("outdated"), "in:\n{text}");
}

#[test]
fn every_thread_row_fits_the_budget() {
  let long = "S".repeat(400);
  let state = loaded(
    vec![thread(
      &format!("src/{long}.rs"),
      Some(999),
      Some(1),
      &format!("@@ -1 +1 @@\n+{long}"),
      &[&long],
    )],
    1,
  );

  // Preformatted rows excepted (issue #551): a diff hunk and a fenced code
  // line are kept whole and clipped by the renderer against the horizontal
  // offset, because reflowing them would change what they say.
  for row in rich_pr_rows(&sample_pr(), &state, W) {
    if row.preformatted {
      continue;
    }
    let width = row_width(&row);
    assert!(width <= W, "row overflows the modal: {width} > {W} — {row:?}");
  }
}

#[test]
fn a_thread_body_is_sanitised_like_every_other_remote_text() {
  // Same boundary as the description and the conversation: a body comes
  // from a remote forge and can carry a bidi override (#502).
  let state = loaded(
    vec![thread(
      "a.rs",
      Some(1),
      None,
      "@@ -1 +1 @@\n+x",
      &["safe\u{202e}reversed"],
    )],
    1,
  );

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n");

  assert!(!text.contains('\u{202e}'), "a bidi override reached the renderer");
}

#[test]
fn the_hunk_is_sanitised_too() {
  let state = loaded(
    vec![thread(
      "a.rs",
      Some(1),
      None,
      "@@ -1 +1 @@\n+safe\u{202e}reversed",
      &["c"],
    )],
    1,
  );

  let text = values(&rich_pr_rows(&sample_pr(), &state, W)).join("\n");

  assert!(!text.contains('\u{202e}'), "the hunk is remote text as well");
}

#[test]
fn a_thread_row_carries_the_comment_permalink() {
  let state = loaded(vec![thread("a.rs", Some(1), None, "@@ -1 +1 @@\n+x", &["c"])], 1);

  let rows = rich_pr_rows(&sample_pr(), &state, W);

  assert!(
    rows
      .iter()
      .any(|r| r.meta.as_deref() == Some("https://github.com/kbrdn1/gwm-cli/pull/514#discussion_r1")),
    "Enter has to open the thread the caps elided"
  );
}

#[test]
fn an_issue_view_has_no_threads_section() {
  // Issues have no diff, so the section would be meaningless — and
  // `rich_issue_rows` keeps its two-argument shape.
  let text = values(&rich_issue_rows(&sample_issue(), W)).join("\n").to_lowercase();

  assert!(!text.contains("inline comments"));
}

/// The roles used on the row labelled `label`.
fn roles_for(rows: &[gwm::tui::state::detail_overlay::DetailRow], label: &str) -> Vec<(String, Emphasis)> {
  rows
    .iter()
    .find(|r| r.label == label)
    .map(|r| r.segments.iter().map(|s| (s.text.clone(), s.emphasis)).collect())
    .unwrap_or_default()
}

#[test]
fn the_metadata_block_is_coloured_the_way_the_status_pane_colours_it() {
  // Issue #551. `state`, `checks` and `diff` all read at the same weight as
  // the URL, while the Status pane right behind the modal colours the same
  // facts. Same facts, same vocabulary: an open PR is `Success`, the way
  // `pr_badge_color` sends it to `theme.clean`.
  let rows = rich_pr_rows(&sample_pr(), &NO_THREADS, W);
  assert_eq!(roles_for(&rows, "state"), vec![("open".to_string(), Emphasis::Success)]);
  assert_eq!(
    roles_for(&rows, "checks"),
    vec![("passing 7/7".to_string(), Emphasis::Success)]
  );
  // The one row that carries two outcomes at once, which is why a role per
  // ROW could never have expressed it.
  assert_eq!(
    roles_for(&rows, "diff"),
    vec![
      ("+1198".to_string(), Emphasis::Success),
      (" ".to_string(), Emphasis::Plain),
      ("−12".to_string(), Emphasis::Failure),
    ]
  );
}

#[test]
fn every_pr_state_takes_the_status_panes_own_colour() {
  // The mapping has to agree with `pr_badge_color`, or the same PR reads as
  // one thing in the pane and another in the overlay one keypress away.
  for (state, expected) in [
    (PrState::Open, Emphasis::Success),
    (PrState::Draft, Emphasis::Muted),
    (PrState::Merged, Emphasis::Notice),
    (PrState::Closed, Emphasis::Failure),
  ] {
    let mut pr = sample_pr();
    pr.state = state;
    let rows = rich_pr_rows(&pr, &NO_THREADS, W);
    assert_eq!(
      roles_for(&rows, "state").first().map(|(_, e)| *e),
      Some(expected),
      "{state:?} must carry the colour the Status pane gives it"
    );
  }
}

#[test]
fn a_closed_issue_is_not_painted_like_a_closed_pr() {
  // `issue_badge_color` sends a closed issue to `locked`, not to `prunable`:
  // a closed issue is resolved, a closed PR is abandoned.
  let mut issue = sample_issue();
  issue.state = gwm::github::IssueState::Closed;
  assert_eq!(
    roles_for(&rich_issue_rows(&issue, W), "state").first().map(|(_, e)| *e),
    Some(Emphasis::Notice)
  );
}

#[test]
fn a_preformatted_row_may_outrun_the_budget_and_says_that_it_is_one() {
  // Issue #551, and the one exception to `no_row_overflows_the_width_it_was
  // _built_for` above. A fenced line is kept whole rather than reflowed,
  // because in code the column is the meaning — the same call `hunk_rows`
  // already made for a diff hunk. The row is flagged so the renderer knows
  // to clip it against the horizontal offset instead of assuming it fits.
  let mut pr = sample_pr();
  let long = "x".repeat(200);
  pr.detail.body = format!("prose\n\n```\n{long}\n```");

  let rows = rich_pr_rows(&pr, &NO_THREADS, W);
  let wide: Vec<_> = rows.iter().filter(|r| r.value.chars().count() > W).collect();

  assert_eq!(wide.len(), 1, "exactly the fenced line: {wide:?}");
  assert!(wide[0].preformatted, "and it is flagged as preformatted");
  assert!(
    rows.iter().filter(|r| !r.preformatted).all(|r| row_width(r) <= W),
    "every other row still fits: {:?}",
    rows
      .iter()
      .filter(|r| !r.preformatted && row_width(r) > W)
      .collect::<Vec<_>>()
  );
}

#[test]
fn a_diff_hunk_row_is_preformatted_too() {
  // Same reason, and it predates the flag: `hunk_rows` truncates rather than
  // wraps because a wrapped `+` line's continuation reads as context.
  let state = loaded(
    vec![thread(
      "src/tui/ui.rs",
      Some(7),
      Some(7),
      &format!("@@ -1 +1 @@\n+{}", "z".repeat(200)),
      &["looks long"],
    )],
    1,
  );
  let rows = rich_pr_rows(&sample_pr(), &state, W);
  assert!(
    rows.iter().any(|r| r.preformatted && r.value.contains('z')),
    "the hunk line must be flagged: {:?}",
    rows.iter().map(|r| &r.value).collect::<Vec<_>>()
  );
}
