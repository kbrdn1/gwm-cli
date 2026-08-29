//! Tests for the generic chord buffer in `App` (issue #87).
//!
//! Drives `App::dispatch_key` end-to-end: feed it crossterm
//! `KeyEvent`s, observe the returned `Option<Action>` and the
//! internal pending-keys buffer state. The dispatcher is the bridge
//! between raw terminal events and the rebindable keymap — it owns
//! the "is this the first half of a chord?" decision the old hard-
//! coded `gg` handler used to make in `App::handle_g`.
//!
//! These tests deliberately stay below the event-loop layer: they do
//! not touch ratatui or crossterm's terminal handle. The harness
//! mirrors the one in `tests/tui_app_tests.rs` (an `App` built on a
//! tempdir-backed repo).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwm::tui::keymap::Action;
use gwm::tui::App;

mod common;
use common::init_repo;

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();
  (dir, app)
}

fn press(c: char) -> KeyEvent {
  KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn press_named(code: KeyCode) -> KeyEvent {
  KeyEvent::new(code, KeyModifiers::empty())
}

/// An uppercase letter delivered *with* the SHIFT bit, as modern
/// terminals (Ghostty / Kitty / WezTerm) report shifted keys.
fn press_shift_upper(c: char) -> KeyEvent {
  KeyEvent::new(KeyCode::Char(c.to_ascii_uppercase()), KeyModifiers::SHIFT)
}

/// The base (lowercase) letter delivered with the SHIFT bit, as the
/// kitty keyboard protocol reports a shifted key (base layout key +
/// modifier rather than the resolved uppercase glyph).
fn press_shift_lower(c: char) -> KeyEvent {
  KeyEvent::new(KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::SHIFT)
}

#[test]
fn single_key_dispatches_action_immediately() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('j')), Some(Action::Down));
  assert!(app.pending_chord_is_empty(), "buffer must clear after a matched action");
}

#[test]
fn first_g_arms_pending_buffer() {
  // `g` alone is the strict prefix of the default `g g` (Top). The
  // dispatcher must keep the buffer armed and return None so the
  // event loop can render a status hint.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert!(
    !app.pending_chord_is_empty(),
    "first g must leave the chord buffer armed"
  );
}

#[test]
fn second_g_completes_chord() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert_eq!(app.dispatch_key(press('g')), Some(Action::Top));
  assert!(
    app.pending_chord_is_empty(),
    "buffer must clear after the chord matches"
  );
}

#[test]
fn mismatched_second_key_falls_back_to_single_key_dispatch() {
  // Vim's classic recovery: `g j` is not a bound chord, but `j` alone
  // is bound to Down. The dispatcher must clear the buffer AND retry
  // the new stroke alone so the user's `j` still navigates down.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert_eq!(app.dispatch_key(press('j')), Some(Action::Down));
  assert!(app.pending_chord_is_empty());
}

#[test]
fn mismatched_second_key_with_no_fallback_clears_buffer() {
  // `g` then `u` — neither a chord match nor a single-key binding.
  // Returns None and clears the buffer; the event loop ignores it.
  //
  // The key here is whichever one is currently UNBOUND, and it has moved
  // twice: `z` until #484 gave it `cycle_sidebar_layout`, then `m` until
  // #551 gave it `merge_pr`. That churn is the test doing its job — it can
  // only assert "an unbound key arms nothing" while the key it names is
  // actually unbound.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('g')), None);
  assert_eq!(app.dispatch_key(press('u')), None);
  assert!(app.pending_chord_is_empty());
}

#[test]
fn unbound_key_returns_none_without_arming_buffer() {
  let (_dir, mut app) = make_app();
  // `u` is the free key since #551 put `merge_pr` on `m`.
  assert_eq!(app.dispatch_key(press('u')), None);
  assert!(app.pending_chord_is_empty());
}

#[test]
fn named_key_dispatch() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press_named(KeyCode::Down)), Some(Action::Down));
  assert_eq!(app.dispatch_key(press_named(KeyCode::Tab)), Some(Action::FocusSwap));
}

#[test]
fn shifted_uppercase_g_dispatches_bottom() {
  // The default `Bottom` binding is `G` (uppercase char, no modifier),
  // not `Shift+g`. Most terminals deliver Shift+G as `KeyCode::Char('G')`
  // sans modifier, which is what `KeyStroke::from_event` consumes.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('G')), Some(Action::Bottom));
}

#[test]
fn uppercase_binding_matches_shift_modifier_variants() {
  // Issue #188 regression: on terminals that DO report the SHIFT bit
  // for an uppercase letter (Ghostty / Kitty / WezTerm), `from_event`
  // used to keep SHIFT while the bound chord `"V"` carried none, so the
  // lookup missed and nothing fired — the symptom reported on PR #192
  // where `V` / `H` did nothing, not even a status-bar update. A char
  // keystroke must match its binding regardless of how the terminal
  // encodes the shift (bare uppercase, uppercase+SHIFT, or base+SHIFT).
  let (_dir, mut app) = make_app();
  assert_eq!(
    app.dispatch_key(press_shift_upper('V')),
    Some(Action::ToggleSidebar),
    "uppercase char + SHIFT must resolve the `V` binding"
  );
  assert_eq!(
    app.dispatch_key(press_shift_lower('h')),
    Some(Action::Macro2),
    "base char + SHIFT (kitty-style) must resolve the `H` binding"
  );
  // And the pre-existing bare-uppercase path still works.
  assert_eq!(app.dispatch_key(press('G')), Some(Action::Bottom));
}

#[test]
fn digit_keys_dispatch_pane_focus_actions() {
  // Issue #217: `1` focuses the worktrees pane, `2` the status (sidebar)
  // pane. Both are rebindable verbs so they route through the keymap.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('1')), Some(Action::FocusWorktrees));
  assert_eq!(app.dispatch_key(press('2')), Some(Action::FocusStatus));
}

#[test]
fn the_commit_and_check_keys_mean_the_same_in_both_panes() {
  // Issue #593: `c` opens the commit listing, `C` the CI checks, and
  // neither changes meaning under the focus. That uniformity is the whole
  // point of the rebind: it replaced the #436 contextual routing, which
  // gave the status pane its own `c`. It displaced the rename onto `e`,
  // the letter that names it, and `exit_to_worktree` onto `E`.
  let (_dir, mut app) = make_app();
  for focus_status in [false, true] {
    if focus_status {
      app.focus_status();
    } else {
      app.focus_worktrees();
    }
    assert_eq!(
      app.dispatch_key(press('c')),
      Some(Action::Commits),
      "c is the commit listing (status focus = {focus_status})"
    );
    assert_eq!(
      app.dispatch_key(press_shift_upper('C')),
      Some(Action::CiChecks),
      "C is the CI checks (status focus = {focus_status})"
    );
    assert_eq!(
      app.dispatch_key(press('e')),
      Some(Action::EditWorktree),
      "the rename took the letter that names it (status focus = {focus_status})"
    );
    assert_eq!(
      app.dispatch_key(press_shift_upper('E')),
      Some(Action::ExitToWorktree),
      "and the exit took the shifted one (status focus = {focus_status})"
    );
  }
}

#[test]
fn focus_actions_respect_user_keymap_override() {
  // `[tui.keys]` must be able to rebind the new focus verbs like any other
  // action — the override replaces the default `2`.
  let (dir, _) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[tui.keys]
focus_status = ["F2"]
"#,
  )
  .unwrap();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  assert_eq!(
    app.dispatch_key(press('2')),
    None,
    "default 2 must be replaced by the override"
  );
  assert_eq!(
    app.dispatch_key(press_named(KeyCode::F(2))),
    Some(Action::FocusStatus),
    "the F2 override must fire focus_status"
  );
}

#[test]
fn help_overlay_lists_pane_focus_bindings() {
  use gwm::tui::help_lines;
  use gwm::tui::keymap::Keymap;

  let km = Keymap::defaults();
  let lines = help_lines(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults(), false);
  assert!(
    lines.iter().any(|l| l.starts_with("  1 ")),
    "expected the default `1` focus binding in the help overlay:\n{}",
    lines.join("\n")
  );
  assert!(
    lines.iter().any(|l| l.starts_with("  2 ")),
    "expected the default `2` focus binding in the help overlay:\n{}",
    lines.join("\n")
  );
}

#[test]
fn s_dispatches_sync() {
  // #290: `s` is now bound to `Sync` (git fetch + pull --rebase).
  // `ToggleSidebarMode` was moved to an unbound action.
  let (_dir, mut app) = make_app();
  assert_eq!(app.dispatch_key(press('s')), Some(gwm::tui::keymap::Action::Sync));
}

#[test]
fn help_overlay_lists_sync() {
  // #290: `s` → Sync. The help overlay must show `s` next to the
  // sync/pull+fetch entry, not `toggle_sidebar_mode`.
  use gwm::tui::help_lines;
  use gwm::tui::keymap::Keymap;

  let km = Keymap::defaults();
  let lines = help_lines(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults(), false);
  let row = lines
    .iter()
    .find(|l| l.contains("sync") || l.contains("pull") || l.contains("fetch"))
    .unwrap_or_else(|| panic!("expected a sync row in:\n{}", lines.join("\n")));
  assert!(
    row.starts_with("  s ") || row.starts_with("  s\t"),
    "expected the default `s` binding in the keys column, got: {row}"
  );
}

#[test]
fn help_overlay_reflects_user_keymap_override() {
  // The help overlay must read from the resolved keymap rather than
  // hard-coded strings, so a user who rebinds `down = ["Ctrl+n"]`
  // sees `Ctrl+n` next to "next" in `?`. Otherwise the discoverable
  // documentation drifts from the actual bindings — the worst kind
  // of doc bug for a TUI.
  use gwm::tui::help_lines;
  use gwm::tui::keymap::Keymap;

  let mut km = Keymap::defaults();
  km.apply_override(
    gwm::tui::keymap::Action::Down,
    vec![gwm::tui::keymap::KeyStroke::parse_chord("Ctrl+n").unwrap()],
  )
  .unwrap();

  let lines = help_lines(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults(), false);
  let next_line = lines
    .iter()
    .find(|l| l.contains("next"))
    .unwrap_or_else(|| panic!("expected a `next` line in:\n{}", lines.join("\n")));
  assert!(
    next_line.contains("Ctrl+n"),
    "expected the override to appear next to `next`, got: {next_line}"
  );
  assert!(
    !next_line.contains("j"),
    "the default `j` binding must NOT appear after the override, got: {next_line}"
  );
}

#[test]
fn user_keymap_override_dispatches_through_app() {
  // End-to-end: writing `[tui.keys] down = ["Ctrl+n"]` in `.gwm.toml`
  // must change which keystroke fires `Action::Down` in the TUI. This
  // is the contract the whole #87 stack exists to deliver — surface
  // it as one concrete test so a regression in any layer
  // (Config::load → Keymap → App::keymap → dispatch_key) lights up
  // here with an unambiguous message.
  let (dir, _) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[tui.keys]
down = ["Ctrl+n"]
"#,
  )
  .unwrap();

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();

  // The default `j` no longer fires Down — the override replaces.
  assert_eq!(app.dispatch_key(press('j')), None);

  // `Ctrl+n` does.
  let ctrl_n = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
  assert_eq!(app.dispatch_key(ctrl_n), Some(Action::Down));
}

#[test]
fn help_rows_structures_title_sections_and_entries() {
  // #187: the help overlay is built from a structured `HelpRow` list so
  // the renderer can paint coloured section headers and key badges. The
  // first row must be the title; sections and rebindable entries must
  // surface as their own variants (not flattened strings).
  use gwm::tui::help_rows;
  use gwm::tui::keymap::{Action, Keymap};
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  let rows = help_rows(
    &km,
    &gwm::tui::modal_keymap::ModalKeymap::defaults(),
    HintContext::Worktrees,
  );

  // Issue #217: the overlay title is now "Keybindings", followed by a
  // context subtitle reflecting the focused pane.
  assert!(
    matches!(rows.first(), Some(HelpRow::Title(t)) if t == "Keybindings"),
    "first row must be the Keybindings title, got: {:?}",
    rows.first()
  );
  assert!(
    rows
      .iter()
      .any(|r| matches!(r, HelpRow::Subtitle(s) if s == "worktrees")),
    "expected a `worktrees` context subtitle"
  );
  assert!(
    rows.iter().any(|r| matches!(r, HelpRow::Section(s) if s == "Global")),
    "expected a `Global` section header"
  );
  assert!(
    rows
      .iter()
      .any(|r| matches!(r, HelpRow::Section(s) if s == "Delete Worktree")),
    "expected a `Delete Worktree` section header"
  );
  assert!(
    rows
      .iter()
      .any(|r| matches!(r, HelpRow::Section(s) if s == "Issue / PR")),
    "expected a clean `Issue / PR` section header"
  );
  assert!(
    !rows
      .iter()
      .any(|r| matches!(r, HelpRow::Section(s) if s.contains("(#67)"))),
    "Which Key section headers must not expose internal issue references"
  );
  for pair in rows.windows(2) {
    if matches!(pair[0], HelpRow::Section(_)) {
      assert!(
        matches!(pair[1], HelpRow::Blank),
        "section headings should be followed by a visual gap, got {:?}",
        pair
      );
    }
  }
  // The `Down` action's default `j` binding must surface as an Entry
  // with the resolved chord in its `keys`, not baked into a string.
  let keys = km
    .list()
    .iter()
    .find(|b| b.action == Action::Down)
    .map(|b| {
      b.chords
        .iter()
        .map(|c| c.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(", ")
    })
    .unwrap();
  assert!(
    rows
      .iter()
      .any(|r| matches!(r, HelpRow::Entry { keys: k, label } if *k == keys && label.starts_with("next"))),
    "expected a `next` entry carrying the resolved `Down` chord {keys:?}"
  );
}

#[test]
fn help_rows_link_descriptions_track_modal_rebindings() {
  // #219 review (P3): the help table's `open menu` / `link prompt` rows hard-
  // coded `i=issue · p=pull request` and `j/k + enter, or i/p`. Those keys are
  // the OpenMenu / LinkChooseTarget modal verbs, so a rebind must show through
  // instead of advertising keys that no longer work.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::OpenMenuIssue, vec![parse_single("x").unwrap()])
    .unwrap();
  modal
    .apply_override(ModalAction::LinkChooseIssue, vec![parse_single("z").unwrap()])
    .unwrap();
  let rows = help_rows(&km, &modal, HintContext::Worktrees);

  let label_of = |needle: &str| -> String {
    rows
      .iter()
      .find_map(|r| match r {
        HelpRow::Entry { label, .. } if label.contains(needle) => Some(label.clone()),
        _ => None,
      })
      .unwrap_or_else(|| panic!("no help entry containing {needle:?}: {rows:?}"))
  };

  let open_menu = label_of("open menu");
  assert!(
    open_menu.contains("x=issue") && !open_menu.contains("i=issue"),
    "open-menu help must use the rebound issue key: {open_menu:?}"
  );
  let link_prompt = label_of("link prompt");
  assert!(
    link_prompt.contains('z'),
    "link-prompt help must use the rebound choose-target issue key: {link_prompt:?}"
  );
}

#[test]
fn help_rows_link_descriptions_drop_unbound_keys() {
  // #219 review (P3): when a direct-pick verb is explicitly unbound, the help
  // description must not fall back to the literal `i` / `enter` — drop the
  // clause like every other modal hint instead of advertising a dead key.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::{ModalAction, ModalKeymap};
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  let mut modal = ModalKeymap::defaults();
  modal.apply_override(ModalAction::OpenMenuIssue, vec![]).unwrap();
  modal.apply_override(ModalAction::LinkChooseIssue, vec![]).unwrap();
  let rows = help_rows(&km, &modal, HintContext::Worktrees);
  let label_of = |needle: &str| -> String {
    rows
      .iter()
      .find_map(|r| match r {
        HelpRow::Entry { label, .. } if label.contains(needle) => Some(label.clone()),
        _ => None,
      })
      .unwrap_or_else(|| panic!("no help entry containing {needle:?}"))
  };

  let open_menu = label_of("open menu");
  assert!(
    !open_menu.contains("=issue"),
    "an unbound open-menu issue pick must drop from the help text: {open_menu:?}"
  );
  assert!(
    open_menu.contains("pull request"),
    "the still-bound pr pick must remain: {open_menu:?}"
  );
  let link_prompt = label_of("link prompt");
  assert!(
    !link_prompt.contains("i/p"),
    "the unbound link issue pick must drop (no `i/p`): {link_prompt:?}"
  );
}

#[test]
fn help_overlay_documents_every_action() {
  // #334: the exec / clean overlays were wired into the keymap, the
  // command palette, and the modal contexts but silently missed the
  // hand-curated `help_rows` list, so the `?` overlay never documented
  // `x` / `X` — and no test caught it (CI stayed green). The palette
  // guards itself with `registry_covers_every_action_variant`; mirror
  // that for the help overlay so a future `Action` can't land bound and
  // reachable yet undocumented in `?`.
  //
  // Every `Action` carries a default binding (the loop below asserts
  // it), so each is identified by the chord string the overlay renders.
  // The list-view (`Worktrees`) context documents the full surface — the
  // picker context deliberately hides the worktree-mutating verbs.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::{Action, Keymap};
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  let rows = help_rows(&km, &ModalKeymap::defaults(), HintContext::Worktrees);

  // The chord(s) each rebindable entry documents, exactly as `help_rows`
  // renders them (per-chord keys joined by spaces, chords by ", ").
  let documented: std::collections::HashSet<&str> = rows
    .iter()
    .filter_map(|r| match r {
      HelpRow::Entry { keys, .. } if !keys.is_empty() => Some(keys.as_str()),
      _ => None,
    })
    .collect();

  // Same chord rendering `help_rows` uses internally (its `keys_for`).
  let keys_for = |action: Action| -> String {
    km.list()
      .iter()
      .find(|b| b.action == action)
      .map(|b| {
        b.chords
          .iter()
          .map(|c| c.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" "))
          .collect::<Vec<_>>()
          .join(", ")
      })
      .unwrap_or_default()
  };

  for action in Action::all() {
    let keys = keys_for(action);
    assert!(
      !keys.is_empty(),
      "Action::{action:?} has no default binding — the help-completeness guard \
       identifies actions by their chord; bind it (or document the exception)"
    );
    assert!(
      documented.contains(keys.as_str()),
      "Action::{action:?} (keys {keys:?}) is not documented in the list-view help \
       overlay — add an `entry(Action::{action:?}, …)` to `help_rows` (issue #334)"
    );
  }
}

#[test]
fn help_overlay_documents_every_modal_action_in_its_section() {
  // #453: the #334 guard above pins `Action::all()` only, so modal verbs
  // kept drifting out of the overlay — most of the modal key contexts were
  // absent entirely. A flat chord-string check would be toothless here
  // (modal verbs share their defaults: `Esc, q` closes half the modals),
  // so the guard is per SECTION: each `KeyContext` maps to the section
  // title expected to document it, and every one of its `ModalAction`s
  // must surface inside that section with the exact keys string
  // `modal_entry` renders (`keys_display`). Rebinds show through since
  // both sides read the same `ModalKeymap`.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::{KeyContext, ModalAction, ModalKeymap};
  use gwm::tui::{HelpRow, HintContext};

  let section_for = |ctx: KeyContext| -> &'static str {
    match ctx {
      KeyContext::Create => "Create Form",
      KeyContext::Confirm => "Delete Worktree",
      KeyContext::Help => "Help Overlay",
      KeyContext::Detail => "Agent Sessions",
      KeyContext::CommandLogs => "Command Logs",
      KeyContext::WorkingTree => "Working Tree",
      KeyContext::Commits => "Commits",
      KeyContext::Config | KeyContext::ConfigEdit => "Settings",
      KeyContext::Report => "Bootstrap Report",
      KeyContext::OpenMenu => "Browse Links",
      KeyContext::CommandPalette => "Command Palette",
      KeyContext::LinkChooseTarget | KeyContext::LinkInputNumber => "Link Prompt",
      KeyContext::ExecPicker => "Exec Profiles",
      KeyContext::Clean => "Clean Reclaim",
      KeyContext::CiChecks => "CI Checks",
      KeyContext::RichView => "PR / Issue View",
      KeyContext::Note => "Note Editor",
    }
  };

  let modal = ModalKeymap::defaults();
  let rows = help_rows(&Keymap::defaults(), &modal, HintContext::Worktrees);

  // section title -> keys string -> number of entries documenting it. A
  // COUNT, not a set (Codex review #456): verbs can share their keys
  // inside one section (both Link Prompt cancels are `Esc`), so mere
  // presence would let one of them drop out of the overlay unnoticed.
  let mut sections: std::collections::HashMap<String, std::collections::HashMap<String, usize>> =
    std::collections::HashMap::new();
  let mut current: Option<String> = None;
  for row in &rows {
    match row {
      HelpRow::Section(title) => current = Some(title.clone()),
      HelpRow::Entry { keys, .. } if !keys.is_empty() => {
        if let Some(section) = &current {
          *sections
            .entry(section.clone())
            .or_default()
            .entry(keys.clone())
            .or_default() += 1;
        }
      }
      _ => {}
    }
  }

  // Expected multiset: how many entries must carry each (section, keys)
  // pair. Modal verbs AND the fixed (non-rebindable) controls the contexts
  // handle outside ModalAction share ONE ledger (Codex review #456, third
  // round): a fixed `Backspace` row must not be able to cover for a future
  // modal verb defaulting to `Backspace` in the same section — summing
  // both expectations makes either omission trip the count.
  let mut expected: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
  for action in ModalAction::all() {
    let keys = modal.keys_display(action);
    assert!(
      !keys.is_empty(),
      "ModalAction::{action:?} has no default binding — the guard identifies \
       verbs by their keys string; bind it (or document the exception)"
    );
    *expected
      .entry((section_for(action.context()).to_string(), keys))
      .or_default() += 1;
  }
  // The exhaustive fixed-control ledger of the modal sections: the free
  // typing of the Create form's issue / description fields, the palette's
  // fuzzy filter charset (lowercase ASCII, digits, `_`, `-` — nothing
  // else), the value / number typing and Backspace erasers, the two input
  // sub-modes that bypass the rebindable verbs entirely (agents
  // attach-by-id prompt, CI checks filter), and the PTY escape hatch.
  for (section, keys) in [
    ("Create Form", "0-9"),
    ("Create Form", "any char"),
    ("Create Form", "Backspace"),
    ("Link Prompt", "0-9"),
    ("Link Prompt", "Backspace"),
    ("Command Palette", "a-z 0-9 _ -"),
    ("Command Palette", "Backspace"),
    ("Settings", "any char"),
    ("Settings", "Backspace"),
    ("Settings", "enter"),
    ("Settings", "Esc"),
    ("Agent Sessions", "any char"),
    ("Agent Sessions", "Backspace"),
    ("Agent Sessions", "Up/Down"),
    ("Agent Sessions", "enter"),
    ("Agent Sessions", "Esc"),
    ("CI Checks", "any char"),
    ("CI Checks", "Backspace"),
    ("CI Checks", "Up/Down"),
    ("CI Checks", "enter"),
    ("CI Checks", "Esc"),
    ("PTY Overlay", "Esc"),
  ] {
    *expected.entry((section.to_string(), keys.to_string())).or_default() += 1;
  }
  for ((section, keys), wanted) in &expected {
    let have = sections.get(section).and_then(|m| m.get(keys)).copied().unwrap_or(0);
    assert!(
      have >= *wanted,
      "the {section:?} section documents {have} entr(y/ies) with keys {keys:?} but \
       {wanted} are required (modal verbs + fixed controls) — one is missing from \
       `help_rows` (issue #453)"
    );
  }
}

#[test]
fn picker_help_hides_every_modal_section() {
  // Codex review #456 (iteration 8): in a real `gwm switch` the filter
  // bar is ALWAYS active — its only exits confirm (Enter) or cancel
  // (Esc) the pick — so no overlay is reachable no matter what
  // `run_action` would allow: every printable key (`?`, `:`, `3`, `4`,
  // `o`) types into the filter instead. Advertising the modal sections
  // there described an impossible state; the picker help documents the
  // pick/cancel/filter surface only.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::{HelpRow, HintContext};

  let rows = help_rows(&Keymap::defaults(), &ModalKeymap::defaults(), HintContext::Picker);
  let sections: std::collections::HashSet<String> = rows
    .iter()
    .filter_map(|r| match r {
      HelpRow::Section(t) => Some(t.clone()),
      _ => None,
    })
    .collect();
  assert_eq!(
    sections,
    ["Global", "List View"].iter().map(|s| s.to_string()).collect(),
    "the picker help must carry no modal section — none is reachable behind the always-on filter"
  );
  assert!(
    !rows
      .iter()
      .any(|r| matches!(r, HelpRow::Entry { label, .. } if label == "open the command palette")),
    "the palette opener is dead behind the picker filter and must stay hidden"
  );
}

#[test]
fn help_lines_is_help_rows_flattened() {
  // #187: `help_lines` must stay a pure flattening of `help_rows` so the
  // legacy `  {keys:<13} {label}` string contract (asserted elsewhere in
  // this file) is preserved byte-for-byte after the refactor. This pins
  // the two builders together so they can never drift.
  use gwm::tui::keymap::Keymap;
  use gwm::tui::{help_lines, help_rows, HelpRow, HintContext};

  let km = Keymap::defaults();
  for picker_mode in [false, true] {
    let ctx = if picker_mode {
      HintContext::Picker
    } else {
      HintContext::Worktrees
    };
    let expected: Vec<String> = help_rows(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults(), ctx)
      .into_iter()
      .map(|row| match row {
        HelpRow::Title(s) | HelpRow::Subtitle(s) | HelpRow::Section(s) => s,
        HelpRow::Blank => String::new(),
        HelpRow::Entry { keys, label } => {
          let keys = if keys.is_empty() { "(unbound)".to_string() } else { keys };
          format!("  {:<13} {}", keys, label)
        }
      })
      .collect();
    assert_eq!(
      help_lines(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults(), picker_mode),
      expected,
      "picker_mode={picker_mode}"
    );
  }
}

#[test]
fn help_subtitle_tracks_the_pane_context() {
  // Issue #217: opening `?` while the status pane is focused shows a
  // `status` subtitle; the picker shows `switch`.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::Keymap;
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  for (ctx, want) in [
    (HintContext::Worktrees, "worktrees"),
    (HintContext::Status, "status"),
    (HintContext::Picker, "switch"),
  ] {
    let rows = help_rows(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults(), ctx);
    assert!(
      rows.iter().any(|r| matches!(r, HelpRow::Subtitle(s) if s == want)),
      "expected `{want}` subtitle for {want} context"
    );
  }
}

#[test]
fn help_overlay_reflects_a_modal_rebind() {
  // #219: the Create Form / Delete Worktree help rows resolve from the
  // contextual modal keymap, so `[tui.keys.modal.confirm] confirm = ["o"]` shows
  // `o` next to the "confirm" row instead of the default `y`.
  use gwm::tui::help_lines;
  use gwm::tui::keymap::{KeyStroke, Keymap};
  use gwm::tui::modal_keymap::{ModalAction, ModalKeymap};

  let km = Keymap::defaults();
  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(
      ModalAction::ConfirmConfirm,
      vec![KeyStroke::new(KeyCode::Char('o'), KeyModifiers::empty())],
    )
    .unwrap();

  let lines = help_lines(&km, &modal, false);
  let row = lines
    .iter()
    .find(|l| l.trim_end().ends_with("confirm") && !l.contains("Confirm"))
    .unwrap_or_else(|| panic!("expected a `confirm` row in:\n{}", lines.join("\n")));
  assert!(
    row.starts_with("  o "),
    "expected the confirm row to show the rebound `o`, got: {row}"
  );
  // The default `y` row for confirm must be gone.
  assert!(
    !lines
      .iter()
      .any(|l| l.starts_with("  y ") && l.trim_end().ends_with("confirm")),
    "the default `y` binding must not remain after the modal rebind:\n{}",
    lines.join("\n")
  );
}

#[test]
fn help_rows_document_the_exec_and_clean_overlays() {
  // #334: the #325 exec (`x`) / clean (`X`) overlays were added to the keymap
  // and palette but omitted from the hand-curated help overlay (`?`). Pin them
  // so they can't silently disappear from `?` again.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::{Action, Keymap};
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  let modal = ModalKeymap::defaults();
  let rows = help_rows(&km, &modal, HintContext::Worktrees);

  let documents = |action: Action, label_needle: &str| -> bool {
    let keys = km.keys_display(action);
    rows
      .iter()
      .any(|r| matches!(r, HelpRow::Entry { keys: k, label } if *k == keys && label.contains(label_needle)))
  };
  assert!(
    documents(Action::ExecOverlay, "exec"),
    "the help overlay must document the exec picker (x): {rows:?}"
  );
  assert!(
    documents(Action::CleanOverlay, "reclaim"),
    "the help overlay must document the clean overlay (X): {rows:?}"
  );
}

#[test]
fn help_rows_cover_every_bound_list_view_action() {
  // #334: the help overlay is hand-curated, so a newly bound Action can be
  // reachable by key + palette yet missing from `?` (exactly how exec/clean
  // slipped). Guard it: every action bound by default must appear in the List
  // View help, except the handful documented only in their own modal context.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::{Action, Keymap};
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  let modal = ModalKeymap::defaults();
  let rows = help_rows(&km, &modal, HintContext::Worktrees);
  let documented: std::collections::HashSet<String> = rows
    .iter()
    .filter_map(|r| match r {
      HelpRow::Entry { keys, .. } if !keys.is_empty() => Some(keys.clone()),
      _ => None,
    })
    .collect();

  // Strict: EVERY bound action is documented today (no exclusions), so a new
  // omission goes red immediately.
  for action in Action::all() {
    let keys = km.keys_display(action);
    assert!(
      documented.contains(&keys),
      "Action::{action:?} (keys {keys:?}) is bound but missing from the help overlay — add it to help_rows()"
    );
  }
}

#[test]
fn help_rows_hide_exec_and_clean_in_switch_picker_mode() {
  // #334 review: `x` / `X` are picker-gated (`run_action` no-ops them in
  // `gwm switch`), so the help overlay must not advertise them in the Picker
  // context — only in the focus-mode List View.
  use gwm::tui::help_rows;
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::{HelpRow, HintContext};

  let km = Keymap::defaults();
  let modal = ModalKeymap::defaults();
  let labels: Vec<String> = help_rows(&km, &modal, HintContext::Picker)
    .iter()
    .filter_map(|r| match r {
      HelpRow::Entry { label, .. } => Some(label.clone()),
      _ => None,
    })
    .collect();
  assert!(
    !labels.iter().any(|l| l.contains("[exec.profiles]")),
    "the exec overlay must be hidden from the switch-picker help"
  );
  assert!(
    !labels.iter().any(|l| l.contains("reclaim build artifacts")),
    "the clean overlay must be hidden from the switch-picker help"
  );
}
