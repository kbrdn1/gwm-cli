//! The Commits view (issues #593, #635): its state, its rendering and its
//! key handling, in one module.
//!
//! The full-size commit listing is the sidebar's Commits pane given the
//! whole terminal, plus a way to ask for more history instead of leaving
//! gwm for lazygit. `ui.rs` still reaches in for
//! [`recent_commits_lines`], which is what the sidebar pane paints — the
//! sidebar is a view that has not been extracted yet, so that call reads
//! as an inversion today and will read as view-to-view once it has.
//!
//! ## State
//!
//! The rows are an owned snapshot taken when the overlay opens
//! ([`crate::tui::App::enter_commits`]) rather than a read of
//! `SidebarState::cache`: that cache is only rebuilt while the sidebar is
//! *open* and in `Commits` mode, so reading it would leave the overlay
//! blank in exactly the two states, sidebar hidden or `Stashes` selected,
//! where the listing is most useful.
//!
//! The read itself runs on a worker (`TaskKind::Commits`), not inline on
//! the keypress. The revwalk sorts `TIME | TOPOLOGICAL`, which walks the
//! whole reachable graph before it yields the first row: measured on this
//! repo (2058 commits), asking for 300 costs the same as asking for all of
//! them, so the limit truncates the output and bounds nothing about the
//! latency. On a large history an inline call would freeze the event loop
//! for the length of the walk. The overlay opens on a loader and fills in
//! when the worker lands, the shape #592 settled on for the same reason.
//!
//! Paging is a re-read at a larger limit rather than an append.
//! [`crate::worktree::recent_commits_cached`] is keyed by
//! `(repo, tip, limit)`, so each page is a fresh entry rather than an
//! invalidation, and the graph renderer needs the whole list anyway: a
//! connector on row N depends on the parents of rows below it.
//!
//! Scroll follows the help / command-logs contract: the cursor lives here,
//! but `max_scroll` is republished by the renderer each frame against the
//! real viewport, since only the renderer knows both the row count and the
//! inner modal height.

use super::super::app::{App, View};
use super::super::commit_graph;
use super::super::modal_keymap::{KeyContext, ModalAction, ModalKeymap};
use super::super::mouse::MouseMap;
use super::super::state::async_task::{TaskKind, TaskMsg};
use super::super::theme::Theme;
use super::super::ui::{
  author_initials, branch_name_color, centered, freshness_color, meta_pick, modal_hint_line, overlay_worktree,
  scrollable_body_area, MetaColumn, ModalFrame, COMMITS_SUBJECT_FLOOR, COMMIT_HASH_DISPLAY_LEN, META_GAP,
  RECENT_COMMITS_LIMIT,
};
use crate::worktree::{self, WorktreeInfo};
use crossterm::event::KeyEvent;
use ratatui::{
  layout::{Constraint, Layout},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::Paragraph,
  Frame,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One page of history. Matched to the sidebar's own limit so the first
/// snapshot hits the cache entry a sidebar in Commits mode already warmed
/// instead of paying a second revwalk.
pub const COMMITS_PAGE: usize = RECENT_COMMITS_LIMIT;

/// Ceiling on the paged limit, in commits.
///
/// Two reasons to have one at all. Each page is a whole graph walk, so a
/// deeper page is not cheaper than the one before it. And the memo in
/// [`crate::worktree::recent_commits_cached`] holds
/// `RECENT_COMMITS_CACHE_MAX_ENTRIES` (64) entries keyed on the limit,
/// evicting an arbitrary one when full: unbounded paging would push other
/// worktrees' sidebar entries out and make them re-walk. Five pages per
/// worktree keeps that well inside the budget.
pub const COMMITS_MAX: usize = COMMITS_PAGE * 5;

/// Everything one read of the log produces, as it travels from the worker
/// to the overlay.
///
/// One struct rather than a four-field message: the rows, their count and
/// the two metadata columns are read together and installed together, and
/// a caller that could install three of the four would be a bug waiting to
/// happen.
#[derive(Debug, Default)]
pub struct CommitsSnapshot {
  /// The graph rows, or a single sentinel row (empty history, load error).
  pub lines: Vec<Line<'static>>,
  /// Commits the rows describe. Zero for either sentinel.
  pub loaded: usize,
  /// The commits themselves, kept so the metadata columns can be rebuilt
  /// when the diff stats land from the second read.
  pub rows: Vec<crate::worktree::CommitRow>,
  /// The metadata columns, widest first: `author · stats · age`,
  /// `stats · age`, `age`. The stats halves are absent until the second
  /// read fills them in.
  pub tiers: [MetaColumn; 3],
}

/// Owned state for the full-size commit listing: the snapshotted graph
/// rows, the limit they were read at, and the scroll cursor with its
/// renderer-published bound.
#[derive(Debug, Default)]
pub struct CommitsModal {
  /// The commit rows, exactly as the sidebar pane paints them (short hash,
  /// author initials, `○` / `◎` graph, subject) — or the `(no commits)`
  /// row, or a load error.
  pub lines: Vec<Line<'static>>,
  /// The limit [`Self::lines`] was read at.
  pub limit: usize,
  /// Commits the listing describes. NOT `lines.len()`: an unborn HEAD, an
  /// empty history and a failed read each paint one sentinel row, which
  /// would read as a repository with one commit (Codex review, PR #614).
  /// [`super::super::ui::recent_commits_listing`] carries the real number.
  pub loaded: usize,
  /// The commits themselves, kept so [`Self::tiers`] can be rebuilt when
  /// the diff stats land without a second revwalk.
  pub rows: Vec<crate::worktree::CommitRow>,
  /// The metadata columns, widest first. See [`CommitsSnapshot::tiers`].
  pub tiers: [MetaColumn; 3],
  /// `true` once the diff-stat read has landed for the current listing.
  /// The columns are rebuilt then; until it does they carry author and age
  /// alone.
  pub stats_loaded: bool,
  /// Vertical scroll offset, in rows. Clamped to `max_scroll`.
  pub scroll: u16,
  /// Maximum vertical scroll offset, republished by the renderer each
  /// frame as `content_rows.saturating_sub(viewport_rows)`.
  pub max_scroll: u16,
  /// Rows the body can show, republished by the renderer alongside
  /// `max_scroll`. Only the renderer knows it, and the half-page verbs
  /// need it to move by something the user can see.
  pub viewport: u16,
  /// `true` between the request and the worker's payload. The renderer
  /// paints a loader rather than an empty canvas, which would read as "no
  /// commits".
  pub loading: bool,
  /// The worktree [`Self::lines`] describe, so a payload for a selection the
  /// user has navigated away from can be dropped instead of shown.
  pub path: Option<PathBuf>,
  /// The branch tip the in-flight (or loaded) read was taken at.
  ///
  /// Part of a read's identity alongside the path and the limit: close the
  /// overlay mid-read, land a commit, reopen on the same worktree, and
  /// path and limit both still match, so the reopen would ride on a worker
  /// holding the OLD tip and quietly show a log missing the new commits
  /// (Codex review, PR #614).
  pub head: Option<String>,
}

impl CommitsModal {
  /// An empty overlay at the origin.
  pub fn new() -> Self {
    Self::default()
  }

  /// Arm the overlay for `path` at the first page: drop the previous
  /// listing, rewind the scroll, and show the loader until [`Self::load`]
  /// lands. Called on every open, so a previously-scrolled visit starts
  /// fresh and a stale listing is never mistaken for the current one.
  pub fn begin(&mut self, path: Option<&Path>, limit: usize, head: Option<String>) {
    self.lines.clear();
    self.rows.clear();
    self.tiers = Default::default();
    self.stats_loaded = false;
    self.loaded = 0;
    self.limit = limit;
    self.scroll = 0;
    self.max_scroll = 0;
    self.path = path.map(Path::to_path_buf);
    self.head = head;
    // With nothing selected there is nothing to wait for.
    self.loading = path.is_some();
  }

  /// Arm a deeper page, **keeping** the rows and the scroll cursor on
  /// screen while the worker runs. The user pressed load-more from the
  /// bottom of the list: blanking the canvas there would throw away both
  /// the page they were reading and the position they paged from.
  pub fn begin_more(&mut self, limit: usize) {
    self.limit = limit;
    self.loading = true;
  }

  /// Install the worker's payload and clear the loader. `loaded` is the
  /// commit count the read reported, not `lines.len()`. The scroll cursor
  /// is whatever [`Self::begin`] (top) or [`Self::begin_more`] (unchanged)
  /// left; the renderer re-clamps it against the new content.
  pub fn load(&mut self, snap: CommitsSnapshot) {
    self.loaded = snap.loaded;
    self.lines = snap.lines;
    self.rows = snap.rows;
    self.tiers = snap.tiers;
    self.stats_loaded = false;
    self.loading = false;
  }

  /// Replace the metadata columns with ones that carry the diff stats.
  ///
  /// Separate from [`Self::load`] because it lands from a second, slower
  /// read: the rows are already on screen and only the right-hand column
  /// grows. The scroll cursor is untouched — the row count did not change.
  pub fn load_stats(&mut self, tiers: [MetaColumn; 3]) {
    self.tiers = tiers;
    self.stats_loaded = true;
  }

  /// Whether a deeper page exists and is allowed.
  ///
  /// `loaded < limit` means the revwalk ran out of history before the limit
  /// did, so there is nothing deeper to fetch however high the limit goes.
  /// A read already in flight also blocks: `loaded` still describes the
  /// previous page, so a second load-more would otherwise queue a duplicate
  /// walk on the same keypress-repeat.
  pub fn can_load_more(&self) -> bool {
    !self.loading && self.loaded >= self.limit && self.limit < COMMITS_MAX
  }

  /// The limit the next page reads at, clamped to [`COMMITS_MAX`].
  pub fn next_limit(&self) -> usize {
    (self.limit + COMMITS_PAGE).min(COMMITS_MAX)
  }

  /// Scroll down one row, never past the last line.
  pub fn scroll_down(&mut self) {
    self.scroll = (self.scroll + 1).min(self.max_scroll);
  }

  /// Scroll up one row, never above the top.
  pub fn scroll_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(1);
  }

  /// Scroll down half a screen (`D`), never past the last line.
  ///
  /// Half of what the body last showed, and never zero: a viewport the
  /// renderer has not published yet (nothing drawn) would otherwise make
  /// the key silently do nothing.
  pub fn scroll_half_down(&mut self) {
    self.scroll = self.scroll.saturating_add(self.half_page()).min(self.max_scroll);
  }

  /// Scroll up half a screen (`U`), never above the top.
  pub fn scroll_half_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(self.half_page());
  }

  fn half_page(&self) -> u16 {
    (self.viewport / 2).max(1)
  }

  /// Jump to the first row (`g`).
  pub fn scroll_to_top(&mut self) {
    self.scroll = 0;
  }

  /// Jump to the last row (`G`).
  pub fn scroll_to_bottom(&mut self) {
    self.scroll = self.max_scroll;
  }
}

// ── Rendering ──────────────────────────────────────────────────────────────

/// Produce the styled rows of the Recent Commits sidebar block for a
/// worktree, limited to `limit` entries. Each `Line` mirrors lazygit's
/// per-row format:
///
/// ```text
/// <8-char hash>  <author initials>  <graph>  <subject>
/// ```
///
/// where `<graph>` is the per-row output of the topology renderer in
/// [`crate::tui::commit_graph`] — a sequence of `2 * (max_pos + 1)` cells
/// drawing `○` / `◎` nodes plus the `│ ─ ╮ ╭ ╯ ╰ …` connectors that
/// link consecutive commits across branch / merge boundaries. The
/// graph width is deterministic on the commit list — independent of
/// terminal width — so the cache stays valid across resizes.
///
/// The subject is **not** truncated here — the renderer relies on
/// ratatui's view-level hard-clip (no `Wrap`) to match lazygit's gocui
/// behaviour: one commit per visual line, overflow cut at the right
/// edge without `…`.
pub fn recent_commits_lines(w: &WorktreeInfo, limit: usize, theme: &Theme) -> Vec<Line<'static>> {
  // The sidebar pane paints rows only, so the metadata columns it never
  // shows are built and dropped. Cheap next to the revwalk that precedes
  // them, and one listing routine is one place for the row format to live.
  recent_commits_listing(w, limit, worktree::unix_now(), theme).lines
}

/// The full result of one read of the log: the rows, the commit count, and
/// the two right-hand metadata columns.
///
/// The count is not `lines.len()` and the difference is not cosmetic: an
/// unborn HEAD, an empty history or a failed read all paint exactly ONE
/// sentinel row, so a caller inferring the count from the rows reads them
/// as a repository with one commit (Codex review, PR #614). The
/// commit-listing overlay (issue #593) titles itself with this count and
/// decides whether a page is full from it.
///
/// `now` is passed in rather than read here so the ages are deterministic
/// under test.
pub fn recent_commits_listing(w: &WorktreeInfo, limit: usize, now: i64, theme: &Theme) -> CommitsSnapshot {
  match worktree::recent_commits_cached(w, limit) {
    Ok(rows) if !rows.is_empty() => {
      let loaded = rows.len();
      let tiers = commit_meta_columns(&rows, now, &HashMap::new(), theme);
      let graphs = commit_graph::render_commits(&rows, theme);
      let lines = rows
        .iter()
        .cloned()
        .zip(graphs)
        .map(|(row, graph_spans)| commit_row_line(row, graph_spans, theme))
        .collect();
      CommitsSnapshot {
        lines,
        loaded,
        rows,
        tiers,
      }
    }
    Ok(_) => CommitsSnapshot {
      lines: vec![Line::from(Span::styled(
        "(no commits)".to_string(),
        Style::default().fg(theme.muted),
      ))],
      ..Default::default()
    },
    Err(e) => CommitsSnapshot {
      lines: vec![Line::from(Span::styled(
        format!("! {}", e),
        Style::default().fg(theme.prunable),
      ))],
      ..Default::default()
    },
  }
}

fn commit_row_line(row: worktree::CommitRow, graph: Vec<Span<'static>>, theme: &Theme) -> Line<'static> {
  let mut short_hash = row.hash.to_string();
  short_hash.truncate(COMMIT_HASH_DISPLAY_LEN);
  let initials = author_initials(&row.author);
  let mut spans: Vec<Span<'static>> = Vec::with_capacity(5 + graph.len());
  spans.push(Span::styled(short_hash, Style::default().fg(theme.dirty)));
  spans.push(Span::raw("  "));
  spans.push(Span::styled(
    format!("{:<2}", initials),
    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
  ));
  spans.push(Span::raw("  "));
  spans.extend(graph);
  spans.push(Span::raw(" "));
  spans.push(Span::raw(row.subject));
  Line::from(spans)
}

/// Build the two right-hand metadata columns for a commit listing.
///
/// `wide` carries `author · age`, `narrow` the age alone — the initials are
/// already on the left, so the full author is the second tier of
/// information, not the first. The age is what the listing genuinely lacks
/// today.
///
/// Ages are computed against `now` HERE, not stored on the row: the rows
/// are memoised by `(repo, tip, limit)`, so an age baked into them would be
/// frozen at the first read. They are still a snapshot in the sense that
/// the overlay does not re-read itself while open, so a listing left up for
/// an hour keeps saying `2m`.
pub fn commit_meta_columns(
  rows: &[worktree::CommitRow],
  now: i64,
  stats: &HashMap<git2::Oid, worktree::CommitStat>,
  theme: &Theme,
) -> [MetaColumn; 3] {
  let mut wide = MetaColumn::default();
  let mut mid = MetaColumn::default();
  let mut narrow = MetaColumn::default();
  let sep = || Span::styled(" · ".to_string(), Style::default().fg(theme.muted));

  for row in rows {
    let age_d = worktree::commit_age(row.time, now);
    let age = worktree::format_relative_duration(age_d);
    let age_style = Style::default().fg(freshness_color(age_d, theme));
    let age_span = || Span::styled(age.clone(), age_style);
    let author = row.author.trim();
    // Absent means "not read yet", which is not the same as a commit that
    // changed nothing — the second read fills the map and the columns are
    // rebuilt from it.
    let stat = stats.get(&row.hash).copied();

    narrow.lines.push(Line::from(age_span()));

    let mut mid_spans: Vec<Span<'static>> = Vec::new();
    if let Some(s) = stat {
      mid_spans.extend(commit_stat_spans(s, theme));
      mid_spans.push(sep());
    }
    mid_spans.push(age_span());
    mid.lines.push(Line::from(mid_spans));

    let mut wide_spans: Vec<Span<'static>> = Vec::new();
    if !author.is_empty() {
      wide_spans.push(Span::styled(author.to_string(), Style::default().fg(theme.muted)));
      wide_spans.push(sep());
    }
    if let Some(s) = stat {
      wide_spans.extend(commit_stat_spans(s, theme));
      wide_spans.push(sep());
    }
    wide_spans.push(age_span());
    wide.lines.push(Line::from(wide_spans));
  }

  for col in [&mut wide, &mut mid, &mut narrow] {
    col.width = col.lines.iter().map(Line::width).max().unwrap_or(0);
  }
  [wide, mid, narrow]
}

/// One commit's diff counts as coloured spans: `3~ 1+ 1- +120 -34`.
///
/// The file counts reuse the working-tree pane's roles (#287) so a created
/// file reads the same colour here as it does there, and the line counts
/// reuse the diff pair. A category with nothing in it is omitted rather
/// than printed as a zero: five zeroes on every quiet commit is noise, and
/// the row is competing with the subject for width.
pub fn commit_stat_spans(s: worktree::CommitStat, theme: &Theme) -> Vec<Span<'static>> {
  let mut spans: Vec<Span<'static>> = Vec::new();
  let mut push = |text: String, color: Color| {
    if !spans.is_empty() {
      spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(text, Style::default().fg(color)));
  };
  if s.files_modified > 0 {
    push(format!("{}~", s.files_modified), theme.dirty);
  }
  if s.files_added > 0 {
    push(format!("{}+", s.files_added), theme.clean);
  }
  if s.files_deleted > 0 {
    push(format!("{}-", s.files_deleted), theme.prunable);
  }
  if s.insertions > 0 {
    push(format!("+{}", s.insertions), theme.clean);
  }
  if s.deletions > 0 {
    push(format!("-{}", s.deletions), theme.prunable);
  }
  if spans.is_empty() {
    // An empty commit, or a merge that brought nothing onto its first
    // parent. Silence would read as "not loaded yet".
    spans.push(Span::styled("0".to_string(), Style::default().fg(theme.muted)));
  }
  spans
}

/// Pick the widest metadata column that leaves the subject its floor.
///
/// `body_w` is the text area AFTER the scrollbar column is reserved. The
/// tiers are tried widest first; `None` means even the narrowest does not
/// fit and the listing renders full-width as it did before.
///
/// A pure function on widths so the policy is testable without a terminal:
/// the render path only decides which `MetaColumn` this names.
pub fn commits_meta_pick(body_w: usize, tiers: [usize; 3]) -> Option<usize> {
  meta_pick(body_w, &tiers, COMMITS_SUBJECT_FLOOR)
}

/// The fixed context row above the Commits listing (issue #629): the branch
/// the log was walked on.
///
/// Both full-size overlays paint a snapshot with no statement of what it is a
/// snapshot *of* — a commit graph could be any branch's. The modal title
/// cannot carry it: it is centred, so it is clipped from the LEFT, and this
/// one already spends itself on the row count.
///
/// The branch wears the same worst-state colour it wears in the table and the
/// identity card (PR #73), so the overlay reads as the same block at a
/// different size. `-` on a detached HEAD, and on a worktree the refresh
/// dropped from the list.
fn commits_context_line(w: Option<&WorktreeInfo>, theme: &Theme) -> Line<'static> {
  let muted = Style::default().fg(theme.muted);
  let mut value = Span::styled("-".to_string(), muted);
  if let Some(w) = w {
    if let Some(b) = w.branch.as_deref() {
      value = Span::styled(
        crate::naming::sanitise_for_terminal(b),
        Style::default().fg(branch_name_color(&w.status, theme)),
      );
    }
  }
  Line::from(vec![Span::styled("Branch  ".to_string(), muted), value])
}

/// Commit-listing overlay footer hints (issue #593). `load more` / `close`
/// resolve from the `Commits*` modal bindings so a rebind of
/// `[tui.keys.modal.commits]` shows through; the scroll / top-bottom
/// movement pairs stay literal, as the Command Logs footer does.
///
/// `load more` is dropped when `more` is false — there is no deeper page,
/// either because the revwalk ran out of history or because the paging cap
/// was reached. Advertising a key that does nothing is how a working
/// overlay reads as broken. While `loading`, the slot says so instead: the
/// key is equally inert there, but for a reason that resolves on its own.
pub fn commits_footer_hints(modal: &ModalKeymap, more: bool, loading: bool) -> Vec<(String, String)> {
  let mut hints: Vec<(String, String)> = vec![
    ("j/k".to_string(), "scroll".to_string()),
    ("D/U".to_string(), "half page".to_string()),
    ("g/G".to_string(), "top/bottom".to_string()),
  ];
  if loading {
    // `more` is false while a read is out, so without this the hint slot
    // would simply go blank and a deeper page would look refused rather
    // than under way.
    hints.push(("…".to_string(), "loading".to_string()));
  } else if more {
    if let Some(k) = modal.primary_key(ModalAction::CommitsLoadMore) {
      hints.push((k, "load more".to_string()));
    }
  }
  if let Some(k) = modal.primary_key(ModalAction::CommitsClose) {
    hints.push((k, "close".to_string()));
  }
  hints
}

/// Render the Command Logs overlay (issue #226): a ~90% fullscreen modal
/// over the dimmed list showing the lazygit-style transcript of the
/// external commands gwm ran, newest-first. Scrolls like the help overlay —
/// the renderer republishes `command_logs.max_scroll` / `max_x_scroll`
/// against the live viewport so `App`'s scroll cursor can never run past
/// the content. Colours track `[theme]` roles (`clean` ok / `prunable`
/// fail / `muted` output) so a theme override applies here too.
/// Render the full-size commit listing (issue #593).
///
/// The same `~90% x 85%` canvas the Command Logs overlay uses, painting the
/// snapshot `App::enter_commits` took — one row per commit, short hash /
/// author initials / `o`-`@` graph / subject, exactly as the sidebar pane
/// paints them.
///
/// No horizontal pan: `recent_commits_lines` deliberately leaves subjects
/// untruncated and relies on ratatui's hard clip at the right edge, which is
/// lazygit's behaviour. The whole point of the overlay is that the canvas is
/// wide enough for that clip to stop mattering.
///
/// The title carries the row count so `load more` has visible feedback; a
/// trailing `+` means a deeper page exists. It rides the top rule, which is
/// clipped from the LEFT when centred, so the count sits last on purpose.
pub(crate) fn draw_commits(f: &mut Frame, app: &mut App, map: &mut MouseMap) {
  let area = centered(90, 85, f.area());
  let accent = app.theme.accent;
  let muted = app.theme.muted;

  let more = app.commits_can_load_more();
  let loading = app.commits.loading;
  // The `+` tracks "a deeper page exists", which is true while one is being
  // read too: `can_load_more` is false then only because the read is out.
  let deeper = more || (loading && app.commits.loaded >= app.commits.limit);
  let title = format!("Commits ({}{})", app.commits.loaded, if deeper { "+" } else { "" });
  let frame = ModalFrame::resolve_for(app, accent);
  let inner = frame.render(f, map, area, &title, None);

  // The branch this log was walked on, pinned above the scroll region
  // (issue #629). Built before the mutable borrows below.
  let context = commits_context_line(overlay_worktree(app, app.commits.path.as_deref()), &app.theme);

  // A blank row between the listing and the hints, the gap every other
  // modal already leaves: content never sits flush against the footer.
  let [context_area, body_area, _gap, footer_area] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(1),
    Constraint::Length(1),
    Constraint::Length(1),
  ])
  .areas(inner);
  f.render_widget(Paragraph::new(context), context_area);

  // A muted loader rather than an empty canvas while the first page is
  // being walked: blank reads as "no commits", which is the one answer this
  // overlay must not give by accident. A deeper page keeps the rows it
  // already has on screen instead, and says `loading` in the footer.
  let lines: Vec<Line<'static>> = if !app.commits.lines.is_empty() {
    app.commits.lines.clone()
  } else if loading {
    vec![Line::from(Span::styled(
      " loading…".to_string(),
      Style::default().fg(muted),
    ))]
  } else {
    vec![Line::from(Span::styled(
      "No commits.".to_string(),
      Style::default().fg(muted),
    ))]
  };

  // Publish the scroll bound against the BODY viewport only (issue #279).
  let body_viewport = body_area.height as usize;
  app.commits.viewport = body_area.height;
  app.commits.max_scroll = (lines.len().saturating_sub(body_viewport)) as u16;
  app.commits.scroll = app.commits.scroll.min(app.commits.max_scroll);
  let scroll = app.commits.scroll;
  let text_area = scrollable_body_area(f, body_area, scroll, lines.len(), &app.theme);

  // The metadata rides its own rect on the right rather than being appended
  // to each row: the subject is deliberately hard-clipped without an
  // ellipsis (lazygit's gocui behaviour, documented on
  // `recent_commits_lines` and shared with the sidebar pane), so narrowing
  // the left rect IS that same rule applied at a nearer edge. Both
  // paragraphs take the same scroll offset, so the columns stay aligned.
  let widths = [
    app.commits.tiers[0].width,
    app.commits.tiers[1].width,
    app.commits.tiers[2].width,
  ];
  let meta = commits_meta_pick(text_area.width as usize, widths);

  match meta {
    Some(meta_w) => {
      let column = &app.commits.tiers[widths.iter().position(|&w| w == meta_w).unwrap_or(2)];
      let meta_w = meta_w as u16;
      let [left, _gap, right] = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(META_GAP as u16),
        Constraint::Length(meta_w),
      ])
      .areas(text_area);
      f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), left);
      f.render_widget(
        Paragraph::new(column.lines.clone()).right_aligned().scroll((scroll, 0)),
        right,
      );
    }
    None => f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), text_area),
  }

  let footer_owned = commits_footer_hints(&app.modal_keymap, more, loading);
  let footer_hints: Vec<(&str, &str)> = footer_owned.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
  f.render_widget(modal_hint_line(&footer_hints, &app.theme), footer_area);
}

// ── Behaviour ──────────────────────────────────────────────────────────────

impl App {
  /// Open the full-size commit listing (issue #593).
  ///
  /// The overlay opens immediately on a loader and the revwalk runs on a
  /// [`TaskKind::Commits`] worker. It is deliberately NOT inline: the walk
  /// sorts `TIME | TOPOLOGICAL`, so it traverses the whole reachable graph
  /// before yielding a row. Measured on this repo, asking for 300 commits
  /// costs the same as asking for all 2058 — the limit truncates the
  /// output, it bounds nothing about the latency, so an inline call would
  /// freeze the event loop for as long as the history is deep. Same
  /// boundary as the sidebar's own preview (#343), reached from a keypress
  /// rather than from navigation.
  ///
  /// The rows are read fresh rather than taken from `SidebarState::cache`:
  /// that cache is keyed by `(path, mode)` and only rebuilt while the
  /// sidebar is open and in commits mode, so it is empty in the two states
  /// where the overlay is most useful. The read still goes through
  /// [`crate::worktree::recent_commits_cached`] at [`COMMITS_PAGE`], which
  /// is the sidebar's own limit, so a sidebar that already walked this tip
  /// makes the worker a hash lookup.
  ///
  /// The tip comes from `WorktreeInfo.head`, the snapshot `worktree::list`
  /// took at the last refresh, NOT from resolving HEAD here. A commit
  /// landing between two refreshes is therefore invisible to the overlay
  /// until the next one — which is exactly what the sidebar's Commits pane
  /// shows, since it hands the same `WorktreeInfo` to the same memo
  /// (`ui.rs`, `SidebarMode::Commits`). Resolving HEAD at open would make
  /// the overlay disagree with the pane it is a full-size view of, and the
  /// staleness window is one auto-refresh interval. Raised twice by Codex
  /// on PR #614 and declined both times: the snapshot is the contract of
  /// `recent_commits_cached`, not an oversight here.
  ///
  /// With nothing selected the overlay still opens, empty — the
  /// [`Self::enter_config_panel`] precedent: a modal that refuses to open
  /// reads as a dead key.
  pub fn enter_commits(&mut self) {
    let selected = self.selected().cloned();
    let target = selected.as_ref().map(|w| w.path.as_path());
    // Coalescing is only sound while the in-flight read is for the SAME
    // worktree, at the SAME limit, on the SAME tip (the #592 lesson, PR
    // #612; the tip added by a Codex review on PR #614 — a commit landing
    // while the overlay is closed mid-read would otherwise be swallowed by
    // a worker holding the old `head`). Otherwise the
    // request would come back `None` because the slot is still the old
    // read's, no worker would exist for this one, and the old payload is
    // dropped by the checks in the drain — leaving the loader up with
    // nothing left to clear it.
    let tip = selected.as_ref().and_then(|w| w.head.clone());
    if self.commits.loading
      && (self.commits.path.as_deref() != target || self.commits.limit != COMMITS_PAGE || self.commits.head != tip)
    {
      self.tasks.invalidate(TaskKind::Commits);
    }
    self.commits.begin(target, COMMITS_PAGE, tip);
    self.view = View::Commits;
    if let Some(w) = selected {
      self.request_commits_read(w, COMMITS_PAGE);
    }
  }

  /// The worktree the commit overlay opened on, if it is still listed.
  ///
  /// Deliberately not [`Self::selected`]: the auto-refresh moves the
  /// selection while the overlay is up, and the drain matches a payload
  /// against `commits.path`, so a read fired for the newly-selected
  /// worktree is dropped on the path check *after* `complete` freed the
  /// slot, leaving nothing to clear the loader (Codex review, PR #614).
  ///
  /// `None` once another process removes the worktree and the refresh drops
  /// it from the list: there is no longer anything to walk.
  fn commits_target(&self) -> Option<&WorktreeInfo> {
    let path = self.commits.path.as_deref()?;
    self.worktrees.iter().find(|w| w.path == path)
  }

  /// Whether the commit overlay can page deeper: the listing says a page
  /// exists AND the worktree it opened on is still there to read.
  ///
  /// [`CommitsModal::can_load_more`] owns the arithmetic and cannot see the
  /// worktree list, so on its own it keeps saying yes for a worktree that
  /// has been removed underneath the overlay. The renderer and
  /// [`Self::load_more_commits`] both read *this*, so the advertised key
  /// and the key that acts can never disagree.
  pub fn commits_can_load_more(&self) -> bool {
    self.commits.can_load_more() && self.commits_target().is_some()
  }

  /// Re-read the commit listing one page deeper (issue #593).
  ///
  /// A re-read rather than an append: the graph renderer resolves a row's
  /// connectors against the parents of the rows below it, so a page tacked
  /// onto the end would draw its topology against nothing. The memo in
  /// [`crate::worktree::recent_commits_cached`] is keyed on the limit, so
  /// the deeper read is a fresh entry rather than an invalidation of the
  /// sidebar's.
  ///
  /// The rows already on screen stay up while the worker runs, so paging
  /// keeps its place instead of blanking. A no-op when
  /// [`Self::commits_can_load_more`] is false; the footer drops the `load
  /// more` hint on that same predicate, so the key is never advertised
  /// where it would do nothing.
  pub fn load_more_commits(&mut self) {
    if !self.commits_can_load_more() {
      return;
    }
    let Some(w) = self.commits_target().cloned() else {
      return;
    };
    let limit = self.commits.next_limit();
    self.commits.begin_more(limit);
    self.request_commits_read(w, limit);
  }

  /// Spawn the worker that walks `w`'s log to `limit` and renders it.
  ///
  /// A `None` from the slot means a read for this same worktree and limit
  /// is already out: ride on it, which is what keeps a held `6` from
  /// spawning a revwalk per repeat. Callers that need a *different* read
  /// invalidate the slot first.
  fn request_commits_read(&mut self, w: WorktreeInfo, limit: usize) {
    let Some(generation) = self.tasks.request(TaskKind::Commits) else {
      return;
    };
    let theme = self.theme;
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let snap = recent_commits_listing(&w, limit, crate::worktree::unix_now(), &theme);
      let _ = tx.send(TaskMsg::Commits(generation, w.path, limit, snap));
    });
  }

  /// `true` while the commit listing is waiting on its worker.
  pub fn is_commits_loading(&self) -> bool {
    self.commits.loading
  }

  /// Spawn the second, slower read: one `git log --raw --numstat` over the
  /// oids already on screen, rebuilding the metadata columns with the diff
  /// counts (issue #593).
  ///
  /// Chained after the rows rather than folded into them. The revwalk takes
  /// about 0.4s and this takes one to three seconds depending on the page
  /// depth, so folding the two would hold the whole listing behind the
  /// slower half. The rows appear first and the column grows under them.
  ///
  /// A no-op with nothing to read, and when the stats for this listing are
  /// already in place — the drain calls this on every landing, including
  /// the ones that only re-installed the same rows.
  pub fn request_commit_stats(&mut self) {
    if self.commits.stats_loaded || self.commits.rows.is_empty() {
      return;
    }
    let Some(path) = self.commits.path.clone() else {
      return;
    };
    let (limit, tip) = (self.commits.limit, self.commits.head.clone());
    let oids: Vec<git2::Oid> = self.commits.rows.iter().map(|r| r.hash).collect();
    let rows = self.commits.rows.clone();
    // A read for a different listing is still out: free the slot, or this
    // one never starts and the columns never grow.
    if self.tasks.is_loading(TaskKind::CommitStats) {
      self.tasks.invalidate(TaskKind::CommitStats);
    }
    let Some(generation) = self.tasks.request(TaskKind::CommitStats) else {
      return;
    };
    let theme = self.theme;
    let tx = self.task_tx.clone();
    std::thread::spawn(move || {
      let stats = crate::worktree::commit_stats(&path, &oids).unwrap_or_default();
      let now = crate::worktree::unix_now();
      let tiers = commit_meta_columns(&rows, now, &stats, &theme);
      let _ = tx.send(TaskMsg::CommitStats(generation, path, limit, tip, tiers));
    });
  }

  /// Route one keystroke through the full-size commit listing (issues
  /// #593, #635). Returns `true` when the overlay should close.
  ///
  /// A method rather than a `match` in the run loop, the shape
  /// [`Self::handle_working_tree_key`] settled on (#217, #613): the order
  /// the arms resolve in is a contract, and a `match` sitting inside the
  /// event loop cannot be driven from a test, so nothing pinned it.
  ///
  /// The arms are the run loop's, verbatim and in the same order. This
  /// overlay deliberately does NOT open with a
  /// [`App::modal_toggle_stroke`] block the way the Working Tree one
  /// does: `c` closes it as a bound `CommitsClose` alternative, not as a
  /// rebindable global toggle, and adding the toggle here would change
  /// which stroke wins.
  pub fn handle_commits_key(&mut self, key: KeyEvent) -> bool {
    match self.resolve_modal(KeyContext::Commits, key) {
      Some(ModalAction::CommitsClose) => return true,
      Some(ModalAction::CommitsLoadMore) => self.load_more_commits(),
      Some(ModalAction::CommitsScrollDown) => self.commits.scroll_down(),
      Some(ModalAction::CommitsScrollUp) => self.commits.scroll_up(),
      Some(ModalAction::CommitsScrollTop) => self.commits.scroll_to_top(),
      Some(ModalAction::CommitsScrollBottom) => self.commits.scroll_to_bottom(),
      Some(ModalAction::CommitsHalfDown) => self.commits.scroll_half_down(),
      Some(ModalAction::CommitsHalfUp) => self.commits.scroll_half_up(),
      _ => {}
    }
    false
  }
}
