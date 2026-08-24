//! Rich PR / issue view (issue #420) — the third consumer of the generic
//! detail overlay, after the agent session pane (#408) and the CI checks
//! list (#436).
//!
//! **Why the wrapping lives here.** The overlay shell renders exactly one
//! terminal line per [`DetailRow`] and ellipsises a `value` that does not
//! fit; it has no notion of a paragraph. A PR description, a review body
//! and a comment are all multi-line prose, so they are wrapped into N rows
//! *before* they reach the renderer. That keeps the render path pure and
//! keeps the agent / CI consumers laid out exactly as they were.
//!
//! **Why the width is a parameter.** The wrap budget is the modal's inner
//! width, which only the renderer knows. The `App` carries the terminal
//! width it was last drawn at and rebuilds the rows on resize, so the
//! builder itself stays pure.
//!
//! **The scroll window is the budget** (issue #551). Bodies and the comment
//! list used to be capped, each cut marked with an explicit `… N more` row.
//! That was honest, but it rationed something the overlay does not have to
//! ration: the view scrolls, so what the reader sees at once is bounded by
//! the terminal, not by the row count, and a longer list costs only the rows
//! themselves. A description cut at `… 85 more lines` was the loudest
//! complaint against this view. What stays capped is the diff hunk, and for
//! a different reason: see [`hunk_rows`].
//!
//! **The inline review threads are a second transport.** The comments
//! anchored to a diff hunk are reachable through GraphQL only
//! (`--json comments` returns the conversation), so they arrive on their
//! own request, with their own latency and their own failure mode, and
//! [`rich_pr_rows`] takes their fetch state rather than a list (issue
//! #528). Their diff hunks are **truncated, never wrapped**: see
//! [`hunk_rows`].

use super::detail_overlay::{DetailRole, DetailRow};
use super::github_fetch::GitHubFetchState;
use super::markdown::{self, Emphasis, Segment};
use crate::forge::{
  ForgeComment, ForgeReview, IssueStatus, PrState, PrStatus, ReviewState, ReviewThread, ReviewThreads,
};
use crate::naming::{sanitise_block_for_terminal, sanitise_for_terminal};
use crate::tui::ui::{CI_FAILING_ICON, CI_PASSING_ICON, CI_RUNNING_ICON, ISSUE_ICON, PR_ICON};

/// Width the metadata block's label column is expected to need. The wrap
/// budget no longer subtracts it (see [`wrap_budget`]), but the METADATA
/// rows do carry labels, and the shell sizes its column from the widest one
/// it is handed: a longer label added later would push those rows past the
/// modal's inner width. Pinned by
/// `tests/tui_rich_view_tests.rs::every_emitted_label_fits_the_reserved_column`.
pub const LABEL_W: usize = 7;

/// Diff-hunk lines kept, counted **from the end**: the forge puts the
/// anchored line last, so a long hunk drops its head, never its tail.
const HUNK_MAX_LINES: usize = 6;

/// Rows for a pull / merge request. `width` is the modal's inner width in
/// columns, label column included.
///
/// `threads` is the inline-review-comment fetch (issue #528), which is a
/// **separate** transport from the PR itself and therefore has its own
/// state: it is still in flight when this view first opens, and on a
/// backend that cannot answer it never resolves to a list at all.
pub fn rich_pr_rows(
  pr: &PrStatus,
  threads: &GitHubFetchState<ReviewThreads>,
  width: usize,
  noun: &str,
) -> Vec<DetailRow> {
  let mut rows = Vec::new();
  let d = &pr.detail;

  // One line saying identity, state and CI, the way the Status pane behind
  // the modal says them (validation feedback on issue #551). It replaces
  // the `state` and `checks` rows rather than sitting above them.
  let role = pr_state_role(pr.state);
  let mut identity = vec![
    Segment::new(format!("{PR_ICON} "), role),
    // `MR` for GitLab, following the resolved forge the way the overlay
    // title does (issue #419). The caller owns the noun; this only shortens
    // it, since a badge line has no room for `Merge request #519`.
    Segment::new(format!("{} #{}", short_noun(noun), pr.number), Emphasis::Plain),
    Segment::new(" ", Emphasis::Plain),
    Segment::chip(format!(" {} ", pr_state_label(pr.state)), role),
  ];
  if let Some((label, ci_role, icon)) = ci_summary(pr) {
    identity.push(Segment::new(" ", Emphasis::Plain));
    identity.push(Segment::chip(
      format!(" {icon} CI {label} {}/{} ", pr.checks_passed, pr.checks_total),
      ci_role,
    ));
  }
  meta_segments(&mut rows, "", identity);
  if !d.author.is_empty() {
    meta(&mut rows, "author", &sanitise_for_terminal(&d.author));
  }
  // Both refs or neither: "→ dev" alone tells the user nothing about what
  // is being merged, and GitLab is the backend that serves one without the
  // other only when the payload is malformed.
  if !d.head_ref.is_empty() && !d.base_ref.is_empty() {
    let pair = format!(
      "{} → {}",
      sanitise_for_terminal(&d.head_ref),
      sanitise_for_terminal(&d.base_ref)
    );
    meta(&mut rows, "branch", &pair);
  }
  // A zero diff is a measurement gwm does not have (the GitLab backend
  // never fills it), not a PR that changes nothing — so it is omitted
  // rather than rendered as a truthful-looking `+0 −0`.
  if d.additions > 0 || d.deletions > 0 {
    meta_segments(
      &mut rows,
      "diff",
      vec![
        Segment::new(format!("+{}", d.additions), Emphasis::Success),
        Segment::new(" ", Emphasis::Plain),
        Segment::new(format!("−{}", d.deletions), Emphasis::Failure),
      ],
    );
  }
  if !pr.updated_at.is_empty() {
    meta(&mut rows, "updated", day(&pr.updated_at));
  }
  url_row(&mut rows, &pr.url);

  let budget = wrap_budget(width);
  body_section(&mut rows, "description", &d.body, budget);
  reviews_section(&mut rows, &d.reviews, budget);
  threads_section(&mut rows, threads, budget);
  comments_section(&mut rows, &d.comments, budget);
  rows
}

/// Rows for an issue: the same shell without the PR-only blocks (no diff,
/// no checks, no reviews), plus the label list.
pub fn rich_issue_rows(issue: &IssueStatus, width: usize) -> Vec<DetailRow> {
  let mut rows = Vec::new();
  let d = &issue.detail;

  // Following `ui::issue_badge_color`: a closed issue is `locked`, not
  // `prunable`. It is resolved, where a closed PR is abandoned.
  let (label, role) = match issue.state {
    crate::forge::IssueState::Open => ("open", Emphasis::Success),
    crate::forge::IssueState::Closed => ("closed", Emphasis::Notice),
  };
  meta_segments(
    &mut rows,
    "",
    vec![
      Segment::new(format!("{ISSUE_ICON} "), role),
      Segment::new(format!("Issue #{}", issue.number), Emphasis::Plain),
      Segment::new(" ", Emphasis::Plain),
      Segment::chip(format!(" {label} "), role),
    ],
  );
  if !d.author.is_empty() {
    meta(&mut rows, "author", &sanitise_for_terminal(&d.author));
  }
  if !issue.labels.is_empty() {
    let labels = issue
      .labels
      .iter()
      .map(|l| sanitise_for_terminal(l))
      .collect::<Vec<_>>()
      .join(", ");
    meta(&mut rows, "labels", &labels);
  }
  if !issue.updated_at.is_empty() {
    meta(&mut rows, "updated", day(&issue.updated_at));
  }
  url_row(&mut rows, &issue.url);

  let budget = wrap_budget(width);
  body_section(&mut rows, "description", &d.body, budget);
  comments_section(&mut rows, &d.comments, budget);
  rows
}

/// Columns available to a wrapped body line: the modal's whole inner width.
///
/// Every row this module wraps is label-less, and the shell no longer
/// indents a label-less row behind the label column (issue #551, question 2
/// of the issue body). Reserving those columns here as well spent them
/// twice: the line was wrapped nine columns short AND painted nine columns
/// in. Never zero, a zero budget would make the wrap loop unable to
/// advance.
fn wrap_budget(width: usize) -> usize {
  width.max(8)
}

fn pr_state_label(s: PrState) -> &'static str {
  match s {
    PrState::Open => "open",
    PrState::Draft => "draft",
    PrState::Closed => "closed",
    PrState::Merged => "merged",
  }
}

/// The date part of an RFC 3339 timestamp. Cheap on purpose: the exact
/// second is noise in a metadata block, and a full relative-time formatter
/// would need a `now` this builder has no reason to take.
fn day(ts: &str) -> &str {
  ts.split('T').next().unwrap_or(ts)
}

fn meta(rows: &mut Vec<DetailRow>, label: &str, value: &str) {
  meta_segments(rows, label, vec![Segment::new(value, Emphasis::Plain)]);
}

/// A metadata row whose value carries its own colours (issue #551).
///
/// The block used to read at one weight throughout, while the Status pane
/// right behind the modal colours the same facts. The roles here are the
/// pane's: `Success` is where `pr_badge_color` sends an open PR, `Notice`
/// where it sends a merged one.
fn meta_segments(rows: &mut Vec<DetailRow>, label: &str, segments: Vec<Segment>) {
  rows.push(DetailRow {
    label: label.into(),
    value: segments.iter().map(|s| s.text.as_str()).collect(),
    role: DetailRole::Normal,
    segments,
    ..Default::default()
  });
}

/// `PR` / `MR`: the identity line is a badge row, with no room for
/// `Merge request #519`. Derived from the noun the caller resolved rather
/// than from a second forge lookup, so the two cannot disagree.
fn short_noun(noun: &str) -> &'static str {
  if noun.to_ascii_lowercase().starts_with("merge") {
    "MR"
  } else {
    "PR"
  }
}

/// The CI half of the identity row, or `None` when there is nothing
/// measured to say. Mirrors `ui::ci_indicator`, which cannot be called from
/// here: it resolves theme colours, and this module stays ratatui-free.
fn ci_summary(pr: &PrStatus) -> Option<(&'static str, Emphasis, &'static str)> {
  if pr.checks_total == 0 {
    return None;
  }
  match pr.ci {
    crate::forge::CiState::Passing => Some(("passing", Emphasis::Success, CI_PASSING_ICON)),
    crate::forge::CiState::Failing => Some(("failing", Emphasis::Failure, CI_FAILING_ICON)),
    crate::forge::CiState::Running => Some(("running", Emphasis::Running, CI_RUNNING_ICON)),
    // `checks_total > 0` with no state is a payload gwm cannot read, not a
    // PR without checks, so it says nothing rather than saying "none".
    crate::forge::CiState::None => None,
  }
}

/// The colour a PR state takes, following `ui::pr_badge_color`.
fn pr_state_role(s: PrState) -> Emphasis {
  match s {
    PrState::Open => Emphasis::Success,
    PrState::Draft => Emphasis::Muted,
    PrState::Merged => Emphasis::Notice,
    PrState::Closed => Emphasis::Failure,
  }
}

/// The one actionable row of the metadata block: Enter opens it.
fn url_row(rows: &mut Vec<DetailRow>, url: &str) {
  if url.is_empty() {
    return;
  }
  let clean = sanitise_for_terminal(url);
  rows.push(DetailRow {
    label: "url".into(),
    value: clean.clone(),
    role: DetailRole::Normal,
    segments: vec![Segment::new(clean.clone(), Emphasis::Link)],
    meta: Some(clean),
    ..Default::default()
  });
}

fn blank(rows: &mut Vec<DetailRow>) {
  rows.push(DetailRow {
    label: String::new(),
    value: String::new(),
    role: DetailRole::Muted,
    meta: None,
    extra: None,
    ..Default::default()
  });
}

/// A section heading, rendered in the label-less column so the metadata
/// block above keeps its alignment.
fn heading(rows: &mut Vec<DetailRow>, text: String) {
  blank(rows);
  rows.push(DetailRow {
    label: String::new(),
    value: text,
    role: DetailRole::Active,
    meta: None,
    extra: None,
    ..Default::default()
  });
}

/// Wrapped body lines under `title`, or nothing at all when the body is
/// empty — an empty section header is worse than no section.
fn body_section(rows: &mut Vec<DetailRow>, title: &str, body: &str, budget: usize) {
  if body.trim().is_empty() {
    return;
  }
  heading(rows, title.to_string());
  push_body(rows, body, budget, "");
}

fn reviews_section(rows: &mut Vec<DetailRow>, reviews: &[ForgeReview], budget: usize) {
  if reviews.is_empty() {
    return;
  }
  heading(rows, format!("reviews ({})", reviews.len()));
  for r in reviews {
    let role = match r.state {
      ReviewState::Approved => DetailRole::Success,
      ReviewState::ChangesRequested => DetailRole::Failure,
      ReviewState::Pending => DetailRole::Running,
      _ => DetailRole::Muted,
    };
    let segments = vec![
      Segment::chip(format!(" {} ", r.state.label()), verdict_role(r.state)),
      Segment::new(
        truncate(
          &format!(" {} · {}", sanitise_for_terminal(&r.author), day(&r.submitted_at)),
          budget,
        ),
        Emphasis::Plain,
      ),
    ];
    rows.push(DetailRow {
      label: String::new(),
      value: segments.iter().map(|s| s.text.as_str()).collect(),
      role,
      segments,
      ..Default::default()
    });
    // A bare approval carries no body, which is the common case.
    push_body(rows, &r.body, budget.saturating_sub(2).max(8), "  ");
  }
}

fn comments_section(rows: &mut Vec<DetailRow>, comments: &[ForgeComment], budget: usize) {
  if comments.is_empty() {
    return;
  }
  heading(rows, format!("comments ({})", comments.len()));
  for c in comments {
    rows.push(author_header("", &c.author, &c.created_at, budget, c.url.as_deref()));
    push_body(rows, &c.body, budget.saturating_sub(2).max(8), "  ");
  }
}

/// Inline review threads (issue #528) — the comments anchored to a diff
/// hunk, as opposed to [`comments_section`]'s conversation.
///
/// Every state gets a row of its own. The section is the one place in this
/// view whose data comes from a second, slower request, so "still loading"
/// and "this backend has no path to these" are states the reader sees
/// rather than infers from an absence.
fn threads_section(rows: &mut Vec<DetailRow>, state: &GitHubFetchState<ReviewThreads>, budget: usize) {
  let loaded = match state {
    // Nobody asked for the fetch. No heading: a section that claims
    // nothing is better than one that claims zero.
    GitHubFetchState::Idle => return,
    GitHubFetchState::Loading => {
      heading(rows, "inline comments".into());
      more(rows, "  loading…".into());
      return;
    }
    GitHubFetchState::Error(e) => {
      heading(rows, "inline comments".into());
      rows.push(DetailRow {
        label: String::new(),
        value: truncate(&format!("  {}", sanitise_for_terminal(e)), budget),
        role: DetailRole::Failure,
        meta: None,
        extra: None,
        ..Default::default()
      });
      return;
    }
    GitHubFetchState::Loaded(l) => l,
  };

  let ReviewThreads::Threads { threads, total } = loaded else {
    heading(rows, "inline comments".into());
    // Deliberately not "none": gwm has not looked, and cannot here.
    more(rows, "  not available for this forge (GitHub only)".into());
    return;
  };

  if threads.is_empty() {
    heading(rows, "inline comments".into());
    more(rows, "  no inline comments".into());
    return;
  }

  heading(rows, format!("inline comments ({total})"));
  for t in threads {
    thread_rows(rows, t, budget);
  }
  // The fetch itself is paginated, so `total` can still exceed what arrived.
  // That marker survives the cap removal: it reports what gwm does not have,
  // not what it chose to leave out.
  let dropped = (*total as usize).saturating_sub(threads.len());
  if dropped > 0 {
    more(rows, format!("… {dropped} more threads not fetched"));
  }
}

/// One thread: its anchor, the hunk it hangs from, then the chain.
fn thread_rows(rows: &mut Vec<DetailRow>, t: &ReviewThread, budget: usize) {
  let path = sanitise_for_terminal(&t.path);
  // `start_line == line` is what a single-line anchor looks like when the
  // forge fills both, and `7-7` is noise.
  let anchor = match (t.start_line, t.line) {
    (Some(s), Some(l)) if s != l => format!("{path}:{s}-{l}"),
    (_, Some(l)) => format!("{path}:{l}"),
    (Some(s), None) => format!("{path}:{s}"),
    // An outdated thread can lose its line entirely; the file still
    // locates it.
    (None, None) => path,
  };
  let mut header = format!("  {anchor}");
  if t.is_resolved {
    header.push_str(" · resolved");
  }
  if t.is_outdated {
    header.push_str(" · outdated");
  }
  rows.push(DetailRow {
    label: String::new(),
    value: truncate(&header, budget),
    role: if t.is_resolved || t.is_outdated {
      DetailRole::Muted
    } else {
      DetailRole::Normal
    },
    meta: None,
    extra: None,
    ..Default::default()
  });

  hunk_rows(rows, &t.diff_hunk);

  for c in &t.comments {
    rows.push(author_header(
      "    ",
      &c.author,
      &c.created_at,
      budget,
      c.url.as_deref(),
    ));
    push_body(rows, &c.body, budget.saturating_sub(2).max(8), "      ");
  }
  // Same distinction as the thread list: what the fetch did not return.
  let dropped = (t.total_comments as usize).saturating_sub(t.comments.len());
  if dropped > 0 {
    more(rows, format!("    … {dropped} more comments not fetched"));
  }
}

/// The diff hunk, one row per line.
///
/// **Truncated, never wrapped.** [`wrap_line`] splits on whitespace, so a
/// wrapped `+` line's continuation rows carry no sigil and read as
/// context — in a diff the leading `+` / `-` / space *is* the meaning, and
/// a line that silently changes side is worse than one that is visibly
/// cut. Prose can afford the reflow; this cannot.
fn hunk_rows(rows: &mut Vec<DetailRow>, hunk: &str) {
  if hunk.trim().is_empty() {
    return;
  }
  // Remote text, same boundary as every body here (#502).
  let clean = sanitise_block_for_terminal(hunk).replace('\t', "    ");
  let lines: Vec<&str> = clean.lines().collect();
  let start = lines.len().saturating_sub(HUNK_MAX_LINES);
  if start > 0 {
    more(rows, format!("    … {start} earlier hunk lines"));
  }
  for line in &lines[start..] {
    // Whole, not truncated (issue #551): the row is flagged preformatted and
    // the horizontal offset reaches its tail. Truncating here threw the tail
    // away before anything could scroll to it.
    let text = format!("    {line}");
    rows.push(DetailRow {
      label: String::new(),
      value: text.clone(),
      role: DetailRole::Muted,
      preformatted: true,
      segments: vec![Segment::new(text, Emphasis::Code)],
      ..Default::default()
    });
  }
}

/// Render `body` as Markdown and push it whole (issue #551). `indent`
/// prefixes every line (review and comment bodies sit one step in from their
/// header).
///
/// The rows carry both forms: `value` is what the reader sees as one plain
/// string, which is what measuring, filtering and the row tests work
/// against, and `segments` is the same text split into styled runs for the
/// renderer.
fn push_body(rows: &mut Vec<DetailRow>, body: &str, budget: usize, indent: &str) {
  if body.trim().is_empty() {
    return;
  }
  for line in markdown::render(body, budget.saturating_sub(indent.len()).max(8)) {
    let mut segments = line.segments;
    if !indent.is_empty() {
      segments.insert(0, Segment::new(indent.to_string(), Emphasis::Plain));
    }
    rows.push(DetailRow {
      label: String::new(),
      value: segments.iter().map(|s| s.text.as_str()).collect(),
      role: DetailRole::Normal,
      preformatted: line.preformatted,
      segments,
      ..Default::default()
    });
  }
}

/// A comment header: the author as a badge, the date as plain text
/// (validation feedback on issue #551). One helper for the conversation and
/// for the inline threads, so the two sections cannot read as different
/// kinds of content.
fn author_header(indent: &str, author: &str, created_at: &str, budget: usize, url: Option<&str>) -> DetailRow {
  let mut segments = Vec::new();
  if !indent.is_empty() {
    segments.push(Segment::new(indent.to_string(), Emphasis::Plain));
  }
  // `name` rather than an outcome colour: an author is an identity, not a
  // verdict. Same role the directory badge in the header takes.
  segments.push(Segment::chip(
    format!(" {} ", sanitise_for_terminal(author)),
    Emphasis::Plain,
  ));
  segments.push(Segment::new(
    truncate(&format!(" {}", day(created_at)), budget),
    Emphasis::Muted,
  ));
  DetailRow {
    label: String::new(),
    value: segments.iter().map(|s| s.text.as_str()).collect(),
    role: DetailRole::Normal,
    segments,
    // The permalink, so Enter opens the thread on the forge.
    meta: url.map(sanitise_for_terminal),
    ..Default::default()
  }
}

/// The colour a review verdict takes. `Pending` is in flight, `Commented`
/// and `Dismissed` carry no verdict, and `Unknown` is a state this build
/// did not read (see `ReviewState::Unknown`) — none of them is an outcome,
/// so none of them gets an outcome colour.
fn verdict_role(s: ReviewState) -> Emphasis {
  match s {
    ReviewState::Approved => Emphasis::Success,
    ReviewState::ChangesRequested => Emphasis::Failure,
    ReviewState::Pending => Emphasis::Running,
    ReviewState::Commented | ReviewState::Dismissed | ReviewState::Unknown => Emphasis::Muted,
  }
}

fn more(rows: &mut Vec<DetailRow>, text: String) {
  rows.push(DetailRow {
    label: String::new(),
    value: text,
    role: DetailRole::Muted,
    meta: None,
    extra: None,
    ..Default::default()
  });
}

/// Single-line ellipsis for the header rows, which are built from short
/// fields and only overflow on a very narrow modal.
fn truncate(s: &str, budget: usize) -> String {
  if s.chars().count() <= budget {
    return s.to_string();
  }
  let mut out: String = s.chars().take(budget.saturating_sub(1)).collect();
  out.push('…');
  out
}
