//! Layer-aware config writer (`config_cli::set_value_at`) — the persistence
//! behind the in-TUI Settings panel (issue #279).
//!
//! The contract is round-trip THROUGH `Config::load_layered`: a value the
//! writer sets must come back as the typed Rust value after a real load, so
//! a serde case-mismatch (e.g. writing `"Left"` when the deserializer wants
//! `"left"`) shows up as a dead edit here rather than in the TUI. Paths are
//! injected (tempdirs), never `$HOME`.

use gwm::config::{Config, SidebarPosition, TuiOpenMode};
use gwm::config_cli::{set_array_at, set_string_at, set_value_at, unset_at};
use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
use gwm::tui::modal_keymap::{KeyContext, ModalAction};
use std::path::Path;

fn load(repo: &Path, global: Option<&Path>) -> Config {
  Config::load_layered(repo, global).expect("config should load")
}

#[test]
fn set_value_at_persists_repo_layer_and_round_trips_typed() {
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");

  set_value_at(&gwm_toml, "theme.preset", "gruvbox").unwrap();
  set_value_at(&gwm_toml, "tui.sidebar_position", "left").unwrap();

  let cfg = load(repo.path(), None);
  assert_eq!(cfg.theme.preset.as_deref(), Some("gruvbox"));
  assert_eq!(cfg.tui.sidebar_position, SidebarPosition::Left);
}

#[test]
fn set_value_at_creates_nested_tables_from_an_empty_file() {
  // `tui.open.mode` into a fresh file: toml_edit must auto-vivify the
  // `[tui]` then `[tui.open]` tables, and the value must deserialize as the
  // typed enum (lowercase serde).
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");

  set_value_at(&gwm_toml, "tui.open.mode", "editor").unwrap();

  let cfg = load(repo.path(), None);
  assert_eq!(cfg.tui.open.mode, TuiOpenMode::Editor);
}

#[test]
fn set_value_at_writes_global_layer_and_creates_parent_dir() {
  // The user-global file lives under a `gwm/` dir that may not exist yet on
  // first write — the writer must create it. Round-trip through the layered
  // load with the global injected (repo declares nothing).
  let repo = tempfile::tempdir().unwrap();
  let home = tempfile::tempdir().unwrap();
  // Parent dir intentionally absent until the writer creates it.
  let global = home.path().join("gwm").join("config.toml");
  assert!(!global.parent().unwrap().exists());

  set_value_at(&global, "tui.confirm_countdown_secs", "3").unwrap();

  assert!(global.exists(), "writer must create the global file + its parent dir");
  let cfg = load(repo.path(), Some(&global));
  assert_eq!(cfg.tui.confirm_countdown_secs, 3);
}

#[test]
fn set_string_at_preserves_numeric_looking_text_as_a_string() {
  // Review P2: a free-text value that looks like a scalar (`123`, `true`)
  // must persist as a TOML string, not be coerced — otherwise a worktree
  // base / shell command of that form writes an int/bool and breaks the
  // typed load.
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");

  set_string_at(&gwm_toml, "worktree.base", "123").unwrap();

  let raw = std::fs::read_to_string(&gwm_toml).unwrap();
  assert!(
    raw.contains("base = \"123\""),
    "value must be quoted as a string: {raw}"
  );
  let cfg = load(repo.path(), None);
  assert_eq!(cfg.worktree.base, "123");
}

#[test]
fn an_invalid_write_does_not_clobber_the_existing_file() {
  // Review P2: validation happens BEFORE the file is written, so an edit
  // that would produce an invalid Config (here a non-numeric string into a
  // u32 field) errors and leaves the previous good file untouched.
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");
  std::fs::write(&gwm_toml, "[tui]\nconfirm_countdown_secs = 4\n").unwrap();

  let result = set_string_at(&gwm_toml, "tui.confirm_countdown_secs", "abc");
  assert!(result.is_err(), "writing a non-numeric value to a u32 field must fail");

  let cfg = load(repo.path(), None);
  assert_eq!(
    cfg.tui.confirm_countdown_secs, 4,
    "the prior valid file must survive a rejected write"
  );
}

#[test]
fn editing_an_already_invalid_file_still_writes_the_change() {
  // Issue #281: validate-before-write (review P2) must not block recovery
  // edits. A pre-existing invalid file (here a non-numeric countdown) plus
  // an unrelated set still writes the change — the validation error is
  // surfaced, but the edit is not refused, so `gwm config set` can be used
  // to nudge a broken config back toward valid.
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");
  std::fs::write(&gwm_toml, "[tui]\nconfirm_countdown_secs = \"abc\"\n").unwrap();

  let result = set_string_at(&gwm_toml, "theme.preset", "gruvbox");
  assert!(result.is_err(), "the pre-existing invalid value is still surfaced");

  let raw = std::fs::read_to_string(&gwm_toml).unwrap();
  assert!(
    raw.contains("preset = \"gruvbox\""),
    "the unrelated edit must still be written to an already-invalid file: {raw}"
  );
}

// ── array write-back: keymaps (issue #294) ─────────────────────────────────

#[test]
fn set_array_at_writes_a_global_keymap_array_and_round_trips() {
  // The in-TUI Keys tab rebinds a global action by writing its chord list as
  // a TOML array under `[tui.keys]`. The write must round-trip through
  // `resolved_keymap` so the new chord actually fires.
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");

  set_array_at(&gwm_toml, "tui.keys.quit", &["Q".to_string()]).unwrap();

  let raw = std::fs::read_to_string(&gwm_toml).unwrap();
  assert!(raw.contains("quit = [\"Q\"]"), "must write a TOML array: {raw}");

  let cfg = load(repo.path(), None);
  let km = cfg.tui.keys.resolved_keymap().expect("keymap resolves");
  let q = KeyStroke::new(
    crossterm::event::KeyCode::Char('Q'),
    crossterm::event::KeyModifiers::empty(),
  );
  assert_eq!(km.lookup(&[q]), ChordResolution::Matched(Action::Quit));
}

#[test]
fn set_array_at_writes_a_modal_verb_into_its_nested_table() {
  // A modal verb lives under `[tui.keys.modal.<context>]`; the dotted key must
  // auto-vivify the `modal` -> context tables and round-trip through
  // `resolved_modal_keymap`.
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");

  set_array_at(&gwm_toml, "tui.keys.modal.confirm.confirm", &["o".to_string()]).unwrap();

  let cfg = load(repo.path(), None);
  let mk = cfg.tui.keys.resolved_modal_keymap().expect("modal keymap resolves");
  let o = KeyStroke::new(
    crossterm::event::KeyCode::Char('o'),
    crossterm::event::KeyModifiers::empty(),
  );
  assert_eq!(mk.resolve(KeyContext::Confirm, &o), Some(ModalAction::ConfirmConfirm));
}

#[test]
fn set_array_at_can_unbind_with_an_empty_array() {
  // An empty capture writes `key = []` — a legitimate unbind. The file must
  // round-trip (an unbound modal verb simply stops resolving).
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");

  set_array_at(&gwm_toml, "tui.keys.modal.confirm.confirm", &[]).unwrap();

  let cfg = load(repo.path(), None);
  let mk = cfg.tui.keys.resolved_modal_keymap().expect("modal keymap resolves");
  let y = KeyStroke::new(
    crossterm::event::KeyCode::Char('y'),
    crossterm::event::KeyModifiers::empty(),
  );
  assert_eq!(
    mk.resolve(KeyContext::Confirm, &y),
    None,
    "the default `y` is gone once `confirm` is unbound"
  );
}

#[test]
fn set_array_at_rejects_a_prefix_collision_and_leaves_the_file_untouched() {
  // Binding `refresh = ["g"]` while `top` keeps its default `g g` chord is a
  // prefix collision — refused at the validate-before-write gate so a good
  // (here: absent) file is never clobbered with an invalid keymap.
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");

  let result = set_array_at(&gwm_toml, "tui.keys.refresh", &["g".to_string()]);

  assert!(result.is_err(), "a prefix collision must be rejected");
  assert!(!gwm_toml.exists(), "a rejected write must not create the file");
}

#[test]
fn unset_at_removes_a_key_and_tolerates_absent_targets() {
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");
  set_array_at(&gwm_toml, "tui.keys.open_menu", &["B".to_string()]).unwrap();

  unset_at(&gwm_toml, "tui.keys.open_menu").unwrap();
  let raw = std::fs::read_to_string(&gwm_toml).unwrap();
  assert!(!raw.contains("open_menu"), "key removed: {raw}");

  // Removing a missing key (or from an absent file) is a no-op, not an error.
  unset_at(&gwm_toml, "tui.keys.does_not_exist").unwrap();
  unset_at(&repo.path().join("nope.toml"), "tui.keys.x").unwrap();
}

#[test]
fn set_value_at_preserves_other_keys_in_the_file() {
  // Surgical edit: setting one key must not drop unrelated existing keys.
  let repo = tempfile::tempdir().unwrap();
  let gwm_toml = repo.path().join(".gwm.toml");
  std::fs::write(
    &gwm_toml,
    "[worktree]\nbase = \"/tmp/keepme\"\n\n[tui]\nconfirm_countdown_secs = 4\n",
  )
  .unwrap();

  set_value_at(&gwm_toml, "tui.sidebar_position", "left").unwrap();

  let cfg = load(repo.path(), None);
  assert_eq!(cfg.tui.sidebar_position, SidebarPosition::Left);
  assert_eq!(cfg.tui.confirm_countdown_secs, 4, "untouched key must survive");
  assert_eq!(cfg.worktree.base, "/tmp/keepme", "untouched table must survive");
}
