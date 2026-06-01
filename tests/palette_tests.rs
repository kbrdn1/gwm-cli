//! Tests for the TUI command palette (issue #32).
//!
//! Two layers are pinned here:
//!   - the *registry* (`PaletteEntry`, `palette_entries()`) — every
//!     `Action::all()` variant has a paired entry so the palette and
//!     the help overlay never drift on which verbs are addressable;
//!   - the *state machine* (`PaletteState::filter`, `cycle_highlight`,
//!     `accept`) — the bits the event-loop / renderer rely on without
//!     spawning a terminal.

use gwm::tui::keymap::Action;
use gwm::tui::palette::{palette_entries, PaletteEntry, PaletteState};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[test]
fn registry_covers_every_action_variant() {
  // Discoverability through the palette is the whole point of #32 —
  // a missing entry would silently make an action unreachable by name
  // even though its key binding works. Pin the invariant so adding
  // a new Action forces the dev to wire it through here as well.
  let entries = palette_entries();
  let registered: std::collections::HashSet<Action> = entries.iter().map(|e| e.action).collect();
  for variant in Action::all() {
    assert!(
      registered.contains(&variant),
      "Action::{:?} has no palette entry — add it to palette_entries()",
      variant
    );
  }
}

#[test]
fn registry_names_are_unique_and_lowercase_words() {
  // Names are what the user types after `:`. Duplicates would make
  // dispatch ambiguous; non-ascii / whitespace names would break the
  // input parser.
  let entries = palette_entries();
  let mut seen = std::collections::HashSet::new();
  for entry in entries {
    assert!(seen.insert(entry.name), "duplicate palette name {:?}", entry.name);
    assert!(!entry.name.is_empty(), "empty name for {:?}", entry.action);
    assert!(
      entry
        .name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'),
      "palette name {:?} must be lowercase ascii [a-z0-9_-]",
      entry.name
    );
    assert!(
      !entry.description.is_empty(),
      "missing description for palette entry {:?}",
      entry.name
    );
  }
}

#[test]
fn registry_lookup_by_name_returns_action() {
  let entries = palette_entries();
  let create = entries
    .iter()
    .find(|e| e.name == "create")
    .expect("expected a `create` entry");
  assert_eq!(create.action, Action::Create);
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[test]
fn fresh_palette_is_closed_with_empty_buffer() {
  let s = PaletteState::new();
  assert!(!s.open, "palette starts closed");
  assert!(s.buffer().is_empty());
  // Highlight has no meaning when closed but should default to 0 so
  // a freshly opened palette renders with the top entry selected.
  assert_eq!(s.highlight(), 0);
}

#[test]
fn open_arms_palette_and_pre_filters_full_registry() {
  let mut s = PaletteState::new();
  s.open();
  assert!(s.open);
  // Empty buffer → every registered entry visible, in registry order.
  let matches = s.matches();
  let total = palette_entries().len();
  assert_eq!(matches.len(), total, "an empty buffer must surface the full registry");
}

#[test]
fn typing_filters_and_resets_highlight() {
  let mut s = PaletteState::new();
  s.open();
  s.push_char('c');
  s.push_char('r');
  s.push_char('e');
  let matches: Vec<&PaletteEntry> = s.matches();
  assert!(
    matches.iter().any(|e| e.name == "create"),
    "`cre` must rank `create` somewhere in the visible matches"
  );
  // Highlight always lands on a valid index after the buffer changes.
  assert!(
    s.highlight() < matches.len(),
    "highlight {} must point at a real match (len = {})",
    s.highlight(),
    matches.len()
  );
}

#[test]
fn backspace_pops_one_char_and_re_expands_matches() {
  let mut s = PaletteState::new();
  s.open();
  s.push_char('z'); // unlikely to match anything
  let narrow = s.matches().len();
  s.pop_char();
  let wide = s.matches().len();
  assert!(
    wide >= narrow,
    "popping a char must re-broaden the match set ({wide} >= {narrow})"
  );
  assert!(s.buffer().is_empty());
}

#[test]
fn cycle_highlight_down_wraps_at_end() {
  let mut s = PaletteState::new();
  s.open();
  let n = s.matches().len();
  for _ in 0..n {
    s.cycle_highlight_down();
  }
  // Cycling exactly `n` times must land us back at 0.
  assert_eq!(s.highlight(), 0);
}

#[test]
fn cycle_highlight_up_wraps_at_start() {
  let mut s = PaletteState::new();
  s.open();
  s.cycle_highlight_up();
  let n = s.matches().len();
  assert_eq!(s.highlight(), n - 1, "up from 0 must wrap to the last entry");
}

#[test]
fn accept_returns_highlighted_action_and_closes_palette() {
  let mut s = PaletteState::new();
  s.open();
  s.push_char('c');
  s.push_char('r');
  s.push_char('e');
  // Whatever the matcher ranks at #0 — we accept it and assert the
  // palette closes. The exact action depends on the fuzzy matcher's
  // tie-breaking; we only pin that *some* action comes back.
  let action = s.accept().expect("accept must return a highlighted action");
  assert!(
    palette_entries().iter().any(|e| e.action == action),
    "accepted action must be a registry entry"
  );
  assert!(!s.open, "accept must close the palette");
  assert!(s.buffer().is_empty(), "accept must clear the buffer");
}

#[test]
fn accept_with_no_matches_returns_none() {
  let mut s = PaletteState::new();
  s.open();
  // Filter that almost certainly matches nothing in the registry.
  for c in "qqqqq_zz_does_not_exist".chars() {
    s.push_char(c);
  }
  assert!(s.matches().is_empty());
  assert!(s.accept().is_none());
  // Palette stays open so the user can correct the typo without
  // losing context. Matches the contract documented in the issue's
  // "Esc cancels, Enter executes" wording — only a successful
  // accept closes.
  assert!(s.open);
}

#[test]
fn close_clears_buffer_and_resets_highlight() {
  let mut s = PaletteState::new();
  s.open();
  s.push_char('a');
  s.cycle_highlight_down();
  s.close();
  assert!(!s.open);
  assert!(s.buffer().is_empty());
  assert_eq!(s.highlight(), 0);
}
