//! Row builders for the rich PR / issue view (issue #420).
//!
//! The overlay shell renders one *line* per [`DetailRow`] and truncates a
//! long `value` with an ellipsis — there is no wrapping in the render path
//! and adding one would change how the agent and CI consumers lay out. So
//! the wrapping happens here, in pure state: a body becomes N rows, and
//! every assertion below is a pure function call with no ratatui involved.

use gwm::forge::{ForgeComment, ForgeReview, IssueDetail, PrDetail, ReviewState};
use gwm::github::{CheckOutcome, CiState, IssueState, IssueStatus, PrCheck, PrState, PrStatus};
use gwm::tui::state::rich_view::{rich_issue_rows, rich_pr_rows, LABEL_W};

/// The inner width the list-mode overlay hands the builder on a typical
/// terminal: `overlay_modal_width(120) - 6`.
const W: usize = 68;

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

/// `label` + two padding columns + `value`, the shell's own layout.
fn row_width(r: &gwm::tui::state::detail_overlay::DetailRow) -> usize {
  LABEL_W + 2 + r.value.chars().count()
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
  let rows = [rich_pr_rows(&sample_pr(), W), rich_issue_rows(&sample_issue(), W)].concat();
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
  let rows = rich_pr_rows(&sample_pr(), W);

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
  let rows = rich_pr_rows(&sample_pr(), W);
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

  let rows = rich_pr_rows(&pr, W);

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

  let rows = rich_pr_rows(&pr, W);

  for r in &rows {
    assert!(row_width(r) <= W, "unbreakable word must be hard split");
  }
}

#[test]
fn a_long_body_is_capped_and_says_so() {
  let mut pr = sample_pr();
  pr.detail.body = (0..500).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");

  let rows = rich_pr_rows(&pr, W);
  let vals = values(&rows);

  assert!(
    vals.iter().any(|v| v.contains("more lines")),
    "a truncated body must say how much was dropped, not stop silently: {vals:?}"
  );
  assert!(
    vals.iter().filter(|v| v.starts_with("line ")).count() <= 40,
    "the body cap holds"
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

  let rows = rich_pr_rows(&pr, W);

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
  let rows = rich_pr_rows(&sample_pr(), W);
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
  let rows = rich_pr_rows(&sample_pr(), W);

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

  let rows = rich_pr_rows(&pr, W);
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
  assert_eq!(value_for(&rich_pr_rows(&pr, W), "state").as_deref(), Some("draft"));
}

#[test]
fn a_zero_width_budget_does_not_panic_or_loop() {
  // `overlay_modal_width` clamps at 48, so this cannot happen through the
  // TUI — but a wrap loop that never advances hangs the whole render
  // thread, and that is not a failure mode worth leaving reachable.
  let rows = rich_pr_rows(&sample_pr(), 0);
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

  let rows = rich_pr_rows(&pr, W);
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

  let rows = rich_pr_rows(&pr, W);
  let body: Vec<&String> = rows.iter().map(|r| &r.value).filter(|v| v.contains("alpha")).collect();

  assert!(body.len() > 1, "precondition: the line had to wrap");
  for line in &body {
    assert!(line.starts_with("    "), "continuation lost the indent: {line:?}");
    assert!(row_width_of(line) <= W, "indent must be inside the budget: {line:?}");
  }
}

/// `row_width` for a bare value string.
fn row_width_of(v: &str) -> usize {
  LABEL_W + 2 + v.chars().count()
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

  let rows = rich_pr_rows(&pr, W);

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
