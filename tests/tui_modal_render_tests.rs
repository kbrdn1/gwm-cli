//! Modal render net (issue #235).
//!
//! CHARACTERISATION of EXISTING behaviour, not red-first TDD: every test
//! here renders one of the TUI overlays through the real `gwm::tui::draw`
//! entry point (the same path the event loop uses) into a fixed-size
//! `ratatui::backend::TestBackend`, then asserts against the resulting
//! `Buffer`:
//!
//!   1. the modal's title text is present,
//!   2. its button / action labels appear.
//!
//! `TestBackend` hard-clips every widget to the buffer rect, so an
//! *asserted* label that overflows the terminal simply *vanishes* from
//! the buffer rather than panicking — each substring assertion below is
//! therefore also a clip guard for that specific label: if the modal
//! layout truncates an asserted title or button under the chosen size,
//! the search fails and the test goes red. This is a per-label check, not
//! a whole-rect bounds assertion. Where a needle could also be painted by
//! the background behind the modal, the test asserts a body-unique label
//! instead (see the confirm case).
//!
//! The terminal is sized 100×40 — wide enough that no modal needs to
//! clip its content. `make_app` also closes the sidebar (`open = false`),
//! so `draw_body` renders the worktree table full-area with no sidebar:
//! this keeps the render path from shelling out to `git` (deterministic /
//! offline) and stops sidebar labels (e.g. `Path`, `Branch`) from leaking
//! into the buffer behind the modal and masking a regression. (Width
//! alone would not hide the sidebar — its default orientation is
//! `Stacked`, which renders even below 120 columns.)
//!
//! The point is a safety net so the upcoming `ui.rs` refactors (plan
//! items P3-P8) cannot silently regress modal layout.

mod common;

use common::init_repo;
use gwm::bootstrap::{BootstrapReport, StepResult};
use gwm::config::TuiLayout;
use gwm::tui::{draw, App, Field, LinkTarget, TaskKind, View};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
use std::path::PathBuf;

const TERM_W: u16 = 100;
const TERM_H: u16 = 40;

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  // Close the sidebar so `draw_body` renders the worktree table full-area
  // with no sidebar pane. Two reasons: (1) the sidebar shells out to
  // `git status` / `git log`, so leaving it open would make these tests
  // touch git — closing it keeps them deterministic and offline; (2) the
  // sidebar paints labels such as `Path` and `Branch` *behind* the modal,
  // and `buffer_contains` scans the whole buffer, so those would mask a
  // modal regression. NOTE: the default is `open = true` with the
  // `Stacked` orientation, which renders even at < 120 cols — terminal
  // width alone does NOT hide the sidebar; only `open = false` does.
  app.sidebar.open = false;
  // Modals follow `[tui] layout` since #594, and the shipped default is
  // `Compact`. Every case built on this helper pins the BORDERED contract:
  // the `sizing_matrix` numbers, and a `modal_rect` that locates the frame by
  // its `╭`. The layout is pinned here rather than inherited, because without it
  // the whole file would silently re-measure the compact frame under names
  // that promise the boxed one, and `modal_rect` would find no corner at all.
  // Compact coverage flips it back through [`compact_app`], so both halves
  // of the layout are exercised by the same setups.
  pin_bordered(&mut app);
  (dir, app)
}

/// Pin the boxed layout on an app this file builds outside [`make_app`].
/// Same reason, and the same contract: the assertions below describe the
/// bordered frame.
fn pin_bordered(app: &mut App) {
  app.config.tui.layout = TuiLayout::Bordered;
}

/// A synthetic, deletable (non-main) worktree row so the Confirm modal
/// renders its destructive-summary body instead of the "nothing
/// selected" fallback.
fn deletable_worktree(name: &str) -> WorktreeInfo {
  WorktreeInfo {
    name: name.into(),
    id: name.into(),
    path: PathBuf::from(format!("/tmp/gwm-test/{}", name)),
    branch: Some(format!("feat/#235-{}", name)),
    head: Some("0123456789abcdef0123456789abcdef01234567".into()),
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: gwm::github::BranchLink::empty(),
    issue_state: None,
    pr_state: None,
    age: None,
    has_note: false,
  }
}

/// Render `app` through the real `gwm::tui::draw` entry point into a
/// fixed-size `TestBackend` and return the resulting `Buffer`.
fn render(app: &mut App) -> Buffer {
  let backend = TestBackend::new(TERM_W, TERM_H);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, app)).unwrap();
  terminal.backend().buffer().clone()
}

/// Flatten one buffer row into its visible string (cell symbols
/// concatenated left-to-right). A label that the modal layout clipped at
/// the terminal edge is simply absent from every row's flattened text,
/// so a `contains` search over these rows is a faithful "is it actually
/// on screen" check.
fn row_strings(buf: &Buffer) -> Vec<String> {
  let area = *buf.area();
  (0..area.height)
    .map(|y| {
      (0..area.width)
        .map(|x| buf[(area.x + x, area.y + y)].symbol())
        .collect::<String>()
    })
    .collect()
}

/// True when `needle` appears as a contiguous substring inside any single
/// rendered row. Contiguity matters: ratatui paints a `Span`'s text into
/// adjacent cells on one row, so a button label that survives layout
/// lands intact on a row, while a clipped one does not.
fn buffer_contains(buf: &Buffer, needle: &str) -> bool {
  row_strings(buf).iter().any(|row| row.contains(needle))
}

fn assert_present(buf: &Buffer, needle: &str, what: &str) {
  assert!(
    buffer_contains(buf, needle),
    "{what}: expected {needle:?} to be rendered (not clipped) — buffer rows:\n{}",
    row_strings(buf).join("\n")
  );
}

fn assert_absent(buf: &Buffer, needle: &str, what: &str) {
  assert!(
    !buffer_contains(buf, needle),
    "{what}: expected {needle:?} NOT to be rendered — buffer rows:\n{}",
    row_strings(buf).join("\n")
  );
}

#[test]
fn worktrees_table_header_labels_the_issue_pr_badge_column() {
  // The worktree table's badge column (the `●/●` issue/PR pastilles) now
  // carries an `I/P` caption alongside the NAME / BRANCH / STATUS / PATH
  // headers. Render the default list view (no modal) and assert it.
  let (_dir, mut app) = make_app();
  let buf = render(&mut app);
  assert_present(&buf, "I/P", "issue/PR badge column header");
  assert_present(&buf, "NAME", "name column header");
  assert_present(&buf, "BRANCH", "branch column header");
}

#[test]
fn help_modal_renders_title_and_close_hint() {
  let (_dir, mut app) = make_app();
  app.enter_help();
  assert_eq!(app.view, View::Help);
  let buf = render(&mut app);
  // Title row of the Keybindings overlay (see `help_rows`).
  assert_present(&buf, "Keybindings", "help title");
  // The help body lists key→action rows from the top (default
  // `help_scroll == 0`); the `quit` entry sits near the top, so it is a
  // stable canary that the overlay rendered its body, not just the title
  // (the `close` hint footer can scroll off on a short modal, so it is
  // not asserted here).
  assert_present(&buf, "quit", "help quit entry");
}

#[test]
fn help_modal_keeps_title_and_footer_fixed_while_body_scrolls() {
  // Issue #279: the Keybindings overlay scrolls its BODY only — the title
  // and the footer hint stay pinned. Render into a short terminal (so the
  // body definitely overflows), scroll to the bottom, and assert that both
  // the title and the footer hint are still on screen. Pre-#279 the whole
  // content scrolled in one Paragraph, so at max scroll the title rolled
  // off the top — this test would have gone red.
  let (_dir, mut app) = make_app();
  app.enter_help();
  // Drive the scroll cursor past the end; the renderer clamps it to the
  // body's max-scroll, i.e. "scrolled to the bottom".
  app.help_scroll = u16::MAX;

  let backend = TestBackend::new(100, 18);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let buf = terminal.backend().buffer().clone();

  assert_present(&buf, "Keybindings", "help title stays fixed at the top");
  // The footer advertises the close hint — pinned at the bottom, visible
  // even at max scroll.
  assert_present(&buf, "close", "help footer hint stays fixed at the bottom");
}

#[test]
fn create_modal_renders_title_fields_and_buttons() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  assert_eq!(app.view, View::Create);
  let buf = render(&mut app);
  assert_present(&buf, "New Worktree", "create title");
  // The form's field labels and live preview rows.
  assert_present(&buf, "Branch", "create branch preview label");
  assert_present(&buf, "Issue", "create issue field label");
  assert_present(&buf, "Desc", "create desc field label");
  // Button row (chips are `" Create "` / `" Cancel "`; assert the
  // unpadded label so the test is robust to chip padding).
  assert_present(&buf, "Create", "create button");
  assert_present(&buf, "Cancel", "cancel button");
}

#[test]
fn create_from_issue_modal_shows_one_field_and_no_empty_preview() {
  // #625: the triple is empty until the forge answers, so expanding the
  // patterns over it renders the literal `/#-` — a branch this form is not
  // about to write. The preview is replaced rather than shown empty, and the
  // type selector and slug field are not drawn at all: this mode has one
  // input.
  //
  // Asserted against the rendered buffer, not against the mode: a guard that
  // matched by name would pass while the modal drew the structured form.
  let (_dir, mut app) = make_app();
  app.enter_create_from_issue();
  assert_eq!(app.view, View::Create);
  let buf = render(&mut app);

  assert_present(&buf, "from issue", "the title says which mode this is");
  assert_present(&buf, "Issue", "the one field it collects");
  assert_present(&buf, "read off the issue", "what the missing preview is replaced by");
  assert_absent(
    &buf,
    "structured",
    "the toggle is inert here, so the hint row does not offer it",
  );
  assert_absent(&buf, "Branch :", "no preview of a triple that does not exist yet");
  assert_absent(&buf, "Dir    :", "same for the directory row");
  assert_absent(&buf, "Desc", "the slug is derived, not typed here");
}

#[test]
fn create_from_issue_modal_becomes_the_structured_form_once_the_issue_lands() {
  // The prefill is the point: what the user confirms is the ordinary create
  // form, showing the branch it will write.
  let (_dir, mut app) = make_app();
  app.enter_create_from_issue();
  app.create_form.awaiting_issue = Some(594);
  app.apply_awaited_issue(
    594,
    &Ok(gwm::github::IssueStatus {
      number: 594,
      title: "modals should follow layout".into(),
      state: gwm::github::IssueState::Open,
      url: "https://example.test/issues/594".into(),
      labels: vec!["feature".into()],
      updated_at: "2026-08-29T00:00:00Z".into(),
      detail: Default::default(),
    }),
  );

  let buf = render(&mut app);
  assert_present(&buf, "Branch", "the preview is back");
  assert_present(&buf, "Desc", "and so is the slug field");
  assert_present(
    &buf,
    "modals-should-follow-layout",
    "the derived slug is in the form, visible before it is committed to",
  );
}

#[test]
fn create_modal_renders_loader_while_create_is_in_flight() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.tasks.request(TaskKind::CreateWorktree).unwrap();

  let buf = render(&mut app);

  assert_present(&buf, "New Worktree", "create title");
  assert_present(&buf, "creating worktree", "create loader label");
}

#[test]
fn create_modal_renders_create_failure_after_async_create_fails() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_failure = Some("branch already exists".into());

  let buf = render(&mut app);

  assert_present(&buf, "create failed", "create failure label");
  assert_present(&buf, "branch already exists", "create failure detail");
  assert_present(&buf, "Cancel", "cancel button after failure");
}

#[test]
fn confirm_modal_renders_title_target_and_buttons() {
  let (_dir, mut app) = make_app();
  // Inject a deletable worktree and select it so the modal renders its
  // destructive-summary body (the "nothing selected" branch has no
  // buttons). Opened through `enter_confirm_delete` because the overlay
  // renders the batch snapshot it takes (#484), not the cursor row; the
  // guard itself is covered in tui_app_tests.rs.
  app.worktrees.push(deletable_worktree("feat-235-net"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.enter_confirm_delete();
  let buf = render(&mut app);
  assert_present(&buf, "Delete Worktree", "confirm title");
  // Pin the destructive-summary BODY via labels unique to it: the detail
  // grid's "Path" row and the "Delete Branch" toggle row (always pushed —
  // see `draw_confirm`). The target *name* is deliberately NOT asserted:
  // it also appears in the worktree table painted behind the modal, so a
  // name assertion would stay green even if the modal body regressed. At
  // 100 cols the sidebar (the only other "Path" label) stays hidden, so
  // these labels come solely from the confirm body.
  assert_present(&buf, "Path", "confirm detail-grid Path label");
  assert_present(&buf, "Delete Branch", "confirm delete-branch toggle label");
  // Confirm / Cancel buttons.
  assert_present(&buf, "Confirm", "confirm button");
  assert_present(&buf, "Cancel", "cancel button");
}

#[test]
fn confirm_modal_renders_the_batch_size_instead_of_the_rows() {
  // #484: a batch reports how many worktrees it will delete and how many of
  // them carry a branch. It deliberately does NOT list them — the rows are
  // already on screen behind the modal, and sassman asked for the count.
  let (_dir, mut app) = make_app();
  app.worktrees.push(deletable_worktree("feat-484-one"));
  app.worktrees.push(deletable_worktree("feat-484-two"));
  app.list_state.select(Some(app.worktrees.len() - 2));
  app.toggle_select();
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.toggle_select();
  app.enter_confirm_delete();

  let buf = render(&mut app);

  assert_present(&buf, "Delete 2 Worktrees", "batch confirm title");
  assert_present(&buf, "2 selected", "batch size");
  assert_present(&buf, "2 of 2 carry a branch", "branch summary");
  assert_present(&buf, "Delete Branch", "the batch-wide branch toggle");
}

#[test]
fn confirm_modal_delete_branch_row_uses_the_live_toggle_chord() {
  // Codex review on PR #292 (P2): ToggleDeleteBranch moved to `D` in #290, but
  // the delete modal's "Delete Branch" row hardcoded `p`. It must show the live
  // chord (`D`) — a key that actually toggles the option in the confirm context.
  let (_dir, mut app) = make_app();
  app.worktrees.push(deletable_worktree("feat-290-togglekey"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.enter_confirm_delete();
  let buf = render(&mut app);
  let row = row_strings(&buf)
    .into_iter()
    .find(|r| r.contains("Delete Branch"))
    .expect("a Delete Branch row");
  // The ` D ` chip (space-padded) is distinct from the 'D' in "Delete Branch".
  assert!(
    row.contains(" D "),
    "delete-branch row must show the live `D` chord chip: {row:?}"
  );
  assert!(
    !row.contains(" p "),
    "stale `p` chip must be gone from the delete-branch row: {row:?}"
  );
}

#[test]
fn confirm_modal_renders_delete_loader_while_delete_is_in_flight() {
  let (_dir, mut app) = make_app();
  app.worktrees.push(deletable_worktree("feat-257-loader"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.enter_confirm_delete();
  app.tasks.request(TaskKind::DeleteWorktree).unwrap();

  let buf = render(&mut app);

  assert_present(&buf, "Delete Worktree", "confirm title");
  assert_present(&buf, "deleting worktree", "delete loader label");
}

#[test]
fn confirm_modal_renders_delete_failure_after_async_delete_fails() {
  let (_dir, mut app) = make_app();
  app.worktrees.push(deletable_worktree("feat-257-loader"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.enter_confirm_delete();
  app.delete_failure = Some("permission denied".into());

  let buf = render(&mut app);

  assert_present(&buf, "delete failed", "delete failure label");
  assert_present(&buf, "permission denied", "delete failure detail");
  assert_present(&buf, "Cancel", "cancel button after failure");
}

#[test]
fn report_modal_renders_title_and_step_labels() {
  let (_dir, mut app) = make_app();
  app.report = Some(BootstrapReport {
    steps: vec![
      StepResult::ok("copy env file"),
      StepResult::skipped("npm install", "no package.json"),
    ],
  });
  app.view = View::Report;
  let buf = render(&mut app);
  assert_present(&buf, "Bootstrap Report", "report title");
  // The Logs section frame and the rendered step labels.
  assert_present(&buf, "Logs", "report logs section title");
  assert_present(&buf, "copy env file", "report ok step label");
  assert_present(&buf, "npm install", "report skipped step label");
}

#[test]
fn command_logs_modal_renders_title_and_entry_argv() {
  use gwm::command_log::{CommandLogEntry, CommandStatus};
  use std::time::Duration;

  let (_dir, mut app) = make_app();
  // Inject entries directly into owned state so the render is deterministic
  // and never touches the process-global log (the event loop is what syncs
  // the global in; here we pin the *render*).
  app.command_logs.entries = vec![CommandLogEntry {
    command: "gh issue view 226 --json title,body".into(),
    duration: Duration::from_millis(412),
    status: CommandStatus::Exited(Some(0)),
    output: "ok".into(),
  }];
  app.view = View::CommandLogs;
  let buf = render(&mut app);
  assert_present(&buf, "Command Logs", "command logs title");
  assert_present(&buf, "gh issue view 226", "logged command argv");
  // The footer advertises the `y` copy bind (issue #279).
  assert_present(&buf, "copy", "command logs copy hint");
}

/// Drain until the commit-listing worker lands, or fail loudly.
fn settle_commits(app: &mut gwm::tui::App) {
  use std::time::{Duration, Instant};
  let deadline = Instant::now() + Duration::from_secs(10);
  while app.is_commits_loading() && Instant::now() < deadline {
    app.drain_task_results();
    std::thread::sleep(Duration::from_millis(5));
  }
  assert!(
    !app.is_commits_loading(),
    "the commit-listing worker never landed within 10s"
  );
}

#[test]
fn commits_modal_paints_a_loader_before_the_walk_lands() {
  // The revwalk is on a worker, so the first frame has no rows. A blank
  // canvas there reads as "no commits", which is the one answer this
  // overlay must not give by accident.
  let (_dir, mut app) = make_app();
  app.enter_commits();
  let buf = render(&mut app);
  assert_present(&buf, "loading", "loader while the walk is out");
  assert_absent(&buf, "No commits", "empty-state placeholder during a load");
  settle_commits(&mut app);
}

#[test]
fn commits_modal_renders_the_graph_and_counts_its_rows() {
  // Issue #593: `6` paints the sidebar's commit graph on the full canvas.
  // The count rides the title so `load more` has visible feedback; the
  // fixture repo has one commit, so there is no deeper page and no `+`.
  let (_dir, mut app) = make_app();
  app.enter_commits();
  settle_commits(&mut app);
  let buf = render(&mut app);
  assert_present(&buf, "Commits (1)", "commits title with its row count");
  assert_present(&buf, "init", "the commit subject");
  assert_absent(&buf, "load more", "load-more hint on an exhausted history");
  assert_absent(&buf, "loading", "loader after the walk landed");
}

#[test]
fn commits_modal_drops_load_more_when_the_worktree_is_gone() {
  // The render-side half of the same rule: a full page whose worktree left
  // the list must not paint the hint, nor the title's `+`.
  let (_dir, mut app) = make_app();
  app.enter_commits();
  settle_commits(&mut app);
  app.commits.loaded = app.commits.limit;
  app.worktrees.clear();
  let buf = render(&mut app);
  assert_absent(&buf, "load more", "load-more hint for a vanished worktree");
}

#[test]
fn commits_modal_advertises_load_more_only_when_a_page_is_full() {
  // The footer hint and `App::load_more_commits` read the same predicate,
  // so a key that does nothing is never advertised. Forced rather than
  // committed 300 times: this pins the footer, not the revwalk.
  let (_dir, mut app) = make_app();
  app.enter_commits();
  settle_commits(&mut app);
  app.commits.loaded = app.commits.limit;
  let buf = render(&mut app);
  assert_present(&buf, "load more", "load-more hint on a full page");
  assert_present(&buf, "+", "the title flags that a deeper page exists");
}

#[test]
fn commits_modal_pins_its_branch_above_the_scrolling_body() {
  use ratatui::text::Line;

  // Issue #629: the listing said nothing about the worktree it was walked
  // on, and the title cannot carry it — a centred overlay title is clipped
  // from the LEFT, so a branch name is exactly the wrong payload for it,
  // and it already spends itself on the row count. A fixed row above the
  // body carries it instead.
  //
  // Asserted by POSITION, not by presence: a `buffer_contains` scan over
  // the whole buffer passes just as well when the branch is the first line
  // of the *body* and scrolls away with it, which is the bug this pins.
  let (_dir, mut app) = make_app();
  app.enter_commits();
  settle_commits(&mut app);
  let branch = app.worktrees[0]
    .branch
    .clone()
    .expect("the fixture worktree is on a branch");
  // Enough rows that the body overflows a short terminal. Injected rather
  // than committed 40 times: this pins the header, not the revwalk.
  app.commits.lines = (0..40).map(|i| Line::from(format!("commit row {i}"))).collect();

  let top = modal_rows(&render_at(&mut app, 100, 18));
  let row = top
    .iter()
    .position(|r| r.contains(&branch))
    .unwrap_or_else(|| panic!("no branch row — modal rows:\n{}", top.join("\n")));
  assert!(
    top[row + 1].contains("commit row 0"),
    "the listing starts on the row right below it — got {:?}",
    top[row + 1]
  );

  // Drive the cursor past the end; the renderer clamps it to the body's
  // max-scroll, i.e. "scrolled to the bottom".
  app.commits.scroll = u16::MAX;
  let bottom = modal_rows(&render_at(&mut app, 100, 18));
  assert!(
    bottom[row].contains(&branch),
    "the branch row is FIXED: same line at max scroll — got {:?}, modal rows:\n{}",
    bottom[row],
    bottom.join("\n")
  );
  assert!(
    !bottom[row + 1].contains("commit row 0"),
    "and it is the body that moved under it — got {:?}",
    bottom[row + 1]
  );
}

#[test]
fn command_logs_modal_renders_empty_placeholder() {
  let (_dir, mut app) = make_app();
  app.command_logs.entries.clear();
  app.view = View::CommandLogs;
  let buf = render(&mut app);
  assert_present(&buf, "Command Logs", "command logs title");
  assert_present(&buf, "No commands", "empty-state placeholder");
}

#[test]
fn command_logs_modal_keeps_title_and_footer_fixed_while_body_scrolls() {
  use gwm::command_log::{CommandLogEntry, CommandStatus};
  use std::time::Duration;

  // Issue #279: the Command Logs overlay scrolls its body only — title and
  // footer hint stay pinned. Many entries + a short terminal force overflow;
  // scrolling to the bottom must keep both on screen.
  let (_dir, mut app) = make_app();
  app.command_logs.entries = (0..12)
    .map(|i| CommandLogEntry {
      command: format!("command number {i}"),
      duration: Duration::from_millis(10),
      status: CommandStatus::Exited(Some(0)),
      output: "some output".into(),
    })
    .collect();
  app.view = View::CommandLogs;
  app.command_logs.scroll = u16::MAX; // clamps to the bottom on render

  let backend = TestBackend::new(100, 16);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let buf = terminal.backend().buffer().clone();

  assert_present(&buf, "Command Logs", "title stays fixed at the top");
  assert_present(&buf, "scroll", "footer hint stays fixed at the bottom");
}

#[test]
fn command_logs_modal_separates_entries_with_a_dashed_rule() {
  use gwm::command_log::{CommandLogEntry, CommandStatus};
  use std::time::Duration;

  // Issue #279: adjacent log entries are separated by a full-width `-` rule
  // (padded by a blank line above and below).
  let (_dir, mut app) = make_app();
  app.command_logs.entries = vec![
    CommandLogEntry {
      command: "first".into(),
      duration: Duration::from_millis(1),
      status: CommandStatus::Exited(Some(0)),
      output: String::new(),
    },
    CommandLogEntry {
      command: "second".into(),
      duration: Duration::from_millis(1),
      status: CommandStatus::Exited(Some(0)),
      output: String::new(),
    },
  ];
  app.view = View::CommandLogs;
  let buf = render(&mut app);
  assert_present(&buf, "----------", "a dashed rule separates the two entries");
}

#[test]
fn settings_panel_all_tab_renders_title_section_and_source_column() {
  use gwm::config::{ConfigRow, ConfigSource};
  use gwm::tui::SettingsTab;

  let (_dir, mut app) = make_app();
  // Inject rows directly so the render is deterministic (the event loop is
  // what resolves the real config on open; here we pin the *render*).
  app.config_panel.rows = vec![
    ConfigRow {
      key: "worktree.base".into(),
      value: "\"/tmp/repo-wt\"".into(),
      source: ConfigSource::Repo,
    },
    ConfigRow {
      key: "worktree.path_pattern".into(),
      value: "\"{type}-{issue}-{desc}\"".into(),
      source: ConfigSource::Default,
    },
  ];
  // The read-only resolved config now lives under the `All` tab.
  app.config_panel.tab = SettingsTab::All;
  app.view = View::Config;
  let buf = render(&mut app);
  assert_present(&buf, "Settings", "settings panel title (renamed from Configuration)");
  assert_present(&buf, "[worktree]", "grouped section heading");
  assert_present(&buf, "worktree.base", "resolved config key");
  assert_present(&buf, "repo", "source column marker");
  assert_present(&buf, "default", "default source marker");
}

#[test]
fn settings_keys_tab_renders_scopes_bindings_and_capture_input() {
  use gwm::config::ConfigSource;
  use gwm::tui::keymap::{Action, Keymap};
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::{build_key_rows, KeyTarget, SettingsTab};

  let (_dir, mut app) = make_app();
  app.config_panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  app.config_panel.tab = SettingsTab::Keys;
  app.view = View::Config;

  let buf = render(&mut app);
  assert_present(&buf, "Keys", "the Keys tab label in the strip");
  assert_present(&buf, "[global]", "global scope heading");
  // `down` is the first global action, so it sits in the initial viewport
  // (later rows like `quit` need a scroll, exercised by the capture below).
  assert_present(&buf, "down", "the first global action slug");

  // Arm a capture on the `quit` row → selecting it scrolls it into view and
  // its key column becomes a `[ … ]` input.
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Quit))
    .unwrap();
  app.config_panel.selected = idx;
  app.config_panel.begin_capture();
  let buf = render(&mut app);
  assert_present(&buf, "[ ", "capture input box rendered for the selected row");
}

#[test]
fn settings_all_tab_horizontal_pan_reveals_the_last_column_past_the_scrollbar() {
  use gwm::config::{ConfigRow, ConfigSource};
  use gwm::tui::SettingsTab;

  // Review P3: when a vertical scrollbar reserves the rightmost column, the
  // horizontal pan bound must account for the narrower text area so the
  // final cell of a long line is still reachable. A long first row (ending
  // in a unique marker) plus many filler rows forces both a vertical
  // scrollbar and a horizontal overflow.
  let (_dir, mut app) = make_app();
  let mut rows = vec![ConfigRow {
    key: "tui.long".into(),
    value: format!("{}ZEND", "v".repeat(120)),
    source: ConfigSource::Repo,
  }];
  for i in 0..40 {
    rows.push(ConfigRow {
      key: format!("tui.k{i}"),
      value: "x".into(),
      source: ConfigSource::Default,
    });
  }
  app.config_panel.rows = rows;
  app.config_panel.tab = SettingsTab::All;
  app.view = View::Config;
  app.config_panel.x_scroll = u16::MAX; // clamps to max_x_scroll on render

  let buf = render(&mut app);
  assert_present(
    &buf,
    "ZEND",
    "horizontal pan must reveal the final cell even with the scrollbar column reserved",
  );
}

#[test]
fn settings_panel_theme_tab_renders_tabs_layer_and_editable_field() {
  // Issue #279: the default Theme tab shows the category tab strip, the
  // edit-layer indicator, and the editable theme-preset field with its
  // current value.
  let (_dir, mut app) = make_app();
  app.view = View::Config;
  let buf = render(&mut app);
  assert_present(&buf, "Settings", "settings panel title");
  // Tab strip.
  assert_present(&buf, "Theme", "Theme tab label");
  assert_present(&buf, "Worktree", "Worktree tab label");
  assert_present(&buf, "TUI", "TUI tab label");
  // The active layer reads as a plain subtitle (the switch key lives in the
  // footer hints, not the subtitle).
  assert_present(&buf, "project (.gwm.toml)", "edit-layer subtitle");
  assert_present(&buf, "theme preset", "editable theme-preset field label");
}

#[test]
fn open_menu_modal_renders_title_and_targets() {
  let (_dir, mut app) = make_app();
  app.enter_open_menu();
  assert_eq!(app.view, View::OpenMenu);
  let buf = render(&mut app);
  assert_present(&buf, "Open in Browser", "open menu title");
  // The two link targets, each `" key  label "`.
  assert_present(&buf, "Issue", "open menu issue target");
  assert_present(&buf, "Pull Request", "open menu pr target");
}

#[test]
fn link_prompt_choose_target_renders_title_and_targets() {
  let (_dir, mut app) = make_app();
  app.enter_link_prompt();
  assert_eq!(app.view, View::LinkPrompt);
  // Default stage is ChooseTarget — the selectable target picker.
  let buf = render(&mut app);
  assert_present(&buf, "Link", "link prompt title");
  assert_present(&buf, "Issue", "link prompt issue target");
  assert_present(&buf, "Pull Request", "link prompt pr target");
}

#[test]
fn link_prompt_input_number_renders_prompt() {
  let (_dir, mut app) = make_app();
  app.enter_link_prompt();
  // Advance to the number-entry stage by committing a target.
  app.link_prompt_choose(LinkTarget::Issue);
  let buf = render(&mut app);
  // Title is "type the issue number"; the body prompt is "issue #".
  assert_present(&buf, "issue", "link prompt number title");
  assert_present(&buf, "#", "link prompt number field");
}

#[test]
fn command_palette_modal_renders_title_and_entries() {
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  assert_eq!(app.view, View::CommandPalette);
  let buf = render(&mut app);
  assert_present(&buf, "Command Palette", "palette title");
  // The palette has no buttons; its observable surface is the entry
  // list. With an empty query it lists every command, so a real entry
  // from `palette_entries()` must be painted — `create` is the first
  // entry's name. Asserting the input sigil alone would stay green even
  // if every command row regressed (clipped, empty, or skipped).
  assert_present(&buf, "create", "palette lists the 'create' command entry");
  assert!(
    !buffer_contains(&buf, "no matching command"),
    "palette with empty query must list commands, not the empty notice — buffer rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

#[test]
fn command_palette_renders_the_input_above_the_matches() {
  // Issue #262: the palette input moved to the top (input-first), above the
  // matches list. Type a query that still matches a command, then assert the
  // typed text's row sits above the matched command's row.
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  // A distinctive query that is a subsequence of `create` so a match remains.
  for c in "cre".chars() {
    app.palette.push_char(c);
  }
  let buf = render(&mut app);
  let rows = row_strings(&buf);
  let input_row = rows
    .iter()
    .position(|r| r.contains("cre"))
    .expect("the typed query must render in the input field");
  let match_row = rows
    .iter()
    .position(|r| r.contains("create"))
    .expect("a matching command row must render");
  assert!(
    input_row < match_row,
    "the palette input must render above the matches list (input-first), \
     input_row={input_row} match_row={match_row} — buffer rows:\n{}",
    rows.join("\n")
  );
}

#[test]
fn exec_picker_modal_renders_title_profiles_and_hints() {
  // #325: the exec picker lists `[exec.profiles]` and offers run / cancel.
  let (dir, _) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[exec.profiles.build]\ncommand = [\"cargo\", \"build\"]\n",
  )
  .unwrap();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  pin_bordered(&mut app);
  app.sidebar.open = false;
  app.enter_exec_picker();
  assert_eq!(app.view, View::ExecPicker);
  let buf = render(&mut app);
  assert_present(&buf, "Run an exec profile", "exec picker title (capitalised)");
  assert_present(&buf, "build", "exec profile name");
  assert_present(&buf, "run", "exec run hint");
}

#[test]
fn clean_modal_renders_title_report_and_hints() {
  // #325: the clean overlay reports the gated reclaim and offers reclaim /
  // cancel. The scan shells out to `git check-ignore` against the real temp
  // repo, so it is deterministic though not offline like the other modals.
  let (dir, _) = init_repo();
  std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
  std::fs::create_dir(dir.path().join("target")).unwrap();
  std::fs::write(dir.path().join("target").join("blob"), vec![0u8; 4096]).unwrap();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  pin_bordered(&mut app);
  app.sidebar.open = false;
  app.enter_clean_overlay();
  assert_eq!(app.view, View::CleanReport);
  let buf = render(&mut app);
  assert_present(&buf, "Reclaim build artifacts", "clean overlay title (capitalised)");
  assert_present(&buf, "target", "clean artifact name");
  assert_present(&buf, "total", "clean total line");
  // #335 review: the right-aligned size column must fit the drawable area
  // (width − borders − padding), so the unit suffix is never clipped.
  assert_present(&buf, "KiB", "size unit not clipped on the right edge");
}

// ---------------------------------------------------------------------------
// Settings panel: the selected field must be on screen (Codex review #368 P2)
// ---------------------------------------------------------------------------

/// Render `app` at an explicit size — the module default (100×40) is roomy
/// enough to hide every clipping bug, and this case is about a short terminal.
fn render_at(app: &mut App, w: u16, h: u16) -> Buffer {
  let backend = TestBackend::new(w, h);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, app)).unwrap();
  terminal.backend().buffer().clone()
}

#[test]
fn settings_tui_tab_keeps_the_selected_field_visible_on_a_short_terminal() {
  // The Settings modal is 60% of the terminal height, so a 24-line terminal —
  // an entirely ordinary size — leaves only a handful of body lines. The TUI
  // tab now has 8 fields, and the renderer only scrolled the selection into
  // view for the Keys tab: every other tab moved `selected` without touching
  // `scroll`, so the last fields could be selected, cycled and edited while
  // off screen. The user edits a setting they cannot see.
  //
  // #367 added the 8th field and is what pushed this over the edge on a
  // 24-line terminal, but the gap is older — the guard is written against the
  // property (selected ⇒ visible) rather than against a field count, so it
  // holds as fields come and go.
  use gwm::tui::SettingsTab;

  let (_dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Tui;

  let fields = SettingsTab::Tui.fields();
  for (idx, field) in fields.iter().enumerate() {
    app.config_panel.selected = idx;
    let buf = render_at(&mut app, 100, 24);
    let rows = row_strings(&buf);
    let label = field.label();
    assert!(
      rows.iter().any(|r| r.contains(label)),
      "selected field {field:?} ({label:?}) is off screen on a 24-line terminal — \
       the user can edit a row they cannot see.\nRendered:\n{}",
      rows.join("\n")
    );
  }
}

/// The hint row has to describe the mode that is on screen. Free-form has a
/// single field and no type selector, so `field` and `type` advertise verbs
/// that do nothing there — and `toggle_mode`, the only way between the two
/// modes, was advertised by neither. Caught on the install-and-validate pass:
/// the verb existed in the keymap and the help overlay but never reached the
/// footer, which is where a user actually looks.
#[test]
fn the_create_hint_row_describes_the_mode_that_is_on_screen() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.enter_create();
  let buf = render(&mut app);
  assert_present(&buf, "free-form", "structured mode must advertise the way across");
  assert_present(&buf, "field", "structured mode still rotates fields");

  // Through the real key route, not `toggle_mode()` directly: the status
  // line is part of what the user reads, and only the key handler updates
  // it. Toggling the form behind its back would leave the structured
  // message on screen and hide exactly the kind of mismatch this pins.
  app.handle_create_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
  let buf = render(&mut app);
  assert_present(&buf, "structured", "free-form must advertise the way back");

  // Scoped to the overlay's own hint row — the row inside the modal box that
  // carries `submit`. The status line legitimately says "back to
  // type/issue/desc" as prose, and the worktree table behind the modal can
  // hold anything; neither is the row under test.
  let hint_row = row_strings(&buf)
    .into_iter()
    .find(|r| r.contains("submit") && r.contains('│'))
    .expect("the create overlay renders a hint row inside its box");
  for absent in ["field", "type"] {
    assert!(
      !hint_row.contains(absent),
      "`{}` names a verb that does nothing in free-form mode — hint row: {}",
      absent,
      hint_row.trim()
    );
  }
}

/// Issue #416: free-form mode presents a single `Name` field and drops the
/// inputs it has no notion of — the branch type selector and the issue
/// number. Showing them inert would suggest they still apply.
#[test]
fn create_modal_in_freeform_mode_shows_only_the_name_field() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_form.toggle_mode();
  let buf = render(&mut app);

  assert_present(&buf, "free-form", "the title states the active mode");
  assert_present(&buf, "Name", "the free-form name field");
  // The preview rows survive — they are what makes the resolved branch and
  // directory legible while typing.
  assert_present(&buf, "Branch", "branch preview label");
  assert_present(&buf, "Dir", "dir preview label");

  for absent in ["Issue", "Type"] {
    assert!(
      !buffer_contains(&buf, absent),
      "`{}` has no meaning in free-form mode and must not be rendered — buffer rows:\n{}",
      absent,
      row_strings(&buf).join("\n")
    );
  }
}

/// Issue #417, found by validating the branch by hand. Both live previews
/// hardcoded `<type>/#<issue>-<desc>` and `<type>-<issue>-<desc>` instead of
/// expanding the repo's own `branch_pattern` / `path_pattern`, so under a
/// custom pattern they promised names the repo would never create.
///
/// The rename case was the loud one: with `branch_pattern = "feat/#{issue}-{desc}"`,
/// picking `docs` in the type selector previewed `docs/#42-x` while submit
/// would have written `feat/#42-x`, since the pattern has no `{type}` to write
/// into. A preview that disagrees with what submitting does is worse than no
/// preview at all.
#[test]
fn the_create_preview_expands_this_repo_s_own_patterns() {
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "wt/{type}-{issue}-{desc}".into();
  app.config.worktree.path_pattern = "{issue}_{desc}".into();
  app.enter_create();
  app.create_form.issue = "42".into();
  app.create_form.desc = "cache".into();
  let type_str = app.branch_types[app.create_form.type_index].name.clone();

  let buf = render(&mut app);

  assert_present(
    &buf,
    &format!("wt/{}-42-cache", type_str),
    "the branch preview must come from branch_pattern",
  );
  assert_present(&buf, "42_cache", "the dir preview must come from path_pattern");
}

#[test]
fn the_statusbar_follows_the_rename_modal_s_mode() {
  // Codex review on PR #485. The modal's own footer tracked the mode while
  // `hint_context()` still returned `Rename` unconditionally for `View::Edit`,
  // so the statusbar behind it kept advertising `field` and `type` — two verbs
  // free-form mode neither renders nor can act on, and this codebase's rule is
  // to never name a key that does nothing. The create overlay already solves
  // it with one source both read (#416); rename gets the same.
  use gwm::tui::state::create_form::Mode;
  let (_dir, mut app) = make_app();
  let mut wt = deletable_worktree("spike-redis");
  wt.branch = Some("spike-redis".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.create_form.mode, Mode::Freeform);

  let buf = render(&mut app);
  assert_absent(&buf, "↑/↓ type", "free-form has no type selector to advertise");
  // The toggle hint names its *target* mode, so free-form advertises the way back.
  assert_present(
    &buf,
    "structured",
    "the toggle is the one verb the visible inputs cannot suggest",
  );

  app.create_form.toggle_mode();
  let buf = render(&mut app);
  assert_present(&buf, "↑/↓ type", "structured mode does have a type selector");
  assert_present(&buf, "free-form", "and advertises the way across");
}

#[test]
fn the_rename_preview_shows_a_free_form_name_verbatim() {
  // Issue #479. In free-form mode no pattern is expanded at all: the branch IS
  // the name, and the directory is that name flattened. A preview that kept
  // expanding `branch_pattern` here would show a branch the submit will not
  // write, which is exactly the defect found by hand on #476 one mode over.
  // Preview and submit therefore derive from the same `WorktreeName`.
  use gwm::tui::state::create_form::Mode;
  let (_dir, mut app) = make_app();
  let mut wt = deletable_worktree("spike-redis");
  wt.branch = Some("spike-redis".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "the form must open: {}", app.status);
  assert_eq!(app.create_form.mode, Mode::Freeform);

  // A `/` is legal in a free-form branch and has to flatten in the directory.
  app.create_form.name = "spike/valkey".into();
  let buf = render(&mut app);

  assert_present(&buf, "spike/valkey", "the branch preview is the name, verbatim");
  assert_present(&buf, "spike-valkey", "the dir preview is the name, flattened");
  assert!(
    !buffer_contains(&buf, "#0-"),
    "no pattern is expanded in free-form mode — buffer rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

#[test]
fn the_rename_pr_warning_fires_on_a_free_form_rename_too() {
  // The warning added for #481 compares the branch the submit would write
  // against the current one. Free-form renames change the branch just as much
  // as structured ones, so the comparison has to be fed the free-form target
  // rather than a pattern expansion that never matches anything.
  use gwm::github::PrState;
  let (_dir, mut app) = make_app();
  let mut wt = deletable_worktree("spike-redis");
  wt.branch = Some("spike-redis".into());
  wt.link.pr = Some(77);
  wt.pr_state = Some(PrState::Open);
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();

  // Unchanged name: nothing is being renamed, so nothing is being closed.
  let buf = render(&mut app);
  assert_absent(&buf, "closes PR #77", "an unchanged name renames nothing");

  app.create_form.name = "spike-valkey".into();
  let buf = render(&mut app);
  assert_present(&buf, "closes PR #77", "a free-form rename closes the PR just the same");
}

#[test]
fn the_rename_preview_expands_this_repo_s_own_patterns() {
  let (_dir, mut app) = make_app();
  // Freezes the type: whatever the selector says, the branch stays `feat/`.
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  let mut wt = deletable_worktree("login");
  wt.branch = Some("feat/#42-login".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "the form must open: {}", app.status);

  // Move the type selector somewhere the pattern cannot write.
  app.create_form.type_index = app
    .branch_types
    .iter()
    .position(|t| t.name == "docs")
    .expect("docs is configured");

  let buf = render(&mut app);

  assert_present(
    &buf,
    "feat/#42-login",
    "the branch preview must be what branch_pattern would write",
  );
  assert!(
    !buffer_contains(&buf, "docs/#42-login"),
    "the preview must not offer a branch the pattern cannot write — buffer rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

/// The refusal has to fit. `TestBackend` hard-clips, so asserting the whole
/// sentence is the clip guard: the first version ran to 87 characters and the
/// modal cut it at "has no {type} to write," leaving the user with half a
/// reason and no value.
#[test]
fn the_rename_refusal_fits_in_the_modal() {
  // Both patterns freeze the type, which is what the refusal is now about: a
  // `path_pattern` that writes `{type}` gives the new value a destination and
  // the edit is allowed instead.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  app.config.worktree.path_pattern = "fix-{issue}-{desc}".into();
  let mut wt = deletable_worktree("login");
  wt.branch = Some("feat/#42-login".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "the form must open: {}", app.status);

  app.create_form.type_index = app
    .branch_types
    .iter()
    .position(|t| t.name == "docs")
    .expect("docs is configured");
  app.submit_edit_worktree().expect("the refusal is a form failure");
  let failure = app.edit_failure.clone().expect("refused");

  let buf = render(&mut app);
  assert_present(&buf, &failure, "the whole refusal, not the part that fits");
}

/// Issue #481. The remote half of a rename is `git push --atomic origin :<old>
/// <new>:<new>`, a delete plus a create, and GitHub closes a pull request whose
/// head branch is renamed. That is not something gwm can route around: GitHub's
/// own rename endpoint retargets a PR whose *base* is the renamed branch and
/// closes one whose head it is, and a worktree branch is always the head of its
/// own PR. So the only honest protection is to say so before the push.
///
/// Live rather than one-shot, so it appears the moment the form would write a
/// different branch and goes away when the user reverts.
#[test]
fn the_rename_modal_warns_before_a_branch_change_closes_an_open_pr() {
  let (_dir, mut app) = make_app();
  let mut wt = deletable_worktree("login");
  wt.branch = Some("feat/#42-login".into());
  wt.link.pr = Some(476);
  wt.pr_state = Some(gwm::github::PrState::Open);
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "the form must open: {}", app.status);

  // Untouched: the submit would rewrite the same branch, so nothing is at risk.
  let buf = render(&mut app);
  assert_absent(&buf, "closes PR #476", "an unchanged branch renames nothing");

  app.create_form.desc = "login-v2".into();
  let buf = render(&mut app);
  assert_present(
    &buf,
    "closes PR #476",
    "a branch change deletes the remote branch, which closes the PR",
  );
}

/// The counterpart, and the reason the warning is tied to the *branch* rather
/// than to the rename: an edit that only moves the directory returns from
/// `rename_worktree` before it touches a single ref, local or remote, so the
/// PR is never in danger and saying otherwise would train the user to ignore
/// the line.
#[test]
fn the_rename_modal_stays_quiet_when_only_the_directory_moves() {
  let (_dir, mut app) = make_app();
  // Freezes every branch segment, so the branch cannot change; `path_pattern`
  // still writes the description, so the directory can.
  app.config.worktree.branch_pattern = "feat/#42-login".into();
  app.config.worktree.path_pattern = "{type}-{issue}-{desc}".into();
  let mut wt = deletable_worktree("login");
  wt.branch = Some("feat/#42-login".into());
  wt.link.pr = Some(476);
  wt.pr_state = Some(gwm::github::PrState::Open);
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "the form must open: {}", app.status);

  app.create_form.desc = "login-v2".into();
  let buf = render(&mut app);
  assert_absent(&buf, "closes PR", "a path-only edit never touches a ref");
}

/// A merged or closed PR has nothing left to lose, so warning about it would be
/// noise on the common case of renaming a branch after its PR landed.
#[test]
fn the_rename_modal_stays_quiet_about_a_pr_that_is_already_closed() {
  let (_dir, mut app) = make_app();
  let mut wt = deletable_worktree("login");
  wt.branch = Some("feat/#42-login".into());
  wt.link.pr = Some(476);
  wt.pr_state = Some(gwm::github::PrState::Merged);
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();

  app.create_form.desc = "login-v2".into();
  let buf = render(&mut app);
  assert_absent(&buf, "closes PR", "a merged PR cannot be closed by a rename");
}

/// Issue #418. The overlay drew the canonical `Type` / `Issue` / `Desc` triple
/// whatever the repo's patterns said, so a convention that writes no issue
/// number was still shown a field for one — and `BranchSpec::validate_against`
/// then refused to submit until it was filled with a value the patterns
/// discard. The field set now comes from the patterns.
#[test]
fn the_create_modal_omits_a_field_the_patterns_never_write() {
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "{type}/{desc}".into();
  app.config.worktree.path_pattern = "{type}-{desc}".into();
  app.config.worktree.base = "/tmp/wt".into();
  app.apply_create_form_fields();
  app.enter_create();
  let buf = render(&mut app);

  assert_present(&buf, "Type", "the pattern writes a type");
  assert_present(&buf, "Desc", "and a description");
  assert!(
    !buffer_contains(&buf, "Issue"),
    "no pattern carries {{issue}}, so no Issue field — buffer rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

/// `base` feeds the triple too (`BranchSpec::worktree_path` expands it), so a
/// segment only `base` carries still names a real directory on disk and still
/// has to be collected. A field set derived from the two obvious patterns
/// would have dropped it.
#[test]
fn the_create_modal_keeps_a_field_only_the_base_path_writes() {
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "{type}/{desc}".into();
  app.config.worktree.path_pattern = "{type}-{desc}".into();
  app.config.worktree.base = "/tmp/wt/{issue}".into();
  app.apply_create_form_fields();
  app.enter_create();
  let buf = render(&mut app);

  assert_present(&buf, "Issue", "base writes the issue number into the path");
}

/// The rename modal draws the same set from the same place, so the two cannot
/// disagree about which inputs exist — they used to hardcode the triple twice.
#[test]
fn the_rename_modal_omits_the_same_field_the_create_modal_does() {
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "{type}/{desc}".into();
  app.config.worktree.path_pattern = "{type}-{desc}".into();
  app.config.worktree.base = "/tmp/wt".into();
  app.apply_create_form_fields();
  let mut wt = deletable_worktree("foo");
  wt.branch = Some("feat/my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, gwm::tui::View::Edit, "the rename form must open");
  let buf = render(&mut app);

  assert_present(&buf, "Rename", "the rename modal is up");
  assert_present(&buf, "Desc", "the pattern writes a description");
  assert!(
    !buffer_contains(&buf, "Issue"),
    "no pattern carries {{issue}}, so no Issue field — buffer rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

/// Codex review on PR #492. Making the field set dynamic made two hint rows
/// inert, and this codebase's rule is to never name a key that does nothing
/// (the reason free-form drops the same two rows since #416). Introduced by
/// #418, not pre-existing: before it, the structured form always presented the
/// full triple, so both rows were always accurate.
#[test]
fn the_hint_row_drops_the_type_selector_when_no_pattern_carries_one() {
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "#{issue}-{desc}".into();
  app.config.worktree.path_pattern = "{issue}-{desc}".into();
  app.config.worktree.base = "/tmp/wt".into();
  app.apply_create_form_fields();
  app.enter_create();
  let buf = render(&mut app);

  assert_absent(&buf, "↑/↓", "no type selector is rendered, so its keys do nothing");
  assert_present(&buf, "field", "two fields remain, so Tab still moves");
}

/// The other row: one field means `next_field` rotates within a one-element
/// list, so Tab does nothing either.
#[test]
fn the_hint_row_drops_the_field_verb_when_the_pattern_presents_one_field() {
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "wt/{desc}".into();
  app.config.worktree.path_pattern = "{desc}".into();
  app.config.worktree.base = "/tmp/wt".into();
  app.apply_create_form_fields();
  app.enter_create();
  assert_eq!(app.create_form.fields().len(), 1);
  let buf = render(&mut app);

  assert_absent(&buf, "↑/↓", "no type selector either");
  assert_absent(&buf, "field", "one field, so Tab is a no-op");
  assert_present(&buf, "submit", "the verbs that still work stay");
}

/// And the canonical pattern is unchanged, so the fix cannot be a blanket
/// removal of the two rows.
#[test]
fn the_hint_row_is_unchanged_on_the_canonical_pattern() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  let buf = render(&mut app);

  assert_present(&buf, "↑/↓", "the default pattern has a type selector");
  assert_present(&buf, "field", "and three fields to move between");
}

/// Codex review on PR #492, fifth pass, and the third finding of one class:
/// a user-facing string that names a field the patterns do not present. Three
/// passes each named one string, so this stops enumerating strings and
/// enumerates the **property** instead.
///
/// **Invariant: on a repo whose patterns omit a segment, nothing the create or
/// rename surface renders may name that segment.** That covers the modal body,
/// its footer, and the statusbar behind it in one assertion, and it holds for
/// strings nobody has written yet.
#[test]
fn no_create_surface_names_a_segment_the_patterns_omit() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::state::create_form::Mode;

  // Each pattern set omits exactly one segment, so no single hardcoded string
  // can pass all three.
  for (branch, path, omitted) in [
    ("{type}/{desc}", "{type}-{desc}", "issue"),
    ("#{issue}-{desc}", "{issue}-{desc}", "type"),
    ("{type}/#{issue}", "{type}-{issue}", "desc"),
  ] {
    for freeform in [false, true] {
      let (_dir, mut app) = make_app();
      app.config.worktree.branch_pattern = branch.into();
      app.config.worktree.path_pattern = path.into();
      app.config.worktree.base = "/tmp/wt".into();
      app.apply_create_form_fields();
      app.enter_create();
      if freeform {
        // Through the key handler, not `CreateForm::toggle_mode`: the status
        // line is written by the handler, so calling the form method directly
        // leaves the very string this test is about unexercised. (It did, on
        // the first draft, and the test passed with the defect restored.)
        app.handle_create_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(app.create_form.mode, Mode::Freeform);
      }
      let buf = render(&mut app);

      assert!(
        !buffer_contains(&buf, omitted),
        "`{}` / `{}` (free-form: {}) writes no {{{}}}, but the surface names it — buffer rows:\n{}",
        branch,
        path,
        freeform,
        omitted,
        row_strings(&buf).join("\n")
      );
    }
  }
}

/// The degenerate end of the same rule: a pattern set with no editable token
/// presents no field, so there is none to name. `last_field` falls back to
/// `Type`, which the renderer is not drawing.
#[test]
fn an_all_literal_pattern_set_names_no_field_at_all() {
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "wip".into();
  app.config.worktree.path_pattern = "wip".into();
  app.config.worktree.base = "/tmp/wt".into();
  app.apply_create_form_fields();
  app.enter_create();
  assert!(app.create_form.fields().is_empty());

  assert_eq!(app.status, "enter: submit · esc: cancel");
  let buf = render(&mut app);
  for absent in ["Type", "Issue", "Desc"] {
    assert!(
      !buffer_contains(&buf, absent),
      "`{}` is not presented — buffer rows:\n{}",
      absent,
      row_strings(&buf).join("\n")
    );
  }
}

// ---------------------------------------------------------------------------
// Titles ride the border (issue #549)
// ---------------------------------------------------------------------------

/// The row a title landed on, and whether that row also carries the modal's
/// top rule. A title drawn *in* the rule shares its row with the corner and
/// horizontal glyphs; a title on its own content row does not.
fn title_row_has_rule(buf: &Buffer, needle: &str) -> Option<bool> {
  row_strings(buf)
    .into_iter()
    .find(|row| row.contains(needle))
    .map(|row| row.contains('╭') && row.contains('─'))
}

#[test]
fn modal_titles_ride_the_top_rule_rather_than_a_content_row() {
  // Issue #549: the title used to be a centred row inside the frame,
  // followed by a blank spacer — four rows of chrome before a modal's
  // first line of content, two of them carrying text ratatui draws in the
  // rule for free.
  //
  // Checked structurally rather than by counting rows: the title's row
  // must also carry the rounded top-left corner, which only the border
  // draws. A regression that put the title back on its own line would
  // find it on a row with no rule on it.
  let (_dir, mut app) = make_app();

  app.view = View::Create;
  let buf = render(&mut app);
  assert_eq!(
    title_row_has_rule(&buf, "New Worktree"),
    Some(true),
    "the create modal's title must sit in the top rule — rows:\n{}",
    row_strings(&buf).join("\n")
  );

  let (_dir2, mut app) = make_app();
  app.view = View::OpenMenu;
  let buf = render(&mut app);
  assert_eq!(
    title_row_has_rule(&buf, "Open in Browser"),
    Some(true),
    "the open-menu modal's title must sit in the top rule — rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

#[test]
fn modal_frames_are_not_taller_than_their_content() {
  // Validation feedback + Codex review on PR #546: moving the title into
  // the top rule (#549) removed two rows from every modal's `lines`, but
  // three call sites still added `+ 2 /* title */` to their height. The
  // frame stayed two rows too tall, so the hint row floated with dead
  // space under it.
  //
  // Checked as "no blank row between the last content row and the bottom
  // rule": that is what the reader actually sees, and it holds whatever
  // the sizing formula happens to be.
  let (_dir, mut app) = make_app();
  app.view = View::DetailOverlay;
  app.open_agent_overlay();
  let buf = render(&mut app);

  let rows = row_strings(&buf);
  let bottom = rows
    .iter()
    .rposition(|r| r.contains('╰'))
    .expect("the modal must draw a bottom rule");
  let last_content = rows[..bottom]
    .iter()
    .rposition(|r| {
      // A content row inside the frame: has text between the side rules.
      r.contains('│') && r.trim_matches(|c| c == '│' || c == ' ').len() > 1
    })
    .expect("the modal must have content");

  // border row, one padding row, then the bottom rule — nothing else.
  assert_eq!(
    bottom - last_content,
    2,
    "the frame must end one padding row after its last content row, got {} rows of slack — rows:\n{}",
    bottom - last_content - 1,
    rows.join("\n")
  );
}

// ---------------------------------------------------------------------------
// Responsive sizing matrix (issue #550)
// ---------------------------------------------------------------------------

/// The modal's rect, located by the rounded corners its frame draws
/// (`overlay_block` uses `BorderType::Rounded`). Returns `(x, y, w, h)`.
///
/// The oracle is only sound while the surfaces *behind* a modal draw no
/// rounded corner of their own — `the_background_paints_no_rounded_corner`
/// below is what keeps that honest.
fn modal_rect(buf: &Buffer) -> Option<(u16, u16, u16, u16)> {
  let area = *buf.area();
  let mut top_left = None;
  // From column 1: a modal is centred, so its left rule never sits flush
  // with the terminal edge, while a background pane's always does. Under
  // `[tui] layout = "bordered"` the sidebar's sections are rounded too
  // (#594), and scanning from column 0 would return one of those instead:
  // the whole matrix would then measure the pane behind the modal.
  'outer: for y in 0..area.height {
    for x in 1..area.width {
      if buf[(x, y)].symbol() == "╭" {
        top_left = Some((x, y));
        break 'outer;
      }
    }
  }
  let (x0, y0) = top_left?;
  let x1 = (x0 + 1..area.width)
    .find(|&x| buf[(x, y0)].symbol() == "╮")
    .unwrap_or(x0);
  let y1 = (y0 + 1..area.height)
    .find(|&y| buf[(x0, y)].symbol() == "╰")
    .unwrap_or(y0);
  Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// The rendered rows *inside* the modal frame. Needles asserted against the
/// whole buffer can be satisfied by the statusbar or the worktree table behind
/// the modal; these rows can only come from the modal itself.
fn modal_rows(buf: &Buffer) -> Vec<String> {
  let Some((x, y, w, h)) = modal_rect(buf) else {
    return Vec::new();
  };
  (y..(y + h).min(buf.area().height))
    .map(|row| {
      (x..(x + w).min(buf.area().width))
        .map(|col| buf[(col, row)].symbol())
        .collect()
    })
    .collect()
}

/// Byte offset of `needle` in `row`, for slicing what precedes it. Panics
/// when absent, which the caller has already asserted against.
fn tail_of(row: &str, needle: &str) -> usize {
  row.find(needle).expect("needle present in row")
}

fn modal_width_at(setup: &dyn Fn() -> (tempfile::TempDir, App), w: u16, h: u16) -> u16 {
  let (_dir, mut app) = setup();
  let buf = render_at(&mut app, w, h);
  modal_rect(&buf)
    .unwrap_or_else(|| panic!("no modal rendered at {w}x{h} — rows:\n{}", row_strings(&buf).join("\n")))
    .2
}

type ModalSetup = Box<dyn Fn() -> (tempfile::TempDir, App)>;

/// Every modal, with the width it must resolve to at the advertised 80-column
/// floor and on an ultra-wide terminal. Exact values, deliberately: this is a
/// characterisation matrix, so a refactor that moves a number has to say so.
fn sizing_matrix() -> Vec<(&'static str, ModalSetup, u16, u16)> {
  vec![
    (
      "help",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.enter_help();
        (d, a)
      }) as ModalSetup,
      64,
      96,
    ),
    (
      "config-panel",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.enter_config_panel();
        (d, a)
      }),
      64,
      96,
    ),
    (
      "command-palette",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.open_command_palette();
        (d, a)
      }),
      64,
      96,
    ),
    (
      "create",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.enter_create();
        (d, a)
      }),
      56,
      72,
    ),
    (
      "edit/rename",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.worktrees.push(deletable_worktree("feat-550-rename"));
        a.list_state.select(Some(a.worktrees.len() - 1));
        a.enter_edit_worktree();
        (d, a)
      }),
      56,
      72,
    ),
    (
      "confirm",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.worktrees.push(deletable_worktree("feat-550-one"));
        a.list_state.select(Some(a.worktrees.len() - 1));
        a.enter_confirm_delete();
        (d, a)
      }),
      64,
      88,
    ),
    (
      "open-menu",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.enter_open_menu();
        (d, a)
      }),
      64,
      72,
    ),
    (
      "link-prompt",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.enter_link_prompt();
        (d, a)
      }),
      64,
      72,
    ),
    (
      "exec-picker",
      Box::new(|| {
        let (d, _) = init_repo();
        std::fs::write(
          d.path().join(".gwm.toml"),
          "[exec.profiles.build]\ncommand = [\"cargo\", \"build\"]\n",
        )
        .unwrap();
        let mut a = App::new_at_layered(Some(d.path()), None).unwrap();
        pin_bordered(&mut a);
        a.sidebar.open = false;
        a.enter_exec_picker();
        (d, a)
      }),
      72,
      88,
    ),
    (
      "detail/agents",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.worktrees.push(deletable_worktree("feat-550-agents"));
        a.list_state.select(Some(a.worktrees.len() - 1));
        a.open_agent_overlay();
        (d, a)
      }),
      72,
      88,
    ),
    // Text canvases: the bootstrap report, the log transcript and the note
    // editor render arbitrary external text, so they keep spending a bare
    // percentage of the frame rather than going through `modal_width`. They
    // have NO floor — the report's 64 at 80 columns is `80 * 80 / 100`, which
    // merely happens to land on the same number the bounded surfaces are
    // floored at. Pinned here anyway: an exemption nobody measures is how a
    // matrix goes green while missing a surface.
    (
      "report",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.report = Some(BootstrapReport {
          steps: vec![StepResult::ok("copy env file"), StepResult::skipped("npm i", "no pkg")],
        });
        a.view = View::Report;
        (d, a)
      }),
      64,
      160,
    ),
    (
      "command-logs",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.view = View::CommandLogs;
        (d, a)
      }),
      72,
      180,
    ),
    (
      "working-tree",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.view = View::WorkingTree;
        (d, a)
      }),
      72,
      180,
    ),
    (
      "commits",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.enter_commits();
        settle_commits(&mut a);
        (d, a)
      }),
      72,
      180,
    ),
    (
      "note-editor",
      Box::new(|| {
        let (d, mut a) = make_app();
        a.worktrees.push(deletable_worktree("feat-550-note"));
        a.list_state.select(Some(a.worktrees.len() - 1));
        a.open_note_editor();
        (d, a)
      }),
      64,
      160,
    ),
  ]
}

#[test]
fn the_background_paints_no_rounded_corner_a_modal_could_be_mistaken_for() {
  // The matrix below finds each modal by its rounded top-left corner, from
  // column 1 (see `modal_rect`). That only works while nothing behind the
  // modal draws one there. If the worktree table or the sidebar ever grows
  // a rounded frame off the left edge, every measurement below silently
  // starts describing the wrong rect. Prove the oracle, then use it.
  //
  // The sidebar's own sections ARE rounded in the boxed layout, which is
  // exactly why the claim is about column 1 onwards and not about the
  // buffer: they sit flush at column 0, and a centred modal never can.
  for sidebar_open in [false, true] {
    let (_dir, mut app) = make_app();
    app.sidebar.open = sidebar_open;
    let buf = render_at(&mut app, 120, 40);
    let corners: Vec<(u16, u16)> = (0..buf.area().height)
      .flat_map(|y| (1..buf.area().width).map(move |x| (x, y)))
      .filter(|&(x, y)| buf[(x, y)].symbol() == "╭")
      .collect();
    assert!(
      corners.is_empty(),
      "View::List (sidebar open = {sidebar_open}) must paint no rounded corner past column 0, found {corners:?}. rows:\n{}",
      row_strings(&buf).join("\n")
    );
  }
}

#[test]
fn every_modal_resolves_to_its_pinned_width_at_the_80_column_floor() {
  // The docs advertise the TUI at 80 columns. Pre-#550 the confirm modal was
  // 49 columns wide there and its hint row read `Enter activa`; help was 48
  // and its rows lost their tail behind the scrollbar. Every *bounded* modal
  // now has a floor, so 80 columns is a size those surfaces were actually
  // sized for. The three text canvases have none: their entries below are the
  // plain percentage, pinned so a change to it still has to be declared.
  for (name, setup, want_at_80, _) in sizing_matrix() {
    assert_eq!(
      modal_width_at(setup.as_ref(), 80, 24),
      want_at_80,
      "{name}: width at the 80-column floor"
    );
  }
}

#[test]
fn every_modal_resolves_to_its_pinned_width_on_an_ultra_wide_terminal() {
  // Pre-#550 the confirm modal was 124 columns wide at 200 for a four-row
  // detail grid, and help / config / palette 120, because they sized on a
  // bare percentage with no ceiling. The three text canvases (report,
  // command-log transcript, note editor) still do, deliberately, and are
  // pinned at that width rather than exempted from the matrix.
  for (name, setup, _, want_at_200) in sizing_matrix() {
    assert_eq!(
      modal_width_at(setup.as_ref(), 200, 80),
      want_at_200,
      "{name}: width on a 200-column terminal"
    );
  }
}

#[test]
fn no_modal_gets_narrower_as_the_terminal_gets_wider() {
  // The render-level companion to
  // `a_modal_never_shrinks_when_the_terminal_grows` in
  // tests/tui_ui_helpers_tests.rs: the helpers being monotonic is worth
  // nothing if a call site reintroduces the seam. Sampled across the
  // 80-column boundary where the old branch lived.
  const WIDTHS: [u16; 8] = [60, 79, 80, 81, 90, 100, 140, 200];
  for (name, setup, _, _) in sizing_matrix() {
    let mut previous = 0u16;
    for w in WIDTHS {
      let got = modal_width_at(setup.as_ref(), w, 40);
      assert!(
        got >= previous,
        "{name}: widening the terminal to {w} cols shrank the modal from {previous} to {got}"
      );
      previous = got;
    }
  }
}

#[test]
fn the_confirm_hint_row_is_not_cut_mid_word_at_80_columns() {
  // The concrete symptom the floor fixes: at 49 columns the confirm modal's
  // hint row was clipped by ratatui to `Enter activa` — no ellipsis, just a
  // half-word. The last hint the row advertises must survive intact.
  let (_dir, mut app) = make_app();
  app.worktrees.push(deletable_worktree("feat-550-hint"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.enter_confirm_delete();

  let buf = render_at(&mut app, 80, 24);
  // Scanned INSIDE the modal rect, not over the whole buffer: the bottom
  // statusbar advertises `Enter activate` too, so a whole-buffer search stays
  // green with the modal's own hint row cut in half.
  let rows = modal_rows(&buf);
  assert!(
    rows.iter().any(|r| r.contains("Enter activate")),
    "the confirm hint row must render its last hint in full at 80 columns — modal rows:\n{}",
    rows.join("\n")
  );
}

#[test]
fn the_confirm_modal_ellipsizes_to_the_width_its_frame_actually_gets() {
  // #550, Codex review (P2). The confirm modal computed its ellipsis budget
  // from `term.width * 62 / 100` by hand: a second copy of the sizing rule.
  // Harmless while the frame was the same bare percentage, but once the
  // policy gained an 88-column ceiling the two drifted, and at 200 columns
  // the text was sized for 124 columns inside an 88-column frame.
  // `ellipsize_middle` then left the string untouched and ratatui clipped it
  // at the border, cutting off the very tail a middle-ellipsis exists to
  // keep. One rule written twice is what let it drift, so the budget now
  // comes from the frame itself.
  let (_dir, mut app) = make_app();
  let mut long = deletable_worktree("feat-550-long");
  long.path =
    PathBuf::from("/tmp/gwm-test/a-deliberately-long-worktree-directory-name/nested/deeper/still-deeper/TAIL-MARKER");
  app.worktrees.push(long);
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.enter_confirm_delete();

  let rows = modal_rows(&render_at(&mut app, 200, 40));
  let path_row = rows.iter().find(|r| r.contains("Path")).unwrap_or_else(|| {
    panic!(
      "the confirm modal must render its Path row — modal rows:\n{}",
      rows.join("\n")
    )
  });
  assert!(
    path_row.contains('…'),
    "a path this long must be middle-ellipsized, not left for ratatui to clip — modal rows:\n{}",
    rows.join("\n")
  );
  assert!(
    path_row.contains("TAIL-MARKER"),
    "the path's tail must survive: the budget has to be the frame's own width — row:\n{path_row}"
  );
}

#[test]
fn the_confirm_modal_ellipsizes_a_wide_glyph_path_by_its_cell_width() {
  // #554, the other half of the same finding. The budget is a rect width,
  // in cells; `ellipsize_middle` counted chars. This path is 52 chars and
  // 77 cells against a value column the 88-column ceiling caps at 67, so
  // the char count called it short and returned it whole. Measured, the
  // row then renders as `Path` alone: label + value overflows the frame,
  // the paragraph wraps (`Wrap { trim: false }`), and the value drops to
  // the next row — the aligned label/value grid #187 built this modal for,
  // gone. Narrower still and ratatui clips the tail outright.
  let (_dir, mut app) = make_app();
  let mut long = deletable_worktree("fix-554-wide");
  long.path = PathBuf::from("/tmp/gwm-test/作業ディレクトリの深い入れ子/さらに深い階層構造の中/TAIL-MARKER");
  app.worktrees.push(long);
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.enter_confirm_delete();

  let rows = modal_rows(&render_at(&mut app, 200, 40));
  let path_row = rows.iter().find(|r| r.contains("Path")).unwrap_or_else(|| {
    panic!(
      "the confirm modal must render its Path row — modal rows:\n{}",
      rows.join("\n")
    )
  });
  assert!(
    path_row.contains('…'),
    "a path this wide must be middle-ellipsized, not left for ratatui to clip — row:\n{path_row}"
  );
  assert!(
    path_row.contains("TAIL-MARKER"),
    "the path's tail must survive a cell-measured budget — row:\n{path_row}"
  );
}

#[test]
fn the_bootstrap_report_shows_a_long_hook_line_on_a_wide_terminal() {
  // #550, Codex review (P2). The report displays arbitrary external text:
  // hook stdout, error messages, paths. `render_section` hard-clips by
  // design (one logical row = one visual row, no wrap, no horizontal
  // scroll), so whatever does not fit the frame is simply unreachable.
  //
  // Capping the report at 96 columns therefore made a hook's error message
  // 64 cells shorter at 200 columns than it had been. Nothing had reported
  // that its 80 % width was a problem; the cap was taste, not a fix, so it
  // is gone and the report sits with the other text canvases.
  let (_dir, mut app) = make_app();
  let detail = format!(
    "error[E0432]: unresolved import `{}` at the end",
    "very::long::module::path".repeat(3)
  );
  assert!(detail.chars().count() > 96, "the fixture must exceed the old cap");
  app.report = Some(BootstrapReport {
    steps: vec![StepResult::skipped("cargo build", &detail)],
  });
  app.view = View::Report;

  let rows = modal_rows(&render_at(&mut app, 200, 40));
  assert!(
    rows.iter().any(|r| r.contains("at the end")),
    "the tail of a long hook line must stay reachable on a wide terminal — modal rows:\n{}",
    rows.join("\n")
  );
}

// ---------------------------------------------------------------------------
// Vertical overflow: a focused field must stay on screen (issue #553)
// ---------------------------------------------------------------------------

/// The modal's rows, with the frame proven *closed*.
///
/// `modal_rect` falls back to `y1 = y0` when it finds no bottom border, which
/// collapses the rect to a single row — every "is this label on screen"
/// assertion below would then fail for the wrong reason. On the short
/// terminals this section renders, that fallback is exactly the plausible
/// failure, so the oracle is checked before it is used.
fn closed_modal_rows(buf: &Buffer, what: &str) -> Vec<String> {
  let rows = modal_rows(buf);
  assert!(
    rows.last().is_some_and(|r| r.starts_with('╰')),
    "{what}: the modal frame must be closed (bottom border found), \
     otherwise `modal_rect` is describing the wrong rect — rows:\n{}",
    rows.join("\n")
  );
  rows
}

/// How a form field's row starts: two columns of indent, the label padded to
/// the 5-cell label column, then the two-column gap before the value. Precise
/// enough that the footer hints — which name `field` and `type` as verbs —
/// cannot satisfy it.
fn field_row_needle(label: &str) -> String {
  format!("  {:<5}  ", label)
}

fn field_label(field: Field) -> &'static str {
  match field {
    Field::Type => "Type",
    Field::Issue => "Issue",
    Field::Desc => "Desc",
    Field::Name => "Name",
  }
}

fn create_form_app() -> (tempfile::TempDir, App) {
  let (d, mut a) = make_app();
  a.enter_create();
  (d, a)
}

fn rename_form_app() -> (tempfile::TempDir, App) {
  let (d, mut a) = make_app();
  a.worktrees.push(deletable_worktree("feat-553-rename"));
  a.list_state.select(Some(a.worktrees.len() - 1));
  a.enter_edit_worktree();
  (d, a)
}

#[test]
fn the_rename_form_keeps_its_desc_field_on_screen_at_16_rows() {
  // The literal case from issue #553. The rename modal sizes to its content
  // (18 rows: preview, blank, the field triple, buttons, hints) and
  // `centered_abs` clamps that to the frame, so at 16 rows ratatui simply cut
  // the tail off. What fell off was `Desc` — an *editable* field, and the one
  // the modal opens focused on (`CreateForm::last_field`). The user types into
  // a row that is not on screen.
  let (_dir, mut app) = rename_form_app();
  let buf = render_at(&mut app, 120, 16);
  let rows = closed_modal_rows(&buf, "rename at 120x16");
  let needle = field_row_needle("Desc");
  assert!(
    rows.iter().any(|r| r.contains(&needle)),
    "the focused Desc field must be rendered at 120x16 — modal rows:\n{}",
    rows.join("\n")
  );
}

#[test]
fn every_focused_form_field_stays_on_screen_on_a_short_terminal() {
  // Written against the property (focused ⇒ visible) over the fields the
  // repo's patterns actually present, not against a hand-typed list of cases:
  // a pattern that drops a field must not be able to leave a hole here.
  //
  // 16 rows is where the rename modal first overflows, 12 is where both forms
  // do by a wide margin.
  for (name, setup) in [
    ("create", create_form_app as fn() -> (tempfile::TempDir, App)),
    ("rename", rename_form_app as fn() -> (tempfile::TempDir, App)),
  ] {
    for h in [16u16, 12] {
      let (_dir, mut app) = setup();
      let fields = app.create_form.fields().to_vec();
      assert!(!fields.is_empty(), "{name}: the form must present at least one field");
      for field in fields {
        app.create_form.field = field;
        let buf = render_at(&mut app, 120, h);
        let rows = closed_modal_rows(&buf, &format!("{name} at 120x{h}"));
        let needle = field_row_needle(field_label(field));
        assert!(
          rows.iter().any(|r| r.contains(&needle)),
          "{name} at 120x{h}: the focused {field:?} field is off screen — \
           the user edits a row they cannot see. Modal rows:\n{}",
          rows.join("\n")
        );
      }
    }
  }
}

#[test]
fn the_free_form_name_field_stays_on_screen_on_a_short_terminal() {
  // Free-form mode renders one field instead of the triple, so it overflows
  // later — but when it does, the row that falls off is the *only* input the
  // mode has. `fields()` keeps reporting the structured triple in this mode
  // (`toggle_mode` moves focus, not the field set), so the loop above never
  // exercises `Field::Name`; it needs its own case.
  for (name, setup) in [
    ("create", create_form_app as fn() -> (tempfile::TempDir, App)),
    ("rename", rename_form_app as fn() -> (tempfile::TempDir, App)),
  ] {
    for h in [16u16, 12] {
      let (_dir, mut app) = setup();
      app.create_form.toggle_mode();
      assert_eq!(app.create_form.field, Field::Name, "{name}: free-form focuses Name");
      let buf = render_at(&mut app, 120, h);
      let rows = closed_modal_rows(&buf, &format!("{name} free-form at 120x{h}"));
      let needle = field_row_needle("Name");
      assert!(
        rows.iter().any(|r| r.contains(&needle)),
        "{name} free-form at 120x{h}: the Name field is off screen — it is the \
         only input this mode has. Modal rows:\n{}",
        rows.join("\n")
      );
    }
  }
}

#[test]
fn a_form_that_had_to_scroll_says_so() {
  // Scrolling the focused field into view fixes the loss, but on its own it
  // trades a silent truncation for a silent scroll. The forms borrow the
  // Settings panel's scrollbar (`scrollable_body_area`), which paints a thumb
  // only when the content outruns its viewport — so the indicator is also the
  // assertion that the form is not scrolling when it does not need to.
  let (_dir, mut app) = rename_form_app();

  let rows = closed_modal_rows(&render_at(&mut app, 120, 12), "rename at 120x12");
  assert!(
    rows.iter().any(|r| r.contains('█')),
    "a form whose fields do not fit must show a scrollbar — modal rows:\n{}",
    rows.join("\n")
  );

  let rows = closed_modal_rows(&render_at(&mut app, 120, 40), "rename at 120x40");
  assert!(
    !rows.iter().any(|r| r.contains('█')),
    "a form that fits must not show a scrollbar — modal rows:\n{}",
    rows.join("\n")
  );
}

/// A PR whose description is one long paragraph, linked and fetched onto a
/// freshly-initialised repo, with the rich view open (issue #551).
fn app_with_the_rich_view_open(body: &str) -> (tempfile::TempDir, App) {
  let (dir, repo) = init_repo();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  pin_bordered(&mut app);
  app.sidebar.open = false;
  gwm::github::link_pr(&repo, &branch, 551).unwrap();
  app.refresh_link();
  let mut pr = gwm::github::PrStatus {
    number: 551,
    title: "polish the rich PR / issue view".into(),
    state: gwm::github::PrState::Open,
    url: "https://example.test/pull/551".into(),
    updated_at: "2026-08-24T10:00:00Z".into(),
    checks_passed: 13,
    checks_total: 13,
    ci: gwm::github::CiState::Passing,
    checks: vec![],
    detail: gwm::forge::PrDetail {
      body: String::new(),
      author: "kbrdn1".into(),
      additions: 1,
      deletions: 0,
      base_ref: "dev".into(),
      head_ref: branch,
      reviews: vec![],
      comments: vec![],
    },
  };
  pr.detail.body = body.to_string();
  app.apply_pr_fetch_result(Ok(pr));
  app.enter_rich_view();
  (dir, app)
}

/// The horizontal span of the modal's frame, read off its top rule.
fn frame_width(buf: &Buffer) -> usize {
  let rows = row_strings(buf);
  let rule = rows
    .iter()
    .find(|r| r.contains('╭') && r.contains('╮'))
    .unwrap_or_else(|| panic!("no modal frame — rows:\n{}", rows.join("\n")));
  let start = rule.chars().position(|c| c == '╭').unwrap();
  let end = rule.chars().position(|c| c == '╮').unwrap();
  end - start + 1
}

#[test]
fn the_rich_view_is_painted_at_its_own_width_not_the_shared_overlays() {
  // Issue #551. The width is decided TWICE: `App::rich_view_width` wraps the
  // rows against it, `draw_detail_overlay` paints the frame at it. Nothing
  // ties the two together but this pair of assertions — and the failure is
  // silent in both directions. Painting narrower than the wrap ellipsises
  // the tail of every line of prose; painting wider leaves a column of dead
  // space the wrap already refused to use.
  let (_dir, mut app) = app_with_the_rich_view_open("A description worth reading.");
  app.set_term_width(200);
  let buf = render_at(&mut app, 200, 50);
  assert_eq!(
    frame_width(&buf),
    gwm::tui::rich_view_modal_width(200) as usize,
    "the frame must be painted at the rich view's own policy — rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

#[test]
fn a_wrapped_body_line_is_never_ellipsised_by_the_renderer() {
  // The other half of the pair above, and the one that reads as the bug:
  // the wrap already fitted every line to the inner width, so an ellipsis on
  // a body row can only mean the paint budget was smaller than the wrap
  // budget. Asserted on a body long enough to wrap several times at any
  // plausible width.
  let (_dir, mut app) = app_with_the_rich_view_open(&"lorem ipsum dolor sit amet ".repeat(40));
  app.set_term_width(200);
  let buf = render_at(&mut app, 200, 50);
  let rows = row_strings(&buf);
  // The negative assertion below is vacuous unless the body is on screen at
  // all: an overlay that failed to open has no `lorem` row to ellipsise.
  assert!(
    rows.iter().any(|r| r.contains("lorem")),
    "the body must be rendered before its ellipsis means anything — rows:\n{}",
    rows.join("\n")
  );
  let culprit = rows.iter().find(|r| {
    // A body row: inside the frame, carrying prose, cut with an ellipsis.
    r.contains("lorem") && r.contains('…')
  });
  assert!(
    culprit.is_none(),
    "a body row was ellipsised, so the paint width is under the wrap width: {:?}\nrows:\n{}",
    culprit,
    rows.join("\n")
  );
}

#[test]
fn a_body_row_starts_at_the_frame_edge_not_behind_an_empty_label_column() {
  // Issue #551, question 2 of the issue body: does the label column earn
  // its width on rows that are pure prose? It does not. The shell sizes one
  // label column from the widest label it is handed and indents EVERY row
  // by it, so each wrapped line of a description paid nine columns for a
  // label it does not have — on top of wrapping against a budget nine
  // columns short, which is the same nine columns spent twice.
  let (_dir, mut app) = app_with_the_rich_view_open(&"lorem ipsum dolor sit amet ".repeat(40));
  app.set_term_width(200);
  let buf = render_at(&mut app, 200, 50);
  // The modal's rows, not the terminal's: in the boxed layout the worktree
  // pane behind it owns column 0's `│`, and `left_rule` below would find
  // that one rather than the modal's own (issue #594).
  let rows = modal_rows(&buf);

  let body = rows
    .iter()
    .find(|r| r.contains("lorem"))
    .unwrap_or_else(|| panic!("the body must be on screen — rows:\n{}", rows.join("\n")));
  let left_rule = body
    .chars()
    .position(|c| c == '│')
    .expect("a body row sits inside the frame");
  let text = body.chars().position(|c| c == 'l').expect("the body text");

  // The frame's own inset: the rule, then the block's two padding columns.
  // Anything past that is the empty label column.
  assert_eq!(
    text - left_rule,
    3,
    "a label-less row must start at the frame's padding, not {} columns in — row: {body:?}",
    text - left_rule
  );
}

#[test]
fn markdown_reaches_the_terminal_rendered_not_as_source() {
  // Issue #551, the complaint in one assertion: `## Description` and
  // `**bold**` were painted with their markers, because the body reached the
  // renderer as the Markdown source it arrived as.
  let (_dir, mut app) = app_with_the_rich_view_open(
    "## Description\n\nA **bold** claim and `some_code` and a [link](https://example.test/x).\n\n- one\n- [x] done\n\n<!-- hidden -->",
  );
  app.set_term_width(200);
  let buf = render_at(&mut app, 200, 50);
  let rows = row_strings(&buf);
  let all = rows.join("\n");

  assert!(all.contains("Description"), "the heading text is there — rows:\n{all}");
  assert!(
    !all.contains("## Description"),
    "and it is a heading, not its source — rows:\n{all}"
  );
  assert!(all.contains("bold"), "the emphasised word is there");
  assert!(!all.contains("**bold**"), "without its markers — rows:\n{all}");
  assert!(all.contains("• one"), "a list item gets a bullet — rows:\n{all}");
  assert!(all.contains("☑ done"), "a task gets its box — rows:\n{all}");
  assert!(!all.contains("hidden"), "an HTML comment is not shown — rows:\n{all}");
  assert!(all.contains("link"), "a link shows its text");
  assert!(!all.contains("https://example.test/x"), "not its URL — rows:\n{all}");
}

#[test]
fn an_emphasised_run_is_painted_in_its_own_style() {
  // The needle the assertions above cannot reach: the text can be correct
  // while every run is painted identically, which is the same view with
  // extra steps. Read off the real cells.
  let (_dir, mut app) = app_with_the_rich_view_open("plainword **boldword** `codeword`");
  app.set_term_width(200);
  let buf = render_at(&mut app, 200, 50);

  let cell_style = |needle: &str| {
    let area = *buf.area();
    for y in 0..area.height {
      let row: String = (0..area.width).map(|x| buf[(x, y)].symbol()).collect();
      if let Some(at) = row.find(needle) {
        // `find` is a byte offset and every character here is ASCII.
        let cell = &buf[(at as u16, y)];
        return Some((cell.fg, cell.modifier));
      }
    }
    None
  };

  let plain = cell_style("plainword").expect("plain prose on screen");
  let bold = cell_style("boldword").expect("the emphasised run on screen");
  let code = cell_style("codeword").expect("the code run on screen");

  assert_ne!(bold, plain, "an emphasised run must not paint like plain prose");
  assert_ne!(code, plain, "inline code must not paint like plain prose");
  assert_ne!(code, bold, "code and emphasis are different things");
}

#[test]
#[ignore = "not an assertion: prints the rich view so a human can look at it"]
fn dump_the_rich_view() {
  // Question 1 of issue #551: "screenshot the view against a real PR with a
  // long body — that picture is the brief". `GWM_DUMP_BODY` points at a file
  // holding one, so the picture can be retaken after any change here:
  //
  //   gh pr view 582 --json body -q .body > /tmp/body.md
  //   GWM_DUMP_BODY=/tmp/body.md cargo test --test tui_modal_render_tests \
  //     dump_the_rich_view -- --ignored --nocapture
  //
  // `GWM_DUMP_TABS=1` instead prints the two-tab case.
  let body = std::env::var("GWM_DUMP_BODY")
    .ok()
    .and_then(|p| std::fs::read_to_string(p).ok())
    .unwrap_or_else(|| "## Heading\n\nSome **bold** prose.".into());
  // With `GWM_DUMP_TABS` set, both sides are linked so the tab bar shows.
  let (_dir, mut app) = if std::env::var_os("GWM_DUMP_TABS").is_some() {
    app_with_both_tabs()
  } else {
    app_with_the_rich_view_open(&body)
  };
  let (w, h) = (160, 60);
  app.set_term_width(w);
  let buf = render_at(&mut app, w, h);
  println!("{}", row_strings(&buf).join("\n"));
}

/// The rich view with BOTH sides linked and fetched, so the tab bar renders.
fn app_with_both_tabs() -> (tempfile::TempDir, App) {
  let (dir, repo) = init_repo();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  pin_bordered(&mut app);
  app.sidebar.open = false;
  gwm::github::link_pr(&repo, &branch, 551).unwrap();
  gwm::github::link_issue(&repo, &branch, 420).unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(gwm::github::IssueStatus {
    number: 420,
    title: "the view itself".into(),
    state: gwm::github::IssueState::Open,
    url: "https://example.test/issues/420".into(),
    labels: vec!["tui".into()],
    updated_at: "2026-08-01T10:00:00Z".into(),
    detail: gwm::forge::IssueDetail {
      body: "The issue body.".into(),
      author: "kbrdn1".into(),
      comments: vec![],
    },
  }));
  app.apply_pr_fetch_result(Ok(gwm::github::PrStatus {
    number: 551,
    title: "polish the rich view".into(),
    state: gwm::github::PrState::Open,
    url: "https://example.test/pull/551".into(),
    updated_at: "2026-08-24T10:00:00Z".into(),
    checks_passed: 13,
    checks_total: 13,
    ci: gwm::github::CiState::Passing,
    checks: vec![],
    detail: gwm::forge::PrDetail {
      body: "The PR body.".into(),
      author: "kbrdn1".into(),
      additions: 1,
      deletions: 0,
      base_ref: "dev".into(),
      head_ref: branch,
      reviews: vec![],
      comments: vec![],
    },
  }));
  app.enter_rich_view();
  (dir, app)
}

#[test]
fn the_tab_bar_is_on_screen_when_both_sides_are_linked() {
  // Issue #551: the PR wins whenever one is linked, which left the issue
  // with no way back. The bar is what says the other side is one key away.
  let (_dir, mut app) = app_with_both_tabs();
  app.set_term_width(160);
  let buf = render_at(&mut app, 160, 50);
  let inside = modal_rows(&buf).join("\n");

  assert!(inside.contains("Issue #420"), "the issue tab — modal:\n{inside}");
  assert!(inside.contains("PR #551"), "the PR tab — modal:\n{inside}");
}

#[test]
fn the_tab_bar_does_not_push_the_hint_row_out_of_the_frame() {
  // Two rows were added to `lines`, so two rows had to be added to the
  // height. Under-count them and `Paragraph` simply drops the tail: the hint
  // bar, which is the row that tells the reader `Tab` exists at all.
  //
  // Asserted inside the frame. The footer at the bottom of the screen
  // advertises the very same verbs, so a whole-buffer search passes with the
  // hint bar missing — which is exactly what it did.
  let (_dir, mut app) = app_with_both_tabs();
  app.set_term_width(160);
  let buf = render_at(&mut app, 160, 50);
  let inside = modal_rows(&buf).join("\n");

  assert!(
    inside.contains("issue/pr"),
    "the tab hint must survive the frame — modal:\n{inside}"
  );
  assert!(
    inside.contains("close"),
    "and so must the rest of the hint bar — modal:\n{inside}"
  );
}

#[test]
fn scrolling_right_brings_a_code_lines_tail_on_screen() {
  // Issue #551. The offset is state; this is the half that matters. A fenced
  // line is kept whole rather than reflowed, so without the renderer
  // honouring the offset its tail is simply unreachable — and the row-level
  // ellipsis that used to cut it would throw those columns away before
  // anything could scroll to them.
  //
  // The needle is a marker placed at column 300 of a 400-column line, which
  // no plausible modal width can show at rest.
  let mut line = "x".repeat(400);
  line.replace_range(300..309, "NEEDLEHIT");
  let (_dir, mut app) = app_with_the_rich_view_open(&format!("```\n{line}\n```"));
  app.set_term_width(160);

  let before = modal_rows(&render_at(&mut app, 160, 50)).join("\n");
  assert!(
    !before.contains("NEEDLEHIT"),
    "precondition: the tail is off screen at rest — modal:\n{before}"
  );

  for _ in 0..40 {
    app.rich_view_scroll_right();
  }
  let after = modal_rows(&render_at(&mut app, 160, 50)).join("\n");

  assert!(
    after.contains("NEEDLEHIT"),
    "scrolling right must reach it — modal:\n{after}"
  );
}

#[test]
fn scrolling_leaves_the_wrapped_prose_where_it_was() {
  // The offset is bounded to preformatted rows on purpose: prose was
  // wrapped to fit, so it has no tail to reach and sliding it would only
  // hide its left edge.
  let (_dir, mut app) =
    app_with_the_rich_view_open(&format!("A paragraph that stays put.\n\n```\n{}\n```", "x".repeat(400)));
  app.set_term_width(160);
  let _ = render_at(&mut app, 160, 50);
  for _ in 0..40 {
    app.rich_view_scroll_right();
  }
  let after = modal_rows(&render_at(&mut app, 160, 50)).join("\n");

  assert!(
    after.contains("A paragraph that stays put."),
    "the prose must not slide out of the frame — modal:\n{after}"
  );
}

#[test]
fn scrolling_reaches_the_tail_of_a_line_of_wide_glyphs() {
  // Codex review, pass 1 (P2): the offset bound and the render clip both
  // counted CHARS, while the terminal spends CELLS. A line of CJK is twice
  // as wide as it is long, so the bound stopped at half the columns it
  // needed to and the tail could not be reached at any offset — on the one
  // feature whose whole purpose is reaching that tail.
  let line = format!("{}NEEDLEHIT", "界".repeat(120));
  let (_dir, mut app) = app_with_the_rich_view_open(&format!("```\n{line}\n```"));
  app.set_term_width(160);

  let before = modal_rows(&render_at(&mut app, 160, 50)).join("\n");
  assert!(
    !before.contains("NEEDLEHIT"),
    "precondition: 240 cells of glyphs put the tail off screen — modal:\n{before}"
  );

  for _ in 0..80 {
    app.rich_view_scroll_right();
  }
  let after = modal_rows(&render_at(&mut app, 160, 50)).join("\n");
  assert!(
    after.contains("NEEDLEHIT"),
    "the tail must be reachable — modal:\n{after}"
  );
}

#[test]
fn a_segmented_row_too_wide_for_the_modal_is_ellipsised() {
  // Codex review, pass 2 (P2): `value` carried the ellipsised text but the
  // segment branch walked the original runs until the budget ran out, so a
  // styled row was cut silently. The `url` row is the one that hits this
  // first, and losing the end of a URL with no mark saying so is the exact
  // failure the ellipsis exists to prevent.
  let (_dir, mut app) = app_with_the_rich_view_open("body");
  app.set_term_width(44);
  let buf = render_at(&mut app, 44, 40);
  let rows = modal_rows(&buf);
  let url = rows
    .iter()
    .find(|r| r.contains("example.test"))
    .unwrap_or_else(|| panic!("the url row must be on screen — modal:\n{}", rows.join("\n")));

  assert!(url.contains('…'), "a row cut by the modal must say so: {url:?}");
}

#[test]
fn a_row_cut_after_a_badge_keeps_its_ellipsis_on_screen() {
  // Codex review, pass 4 (P2), on the ellipsis added in pass 2. It reserved
  // its column against the whole row's width instead of against what was
  // LEFT after the runs already painted, so a row opening with a badge came
  // out one column over and ratatui clipped the ellipsis itself — putting
  // the silent truncation back exactly where pass 2 had removed it.
  //
  // The identity row is the one that opens with a badge, and a narrow
  // terminal is where it stops fitting.
  let (_dir, mut app) = app_with_both_tabs();
  app.set_term_width(40);
  let buf = render_at(&mut app, 40, 40);
  let rows = modal_rows(&buf);
  let identity = rows
    .iter()
    // Not the title, which rides the top rule and carries the same number,
    // and not the tab bar, which names both sides on one row.
    .find(|r| r.contains("#551") && !r.contains('╭') && !r.contains("Issue"))
    .unwrap_or_else(|| panic!("the identity row must be on screen — modal:\n{}", rows.join("\n")));

  assert!(
    identity.contains('…'),
    "a row the modal cut must say so, and the mark must survive the clip: {identity:?}"
  );
}

// ── the note editor's mode indicator (#557) ────────────────────────────────

#[test]
fn the_note_title_names_the_mode_when_the_knob_is_on() {
  // A mode the user cannot see is a mode they type verbs into by accident.
  // The title is the one piece of chrome the editor already has.
  let (_dir, mut app) = make_app();
  app.config.tui.note_vim = true;
  app.list_state.select(Some(0));
  app.open_note_editor();

  let buf = render_at(&mut app, TERM_W, TERM_H);
  assert!(
    buffer_contains(&buf, "NORMAL"),
    "the modal must say which mode it is in:\n{}",
    row_strings(&buf).join("\n")
  );

  app.handle_note_key(crossterm::event::KeyEvent::new(
    crossterm::event::KeyCode::Char('i'),
    crossterm::event::KeyModifiers::NONE,
  ));
  let buf = render_at(&mut app, TERM_W, TERM_H);
  assert!(
    buffer_contains(&buf, "INSERT"),
    "and it must follow the mode:\n{}",
    row_strings(&buf).join("\n")
  );
}

#[test]
fn the_note_title_says_nothing_about_modes_with_the_knob_off() {
  // The #515 title, unchanged, for everyone who turns the mode back off.
  let (_dir, mut app) = make_app();
  app.config.tui.note_vim = false;
  app.list_state.select(Some(0));
  app.open_note_editor();

  let buf = render_at(&mut app, TERM_W, TERM_H);
  assert!(!buffer_contains(&buf, "INSERT"), "no mode chip without the knob");
  assert!(!buffer_contains(&buf, "NORMAL"));
}

#[test]
fn a_long_branch_name_does_not_push_the_mode_chip_off_the_title() {
  // The title is clipped from the right, so whichever half sits last is the
  // half a long branch name costs. Whether the keys are text is worth more
  // than the name of the branch the modal was opened from.
  let (_dir, mut app) = make_app();
  app.config.tui.note_vim = true;
  app.note_editor = Some(gwm::tui::state::note_editor::NoteEditor::open(
    "feat/#557-a-branch-name-long-enough-to-run-past-the-right-hand-edge-of-the-modal".into(),
    PathBuf::from("/tmp/n.md"),
    "",
  ));
  app.note_editor.as_mut().unwrap().enter_normal();
  app.view = View::Note;

  let buf = render_at(&mut app, TERM_W, TERM_H);
  assert!(
    buffer_contains(&buf, "NORMAL"),
    "the mode chip was clipped off by the branch name:\n{}",
    row_strings(&buf).join("\n")
  );
}

#[test]
fn the_note_modal_carries_the_mode_line_on_its_own_last_row() {
  // The app footer says the same thing, but it sits at the bottom of the
  // terminal: on a tall screen that is thirty rows away from the box the
  // eye is in, which is where the keys are being pressed. Asserted on the
  // modal's own last inner row, so the footer behind it cannot satisfy it.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.open_note_editor();

  let buf = render_at(&mut app, TERM_W, TERM_H);
  // Frame anatomy from the bottom: the border, the block's one row of
  // bottom padding, then the mode line.
  let rows = closed_modal_rows(&buf, "note at 100x40");
  let last_inner = rows[rows.len() - 3].clone();
  assert!(
    last_inner.contains("hjkl"),
    "the modal's last row must carry the normal-mode keys, got:\n{}",
    rows.join("\n")
  );

  app.handle_note_key(crossterm::event::KeyEvent::new(
    crossterm::event::KeyCode::Char('i'),
    crossterm::event::KeyModifiers::NONE,
  ));
  let buf = render_at(&mut app, TERM_W, TERM_H);
  let rows = closed_modal_rows(&buf, "note in insert at 100x40");
  let last_inner = rows[rows.len() - 3].clone();
  assert!(
    last_inner.contains("INSERT") && !last_inner.contains("hjkl"),
    "and it must follow the mode into insert, got:\n{last_inner}"
  );

  // Where the row has the width for the whole list, `Esc` names what it
  // does in this mode rather than the gesture it performs in the other.
  // At 100 columns the tail is what the truncation eats, which is the
  // documented order and why the help overlay carries the same keys.
  let buf = render_at(&mut app, 160, TERM_H);
  let rows = closed_modal_rows(&buf, "note in insert at 160x40");
  let last_inner = rows[rows.len() - 3].clone();
  assert!(
    last_inner.contains("Esc normal mode") && !last_inner.contains("save & close"),
    "the full insert line must say where `Esc` goes, got:\n{last_inner}"
  );
}

#[test]
fn the_note_modal_still_renders_when_there_is_no_room_for_both() {
  // The mode line takes a row off the buffer, so the smallest modal has to
  // stay drawable: a layout whose text pane collapses to zero must not
  // panic or lose the frame.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.open_note_editor();
  for c in "note".chars() {
    app.handle_note_key(crossterm::event::KeyEvent::new(
      crossterm::event::KeyCode::Char(c),
      crossterm::event::KeyModifiers::NONE,
    ));
  }

  for (w, h) in [(40u16, 6u16), (30, 5), (20, 4)] {
    let buf = render_at(&mut app, w, h);
    assert!(!row_strings(&buf).is_empty(), "the note modal must survive {w}x{h}");
  }
}

#[test]
fn the_mode_badge_is_painted_not_just_spelled() {
  // A word in a row of words is a word; the mode is state, and it reads as
  // state when it is a block of colour. Same reverse-video treatment the
  // statusbar's context anchor has always had.
  use ratatui::style::Modifier;

  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.open_note_editor();

  let buf = render_at(&mut app, TERM_W, TERM_H);
  let (x, y, w, h) = modal_rect(&buf).expect("the note modal");
  // The frame from the bottom: border, one row of block padding, the mode
  // line.
  let row = y + h - 3;
  let painted: String = (x..x + w)
    .filter(|col| buf[(*col, row)].modifier.contains(Modifier::REVERSED))
    .map(|col| buf[(col, row)].symbol())
    .collect();
  assert_eq!(
    painted.trim(),
    "NORMAL",
    "the mode badge must be the reverse-video run on the mode line, row:\n{}",
    (x..x + w).map(|col| buf[(col, row)].symbol()).collect::<String>()
  );

  app.handle_note_key(crossterm::event::KeyEvent::new(
    crossterm::event::KeyCode::Char('i'),
    crossterm::event::KeyModifiers::NONE,
  ));
  let buf = render_at(&mut app, TERM_W, TERM_H);
  let painted: String = (x..x + w)
    .filter(|col| buf[(*col, row)].modifier.contains(Modifier::REVERSED))
    .map(|col| buf[(col, row)].symbol())
    .collect();
  assert_eq!(painted.trim(), "INSERT", "and it follows the mode");
}

#[test]
fn the_mode_badge_is_absent_with_the_mode_off() {
  // No badge for a state nobody can be in: with `note_vim = false` the
  // editor has one mode and naming it would be chrome.
  use ratatui::style::Modifier;

  let (_dir, mut app) = make_app();
  app.config.tui.note_vim = false;
  app.list_state.select(Some(0));
  app.open_note_editor();

  let buf = render_at(&mut app, TERM_W, TERM_H);
  let (x, y, w, h) = modal_rect(&buf).expect("the note modal");
  let row = y + h - 3;
  assert!(
    (x..x + w).all(|col| !buf[(col, row)].modifier.contains(Modifier::REVERSED)),
    "no badge without the mode, row:\n{}",
    (x..x + w).map(|col| buf[(col, row)].symbol()).collect::<String>()
  );
}

#[test]
fn working_tree_modal_renders_its_title_body_and_footer() {
  use gwm::tui::{WorkingTreeCounts, WT_CREATED_ICON, WT_MODIFIED_ICON};
  use ratatui::text::Line;

  // Issue #592: the sidebar pane's tree, given the whole screen. The rows
  // are injected as owned state (the same boundary the command-logs render
  // test pins) so this stays offline and deterministic — `enter_working_tree`
  // is what shells out, and `tui_app_tests` covers that half.
  let (_dir, mut app) = make_app();
  app.working_tree.lines = vec![Line::from("src/tui/"), Line::from("└─ ui.rs"), Line::from("README.md")];
  app.working_tree.counts = WorkingTreeCounts {
    created: 1,
    modified: 2,
    deleted: 0,
  };
  app.view = View::WorkingTree;

  let buf = render(&mut app);
  assert_present(&buf, "Working Tree", "working tree overlay title");
  assert_present(&buf, "ui.rs", "a tree row from the injected listing");
  assert_present(&buf, "README.md", "a second tree row");
  // The pane's change-count footer travels with the listing (issue #287):
  // the same `<glyph> <n>` segments the bordered sidebar pane puts on its
  // bottom rule, asserted through the constants so a glyph change here is a
  // deliberate edit rather than a silently-passing literal.
  //
  // Scanned on the modal's LAST row and pinned to the right (Copilot review,
  // PR #612): a whole-buffer `assert_present` catches the counts vanishing,
  // but not `title_bottom(...)` losing its `.right_aligned()` or the segments
  // migrating into the body. Placement is the observable part here. The
  // per-category COLOURS are not re-pinned: `working_tree_counts_footer` owns
  // them and `tui_ui_helpers_tests::working_tree_counts_footer_shows_only_nonzero_colored_segments`
  // is where they are asserted, at the shared source both surfaces call.
  let rows = modal_rows(&buf);
  let bottom = rows.last().expect("the modal renders at least one row").clone();
  let created = format!("{WT_CREATED_ICON} 1");
  let modified = format!("{WT_MODIFIED_ICON} 2");
  assert!(
    bottom.contains(&created) && bottom.contains(&modified),
    "the change counts ride the modal's bottom rule — bottom row was {bottom:?}, modal rows:\n{}",
    rows.join("\n")
  );
  // Right-aligned means nothing but padding and the corner follows the last
  // segment. Left or centred would leave `─` rule on its right.
  let last = bottom.find(&modified).expect("modified segment on the bottom rule") + modified.len();
  assert!(
    bottom[last..].chars().all(|c| c == ' ' || c == '╯'),
    "the counts ride the RIGHT of the bottom rule; found rule after them in {bottom:?}"
  );
  assert!(
    bottom[..tail_of(&bottom, &created)].contains('─'),
    "and the rule runs up to them from the left in {bottom:?}"
  );
  assert_present(&buf, "close", "the modal footer advertises the exit");
}

#[test]
fn the_working_tree_counts_ride_the_right_edge_and_yield_to_a_narrow_name() {
  use gwm::tui::MetaColumn;
  use ratatui::text::Line;

  // Issue #592, the responsive half of the commit listing's treatment
  // (#593). The counts sit in their own rect on the right, so what a narrow
  // terminal loses is the column, never the file name.
  let (_dir, mut app) = make_app();
  app.working_tree.lines = vec![Line::from("├─ src/tui/"), Line::from("└─ ui.rs")];
  app.working_tree.meta = MetaColumn {
    lines: vec![Line::from(""), Line::from("+120 -34")],
    width: 8,
  };
  app.view = View::WorkingTree;

  // Wide: the counts are drawn, and nothing but the border follows them.
  // #622 adds the status letter to their right, so what trails the counts is
  // asserted up to that letter rather than to the border directly.
  let wide = render_at(&mut app, 180, 40);
  let rows = modal_rows(&wide);
  let row = rows
    .iter()
    .find(|r| r.contains("ui.rs"))
    .unwrap_or_else(|| panic!("no row for the file. modal was:\n{}", rows.join("\n")));
  assert!(
    row.contains("+120 -34"),
    "the counts ride the row they describe: got {row:?}"
  );
  let after = row.find("+120 -34").unwrap() + "+120 -34".len();
  assert!(
    row[after..].chars().all(|c| c == ' ' || c == '│' || c == '║'),
    "and they are pinned right: only padding and the border follow, got {row:?}"
  );

  // Narrow: the name survives whole, the column is what goes. The modal takes
  // 90% of the terminal and spends two more cells on its border, so 30
  // columns leave a 25-cell body — less than the 34 the column needs beside
  // `WT_NAME_FLOOR`. 40 would NOT do: it leaves exactly 34, which fits, and
  // the assertion would fail on the boundary rather than past it. The exact
  // boundary is pinned on `meta_pick` itself, in `tui_app_tests`.
  let narrow = render_at(&mut app, 30, 40);
  let rows = modal_rows(&narrow);
  let row = rows
    .iter()
    .find(|r| r.contains("ui.rs"))
    .unwrap_or_else(|| panic!("the name is never what is dropped — modal was:\n{}", rows.join("\n")));
  assert!(
    !row.contains("+120"),
    "the column yields before the name does — got {row:?}"
  );
}

#[test]
fn the_working_tree_title_counts_the_changed_files_not_the_rows() {
  use gwm::tui::WorkingTreeCounts;
  use ratatui::text::Line;

  // Issue #622: the reference layout puts a progress counter in the header.
  // The honest gwm equivalent is the changed-file count, and it comes from
  // the counts rather than `lines.len()`: the rows also hold directories
  // and the sentinels, none of which is a changed file.
  let (_dir, mut app) = make_app();
  app.working_tree.lines = vec![
    Line::from("├─ src/tui/"),
    Line::from("│  └─ ui.rs"),
    Line::from("└─ README.md"),
    Line::from("… 4 more"),
  ];
  app.working_tree.counts = WorkingTreeCounts {
    created: 1,
    modified: 5,
    deleted: 1,
  };
  app.view = View::WorkingTree;

  let rows = modal_rows(&render_at(&mut app, 180, 40));
  let title = rows
    .iter()
    .find(|r| r.contains("Working Tree"))
    .unwrap_or_else(|| panic!("no title row: modal was:\n{}", rows.join("\n")));
  assert!(
    title.contains("Working Tree (7)"),
    "the title counts the changed files, not the four rendered rows: got {title:?}"
  );

  // While the worker is out the counts are zero, and `(0)` over a listing
  // nobody has read yet is a claim rather than a count.
  app.working_tree.begin(Some(std::path::Path::new("/tmp/whatever")));
  assert!(app.working_tree.loading, "begin arms the loader");
  let rows = modal_rows(&render_at(&mut app, 180, 40));
  let title = rows
    .iter()
    .find(|r| r.contains("Working Tree"))
    .unwrap_or_else(|| panic!("no title row: modal was:\n{}", rows.join("\n")));
  assert!(
    !title.contains('('),
    "the loader frame withholds the count rather than claiming zero: got {title:?}"
  );
}

#[test]
fn the_status_letter_rides_the_right_edge_and_outlives_the_counts() {
  use gwm::tui::MetaColumn;
  use ratatui::text::Line;

  // Issue #622: two right-hand columns, and they yield in a fixed order.
  // `+120 -34` says how much changed, the letter says what changed. A row
  // that no longer says what it is has lost its subject, not its detail, so
  // the letter takes the outer slot and is the one that survives a squeeze.
  let (_dir, mut app) = make_app();
  app.working_tree.lines = vec![Line::from("├─ src/tui/"), Line::from("└─ ui.rs")];
  app.working_tree.meta = MetaColumn {
    lines: vec![Line::from(""), Line::from("+120 -34")],
    width: 8,
  };
  app.working_tree.badges = MetaColumn {
    lines: vec![Line::from(""), Line::from("M")],
    width: 1,
  };
  app.view = View::WorkingTree;

  // Wide: both are drawn, the letter outermost and flush against the border.
  let rows = modal_rows(&render_at(&mut app, 180, 40));
  let row = rows
    .iter()
    .find(|r| r.contains("ui.rs"))
    .unwrap_or_else(|| panic!("no row for the file. modal was:\n{}", rows.join("\n")));
  let counts_at = row
    .find("+120 -34")
    .unwrap_or_else(|| panic!("the counts are drawn: got {row:?}"));
  let letter_at = row
    .rfind('M')
    .unwrap_or_else(|| panic!("the status letter is drawn: got {row:?}"));
  assert!(letter_at > counts_at, "the letter sits outside the counts: got {row:?}");
  assert!(
    row[letter_at + 1..].chars().all(|c| c == ' ' || c == '│' || c == '║'),
    "and nothing but padding and the border follows it: got {row:?}"
  );

  // Narrow enough to drop the counts, wide enough to keep the letter. The
  // modal takes 90% of the terminal and spends two cells on its border, so
  // 38 columns leave a 32-cell body: past the `1 + META_GAP + WT_BADGE_FLOOR`
  // (11) the letter needs, and the 29 cells it leaves are under the
  // `8 + META_GAP + WT_NAME_FLOOR` (34) the counts would need. 48 would NOT
  // do: it leaves 40, which seats both, and the assertion would pass for the
  // wrong reason.
  let rows = modal_rows(&render_at(&mut app, 38, 40));
  let row = rows
    .iter()
    .find(|r| r.contains("ui.rs"))
    .unwrap_or_else(|| panic!("the name is never what is dropped. modal was:\n{}", rows.join("\n")));
  assert!(
    !row.contains("+120"),
    "the counts are the column that yields first: got {row:?}"
  );
  assert!(
    row.trim_end_matches([' ', '│', '║']).ends_with('M'),
    "the letter survives the squeeze, still pinned right: got {row:?}"
  );

  // Narrower still: on a 22-column terminal the body is 17 cells, under the
  // 11 + 8 both columns would need together but past what the letter alone
  // costs. It is the last thing standing beside the name.
  let rows = modal_rows(&render_at(&mut app, 22, 40));
  let row = rows
    .iter()
    .find(|r| r.contains("ui.rs"))
    .unwrap_or_else(|| panic!("the name survives whatever else goes. modal was:\n{}", rows.join("\n")));
  assert!(
    row.trim_end_matches([' ', '│', '║']).ends_with('M'),
    "the letter is what a narrow overlay keeps: got {row:?}"
  );
}

#[test]
fn working_tree_modal_renders_a_loader_while_the_worker_is_out() {
  // The read moved to a worker (Copilot review, PR #612), so there is a
  // frame with no rows yet. It must say so: an empty canvas reads as "no
  // changes", which is the one answer this overlay must not give by
  // accident.
  let (_dir, mut app) = make_app();
  app
    .working_tree
    .begin(Some(std::path::Path::new("/tmp/gwm-test/pending")));
  app.view = View::WorkingTree;

  let buf = render(&mut app);
  assert_present(&buf, "Working Tree", "working tree overlay title");
  assert_present(&buf, "loading", "the loader, not a blank canvas");
  // The exit is still advertised while it waits.
  assert_present(&buf, "close", "the modal footer advertises the exit");
}

#[test]
fn working_tree_modal_pins_its_worktree_and_path_above_the_scrolling_body() {
  use ratatui::text::Line;

  // Issue #629, the other half: the listing is a set of file names with no
  // statement of which worktree they were read from, and the auto-refresh
  // can move the list selection while the overlay is up. The row resolves
  // from the path pinned in `WorkingTreeModal` rather than from the live
  // selection, so it describes the same worktree the rows describe.
  //
  // Asserted by POSITION for the same reason the commits case is: presence
  // alone passes on a row that scrolls away.
  let (_dir, mut app) = make_app();
  let wt = deletable_worktree("payment-webhooks");
  app.worktrees = vec![wt.clone()];
  app.working_tree.begin(Some(&wt.path));
  app.working_tree.loading = false;
  app.working_tree.lines = (0..40).map(|i| Line::from(format!("file-{i}.rs"))).collect();
  app.view = View::WorkingTree;

  let top = modal_rows(&render_at(&mut app, 100, 18));
  let row = top
    .iter()
    .position(|r| r.contains("/tmp/gwm-test/payment-webhooks"))
    .unwrap_or_else(|| panic!("no context row — modal rows:\n{}", top.join("\n")));
  assert!(
    top[row].contains("payment-webhooks"),
    "the row carries the worktree name beside its path — got {:?}",
    top[row]
  );
  assert!(
    top[row + 1].contains("file-0.rs"),
    "the listing starts on the row right below it — got {:?}",
    top[row + 1]
  );

  app.working_tree.scroll = u16::MAX;
  let bottom = modal_rows(&render_at(&mut app, 100, 18));
  assert!(
    bottom[row].contains("/tmp/gwm-test/payment-webhooks"),
    "the context row is FIXED: same line at max scroll — got {:?}, modal rows:\n{}",
    bottom[row],
    bottom.join("\n")
  );
  assert!(
    !bottom[row + 1].contains("file-0.rs"),
    "and it is the body that moved under it — got {:?}",
    bottom[row + 1]
  );
}

#[test]
fn working_tree_modal_keeps_its_context_row_while_the_worker_is_out() {
  use ratatui::text::Line;

  // The loader arm returns early (PR #612), so it is a second renderer with
  // its own layout. Without the row there, the loader and the first loaded
  // row land on different lines and the content visibly jumps when the
  // worker returns.
  let (_dir, mut app) = make_app();
  let wt = deletable_worktree("payment-webhooks");
  app.worktrees = vec![wt.clone()];
  app.working_tree.begin(Some(&wt.path));
  app.view = View::WorkingTree;
  assert!(app.is_working_tree_loading(), "the overlay opens on its loader");

  let waiting = modal_rows(&render(&mut app));
  let row = waiting
    .iter()
    .position(|r| r.contains("/tmp/gwm-test/payment-webhooks"))
    .unwrap_or_else(|| panic!("no context row while loading — modal rows:\n{}", waiting.join("\n")));
  assert!(
    waiting[row + 1].contains("loading"),
    "the loader sits under it — got {:?}",
    waiting[row + 1]
  );

  // The same line in the loaded arm: the two renderers are separate, so a
  // row present in only one of them makes the content jump when the worker
  // lands.
  app.working_tree.loading = false;
  app.working_tree.lines = vec![Line::from("src/tui/ui.rs")];
  let loaded = modal_rows(&render(&mut app));
  assert!(
    loaded[row].contains("/tmp/gwm-test/payment-webhooks") && loaded[row + 1].contains("src/tui/ui.rs"),
    "the listing lands exactly where the loader was — rows {:?} / {:?}, modal rows:\n{}",
    loaded[row],
    loaded[row + 1],
    loaded.join("\n")
  );
}

#[test]
fn working_tree_modal_renders_an_empty_listing_without_panicking() {
  // The empty snapshot is what `enter_working_tree` loads when nothing is
  // selected. It is NOT the errored-`git status` case: that one still
  // produces a row (`! <error>`, `working_tree_lines`), which is the point
  // of rendering it. The overlay still opens on its frame either way.
  let (_dir, mut app) = make_app();
  app.working_tree.lines.clear();
  app.view = View::WorkingTree;

  let buf = render(&mut app);
  assert_present(&buf, "Working Tree", "working tree overlay title");
}

// ── The compact frame (issue #594) ─────────────────────────────────────────
//
// Everything above pins the boxed frame. Below is the other half of
// `[tui] layout`: no rules at all, a filled title band on the frame's first
// row, and the quiet `section_bg` band under its last one. The cases are
// enumerated rather than sampled: `compact_case_for` carries no wildcard,
// so a new overlay stops this file compiling until it is accounted for.

/// The theme role a modal's frame is built from: the rules in the boxed
/// layout, the title band's ground in the compact one. Three of them, and
/// the band is mixed from whichever the modal passes: a destructive
/// confirmation mixed from `accent` would drop the one signal it exists
/// to carry.
#[derive(Debug, Clone, Copy)]
enum Band {
  Accent,
  /// The two forms: creating and renaming a worktree.
  Clean,
  /// The two irreversible confirmations: delete and merge.
  Danger,
}

/// One overlay, under `[tui] layout = "compact"`.
struct CompactCase {
  /// Matches the name the boxed `sizing_matrix` uses where the surface is
  /// in both, so a failure names the same modal in either half.
  name: &'static str,
  /// The `View` this case must actually leave the app in. Checked at
  /// render time: without it the coverage guard below matches a case by
  /// its *name*, and a setup that opened the wrong overlay (or none) would
  /// satisfy it.
  view: View,
  setup: ModalSetup,
  /// The role its band is mixed from.
  band: Band,
  /// `false` on the surfaces whose last row is content rather than a
  /// footer, and which therefore paint no band under it.
  footer: bool,
}

/// The app `setup` builds, flipped back to the layout gwm actually ships.
/// Every setup in this file pins `Bordered` (see `make_app`), which is what
/// the assertions above want and the opposite of what these want.
fn compact_app(setup: &ModalSetup) -> (tempfile::TempDir, App) {
  let (dir, mut app) = setup();
  app.config.tui.layout = TuiLayout::Compact;
  (dir, app)
}

/// The modal's rect in the compact layout.
///
/// There is no glyph to hunt: the frame paints no rules, which is the whole
/// point of the layout, so `modal_rect`'s `╭` is gone and a locator built on
/// it would return `None` and make every assertion below pass vacuously.
///
/// What marks the modal out instead is the one thing `draw` does to the
/// buffer before an overlay paints: everything already on screen is set
/// `DIM` (#594), and the overlay's own `Clear` resets the cells under it.
/// So the modal is exactly the bounding box of the cells carrying no `DIM`,
/// and "no overlay was painted" comes back as `None` rather than as a rect
/// describing something else.
fn compact_modal_rect(buf: &Buffer) -> Option<(u16, u16, u16, u16)> {
  use ratatui::style::Modifier;
  let area = *buf.area();
  let lit = |x: u16, y: u16| !buf[(x, y)].modifier.contains(Modifier::DIM);
  let mut rect: Option<(u16, u16, u16, u16)> = None;
  for y in 0..area.height {
    for x in 0..area.width {
      if !lit(x, y) {
        continue;
      }
      rect = Some(match rect {
        None => (x, y, x, y),
        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
      });
    }
  }
  rect.map(|(x0, y0, x1, y1)| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// A PR carrying `n` CI checks, so `enter_ci_checks` has something to open.
fn app_with_ci_checks(n: usize) -> (tempfile::TempDir, App) {
  let (dir, mut app) = app_with_the_rich_view_open("body");
  let mut pr = match app.pr_fetch_state() {
    gwm::tui::GitHubFetchState::Loaded(pr) => pr.clone(),
    _ => panic!("the fixture fetched a PR"),
  };
  pr.checks = (0..n)
    .map(|i| gwm::forge::PrCheck {
      name: format!("build-{i}"),
      outcome: gwm::forge::CheckOutcome::Passing,
      url: Some(format!("https://example.test/checks/{i}")),
      workflow_name: Some("ci".into()),
      started_at: None,
      completed_at: None,
    })
    .collect();
  app.apply_pr_fetch_result(Ok(pr));
  app.enter_ci_checks();
  (dir, app)
}

/// Every overlay, in the compact layout: the boxed matrix, plus the four
/// surfaces it does not carry.
fn compact_cases() -> Vec<CompactCase> {
  let mut cases: Vec<CompactCase> = sizing_matrix()
    .into_iter()
    .map(|(name, setup, _, _)| CompactCase {
      band: match name {
        "confirm" => Band::Danger,
        "create" | "edit/rename" => Band::Clean,
        _ => Band::Accent,
      },
      view: match name {
        "help" => View::Help,
        "config-panel" => View::Config,
        "command-palette" => View::CommandPalette,
        "create" => View::Create,
        "edit/rename" => View::Edit,
        "confirm" => View::Confirm,
        "open-menu" => View::OpenMenu,
        "link-prompt" => View::LinkPrompt,
        "exec-picker" => View::ExecPicker,
        "detail/agents" => View::DetailOverlay,
        "report" => View::Report,
        "command-logs" => View::CommandLogs,
        "working-tree" => View::WorkingTree,
        "commits" => View::Commits,
        "note-editor" => View::Note,
        other => panic!("the boxed matrix grew {other:?}; name the View it renders"),
      },
      name,
      setup,
      footer: true,
    })
    .collect();
  cases.push(CompactCase {
    name: "confirm/merge",
    view: View::Confirm,
    setup: Box::new(|| {
      let (dir, mut app) = app_with_the_rich_view_open("body");
      app.enter_confirm_merge();
      assert_eq!(app.view, View::Confirm, "the merge confirmation must be up");
      (dir, app)
    }),
    band: Band::Danger,
    footer: true,
  });
  cases.push(CompactCase {
    name: "clean",
    view: View::CleanReport,
    setup: Box::new(|| {
      let (dir, _) = init_repo();
      std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
      std::fs::create_dir(dir.path().join("target")).unwrap();
      std::fs::write(dir.path().join("target").join("blob"), vec![0u8; 4096]).unwrap();
      let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
      pin_bordered(&mut app);
      app.sidebar.open = false;
      app.enter_clean_overlay();
      (dir, app)
    }),
    band: Band::Accent,
    footer: true,
  });
  cases.push(CompactCase {
    name: "detail/attach",
    view: View::DetailOverlay,
    setup: Box::new(|| {
      let (dir, mut app) = make_app();
      app.open_agent_overlay();
      app.open_agent_input();
      (dir, app)
    }),
    band: Band::Accent,
    footer: true,
  });
  cases.push(CompactCase {
    name: "detail/ci-filter",
    view: View::DetailOverlay,
    setup: Box::new(|| {
      // Enough checks to FILL the fixed listing window at the size these
      // tests render (`(term.height - 12).clamp(3, 10)`, so 10 rows at
      // 120x40). A shorter list blank-pads its tail, which would leave the
      // last row empty and exercise the `without_footer` path without ever
      // reaching the condition that motivates it: a data row on the frame's
      // bottom line.
      let (dir, mut app) = app_with_ci_checks(16);
      app.ci_input_open();
      (dir, app)
    }),
    band: Band::Accent,
    // The last row here is a listing row whenever the fixed window is
    // full, so the frame paints no ground under it.
    footer: false,
  });
  cases.push(CompactCase {
    name: "link-prompt/number",
    view: View::LinkPrompt,
    setup: Box::new(|| {
      let (dir, mut app) = make_app();
      app.enter_link_prompt();
      app.handle_link_prompt_key(ratatui::crossterm::event::KeyEvent::from(
        ratatui::crossterm::event::KeyCode::Char('i'),
      ));
      (dir, app)
    }),
    band: Band::Accent,
    footer: true,
  });
  cases
}

/// Every `View`, so the guard below can walk them.
const ALL_VIEWS: [View; 18] = [
  View::List,
  View::Create,
  View::Confirm,
  View::Report,
  View::Help,
  View::OpenMenu,
  View::LinkPrompt,
  View::CommandPalette,
  View::CommandLogs,
  View::WorkingTree,
  View::Commits,
  View::Config,
  View::Pty,
  View::ExecPicker,
  View::CleanReport,
  View::Edit,
  View::DetailOverlay,
  View::Note,
];

/// The compact case that covers `view`, or why there is none.
///
/// No wildcard arm, deliberately: a new overlay variant stops this file
/// compiling until someone says which case renders it. That is the guard:
/// "no modal missed" enumerated by construction rather than by eyeball.
fn compact_case_for(view: View) -> Option<&'static str> {
  match view {
    // Not an overlay.
    View::List => None,
    // The PTY overlay renders a live child process. There is no fixture
    // for one that does not spawn a shell, and its frame is the one that
    // opts out of the footer band anyway (`without_footer`), which the CI
    // filter case below covers. Its title band goes through the same
    // `ModalFrame::render` as every other.
    View::Pty => None,
    View::Help => Some("help"),
    View::Create => Some("create"),
    View::Confirm => Some("confirm"),
    View::Report => Some("report"),
    View::OpenMenu => Some("open-menu"),
    View::LinkPrompt => Some("link-prompt"),
    View::CommandPalette => Some("command-palette"),
    View::CommandLogs => Some("command-logs"),
    View::WorkingTree => Some("working-tree"),
    View::Commits => Some("commits"),
    View::Config => Some("config-panel"),
    View::ExecPicker => Some("exec-picker"),
    View::CleanReport => Some("clean"),
    View::Edit => Some("edit/rename"),
    View::DetailOverlay => Some("detail/agents"),
    View::Note => Some("note-editor"),
  }
}

#[test]
fn every_overlay_view_has_a_compact_case() {
  let cases = compact_cases();
  for view in ALL_VIEWS {
    let Some(name) = compact_case_for(view) else {
      continue;
    };
    let case = cases
      .iter()
      .find(|c| c.name == name)
      .unwrap_or_else(|| panic!("{view:?} names the case {name:?}, which the compact matrix does not carry"));
    assert_eq!(
      case.view, view,
      "the case {name:?} is registered against {:?}, not the {view:?} it is supposed to cover",
      case.view
    );
  }
  // A duplicated name would let one case answer for two views.
  let mut names: Vec<&str> = cases.iter().map(|c| c.name).collect();
  names.sort_unstable();
  let before = names.len();
  names.dedup();
  assert_eq!(before, names.len(), "two compact cases share a name");
}

#[test]
fn every_compact_modal_is_painted_and_locatable() {
  // The oracle for everything below: a rect that came back `None` would
  // make each assertion pass over an empty buffer instead of failing.
  for case in compact_cases() {
    let (_dir, mut app) = compact_app(&case.setup);
    // The setup opened what the case says it opens. Without this the
    // coverage guard above matches a case by its name alone.
    assert_eq!(
      app.view, case.view,
      "{}: the setup left the app on {:?}",
      case.name, app.view
    );
    let buf = render_at(&mut app, 120, 40);
    let rect = compact_modal_rect(&buf);
    assert!(
      rect.is_some(),
      "{}: no compact modal on screen. rows:\n{}",
      case.name,
      row_strings(&buf).join("\n")
    );
    let (_, _, w, h) = rect.unwrap();
    assert!(
      w > 2 && h > 1,
      "{}: the frame collapsed to {w}x{h}. rows:\n{}",
      case.name,
      row_strings(&buf).join("\n")
    );
  }
}

#[test]
fn a_compact_modal_paints_no_rule_on_any_side() {
  // The ask of #594: no border on the horizontal sides, and none top or
  // bottom either: the bands replace them. Only the frame's own edges are
  // examined: a modal's *content* may legitimately hold box-drawing glyphs
  // (the commit graph does), and it never reaches the edge columns.
  const RULES: [&str; 10] = ["│", "─", "╭", "╮", "╰", "╯", "┌", "┐", "└", "┘"];
  for case in compact_cases() {
    let (_dir, mut app) = compact_app(&case.setup);
    let buf = render_at(&mut app, 120, 40);
    let (x, y, w, h) = compact_modal_rect(&buf).expect("a compact modal");
    for row in y..y + h {
      for col in [x, x + w - 1] {
        let symbol = buf[(col, row)].symbol().to_string();
        assert!(
          !RULES.contains(&symbol.as_str()),
          "{}: a rule {symbol:?} at ({col},{row}), and the compact frame has no sides. rows:\n{}",
          case.name,
          row_strings(&buf).join("\n")
        );
      }
    }
    for col in x..x + w {
      for row in [y, y + h - 1] {
        let symbol = buf[(col, row)].symbol().to_string();
        assert!(
          !RULES.contains(&symbol.as_str()),
          "{}: a rule {symbol:?} at ({col},{row}), and the compact frame has no top or bottom. rows:\n{}",
          case.name,
          row_strings(&buf).join("\n")
        );
      }
    }
  }
}

#[test]
fn a_compact_modal_opens_on_a_filled_title_band() {
  // A *background* role, so the assertion is on the cells' `bg` and not on
  // anything the flattened rows could show. The band spans the frame edge
  // to edge: a ground that stopped at its text would read as a highlighted
  // word rather than as the top of a panel.
  for case in compact_cases() {
    let (_dir, mut app) = compact_app(&case.setup);
    let role = match case.band {
      Band::Accent => app.theme.accent,
      Band::Clean => app.theme.clean,
      Band::Danger => app.theme.prunable,
    };
    let expected = gwm::tui::band_fill(role, app.theme.section_bg);
    let buf = render_at(&mut app, 120, 40);
    let (x, y, w, _) = compact_modal_rect(&buf).expect("a compact modal");
    for col in x..x + w {
      assert_eq!(
        buf[(col, y)].bg,
        expected,
        "{}: the title band must be filled edge to edge, cell ({col},{y}) is not. rows:\n{}",
        case.name,
        row_strings(&buf).join("\n")
      );
    }
  }
}

#[test]
fn a_compact_modal_closes_on_a_muted_footer_band() {
  for case in compact_cases().into_iter().filter(|c| c.footer) {
    let (_dir, mut app) = compact_app(&case.setup);
    let expected = app.theme.section_bg;
    let buf = render_at(&mut app, 120, 40);
    let (x, y, w, h) = compact_modal_rect(&buf).expect("a compact modal");
    let footer = y + h - 1;
    for col in x..x + w {
      assert_eq!(
        buf[(col, footer)].bg,
        expected,
        "{}: the footer band must be filled edge to edge, cell ({col},{footer}) is not. rows:\n{}",
        case.name,
        row_strings(&buf).join("\n")
      );
    }
  }
}

#[test]
fn a_compact_modal_keeps_a_blank_row_at_each_end_of_its_content() {
  // User feedback on PR #616: content must never sit flush against a band,
  // at either end, the way the boxed layout's interior padding already
  // guarantees. The top row is the frame's (its `inner` starts past it);
  // the bottom one belongs to the modal, which is why it is asserted here
  // rather than assumed: the four full-size overlays and the note editor
  // had no gap above their hints before this.
  for case in compact_cases() {
    let (_dir, mut app) = compact_app(&case.setup);
    let buf = render_at(&mut app, 120, 40);
    let (x, y, w, h) = compact_modal_rect(&buf).expect("a compact modal");
    let row = |r: u16| -> String { (x..x + w).map(|col| buf[(col, r)].symbol()).collect() };
    assert!(
      row(y + 1).trim().is_empty(),
      "{}: the row under the title band must be blank, got {:?}",
      case.name,
      row(y + 1)
    );
    if case.footer {
      assert!(
        row(y + h - 2).trim().is_empty(),
        "{}: the row above the footer band must be blank, got {:?}",
        case.name,
        row(y + h - 2)
      );
    }
  }
}

#[test]
fn a_modal_that_opts_out_paints_no_footer_band() {
  // `without_footer` is a claim about the last row, so it is worth an
  // assertion rather than an exemption from one: skipping these cases in
  // the test above would leave the opt-out unpinned in both directions.
  for case in compact_cases().into_iter().filter(|c| !c.footer) {
    let (_dir, mut app) = compact_app(&case.setup);
    let ground = app.theme.section_bg;
    let buf = render_at(&mut app, 120, 40);
    let (x, y, w, h) = compact_modal_rect(&buf).expect("a compact modal");
    let footer = y + h - 1;
    assert!(
      (x..x + w).any(|col| buf[(col, footer)].bg != ground),
      "{}: the last row is content, so it must not come back painted edge to edge in the footer ground. rows:\n{}",
      case.name,
      row_strings(&buf).join("\n")
    );
  }
}

#[test]
fn the_hints_ride_the_footer_band_rather_than_a_row_of_their_own() {
  // The band is a ground under the row the modal already spends on its
  // hint line, not an extra row: if the frame reserved one, the hint would
  // sit above it and the band would come back blank.
  let (_dir, mut app) = compact_app(
    &(Box::new(|| {
      let (d, mut a) = make_app();
      a.enter_help();
      (d, a)
    }) as ModalSetup),
  );
  let buf = render_at(&mut app, 120, 40);
  let (x, y, w, h) = compact_modal_rect(&buf).expect("a compact modal");
  let footer: String = (x..x + w).map(|col| buf[(col, y + h - 1)].symbol()).collect();
  assert!(
    footer.contains("close"),
    "the help overlay's close hint must be on the band itself. footer: {footer:?}"
  );
}

#[test]
fn the_working_tree_counts_ride_the_footer_band_in_compact() {
  // The boxed frame puts them in the bottom rule (`title_bottom`). Compact
  // has no rule to put them in, so they take the right of the band, the
  // same place a compact pane puts its counter.
  let (_dir, mut app) = make_app();
  app.config.tui.layout = TuiLayout::Compact;
  app.enter_working_tree();
  let counts = gwm::tui::working_tree_counts_footer(&app.working_tree.counts, &app.theme);
  let buf = render_at(&mut app, 120, 40);
  let (x, y, w, h) = compact_modal_rect(&buf).expect("a compact modal");
  let footer: String = (x..x + w).map(|col| buf[(col, y + h - 1)].symbol()).collect();
  match counts {
    // The fixture's working tree is what it is; assert against whatever the
    // pane itself would show rather than against a count pinned by hand.
    Some(line) => {
      let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
      let text = text.trim();
      assert!(
        footer.contains(text),
        "the counts {text:?} must ride the footer band. footer: {footer:?}"
      );
    }
    None => assert!(
      footer.contains("close"),
      "with no counts the band still carries the hints. footer: {footer:?}"
    ),
  }
}

#[test]
fn the_background_is_shaded_behind_a_compact_modal_and_the_modal_is_not() {
  // A compact modal has no rule, so the ground does the separating.
  use ratatui::style::Modifier;
  let (_dir, mut app) = make_app();
  app.config.tui.layout = TuiLayout::Compact;
  // An RGB preset: the shading mixes toward black, and a palette built from
  // ANSI names has no components to mix, so that theme keeps `DIM` alone,
  // which is the documented fallback and not what this test is about. The
  // default theme is exactly such a palette.
  app.theme = gwm::tui::theme::Theme::preset("claude-dark").expect("a built-in RGB preset");
  let band = gwm::tui::compact_header_fill(&app.theme);
  // The worktrees pane's own header band, on the row under the app header.
  let lit = render_at(&mut app, 120, 40)[(0, 1)].bg;
  assert_eq!(lit, band, "the fixture must put the pane's band where this reads it");

  app.enter_help();
  let buf = render_at(&mut app, 120, 40);
  let (x, y, w, h) = compact_modal_rect(&buf).expect("a compact modal");
  assert!(
    buf[(0, 0)].modifier.contains(Modifier::DIM),
    "the header behind the modal must be dimmed"
  );
  // DIM alone would not do it: it touches the foreground only, and the
  // pane's band sits directly above a full-size overlay's own band. The
  // grounds have to move apart, not just the text on them.
  assert_ne!(
    buf[(0, 1)].bg,
    band,
    "the pane's band behind the modal must be darkened, not merely dimmed"
  );
  assert!(
    !buf[(x + w / 2, y + h / 2)].modifier.contains(Modifier::DIM),
    "the modal itself must come back at full strength"
  );
}

#[test]
fn a_bordered_modal_leaves_the_background_alone() {
  // The shading is the compact frame's compensation for its missing rule.
  // The boxed layout already has the boundary, and #594 changes nothing
  // there.
  use ratatui::style::Modifier;
  let (_dir, mut app) = make_app();
  app.enter_help();
  let buf = render_at(&mut app, 120, 40);
  assert!(
    !buf[(0, 0)].modifier.contains(Modifier::DIM),
    "the boxed layout must not dim what it floats over"
  );
}

#[test]
fn a_content_sized_modal_spends_two_rows_less_in_compact() {
  // The sizing contract, measured rather than restated: boxed costs two
  // rules and two padding rows, compact costs the title band and the blank
  // row under it (the footer band is a ground under a row the modal
  // already had). A call site that kept its own `+ 4` would show up here
  // as a modal two rows too tall with dead space above the band.
  for name in ["open-menu", "link-prompt", "create", "exec-picker"] {
    let (_name, setup, _, _) = sizing_matrix()
      .into_iter()
      .find(|(n, _, _, _)| *n == name)
      .expect("the matrix carries this modal");
    let (_dir, mut boxed) = setup();
    let boxed_h = modal_rect(&render_at(&mut boxed, 120, 40)).expect("a boxed modal").3;
    let (_dir, mut compact) = compact_app(&setup);
    let compact_h = compact_modal_rect(&render_at(&mut compact, 120, 40))
      .expect("a compact modal")
      .3;
    assert_eq!(
      compact_h + 2,
      boxed_h,
      "{name}: compact is {compact_h} rows against the boxed {boxed_h}"
    );
  }
}

// ---------------------------------------------------------------------------
// Settings panel: the value column, the named sections and the tab glyphs
// (issue #623)
// ---------------------------------------------------------------------------

/// The rightmost column of `y` carrying something other than blank space or a
/// frame rule, searched inside `[x0, x1]`.
///
/// The frame glyphs are excluded rather than the scan bounded short of them,
/// because the two layouts put the modal's edge in different places: bordered
/// paints `│`, compact paints nothing at all. Excluding the glyph makes one
/// oracle serve both.
fn last_content_col(buf: &Buffer, y: u16, x0: u16, x1: u16) -> Option<u16> {
  (x0..=x1).rev().find(|&x| {
    let s = buf[(x, y)].symbol();
    !matches!(s, " " | "│" | "╮" | "╯" | "╭" | "╰" | "─" | "")
  })
}

/// The first column of `y` carrying content, same exclusions.
fn first_content_col(buf: &Buffer, y: u16, x0: u16, x1: u16) -> Option<u16> {
  (x0..=x1).find(|&x| {
    let s = buf[(x, y)].symbol();
    !matches!(s, " " | "│" | "╮" | "╯" | "╭" | "╰" | "─" | "")
  })
}

/// Locate the buffer row inside `rect` whose text contains `needle`, and
/// return `(y, right_edge)` — the row and the column its last visible cell
/// sits in. Panics when the label is not on screen, which every caller has a
/// reason to expect.
fn row_right_edge(buf: &Buffer, rect: (u16, u16, u16, u16), needle: &str) -> (u16, u16) {
  let (x, y, w, h) = rect;
  let (x0, x1) = (x, x + w - 1);
  for row in y..(y + h).min(buf.area().height) {
    let text: String = (x0..=x1).map(|c| buf[(c, row)].symbol()).collect();
    if text.contains(needle) {
      let edge = last_content_col(buf, row, x0, x1)
        .unwrap_or_else(|| panic!("row {row} contains {needle:?} but has no content cell"));
      return (row, edge);
    }
  }
  panic!(
    "label {needle:?} is not rendered inside the modal — rows:\n{}",
    row_strings(buf).join("\n")
  );
}

/// The Settings app on the TUI tab, which is the tab the issue names: it is
/// the only one long enough for sections to matter, and the only one mixing
/// every field kind (choice, bool, uint, text) in one run.
fn tui_tab_app() -> (tempfile::TempDir, App) {
  use gwm::tui::SettingsTab;
  let (dir, mut app) = make_app();
  app.config_panel.tab = SettingsTab::Tui;
  app.view = View::Config;
  (dir, app)
}

/// Labels that between them cover every field kind on the TUI tab, plus the
/// one whose label is 26 characters — two past the `{:<24}` pad the panel used
/// before #623, so its value started two cells right of everyone else's. That
/// row is why the column was broken and not merely loose.
const TUI_TAB_PROBE_LABELS: [&str; 5] = [
  "layout",                     // choice, short value
  "sidebar layout",             // choice, the widest value on the tab
  "dim unfocused pane",         // bool
  "auto refresh (s)",           // uint
  "terminal browser placed by", // choice, the 26-character label
];

#[test]
fn settings_tui_tab_anchors_every_value_to_one_right_edge() {
  // Issue #623 point 1: "a choice reads `‹ value ›`, a boolean reads `[✓]` /
  // `[ ]`, both anchored to the same right edge". Before this the value was a
  // span glued after a `{:<24}` label pad, so a tab of sixteen rows had
  // sixteen different value positions — and the 26-character label overflowed
  // the pad outright.
  let (_dir, mut app) = tui_tab_app();
  let buf = render(&mut app);
  let rect = modal_rect(&buf).expect("the Settings modal is rendered");

  let edges: Vec<(&str, u16)> = TUI_TAB_PROBE_LABELS
    .iter()
    .map(|label| (*label, row_right_edge(&buf, rect, label).1))
    .collect();
  let first = edges[0].1;
  assert!(
    edges.iter().all(|(_, e)| *e == first),
    "every value must end in the same column, got {edges:?} — rows:\n{}",
    modal_rows(&buf).join("\n")
  );
}

#[test]
fn settings_tui_tab_anchors_every_value_to_one_right_edge_in_the_compact_layout() {
  // Both `[tui] layout` values must keep working (issue #623, and #594 which
  // made the modal frame follow that key). The compact frame has a different
  // origin and a different width, so an assertion that held only under the
  // boxed frame would be half a guard.
  let (_dir, mut app) = tui_tab_app();
  app.config.tui.layout = TuiLayout::Compact;
  let buf = render(&mut app);
  let rect = compact_modal_rect(&buf).expect("the Settings modal is rendered in the compact layout");

  let edges: Vec<(&str, u16)> = TUI_TAB_PROBE_LABELS
    .iter()
    .map(|label| (*label, row_right_edge(&buf, rect, label).1))
    .collect();
  let first = edges[0].1;
  assert!(
    edges.iter().all(|(_, e)| *e == first),
    "compact layout: every value must end in the same column, got {edges:?} — rows:\n{}",
    row_strings(&buf).join("\n")
  );
}

#[test]
fn settings_value_column_follows_its_labels_and_not_the_frame_edge() {
  // Issue #622's lesson, one modal over: a column pinned to the right edge of
  // a rect the content does not fill is worse than the inline value it
  // replaces — the eye loses the row↔value link across the gap. So the column
  // is placed against the widest *label*, and the frame growing by 32 columns
  // must not move it.
  //
  // This is what makes the guard above non-vacuous: on its own, "every value
  // ends in the same column" is satisfied by a column welded to the frame.
  let (_dir, mut app) = tui_tab_app();

  let mut gaps = Vec::new();
  for term_w in [100u16, 200] {
    let buf = render_at(&mut app, term_w, 44);
    let rect = modal_rect(&buf).expect("the Settings modal is rendered");
    let (row, edge) = row_right_edge(&buf, rect, "sidebar layout");
    let start = first_content_col(&buf, row, rect.0, rect.0 + rect.2 - 1).expect("the row has content");
    // The whole block: marker + the widest label + the gap + the widest value.
    gaps.push((term_w, edge - start, rect.2));
  }
  assert_eq!(
    gaps[0].1, gaps[1].1,
    "the value column must not stretch with the frame: {gaps:?}"
  );
  assert!(
    gaps[1].2 > gaps[0].2,
    "the frame itself must actually be wider at 200 columns, else the check is vacuous: {gaps:?}"
  );
}

#[test]
fn settings_tui_tab_rules_off_its_named_sections() {
  // Issue #623 point 2: the TUI tab mixes layout, sidebar, mux, clipboard,
  // browser and refresh knobs in one undivided run. A labelled rule groups
  // them.
  let (_dir, mut app) = tui_tab_app();
  let buf = render(&mut app);
  let rows = modal_rows(&buf).join("\n");
  for section in ["Appearance", "Sidebar", "Multiplexer", "Browser"] {
    assert!(
      rows.contains(section),
      "the TUI tab must rule off a {section:?} section — rows:\n{rows}"
    );
  }
  assert!(
    rows.contains("─ Sidebar "),
    "a section reads as a labelled rule, not a bare word — rows:\n{rows}"
  );
}

#[test]
fn settings_tab_strip_carries_one_glyph_per_tab() {
  // Issue #623 point 3: the tab strip is the first thing read, and five bare
  // words give the eye nothing to land on.
  use gwm::tui::SettingsTab;
  let (_dir, mut app) = make_app();
  app.view = View::Config;
  let buf = render(&mut app);
  let rows = modal_rows(&buf).join("\n");
  for tab in SettingsTab::ALL {
    let glyph = tab.glyph();
    assert!(
      !glyph.is_empty(),
      "{:?} must carry a glyph in the tab strip",
      tab.label()
    );
    assert!(
      rows.contains(&format!("{glyph} {}", tab.label())),
      "the strip must read {glyph:?} then {:?} — rows:\n{rows}",
      tab.label()
    );
  }
  // Every glyph is distinct, or the strip orients nothing.
  let mut seen: Vec<&str> = SettingsTab::ALL.iter().map(|t| t.glyph()).collect();
  seen.sort_unstable();
  let before = seen.len();
  seen.dedup();
  assert_eq!(before, seen.len(), "the tab glyphs must all differ, got {seen:?}");
}

#[test]
fn settings_footer_names_the_move_and_adjust_verbs_on_an_editable_tab() {
  // Issue #623 point 4. The footer has existed since #279, but on an editable
  // tab it named `cycle`, `section`, `layer` and `close` — nothing about
  // moving between rows, and nothing about the arrows the `‹ ›` marker
  // advertises.
  let (_dir, mut app) = tui_tab_app();
  let buf = render(&mut app);
  let rows = modal_rows(&buf).join("\n");
  assert!(
    rows.contains("move"),
    "the footer must name the move verb — rows:\n{rows}"
  );
  assert!(
    rows.contains("adjust"),
    "the footer must name the adjust verb — rows:\n{rows}"
  );
}

#[test]
#[ignore = "not an assertion: prints the Settings panel so a human can look at it"]
fn dump_the_settings_panel() {
  // Issue #623 is a layout change, and a column, a rule and a glyph strip are
  // only really judged by eye. `GWM_DUMP_TAB` picks the tab (`theme`,
  // `worktree`, `tui`, `keys`, `all`; default `tui`), `GWM_DUMP_COMPACT=1`
  // flips the frame:
  //
  //   GWM_DUMP_TAB=tui cargo test --test tui_modal_render_tests \
  //     dump_the_settings_panel -- --ignored --nocapture
  use gwm::tui::SettingsTab;
  let (_dir, mut app) = make_app();
  app.config_panel.tab = match std::env::var("GWM_DUMP_TAB").as_deref() {
    Ok("theme") => SettingsTab::Theme,
    Ok("worktree") => SettingsTab::Worktree,
    Ok("keys") => SettingsTab::Keys,
    Ok("all") => SettingsTab::All,
    _ => SettingsTab::Tui,
  };
  if std::env::var_os("GWM_DUMP_COMPACT").is_some() {
    app.config.tui.layout = TuiLayout::Compact;
  }
  app.view = View::Config;
  let buf = render(&mut app);
  for row in row_strings(&buf) {
    println!("{row}");
  }
}
