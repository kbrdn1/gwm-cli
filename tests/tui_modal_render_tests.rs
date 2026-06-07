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
use gwm::tui::{draw, App, LinkTarget, View};
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
  (dir, app)
}

/// A synthetic, deletable (non-main) worktree row so the Confirm modal
/// renders its destructive-summary body instead of the "nothing
/// selected" fallback.
fn deletable_worktree(name: &str) -> WorktreeInfo {
  WorktreeInfo {
    name: name.into(),
    path: PathBuf::from(format!("/tmp/gwm-test/{}", name)),
    branch: Some(format!("feat/#235-{}", name)),
    head: Some("0123456789abcdef0123456789abcdef01234567".into()),
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: gwm::github::BranchLink::empty(),
    age: None,
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
fn confirm_modal_renders_title_target_and_buttons() {
  let (_dir, mut app) = make_app();
  // Inject a deletable worktree and select it so the modal renders its
  // destructive-summary body (the "nothing selected" branch has no
  // buttons). `view` is set directly: this test pins the *render*, not
  // the `enter_confirm_delete` guard (covered in tui_app_tests.rs).
  app.worktrees.push(deletable_worktree("feat-235-net"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.view = View::Confirm;
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
