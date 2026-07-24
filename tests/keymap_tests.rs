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
  // #290: V=toggle show/hide, Space=cycle orientation, v=toggle left/right position.
  let km = Keymap::defaults();

  let toggle = KeyStroke::parse_chord("V").unwrap();
  assert!(matches!(
    km.lookup(&toggle),
    ChordResolution::Matched(Action::ToggleSidebar)
  ));

  let cycle = KeyStroke::parse_chord("Space").unwrap();
  assert!(matches!(
    km.lookup(&cycle),
    ChordResolution::Matched(Action::CycleSidebarLayout)
  ));

  let toggle_pos = KeyStroke::parse_chord("v").unwrap();
  assert!(matches!(
    km.lookup(&toggle_pos),
    ChordResolution::Matched(Action::ToggleSidebarPosition)
  ));
}

#[test]
fn default_keymap_binds_sync_to_lowercase_s() {
  // #290: `s` (lowercase) = Sync; `S` (uppercase) = ToggleSidebarMode (Commits↔Stashes).
  let km = Keymap::defaults();

  let sync = KeyStroke::parse_chord("s").unwrap();
  assert!(matches!(km.lookup(&sync), ChordResolution::Matched(Action::Sync)));

  let sidebar_mode = KeyStroke::parse_chord("S").unwrap();
  assert!(
    matches!(
      km.lookup(&sidebar_mode),
      ChordResolution::Matched(Action::ToggleSidebarMode)
    ),
    "S must bind ToggleSidebarMode (Commits↔Stashes)"
  );
}

#[test]
fn default_keymap_binds_working_tree_scroll() {
  // Issue #437: `J` / `K` (Shift+j / Shift+k) scroll the Working Tree
  // pane from the status context. Rebindable as `wt_scroll_down` /
  // `wt_scroll_up` under `[tui.keys]`.
  let km = Keymap::defaults();

  let down = KeyStroke::parse_chord("J").unwrap();
  assert!(matches!(
    km.lookup(&down),
    ChordResolution::Matched(Action::WtScrollDown)
  ));

  let up = KeyStroke::parse_chord("K").unwrap();
  assert!(matches!(km.lookup(&up), ChordResolution::Matched(Action::WtScrollUp)));
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
    .apply_override(Action::TerminalFullscreen, vec![KeyStroke::parse_chord("g").unwrap()])
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

// ── Issue #290: keymap redesign — new bindings for existing actions ────────
// These tests assert the *new* default key assignments introduced in #290.
// They are intentionally written *before* the keymap is updated (TDD red)
// and will fail until `Keymap::defaults()` is updated in the implementation
// commit.

#[test]
fn sync_binds_to_lowercase_s_not_uppercase() {
  // #290: `s` (lowercase) is now Sync — more ergonomic than `S`.
  // `ToggleSidebarMode` is unbound by default; users who want it can rebind.
  let km = Keymap::defaults();

  let s = KeyStroke::parse_chord("s").unwrap();
  assert!(
    matches!(km.lookup(&s), ChordResolution::Matched(Action::Sync)),
    "s must resolve to Sync after #290"
  );

  // The old `S` must no longer fire Sync.
  let big_s = KeyStroke::parse_chord("S").unwrap();
  assert!(
    !matches!(km.lookup(&big_s), ChordResolution::Matched(Action::Sync)),
    "S must not resolve to Sync after #290 (s is the new binding)"
  );
}

#[test]
fn toggle_sidebar_position_binds_to_v_not_uppercase_h() {
  // #290: `v` (lowercase) is now ToggleSidebarPosition — replaces old `H`.
  // `ToggleSidebar` is unbound by default; users who want it can rebind.
  let km = Keymap::defaults();

  let v = KeyStroke::parse_chord("v").unwrap();
  assert!(
    matches!(km.lookup(&v), ChordResolution::Matched(Action::ToggleSidebarPosition)),
    "v must resolve to ToggleSidebarPosition after #290"
  );

  // The old `H` must not be ToggleSidebarPosition any more.
  let h = KeyStroke::parse_chord("H").unwrap();
  assert!(
    !matches!(km.lookup(&h), ChordResolution::Matched(Action::ToggleSidebarPosition)),
    "H must not resolve to ToggleSidebarPosition after #290"
  );
}

#[test]
fn toggle_delete_branch_binds_to_uppercase_d_not_p() {
  // #290: `D` is now ToggleDeleteBranch — `p` is repurposed as Pull.
  let km = Keymap::defaults();

  let big_d = KeyStroke::parse_chord("D").unwrap();
  assert!(
    matches!(km.lookup(&big_d), ChordResolution::Matched(Action::ToggleDeleteBranch)),
    "D must resolve to ToggleDeleteBranch after #290"
  );

  let p = KeyStroke::parse_chord("p").unwrap();
  assert!(
    !matches!(km.lookup(&p), ChordResolution::Matched(Action::ToggleDeleteBranch)),
    "p must not resolve to ToggleDeleteBranch after #290 (p is Pull)"
  );
}

#[test]
fn link_prompt_binds_to_i_not_uppercase_l() {
  // #290: `i` is now LinkPrompt — `L` is repurposed as LazyGitFullscreen.
  let km = Keymap::defaults();

  let i = KeyStroke::parse_chord("i").unwrap();
  assert!(
    matches!(km.lookup(&i), ChordResolution::Matched(Action::LinkPrompt)),
    "i must resolve to LinkPrompt after #290"
  );
}

// ── #290 backward-compat: user slug overrides must not conflict ───────────
// Users who had `lazygit_pty = ["l"]` or `terminal_fullscreen = ["o"]` in
// their config must be able to upgrade without a "chord conflict" load error.
// The rule: a user override that claims a chord previously held by a *default*
// binding on a different action silently wins — the default chord is vacated.

#[test]
fn user_override_lazygit_fullscreen_reclaims_l_from_pty_default() {
  let mut km = Keymap::defaults();
  // By default `l` is bound to LazyGitPty (#290). Binding LazyGitFullscreen
  // to `l` must succeed and LazyGitPty must lose `l`.
  km.apply_override(Action::LazyGitFullscreen, vec![KeyStroke::parse_chord("l").unwrap()])
    .unwrap();

  let l = KeyStroke::parse_chord("l").unwrap();
  assert!(
    matches!(km.lookup(&l), ChordResolution::Matched(Action::LazyGitFullscreen)),
    "l must resolve to LazyGitFullscreen after the override"
  );
}

#[test]
fn user_override_terminal_fullscreen_reclaims_o_from_pty_default() {
  let mut km = Keymap::defaults();
  // Same as above for the terminal pair (TerminalFullscreen / TerminalPty).
  km.apply_override(Action::TerminalFullscreen, vec![KeyStroke::parse_chord("o").unwrap()])
    .unwrap();

  let o = KeyStroke::parse_chord("o").unwrap();
  assert!(
    matches!(km.lookup(&o), ChordResolution::Matched(Action::TerminalFullscreen)),
    "o must resolve to TerminalFullscreen after the override"
  );
}

#[test]
fn review_pty_has_default_binding_r() {
  let km = Keymap::defaults();
  let r = KeyStroke::parse_chord("r").unwrap();
  assert!(
    matches!(km.lookup(&r), ChordResolution::Matched(Action::ReviewPty)),
    "r must resolve to ReviewPty by default (#290)"
  );
}

#[test]
fn refresh_default_binding_does_not_include_r() {
  let km = Keymap::defaults();
  // `r` moved to ReviewPty — Refresh must only keep `f`.
  let binding = km
    .list()
    .into_iter()
    .find(|b| b.action == Action::Refresh)
    .expect("Refresh must be in keymap");
  let r = KeyStroke::parse_chord("r").unwrap();
  assert!(
    !binding.chords.contains(&r),
    "Refresh must not include `r` after it was reassigned to ReviewPty"
  );
}

#[test]
fn user_override_review_fullscreen_reclaims_r_from_pty_default() {
  let mut km = Keymap::defaults();
  // `r` defaults to ReviewPty (#290). Rebinding ReviewFullscreen to `r` must
  // succeed and ReviewPty must lose `r`.
  km.apply_override(Action::ReviewFullscreen, vec![KeyStroke::parse_chord("r").unwrap()])
    .unwrap();

  let r = KeyStroke::parse_chord("r").unwrap();
  assert!(
    matches!(km.lookup(&r), ChordResolution::Matched(Action::ReviewFullscreen)),
    "r must resolve to ReviewFullscreen after the user override"
  );
}

// ---------------------------------------------------------------------------
// Backward-compatibility aliases (issue #290 slug renames)
// ---------------------------------------------------------------------------

#[test]
fn from_slug_compat_accepts_pre_290_slugs() {
  // Users with existing .gwm.toml [tui.keys] blocks that use the pre-#290
  // slug names must not get a config error after upgrading.
  let cases = [
    ("git_tui", Action::LazyGitFullscreen),
    ("git_tui_overlay", Action::LazyGitPty),
    ("review", Action::ReviewFullscreen),
    ("review_overlay", Action::ReviewPty),
    ("yank", Action::YankPath),
    ("open", Action::TerminalFullscreen),
    ("open_terminal_overlay", Action::TerminalPty),
    ("open_menu", Action::BrowseLinks),
  ];
  for (old_slug, expected) in cases {
    assert_eq!(
      Action::from_slug_compat(old_slug),
      Some(expected),
      "compat alias for {:?} must resolve to {:?}",
      old_slug,
      expected
    );
  }
}

#[test]
fn compat_alias_slugs_is_the_reverse_of_from_slug_compat() {
  // Every alias `compat_alias_slugs` reports for an action must resolve back to
  // that action; a canonical-only action reports none (issue #294 — the Keys
  // tab strips these from a legacy config on rewrite).
  assert_eq!(
    Action::BrowseLinks.compat_alias_slugs().collect::<Vec<_>>(),
    vec!["open_menu"]
  );
  assert_eq!(
    Action::LazyGitFullscreen.compat_alias_slugs().collect::<Vec<_>>(),
    vec!["git_tui"]
  );
  assert!(
    Action::Down.compat_alias_slugs().next().is_none(),
    "an action with no rename reports no alias"
  );
  for action in Action::all() {
    for alias in action.compat_alias_slugs() {
      assert_eq!(
        Action::from_slug_compat(alias),
        Some(action),
        "alias {alias:?} must resolve back to {action:?}"
      );
    }
  }
}

#[test]
fn from_slug_compat_still_resolves_canonical_slugs() {
  // The compat wrapper must not break the canonical path.
  assert_eq!(
    Action::from_slug_compat("lazygit_fullscreen"),
    Some(Action::LazyGitFullscreen)
  );
  assert_eq!(Action::from_slug_compat("yank_path"), Some(Action::YankPath));
  assert_eq!(Action::from_slug_compat("down"), Some(Action::Down));
  assert_eq!(Action::from_slug_compat("nonexistent_slug_xyz"), None);
}

#[test]
fn exec_and_clean_overlays_are_repo_mutating() {
  // Codex #333 review: in workspace mode the stale-selection guard
  // (`workspace_active_stale && is_repo_mutating`) must block `x` / `X` —
  // they resolve their command / dir-set from the active repo's config and
  // act on the selected path, both stale when the row's repo can't activate.
  assert!(Action::ExecOverlay.is_repo_mutating());
  assert!(Action::CleanOverlay.is_repo_mutating());
  // Sanity: read-only / navigation verbs stay non-mutating.
  assert!(!Action::Down.is_repo_mutating());
  assert!(!Action::CommandLogs.is_repo_mutating());
  assert!(!Action::ConfigPanel.is_repo_mutating());
}
