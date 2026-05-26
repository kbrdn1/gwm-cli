//! Tests for the configurable TUI keymap (issue #87).
//!
//! Covers in this file:
//!   - `Action` enum + `ACTIONS` table invariants (slug uniqueness,
//!     kebab-case, every variant present);
//!   - key-string parser (`parse_chord` — single keys, named keys,
//!     modifiers, multi-key chords, every reject path);
//!   - `Keymap` layering (defaults, override replaces single action,
//!     conflict detection at load time, chord/prefix collision is a
//!     hard error per the design decision recorded on PR #87).
//!
//! The chord-buffer integration (`App::dispatch_key`) lives in
//! `tests/tui_chord_tests.rs` so that file stays focused on the event
//! loop side of the contract.

use gwm::tui::keymap::{Action, ChordResolution, KeyStroke, Keymap, Source, ACTIONS};

// ---------------------------------------------------------------------------
// Action enum + ACTIONS table
// ---------------------------------------------------------------------------

#[test]
fn actions_table_covers_every_variant() {
  // Every variant of `Action` MUST appear exactly once in `ACTIONS`. The
  // table is the single source of truth consumed by `gwm tui keys`, the
  // help overlay, and `gwm doctor` — a missing entry would silently
  // drop an action from all three.
  let table_variants: Vec<Action> = ACTIONS.iter().map(|(action, _)| *action).collect();
  let unique: std::collections::HashSet<_> = table_variants.iter().collect();
  assert_eq!(unique.len(), table_variants.len(), "ACTIONS has duplicate variants");

  for variant in Action::all() {
    assert!(
      table_variants.contains(&variant),
      "Action::{:?} is missing from ACTIONS",
      variant
    );
  }
}

#[test]
fn action_slugs_are_unique_and_kebab_case() {
  let mut seen = std::collections::HashSet::new();
  for (action, slug) in ACTIONS.iter() {
    assert!(seen.insert(*slug), "duplicate slug {:?} for {:?}", slug, action);
    assert!(
      slug.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
      "slug {:?} for {:?} is not snake_case ascii",
      slug,
      action
    );
    assert!(!slug.is_empty(), "empty slug for {:?}", action);
  }
}

#[test]
fn action_from_slug_roundtrips() {
  for (action, slug) in ACTIONS.iter() {
    assert_eq!(
      Action::from_slug(slug),
      Some(*action),
      "Action::from_slug({:?}) did not roundtrip to {:?}",
      slug,
      action
    );
    assert_eq!(action.slug(), *slug);
  }
  assert_eq!(Action::from_slug("does-not-exist"), None);
}

// ---------------------------------------------------------------------------
// Key-string parser
// ---------------------------------------------------------------------------

#[test]
fn parse_single_char() {
  let chord = KeyStroke::parse_chord("j").unwrap();
  assert_eq!(chord.len(), 1);
  assert_eq!(chord[0].to_string(), "j");
}

#[test]
fn parse_named_key() {
  let chord = KeyStroke::parse_chord("Tab").unwrap();
  assert_eq!(chord.len(), 1);
  assert_eq!(chord[0].to_string(), "Tab");

  // Esc / Enter / Up / Down / Backspace / Home / End / PageUp / PageDown / F1.
  for name in [
    "Esc",
    "Enter",
    "Up",
    "Down",
    "Left",
    "Right",
    "Backspace",
    "BackTab",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "F1",
    "F12",
    "Space",
  ] {
    let parsed = KeyStroke::parse_chord(name).unwrap();
    assert_eq!(parsed.len(), 1, "{name} did not parse to a single key");
    assert_eq!(parsed[0].to_string(), name);
  }
}

#[test]
fn parse_modifier_combinations() {
  let chord = KeyStroke::parse_chord("Ctrl+c").unwrap();
  assert_eq!(chord.len(), 1);
  assert_eq!(chord[0].to_string(), "Ctrl+c");

  // Modifier order in the source string does not affect equality.
  let a = KeyStroke::parse_chord("Ctrl+Alt+a").unwrap();
  let b = KeyStroke::parse_chord("Alt+Ctrl+a").unwrap();
  assert_eq!(a, b);
}

#[test]
fn parse_chord_sequence() {
  let chord = KeyStroke::parse_chord("g g").unwrap();
  assert_eq!(chord.len(), 2);
  assert_eq!(chord[0].to_string(), "g");
  assert_eq!(chord[1].to_string(), "g");

  let chord = KeyStroke::parse_chord("Ctrl+x Ctrl+s").unwrap();
  assert_eq!(chord.len(), 2);
}

#[test]
fn parse_rejects_empty_string() {
  assert!(KeyStroke::parse_chord("").is_err());
  assert!(KeyStroke::parse_chord("   ").is_err());
}

#[test]
fn parse_rejects_unknown_named_key() {
  assert!(KeyStroke::parse_chord("Foo").is_err());
  assert!(KeyStroke::parse_chord("ControlEnter").is_err());
}

#[test]
fn parse_rejects_dangling_modifier() {
  assert!(KeyStroke::parse_chord("Ctrl+").is_err());
  assert!(KeyStroke::parse_chord("Ctrl").is_err());
}

#[test]
fn parse_rejects_duplicate_modifier() {
  assert!(KeyStroke::parse_chord("Ctrl+Ctrl+c").is_err());
}

#[test]
fn parse_rejects_unknown_modifier() {
  assert!(KeyStroke::parse_chord("Meta+c").is_err());
}

// ---------------------------------------------------------------------------
// Keymap layering
// ---------------------------------------------------------------------------

#[test]
fn default_keymap_resolves_core_navigation() {
  let km = Keymap::defaults();

  let down = KeyStroke::parse_chord("j").unwrap();
  assert!(matches!(km.lookup(&down), ChordResolution::Matched(Action::Down)));

  let up = KeyStroke::parse_chord("k").unwrap();
  assert!(matches!(km.lookup(&up), ChordResolution::Matched(Action::Up)));

  let top = KeyStroke::parse_chord("g g").unwrap();
  assert!(matches!(km.lookup(&top), ChordResolution::Matched(Action::Top)));

  // `g` alone is a pending prefix of `g g`, not a match.
  let g_prefix = vec![top[0].clone()];
  assert!(matches!(km.lookup(&g_prefix), ChordResolution::PendingPrefix));
}

#[test]
fn keymap_lookup_returns_no_match_for_unbound_key() {
  let km = Keymap::defaults();
  let zzz = KeyStroke::parse_chord("Ctrl+Alt+z").unwrap();
  assert!(matches!(km.lookup(&zzz), ChordResolution::NoMatch));
}

#[test]
fn user_override_replaces_default_for_one_action() {
  let mut km = Keymap::defaults();
  km.apply_override(Action::Down, vec![KeyStroke::parse_chord("Ctrl+n").unwrap()])
    .unwrap();

  // New binding wins.
  let ctrl_n = KeyStroke::parse_chord("Ctrl+n").unwrap();
  assert!(matches!(km.lookup(&ctrl_n), ChordResolution::Matched(Action::Down)));

  // The default `j` is gone — overriding replaces, never merges.
  let j = KeyStroke::parse_chord("j").unwrap();
  assert!(matches!(km.lookup(&j), ChordResolution::NoMatch));

  // Other defaults untouched.
  let k = KeyStroke::parse_chord("k").unwrap();
  assert!(matches!(km.lookup(&k), ChordResolution::Matched(Action::Up)));
}

#[test]
fn user_override_can_unbind_an_action() {
  let mut km = Keymap::defaults();
  km.apply_override(Action::Down, vec![]).unwrap();

  let j = KeyStroke::parse_chord("j").unwrap();
  assert!(matches!(km.lookup(&j), ChordResolution::NoMatch));
}

#[test]
fn conflicting_user_bindings_are_rejected() {
  let mut km = Keymap::defaults();
  // Bind `Down` to `x`, then try to bind `Up` to `x` too. Hard error.
  km.apply_override(Action::Down, vec![KeyStroke::parse_chord("x").unwrap()])
    .unwrap();
  let err = km
    .apply_override(Action::Up, vec![KeyStroke::parse_chord("x").unwrap()])
    .unwrap_err();
  assert!(
    err.to_string().to_lowercase().contains("conflict"),
    "expected conflict error, got: {err}"
  );
}

#[test]
fn chord_that_is_strict_prefix_of_existing_binding_is_rejected() {
  // Per the design decision on PR #87: refusing this at load time is
  // preferable to running a Vim-style 500ms timer in the event loop.
  let mut km = Keymap::defaults();
  // Default `g g` is bound to Top. Trying to bind `g` alone to anything
  // else creates a prefix collision and MUST fail.
  let err = km
    .apply_override(Action::Open, vec![KeyStroke::parse_chord("g").unwrap()])
    .unwrap_err();
  assert!(
    err.to_string().to_lowercase().contains("prefix"),
    "expected prefix-collision error, got: {err}"
  );
}

#[test]
fn list_returns_entries_with_source() {
  let mut km = Keymap::defaults();
  km.apply_override(Action::Down, vec![KeyStroke::parse_chord("J").unwrap()])
    .unwrap();
  let listed = km.list();

  let down_entry = listed
    .iter()
    .find(|entry| entry.action == Action::Down)
    .expect("Down should appear in list()");
  assert_eq!(down_entry.source, Source::UserConfig);

  let up_entry = listed
    .iter()
    .find(|entry| entry.action == Action::Up)
    .expect("Up should appear in list()");
  assert_eq!(up_entry.source, Source::Default);
}
