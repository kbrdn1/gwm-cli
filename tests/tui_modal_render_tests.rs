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
  (dir, app)
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
  'outer: for y in 0..area.height {
    for x in 0..area.width {
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
fn the_background_paints_no_rounded_corner() {
  // The matrix below finds each modal by its rounded top-left corner. That
  // only works while nothing behind the modal draws one — if the worktree
  // table or the sidebar ever grows a rounded frame, every measurement below
  // silently starts describing the wrong rect. Prove the oracle, then use it.
  for sidebar_open in [false, true] {
    let (_dir, mut app) = make_app();
    app.sidebar.open = sidebar_open;
    let buf = render_at(&mut app, 120, 40);
    let corners = (0..buf.area().height)
      .flat_map(|y| (0..buf.area().width).map(move |x| (x, y)))
      .filter(|&(x, y)| buf[(x, y)].symbol() == "╭")
      .count();
    assert_eq!(
      corners,
      0,
      "View::List (sidebar open = {sidebar_open}) must paint no rounded corner, found {corners} — rows:\n{}",
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
    last_inner.contains("normal mode") && !last_inner.contains("save & close"),
    "and it must follow the mode into insert, got:\n{last_inner}"
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
