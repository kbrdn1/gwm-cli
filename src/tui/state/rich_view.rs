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
//! **What is capped, and it says so.** A comment thread on a busy PR is
//! unbounded, and a bot review body regularly runs to hundreds of lines.
//! Each body is capped, the comment list is capped, and every cut emits an
//! explicit `… N more` row — a silently truncated view reads as a complete
//! one.
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
use crate::forge::{
  ForgeComment, ForgeReview, IssueStatus, PrState, PrStatus, ReviewState, ReviewThread, ReviewThreads,
};
use crate::naming::{sanitise_block_for_terminal, sanitise_for_terminal};

/// Width reserved for the label column. Every label this module emits fits
/// in it, so the wrap budget is knowable before the rows exist — the shell
/// derives its own `label_w` from the widest label it is handed, which
/// would otherwise be circular.
pub const LABEL_W: usize = 7;

/// Wrapped lines kept from one body (description or review). Enough for a
/// filled-in PR description; past that the browser is the better tool.
const BODY_MAX_LINES: usize = 40;

/// Wrapped lines kept from one comment. Bot comments (CodeRabbit,
/// Copilot) routinely run past this; the header row keeps its permalink so
/// Enter opens the full thread.
const COMMENT_MAX_LINES: usize = 12;

/// Comments rendered before the list itself is cut.
const MAX_COMMENTS: usize = 20;

/// Inline review threads rendered before the list is cut (issue #528).
const MAX_THREADS: usize = 10;

/// Comments rendered per thread. A thread is a discussion; past this the
/// permalink on its header row is the better way in.
const MAX_THREAD_COMMENTS: usize = 5;

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
pub fn rich_pr_rows(pr: &PrStatus, threads: &GitHubFetchState<ReviewThreads>, width: usize) -> Vec<DetailRow> {
  let mut rows = Vec::new();
  let d = &pr.detail;

  meta(&mut rows, "state", pr_state_label(pr.state));
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
    meta(&mut rows, "diff", &format!("+{} −{}", d.additions, d.deletions));
  }
  if pr.checks_total > 0 {
    let label = match pr.ci {
      crate::forge::CiState::Passing => "passing",
      crate::forge::CiState::Failing => "failing",
      crate::forge::CiState::Running => "running",
      crate::forge::CiState::None => "no checks",
    };
    meta(
      &mut rows,
      "checks",
      &format!("{label} {}/{}", pr.checks_passed, pr.checks_total),
    );
  }
  if !pr.updated_at.is_empty() {
    meta(&mut rows, "updated", day(&pr.updated_at));
  }
  url_row(&mut rows, &pr.url);

  let budget = wrap_budget(width);
  body_section(&mut rows, "description", &d.body, budget, BODY_MAX_LINES);
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

  meta(
    &mut rows,
    "state",
    match issue.state {
      crate::forge::IssueState::Open => "open",
      crate::forge::IssueState::Closed => "closed",
    },
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
  body_section(&mut rows, "description", &d.body, budget, BODY_MAX_LINES);
  comments_section(&mut rows, &d.comments, budget);
  rows
}

/// Columns available to a wrapped body line: the modal's inner width minus
/// the label column and its two padding columns. Never zero — a zero
/// budget would make the wrap loop unable to advance.
fn wrap_budget(width: usize) -> usize {
  width.saturating_sub(LABEL_W + 2).max(8)
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
  rows.push(DetailRow {
    label: label.into(),
    value: value.into(),
    role: DetailRole::Normal,
    meta: None,
    extra: None,
  });
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
    meta: Some(clean),
    extra: None,
  });
}

fn blank(rows: &mut Vec<DetailRow>) {
  rows.push(DetailRow {
    label: String::new(),
    value: String::new(),
    role: DetailRole::Muted,
    meta: None,
    extra: None,
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
  });
}

/// Wrapped body lines under `title`, or nothing at all when the body is
/// empty — an empty section header is worse than no section.
fn body_section(rows: &mut Vec<DetailRow>, title: &str, body: &str, budget: usize, cap: usize) {
  if body.trim().is_empty() {
    return;
  }
  heading(rows, title.to_string());
  push_body(rows, body, budget, cap, "");
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
    rows.push(DetailRow {
      label: String::new(),
      value: truncate(
        &format!(
          "{} · {} · {}",
          r.state.label(),
          sanitise_for_terminal(&r.author),
          day(&r.submitted_at)
        ),
        budget,
      ),
      role,
      meta: None,
      extra: None,
    });
    // A bare approval carries no body, which is the common case.
    push_body(rows, &r.body, budget.saturating_sub(2).max(8), BODY_MAX_LINES, "  ");
  }
}

fn comments_section(rows: &mut Vec<DetailRow>, comments: &[ForgeComment], budget: usize) {
  if comments.is_empty() {
    return;
  }
  heading(rows, format!("comments ({})", comments.len()));
  for c in comments.iter().take(MAX_COMMENTS) {
    rows.push(DetailRow {
      label: String::new(),
      value: truncate(
        &format!("{} · {}", sanitise_for_terminal(&c.author), day(&c.created_at)),
        budget,
      ),
      role: DetailRole::Normal,
      // The permalink, so Enter opens the full thread the cap elided.
      meta: c.url.clone().map(|u| sanitise_for_terminal(&u)),
      extra: None,
    });
    push_body(rows, &c.body, budget.saturating_sub(2).max(8), COMMENT_MAX_LINES, "  ");
  }
  if let Some(dropped) = comments.len().checked_sub(MAX_COMMENTS).filter(|n| *n > 0) {
    more(rows, format!("… {dropped} more comments"));
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
  for t in threads.iter().take(MAX_THREADS) {
    thread_rows(rows, t, budget);
  }
  let dropped = (*total as usize).saturating_sub(threads.len().min(MAX_THREADS));
  if dropped > 0 {
    more(rows, format!("… {dropped} more threads"));
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
  });

  hunk_rows(rows, &t.diff_hunk, budget);

  for c in t.comments.iter().take(MAX_THREAD_COMMENTS) {
    rows.push(DetailRow {
      label: String::new(),
      value: truncate(
        &format!("    {} · {}", sanitise_for_terminal(&c.author), day(&c.created_at)),
        budget,
      ),
      role: DetailRole::Normal,
      // The permalink to this very comment, so Enter opens the thread the
      // caps elided.
      meta: c.url.clone().map(|u| sanitise_for_terminal(&u)),
      extra: None,
    });
    push_body(
      rows,
      &c.body,
      budget.saturating_sub(2).max(8),
      COMMENT_MAX_LINES,
      "      ",
    );
  }
  let shown = t.comments.len().min(MAX_THREAD_COMMENTS);
  let dropped = (t.total_comments as usize).saturating_sub(shown);
  if dropped > 0 {
    more(rows, format!("    … {dropped} more comments"));
  }
}

/// The diff hunk, one row per line.
///
/// **Truncated, never wrapped.** [`wrap_line`] splits on whitespace, so a
/// wrapped `+` line's continuation rows carry no sigil and read as
/// context — in a diff the leading `+` / `-` / space *is* the meaning, and
/// a line that silently changes side is worse than one that is visibly
/// cut. Prose can afford the reflow; this cannot.
fn hunk_rows(rows: &mut Vec<DetailRow>, hunk: &str, budget: usize) {
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
    rows.push(DetailRow {
      label: String::new(),
      value: truncate(&format!("    {line}"), budget),
      role: DetailRole::Muted,
      meta: None,
      extra: None,
    });
  }
}

/// Wrap `body` and push it, capped at `cap` lines with an explicit tail
/// row when the cap bites. `indent` prefixes every line (review and
/// comment bodies sit one step in from their header).
fn push_body(rows: &mut Vec<DetailRow>, body: &str, budget: usize, cap: usize, indent: &str) {
  if body.trim().is_empty() {
    return;
  }
  let lines = wrap_block(body, budget.saturating_sub(indent.len()).max(8));
  for line in lines.iter().take(cap) {
    rows.push(DetailRow {
      label: String::new(),
      value: format!("{indent}{line}"),
      role: DetailRole::Normal,
      meta: None,
      extra: None,
    });
  }
  if lines.len() > cap {
    more(rows, format!("{indent}… {} more lines", lines.len() - cap));
  }
}

fn more(rows: &mut Vec<DetailRow>, text: String) {
  rows.push(DetailRow {
    label: String::new(),
    value: text,
    role: DetailRole::Muted,
    meta: None,
    extra: None,
  });
}

/// Sanitise, then wrap a multi-line block to `budget` columns.
///
/// Sanitising first is deliberate: a body comes from a remote forge, so it
/// can carry a bidi override that reorders how the terminal paints the row
/// (issue #502) or a lone CR that repaints over the line already there.
/// Both are neutralised at this boundary, before any width is measured —
/// a `?` is one column, the character it replaced may not have been.
fn wrap_block(body: &str, budget: usize) -> Vec<String> {
  let clean = sanitise_block_for_terminal(body).replace('\t', "    ");
  let mut out = Vec::new();
  for line in clean.lines() {
    if line.trim().is_empty() {
      out.push(String::new());
      continue;
    }
    out.extend(wrap_line(line, budget));
  }
  out
}

/// Word-wrap one line, hard-splitting any single word wider than the
/// budget. A URL or a base64 blob has no break opportunity, and a
/// word-only wrap would emit an over-wide row the renderer then ellipsises
/// — losing exactly the tail the user was after.
///
/// **A line that already fits is returned untouched** (Codex review #529).
/// The word loop below runs on `split_whitespace`, which drops the leading
/// indent and collapses runs of spaces, and that is not cosmetic: a PR
/// description almost always carries a fenced block, and for YAML or
/// Python the indentation *is* the meaning. Since preformatted lines are
/// short by nature, passing short lines through verbatim preserves them
/// while prose still wraps. A preformatted line long enough to need
/// wrapping is re-spaced anyway, but its continuations are re-indented to
/// the original column so the block keeps its shape.
fn wrap_line(line: &str, budget: usize) -> Vec<String> {
  let budget = budget.max(1);
  if line.chars().count() <= budget {
    return vec![line.to_string()];
  }
  let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
  // An indent wider than the budget would leave no room to make progress.
  let indent = if indent.chars().count() + 8 <= budget {
    indent
  } else {
    String::new()
  };
  let mut out = Vec::new();
  let mut cur = String::new();
  let mut cur_cols = 0usize;
  let budget = budget - indent.chars().count();
  for word in line.split_whitespace() {
    let word_cols = word.chars().count();
    if word_cols > budget {
      if cur_cols > 0 {
        out.push(std::mem::take(&mut cur));
        cur_cols = 0;
      }
      let mut chunk = String::new();
      for c in word.chars() {
        chunk.push(c);
        if chunk.chars().count() == budget {
          out.push(std::mem::take(&mut chunk));
        }
      }
      if !chunk.is_empty() {
        cur = chunk;
        cur_cols = cur.chars().count();
      }
      continue;
    }
    let need = if cur_cols == 0 {
      word_cols
    } else {
      cur_cols + 1 + word_cols
    };
    if need > budget {
      out.push(std::mem::take(&mut cur));
      cur.push_str(word);
      cur_cols = word_cols;
    } else {
      if cur_cols > 0 {
        cur.push(' ');
      }
      cur.push_str(word);
      cur_cols = need;
    }
  }
  if !cur.is_empty() {
    out.push(cur);
  }
  if out.is_empty() {
    out.push(String::new());
  }
  if indent.is_empty() {
    out
  } else {
    out.into_iter().map(|l| format!("{indent}{l}")).collect()
  }
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
