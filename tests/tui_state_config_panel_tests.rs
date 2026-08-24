//! Configuration panel modal state slice (issue #232).
//!
//! Pure-state contract for `tui::state::config_panel::ConfigPanel`: the
//! scroll cursor (vertical + horizontal) clamped against bounds the
//! renderer republishes each frame, plus the owned resolved-row snapshot
//! the modal paints. Mirrors the Command Logs slice's clamp tests —
//! ratatui-free, no terminal backend.

use crossterm::event::{KeyCode, KeyModifiers};
use gwm::config::{Config, ConfigRow, ConfigSource};
use gwm::tui::keymap::{Action, KeyStroke, Keymap};
use gwm::tui::modal_keymap::{ModalAction, ModalKeymap};
use gwm::tui::{build_key_rows, ConfigPanel, FieldKind, KeyTarget, SettingField, SettingsLayer, SettingsTab};

/// Index of `field` within the Settings TUI tab. Looked up rather than
/// hardcoded: inserting a field (as #365 did with `sidebar_orientation`)
/// shifts every literal index below it, silently pointing these tests at
/// the wrong row.
fn tui_idx(field: SettingField) -> usize {
  SettingsTab::Tui
    .fields()
    .iter()
    .position(|f| *f == field)
    .unwrap_or_else(|| panic!("{field:?} is not exposed in the Settings TUI tab"))
}

fn sample_row() -> ConfigRow {
  ConfigRow {
    key: "worktree.base".into(),
    value: "\"/tmp/wt\"".into(),
    source: ConfigSource::Repo,
  }
}

#[test]
fn new_starts_empty_at_the_origin() {
  let panel = ConfigPanel::new();
  assert!(panel.rows.is_empty());
  assert_eq!(panel.scroll, 0);
  assert_eq!(panel.x_scroll, 0);
  assert_eq!(panel.max_scroll, 0);
  assert_eq!(panel.max_x_scroll, 0);
}

#[test]
fn scroll_down_clamps_to_max_scroll() {
  let mut panel = ConfigPanel::new();
  panel.max_scroll = 3;
  for _ in 0..10 {
    panel.scroll_down();
  }
  assert_eq!(panel.scroll, 3, "never scrolls past the last line");
}

#[test]
fn scroll_up_saturates_at_zero() {
  let mut panel = ConfigPanel::new();
  panel.max_scroll = 5;
  panel.scroll = 2;
  panel.scroll_up();
  panel.scroll_up();
  panel.scroll_up();
  assert_eq!(panel.scroll, 0);
}

#[test]
fn horizontal_scroll_clamps_both_ways() {
  let mut panel = ConfigPanel::new();
  panel.max_x_scroll = 2;
  for _ in 0..5 {
    panel.scroll_right();
  }
  assert_eq!(panel.x_scroll, 2);
  for _ in 0..5 {
    panel.scroll_left();
  }
  assert_eq!(panel.x_scroll, 0);
}

#[test]
fn scroll_to_top_and_bottom_jump_to_the_bounds() {
  let mut panel = ConfigPanel::new();
  panel.max_scroll = 7;
  panel.scroll_to_bottom();
  assert_eq!(panel.scroll, 7);
  panel.scroll_to_top();
  assert_eq!(panel.scroll, 0);
}

#[test]
fn reset_zeroes_the_cursor_but_keeps_rows() {
  let mut panel = ConfigPanel::new();
  panel.rows.push(sample_row());
  panel.scroll = 4;
  panel.x_scroll = 2;
  panel.reset();
  assert_eq!(panel.scroll, 0);
  assert_eq!(panel.x_scroll, 0);
  assert_eq!(panel.rows.len(), 1, "reset clears the cursor, not the data");
}

// ---------------------------------------------------------------------------
// Editable Settings panel (issue #279): tabs, layer selector, field
// selection, and the numeric-input edit buffer — all pure state.
// ---------------------------------------------------------------------------

#[test]
fn new_panel_defaults_to_theme_tab_project_layer() {
  let panel = ConfigPanel::new();
  assert_eq!(panel.tab, SettingsTab::Theme);
  assert_eq!(panel.layer, SettingsLayer::Project);
  assert_eq!(panel.selected, 0);
  assert!(panel.editing.is_none());
}

#[test]
fn next_tab_cycles_theme_worktree_tui_keys_all_and_wraps() {
  let mut panel = ConfigPanel::new();
  assert_eq!(panel.tab, SettingsTab::Theme);
  panel.next_tab();
  assert_eq!(panel.tab, SettingsTab::Worktree);
  panel.next_tab();
  assert_eq!(panel.tab, SettingsTab::Tui);
  panel.next_tab();
  assert_eq!(
    panel.tab,
    SettingsTab::Keys,
    "Keys tab sits between TUI and All (issue #294)"
  );
  panel.next_tab();
  assert_eq!(panel.tab, SettingsTab::All);
  panel.next_tab();
  assert_eq!(panel.tab, SettingsTab::Theme, "wraps back to the first tab");
}

#[test]
fn prev_tab_wraps_backwards() {
  let mut panel = ConfigPanel::new();
  panel.prev_tab();
  assert_eq!(panel.tab, SettingsTab::All, "prev from the first tab wraps to the last");
}

#[test]
fn switching_tab_resets_selection_and_edit_buffer() {
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Tui;
  panel.selected = tui_idx(SettingField::ConfirmCountdown);
  panel.editing = Some("4".into());
  panel.next_tab();
  assert_eq!(panel.selected, 0, "selection resets on tab change");
  assert!(panel.editing.is_none(), "edit buffer clears on tab change");
}

#[test]
fn selected_field_follows_the_tab() {
  let mut panel = ConfigPanel::new();
  // Theme tab → theme preset.
  assert_eq!(panel.selected_field(), Some(SettingField::ThemePreset));
  // Tui tab → layout / dim unfocused / status one line / note vim / sidebar
  // position / sidebar layout / clipboard / open / countdown / auto refresh
  // in order. `layout` leads since #545: it is the structural choice the
  // rest of the tab refines, and the boolean knobs (#545, #547, #557) sit
  // right under it.
  panel.tab = SettingsTab::Tui;
  assert_eq!(panel.selected_field(), Some(SettingField::Layout));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::DimUnfocused));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::StatusOneLine));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::NoteVim));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::SidebarPosition));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::SidebarOrientation));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::Clipboard));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::OpenMode));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::ConfirmCountdown));
  panel.select_next();
  assert_eq!(panel.selected_field(), Some(SettingField::AutoRefreshSecs));
  // All tab is read-only → no editable field.
  panel.tab = SettingsTab::All;
  panel.selected = 0;
  assert_eq!(panel.selected_field(), None);
}

#[test]
fn select_next_clamps_to_the_last_field() {
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Tui;
  let last = SettingsTab::Tui.fields().len() - 1;
  for _ in 0..(last + 5) {
    panel.select_next();
  }
  assert_eq!(panel.selected, last, "never selects past the last field");
}

#[test]
fn toggle_layer_flips_project_and_global_with_matching_source() {
  let mut panel = ConfigPanel::new();
  assert_eq!(panel.layer, SettingsLayer::Project);
  assert_eq!(panel.layer.source(), ConfigSource::Repo);
  panel.toggle_layer();
  assert_eq!(panel.layer, SettingsLayer::Global);
  assert_eq!(panel.layer.source(), ConfigSource::User);
  panel.toggle_layer();
  assert_eq!(panel.layer, SettingsLayer::Project);
}

#[test]
fn begin_edit_only_arms_for_a_uint_field() {
  let mut panel = ConfigPanel::new();
  // Theme preset is a Choice → begin_edit is a no-op.
  panel.begin_edit("catppuccin");
  assert!(panel.editing.is_none(), "choice fields are not text-edited");
  // Confirm countdown is a Uint → arms the buffer.
  panel.tab = SettingsTab::Tui;
  panel.selected = tui_idx(SettingField::ConfirmCountdown);
  assert_eq!(panel.selected_field().map(SettingField::kind), Some(FieldKind::Uint));
  panel.begin_edit("4");
  assert_eq!(panel.editing.as_deref(), Some("4"));
}

#[test]
fn edit_buffer_takes_digits_only_and_commits() {
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Tui;
  panel.selected = tui_idx(SettingField::ConfirmCountdown);
  panel.begin_edit("");
  panel.push_edit_char('3');
  panel.push_edit_char('x'); // ignored — not a digit
  panel.push_edit_char('0');
  assert_eq!(panel.editing.as_deref(), Some("30"));
  panel.pop_edit_char();
  assert_eq!(panel.editing.as_deref(), Some("3"));
  assert_eq!(panel.take_edit().as_deref(), Some("3"));
  assert!(panel.editing.is_none(), "commit clears the buffer");
}

#[test]
fn auto_refresh_secs_accepts_four_plus_digit_intervals() {
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Tui;
  panel.selected = tui_idx(SettingField::AutoRefreshSecs);
  assert_eq!(panel.selected_field(), Some(SettingField::AutoRefreshSecs));
  assert_eq!(panel.selected_field().map(SettingField::kind), Some(FieldKind::Uint));
  panel.begin_edit("");
  for c in "3600".chars() {
    panel.push_edit_char(c);
  }
  assert_eq!(panel.editing.as_deref(), Some("3600"));
}

#[test]
fn take_edit_returns_the_raw_buffer() {
  // The state layer returns the raw buffer; the App commit path coerces an
  // empty numeric buffer to "0" (an empty text buffer stays empty).
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Tui;
  panel.selected = tui_idx(SettingField::ConfirmCountdown);
  panel.begin_edit("");
  assert_eq!(
    panel.take_edit().as_deref(),
    Some(""),
    "empty buffer round-trips verbatim"
  );
}

#[test]
fn text_fields_accept_non_digit_characters() {
  // Issue #279 follow-up: Worktree patterns are free-text inputs, so the
  // buffer must take letters / punctuation, not digits only.
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Worktree;
  panel.selected = 0; // base directory (Text)
  assert_eq!(panel.selected_field().map(SettingField::kind), Some(FieldKind::Text));
  panel.begin_edit("");
  for c in "{home}/wt".chars() {
    panel.push_edit_char(c);
  }
  assert_eq!(panel.editing.as_deref(), Some("{home}/wt"));
}

#[test]
fn cancel_edit_discards_the_buffer() {
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Tui;
  panel.selected = tui_idx(SettingField::ConfirmCountdown);
  panel.begin_edit("4");
  panel.push_edit_char('2');
  panel.cancel_edit();
  assert!(panel.editing.is_none());
}

// ---------------------------------------------------------------------------
// Keys tab (issue #294): the dynamic keymap-editor row model + the live
// keystroke-capture state machine — all pure, ratatui-free.
// ---------------------------------------------------------------------------

fn ch(c: char) -> KeyStroke {
  KeyStroke::new(KeyCode::Char(c), KeyModifiers::empty())
}

#[test]
fn build_key_rows_enumerates_every_global_and_modal_binding() {
  let rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  let expected = Action::all().count() + ModalAction::all().count();
  assert_eq!(rows.len(), expected, "one row per global action + per modal verb");

  // Global rows come first, in declaration order, scoped "global".
  assert_eq!(rows[0].target, KeyTarget::Global(Action::all().next().unwrap()));
  assert_eq!(rows[0].scope, "global");
  // The modal block carries a context-qualified scope.
  assert!(
    rows.iter().any(|r| r.scope == "modal.confirm" && r.label == "confirm"),
    "modal verbs are grouped by their context path"
  );
}

#[test]
fn key_rows_show_current_chords_and_source() {
  let rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |key| {
    if key == "tui.keys.quit" {
      ConfigSource::Repo
    } else {
      ConfigSource::Default
    }
  });
  let quit = rows
    .iter()
    .find(|r| r.target == KeyTarget::Global(Action::Quit))
    .expect("quit row present");
  assert_eq!(quit.keys, "q", "default quit chord shown");
  assert_eq!(quit.source, ConfigSource::Repo, "source resolved via the lookup");
}

#[test]
fn config_key_addresses_the_right_toml_table() {
  assert_eq!(KeyTarget::Global(Action::Quit).config_key(), "tui.keys.quit");
  assert_eq!(
    KeyTarget::Modal(ModalAction::ConfirmConfirm).config_key(),
    "tui.keys.modal.confirm.confirm"
  );
  // A staged (dotted) context keeps its dotted path.
  assert_eq!(
    KeyTarget::Modal(ModalAction::LinkChooseIssue).config_key(),
    "tui.keys.modal.link.choose_target.issue"
  );
}

#[test]
fn modal_targets_are_single_only_globals_are_not() {
  assert!(
    !KeyTarget::Global(Action::Down).single_only(),
    "global actions accept chords"
  );
  assert!(
    KeyTarget::Modal(ModalAction::ConfirmConfirm).single_only(),
    "modal verbs are single-stroke"
  );
}

#[test]
fn begin_capture_arms_only_on_the_keys_tab_with_a_selected_row() {
  let mut panel = ConfigPanel::new();
  panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);

  // Not on the Keys tab → no-op.
  panel.tab = SettingsTab::Theme;
  panel.begin_capture();
  assert!(panel.capture.is_none());

  // On the Keys tab → arms with the row's single_only flag.
  panel.tab = SettingsTab::Keys;
  panel.selected = 0; // first global action
  panel.begin_capture();
  let cap = panel.capture.as_ref().expect("capture armed");
  assert_eq!(cap.row, 0);
  assert!(!cap.single_only, "first row is a global action");
  assert!(cap.pending.is_empty());
}

#[test]
fn capture_accumulates_strokes_and_pop_drops_the_last() {
  let mut panel = ConfigPanel::new();
  panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  panel.tab = SettingsTab::Keys;
  panel.selected = 0;
  panel.begin_capture();

  panel.capture_push(ch('g'));
  panel.capture_push(ch('g'));
  assert_eq!(panel.capture.as_ref().unwrap().pending, vec![ch('g'), ch('g')]);
  panel.capture_pop();
  assert_eq!(panel.capture.as_ref().unwrap().pending, vec![ch('g')]);
}

#[test]
fn cancel_capture_discards_pending() {
  let mut panel = ConfigPanel::new();
  panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  panel.tab = SettingsTab::Keys;
  panel.begin_capture();
  panel.capture_push(ch('x'));
  panel.cancel_capture();
  assert!(panel.capture.is_none());
}

#[test]
fn capture_as_config_items_joins_a_chord_and_unbinds_when_empty() {
  let mut panel = ConfigPanel::new();
  panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  panel.tab = SettingsTab::Keys;
  panel.selected = 0;
  panel.begin_capture();

  // A multi-stroke global chord serialises to one space-joined element.
  panel.capture_push(ch('g'));
  panel.capture_push(ch('g'));
  assert_eq!(
    panel.capture.as_ref().unwrap().as_config_items(),
    vec!["g g".to_string()]
  );

  // Cleared pending → empty list (an unbind).
  panel.capture_pop();
  panel.capture_pop();
  assert!(panel.capture.as_ref().unwrap().as_config_items().is_empty());
}

#[test]
fn selection_clamps_to_key_rows_on_the_keys_tab() {
  let mut panel = ConfigPanel::new();
  panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  panel.tab = SettingsTab::Keys;
  for _ in 0..(panel.key_rows.len() + 50) {
    panel.select_next();
  }
  assert_eq!(panel.selected, panel.key_rows.len() - 1, "never past the last key row");
}

#[test]
fn switching_tab_clears_an_armed_capture() {
  let mut panel = ConfigPanel::new();
  panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  panel.tab = SettingsTab::Keys;
  panel.begin_capture();
  assert!(panel.capture.is_some());
  panel.next_tab();
  assert!(panel.capture.is_none(), "tab change cancels capture");
}

#[test]
fn select_is_inert_while_editing() {
  let mut panel = ConfigPanel::new();
  panel.tab = SettingsTab::Tui;
  let armed = tui_idx(SettingField::ConfirmCountdown);
  panel.selected = armed;
  panel.begin_edit("4");
  panel.select_prev();
  assert_eq!(panel.selected, armed, "navigation is suppressed while typing");
}

#[test]
fn setting_field_current_reads_the_resolved_config() {
  let cfg = Config::default();
  // Defaults: theme preset unset → "default"; sidebar right; open shell.
  assert_eq!(SettingField::ThemePreset.current(&cfg), "default");
  assert_eq!(SettingField::SidebarPosition.current(&cfg), "right");
  assert_eq!(SettingField::OpenMode.current(&cfg), "shell");
  assert_eq!(SettingField::AutoRefreshSecs.current(&cfg), "60");
}

#[test]
fn choice_fields_cycle_and_wrap_uint_fields_do_not() {
  let cfg = Config::default();
  // sidebar: right → left.
  assert_eq!(SettingField::SidebarPosition.next_choice(&cfg).as_deref(), Some("left"));
  // open mode: shell → editor.
  assert_eq!(SettingField::OpenMode.next_choice(&cfg).as_deref(), Some("editor"));
  // theme preset currently "default" (not a choice) → falls back to the first preset.
  let first = gwm::tui::theme::preset_names()[0];
  assert_eq!(SettingField::ThemePreset.next_choice(&cfg).as_deref(), Some(first));
  // confirm countdown is a Uint → no choice cycle.
  assert_eq!(SettingField::ConfirmCountdown.next_choice(&cfg), None);
  assert_eq!(SettingField::AutoRefreshSecs.next_choice(&cfg), None);
}

#[test]
fn field_source_is_looked_up_from_the_resolved_rows() {
  let mut panel = ConfigPanel::new();
  panel.rows = vec![
    ConfigRow {
      key: "tui.sidebar_position".into(),
      value: "left".into(),
      source: ConfigSource::Repo,
    },
    ConfigRow {
      key: "theme.preset".into(),
      value: "\"gruvbox\"".into(),
      source: ConfigSource::User,
    },
  ];
  assert_eq!(
    panel.field_source(SettingField::SidebarPosition),
    Some(ConfigSource::Repo)
  );
  assert_eq!(panel.field_source(SettingField::ThemePreset), Some(ConfigSource::User));
  // A field absent from the rows resolves to no source.
  assert_eq!(panel.field_source(SettingField::OpenMode), None);
}

// ---------------------------------------------------------------------------
// Choice sets vs the enums they mirror (#365 follow-up)
// ---------------------------------------------------------------------------

#[test]
fn every_choice_is_a_value_the_config_can_load_back() {
  // The Settings panel writes the selected choice string verbatim into
  // `.gwm.toml`. So every string it can offer must round-trip through a real
  // layered load — a choice list that drifts from its enum's serde spelling
  // would let the panel write a file that no longer parses, and the user only
  // finds out on the next launch.
  //
  // Covers every Choice field, not just the sidebar ones: the choice lists are
  // hand-maintained constants sitting next to the enums, and this is the
  // property that actually matters for all of them.
  let dir = tempfile::TempDir::new().unwrap();
  for field in [
    SettingField::SidebarPosition,
    SettingField::SidebarOrientation,
    SettingField::Clipboard,
    SettingField::OpenMode,
  ] {
    let key = field.key_path();
    assert!(!field.choices().is_empty(), "{key} is a Choice field with choices");
    for choice in field.choices() {
      let (table, name) = key.split_once('.').unwrap();
      let toml = match table {
        "tui" => format!("[tui]\n{name} = \"{choice}\"\n"),
        _ => format!("[{table}]\n{name} = \"{choice}\"\n"),
      };
      std::fs::write(dir.path().join(".gwm.toml"), &toml).unwrap();
      let cfg = Config::load_layered(dir.path(), None)
        .unwrap_or_else(|e| panic!("choice {choice:?} for {key} must be loadable, got: {e}"));
      assert_eq!(
        field.current(&cfg),
        *choice,
        "{key}: writing choice {choice:?} must read back as itself"
      );
    }
  }
}

#[test]
fn sidebar_choice_lists_cover_every_variant() {
  // The other drift direction: the choice list and `ALL` disagreeing — a
  // variant offered by one but not the other. The round-trip test above only
  // walks the list, so it cannot see a variant the list omits.
  //
  // Honest limit: this pins list ↔ `ALL`, not `ALL` ↔ the enum. A variant added
  // to the enum but left out of `ALL` still slips past both tests. What stops
  // that is the exhaustive `match` in `label()`, which fails to compile on a new
  // variant and forces a visit to the `ALL` right above it. Closing the gap
  // properly would need a derive (strum); not worth a dependency for two enums.
  use gwm::config::{ClipboardMode, SidebarOrientation, SidebarPosition};

  for o in SidebarOrientation::ALL {
    assert!(
      SettingField::SidebarOrientation.choices().contains(&o.label()),
      "{o:?} ({}) is missing from the sidebar orientation choices",
      o.label()
    );
  }
  assert_eq!(
    SettingField::SidebarOrientation.choices().len(),
    SidebarOrientation::ALL.len(),
    "no stale choice left behind"
  );

  for p in SidebarPosition::ALL {
    assert!(
      SettingField::SidebarPosition.choices().contains(&p.label()),
      "{p:?} ({}) is missing from the sidebar position choices",
      p.label()
    );
  }
  assert_eq!(
    SettingField::SidebarPosition.choices().len(),
    SidebarPosition::ALL.len(),
    "no stale choice left behind"
  );

  for m in ClipboardMode::ALL {
    assert!(
      SettingField::Clipboard.choices().contains(&m.label()),
      "{m:?} ({}) is missing from the clipboard choices",
      m.label()
    );
  }
  assert_eq!(
    SettingField::Clipboard.choices().len(),
    ClipboardMode::ALL.len(),
    "no stale choice left behind"
  );
}

#[test]
fn the_tui_tab_reaches_the_layout_settings() {
  // Codex review, PR #546: `[tui] layout` is the structural choice #545
  // introduced, and `bordered` is its opt-out — both were unreachable
  // from the panel the docs call the editable schema, so the only way
  // back to the pre-1.8 look was hand-editing TOML.
  //
  // Asserted as reachability plus a live round-trip of every choice,
  // rather than a position: the order is a design call and will move,
  // but a field that the tab cannot reach is a bug in any order.
  let fields = SettingsTab::Tui.fields();
  for field in [SettingField::Layout, SettingField::DimUnfocused] {
    assert!(
      fields.contains(&field),
      "{field:?} must be reachable from the TUI tab, got {fields:?}"
    );
    assert_eq!(
      field.kind(),
      if field == SettingField::DimUnfocused {
        FieldKind::Bool
      } else {
        FieldKind::Choice
      },
      "{field:?} cycles through a fixed set rather than being typed"
    );
    assert!(
      !field.choices().is_empty(),
      "{field:?} must offer choices to cycle through"
    );
  }

  // The value the panel shows for a default config is the default itself,
  // so cycling starts from where the user actually is.
  let cfg = gwm::config::Config::default();
  assert_eq!(SettingField::Layout.current(&cfg), "compact");
  assert_eq!(SettingField::DimUnfocused.current(&cfg), "false");
  // And every choice it can write is a value the config can read back —
  // the drift this panel is most able to cause.
  for choice in SettingField::Layout.choices() {
    assert!(
      gwm::config::TuiLayout::ALL.iter().any(|l| l.label() == *choice),
      "layout choice {choice:?} has no matching TuiLayout variant"
    );
  }
  for choice in SettingField::DimUnfocused.choices() {
    assert!(
      choice.parse::<bool>().is_ok(),
      "dim_unfocused choice {choice:?} must parse as a bool"
    );
  }
}

#[test]
fn the_tui_tab_reaches_the_status_fold_setting() {
  // #547: same reasoning as the layout field above — a knob the panel
  // cannot reach is a knob only a TOML editor can turn. `Bool`, not
  // `Choice`: a quoted `"true"` is what serde refuses where a bool
  // belongs, so the write would fail and the setting never change (the
  // trap PR #546 hit with `dim_unfocused`).
  let fields = SettingsTab::Tui.fields();
  assert!(
    fields.contains(&SettingField::StatusOneLine),
    "status_one_line must be reachable from the TUI tab, got {fields:?}"
  );
  assert_eq!(SettingField::StatusOneLine.kind(), FieldKind::Bool);
  assert_eq!(SettingField::StatusOneLine.key_path(), "tui.status_one_line");

  // The panel opens on the live value, and the default is `true` (#547).
  let cfg = gwm::config::Config::default();
  assert_eq!(SettingField::StatusOneLine.current(&cfg), "true");
  // Cycling from the default lands on the opt-out, and both spellings are
  // values the config can read back.
  assert_eq!(SettingField::StatusOneLine.next_choice(&cfg).as_deref(), Some("false"));
  for choice in SettingField::StatusOneLine.choices() {
    assert!(
      choice.parse::<bool>().is_ok(),
      "status_one_line choice {choice:?} must parse as a bool"
    );
  }
}

#[test]
fn the_tui_tab_reaches_the_note_mode_setting() {
  // #557: the vim mode ships on, so `note_vim = false` is the opt-out —
  // and an opt-out only a TOML editor can reach is one most users will
  // never find. Same `Bool` reasoning as the two fields above.
  let fields = SettingsTab::Tui.fields();
  assert!(
    fields.contains(&SettingField::NoteVim),
    "note_vim must be reachable from the TUI tab, got {fields:?}"
  );
  assert_eq!(SettingField::NoteVim.kind(), FieldKind::Bool);
  assert_eq!(SettingField::NoteVim.key_path(), "tui.note_vim");

  let cfg = gwm::config::Config::default();
  assert_eq!(SettingField::NoteVim.current(&cfg), "true");
  assert_eq!(SettingField::NoteVim.next_choice(&cfg).as_deref(), Some("false"));
  for choice in SettingField::NoteVim.choices() {
    assert!(
      choice.parse::<bool>().is_ok(),
      "note_vim choice {choice:?} must parse as a bool"
    );
  }
}
