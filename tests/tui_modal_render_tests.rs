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
use gwm::tui::{draw, App, LinkTarget, TaskKind, View};
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
fn confirm_modal_delete_branch_row_uses_the_live_toggle_chord() {
  // Codex review on PR #292 (P2): ToggleDeleteBranch moved to `D` in #290, but
  // the delete modal's "Delete Branch" row hardcoded `p`. It must show the live
  // chord (`D`) — a key that actually toggles the option in the confirm context.
  let (_dir, mut app) = make_app();
  app.worktrees.push(deletable_worktree("feat-290-togglekey"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.view = View::Confirm;
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
  app.view = View::Confirm;
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
  app.view = View::Confirm;
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
  assert_present(&buf, "run exec profile", "exec picker title");
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
  assert_present(&buf, "reclaim build artifacts", "clean overlay title");
  assert_present(&buf, "target", "clean artifact name");
  assert_present(&buf, "total", "clean total line");
}
