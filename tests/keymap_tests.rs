//! Tests for the configurable TUI keymap (issue #87).
//!
//! Covers in this file:
//!   - `Action` enum + `ACTIONS` table invariants (slug uniqueness,
//!     snake_case, every variant present);
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
fn action_slugs_are_unique_and_snake_case() {
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
fn default_keymap_binds_sidebar_layout_and_position() {
  // Issue #188: `V` cycles the sidebar orientation, `H` toggles the
  // left/right position. Both must be distinct from `v` (ToggleSidebar).
  let km = Keymap::defaults();

  let cycle = KeyStroke::parse_chord("V").unwrap();
  assert!(matches!(
    km.lookup(&cycle),
    ChordResolution::Matched(Action::CycleSidebarLayout)
  ));

  let toggle_pos = KeyStroke::parse_chord("H").unwrap();
  assert!(matches!(
    km.lookup(&toggle_pos),
    ChordResolution::Matched(Action::ToggleSidebarPosition)
  ));

  let open = KeyStroke::parse_chord("v").unwrap();
  assert!(matches!(
    km.lookup(&open),
    ChordResolution::Matched(Action::ToggleSidebar)
  ));
}

#[test]
fn default_keymap_binds_sync_to_uppercase_s() {
  // Issue #258: `S` runs `gwm sync` on the selected worktree. It must be
  // uppercase `S` because lowercase `s` is already ToggleSidebarMode — the
  // mnemonic sits with the other uppercase lifecycle verbs (`F`, `R`).
  let km = Keymap::defaults();

  let sync = KeyStroke::parse_chord("S").unwrap();
  assert!(matches!(km.lookup(&sync), ChordResolution::Matched(Action::Sync)));

  // Guard the conflict that motivated the choice: lowercase `s` stays the
  // sidebar-mode toggle, untouched.
  let sidebar_mode = KeyStroke::parse_chord("s").unwrap();
  assert!(matches!(
    km.lookup(&sidebar_mode),
    ChordResolution::Matched(Action::ToggleSidebarMode)
  ));
}

#[test]
fn default_keymap_binds_open_docs_to_dot() {
  // Issue #233: `.` opens the gwm documentation in the browser, reusing the
  // OpenMenu browser-spawn path. Rebindable as `open_docs` under `[tui.keys]`.
  let km = Keymap::defaults();
  let dot = KeyStroke::parse_chord(".").unwrap();
  assert!(matches!(km.lookup(&dot), ChordResolution::Matched(Action::OpenDocs)));
}

#[test]
fn open_docs_is_rebindable_like_any_action() {
  // The new action takes a user override exactly like the rest of the keymap.
  let mut km = Keymap::defaults();
  km.apply_override(Action::OpenDocs, vec![KeyStroke::parse_chord("Ctrl+d").unwrap()])
    .unwrap();
  let rebound = KeyStroke::parse_chord("Ctrl+d").unwrap();
  assert!(matches!(
    km.lookup(&rebound),
    ChordResolution::Matched(Action::OpenDocs)
  ));
  // The default `.` no longer resolves to OpenDocs once overridden.
  let dot = KeyStroke::parse_chord(".").unwrap();
  assert!(!matches!(km.lookup(&dot), ChordResolution::Matched(Action::OpenDocs)));
}

#[test]
fn default_keymap_binds_command_logs_to_3() {
  // Issue #226: `3` opens the Command Logs modal, completing the `1` / `2`
  // / `3` pane-key family (focus_worktrees / focus_status / command_logs).
  let km = Keymap::defaults();
  let three = KeyStroke::parse_chord("3").unwrap();
  assert!(matches!(
    km.lookup(&three),
    ChordResolution::Matched(Action::CommandLogs)
  ));
}

#[test]
fn default_keymap_binds_config_panel_to_4() {
  // Issue #232: `4` opens the Configuration panel, extending the `1` / `2`
  // / `3` / `4` pane-key family (focus_worktrees / focus_status /
  // command_logs / config_panel).
  let km = Keymap::defaults();
  let four = KeyStroke::parse_chord("4").unwrap();
  assert!(matches!(
    km.lookup(&four),
    ChordResolution::Matched(Action::ConfigPanel)
  ));
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
fn primary_chord_resolves_first_default_binding() {
  // Issue #217: the sidebar's "press <key> to fetch status" prompt must
  // resolve the live binding instead of hard-coding `R`. `primary_chord`
  // returns the first chord bound to an action, rendered canonically.
  let km = Keymap::defaults();
  assert_eq!(km.primary_chord(Action::FetchGithub).as_deref(), Some("F"));
  assert_eq!(km.primary_chord(Action::Help).as_deref(), Some("?"));
  // Multi-chord actions return the first chord in declaration order.
  assert_eq!(km.primary_chord(Action::Refresh).as_deref(), Some("f"));
}

#[test]
fn primary_chord_follows_user_override() {
  let mut km = Keymap::defaults();
  km.apply_override(Action::FetchGithub, vec![KeyStroke::parse_chord("Ctrl+g").unwrap()])
    .unwrap();
  assert_eq!(km.primary_chord(Action::FetchGithub).as_deref(), Some("Ctrl+g"));
}

#[test]
fn primary_chord_is_none_for_unbound_action() {
  let mut km = Keymap::defaults();
  km.apply_override(Action::FetchGithub, vec![]).unwrap();
  assert_eq!(km.primary_chord(Action::FetchGithub), None);
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

// ── Issue #35 backward-compat: old slug overrides must not conflict ────────
// Users who had `git_tui = ["l"]` or `open = ["o"]` in their config before
// #35 switched the defaults to the overlay variants must be able to upgrade
// without a "chord conflict" load error. The rule: a user override that
// claims a chord previously held by a *default* binding on a different action
// silently wins — the default chord is vacated.

#[test]
fn user_override_git_tui_reclaims_l_from_overlay_default() {
  let mut km = Keymap::defaults();
  // By default `l` is bound to GitTuiOverlay. Binding GitTui to `l` must
  // succeed (not fail with "chord conflict") and GitTuiOverlay must lose `l`.
  km.apply_override(Action::GitTui, vec![KeyStroke::parse_chord("l").unwrap()])
    .unwrap();

  let l = KeyStroke::parse_chord("l").unwrap();
  assert!(
    matches!(km.lookup(&l), ChordResolution::Matched(Action::GitTui)),
    "l must resolve to GitTui after the override"
  );
}

#[test]
fn user_override_open_reclaims_o_from_overlay_default() {
  let mut km = Keymap::defaults();
  // Same as above for the `open` / `open_terminal_overlay` pair.
  km.apply_override(Action::Open, vec![KeyStroke::parse_chord("o").unwrap()])
    .unwrap();

  let o = KeyStroke::parse_chord("o").unwrap();
  assert!(
    matches!(km.lookup(&o), ChordResolution::Matched(Action::Open)),
    "o must resolve to Open after the override"
  );
}
