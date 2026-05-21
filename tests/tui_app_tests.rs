mod common;

use common::init_repo;
use gwm::naming::BRANCH_TYPES;
use gwm::tui::{
  branch_name_color, filled_cells_for_progress, freshness_color, pr_badge_color, App, ConfirmKeyAction,
  CountdownTickOutcome, Field, View,
};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use ratatui::style::Color;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Build a synthetic worktree row for state-machine tests that need a known
/// list shape (fuzzy filter ranking, multi-row navigation). Lets the test
/// drive the filter without going through real `git2::worktree_add` calls.
fn worktree_fixture(name: &str) -> WorktreeInfo {
  WorktreeInfo {
    name: name.into(),
    path: PathBuf::from(format!("/tmp/gwm-test/{}", name)),
    branch: Some(format!("feat/#0-{}", name)),
    head: None,
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
  }
}

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at(Some(dir.path())).unwrap();
  (dir, app)
}

#[test]
fn new_loads_main_worktree() {
  let (_dir, app) = make_app();
  assert_eq!(app.worktrees.len(), 1);
  assert!(app.worktrees[0].is_main);
}

#[test]
fn enter_create_initializes_form() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  assert_eq!(app.view, View::Create);
  assert_eq!(app.create_field, Field::Type);
  assert!(app.create_issue.is_empty());
  assert!(app.create_desc.is_empty());
}

#[test]
fn create_field_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_next_field();
  assert_eq!(app.create_field, Field::Issue);
  app.create_next_field();
  assert_eq!(app.create_field, Field::Desc);
  app.create_next_field();
  assert_eq!(app.create_field, Field::Type);
  app.create_prev_field();
  assert_eq!(app.create_field, Field::Desc);
}

#[test]
fn create_type_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_prev_type();
  assert_eq!(app.create_type_index, BRANCH_TYPES.len() - 1);
  app.create_next_type();
  assert_eq!(app.create_type_index, 0);
}

#[test]
fn create_push_only_digits_on_issue() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_field = Field::Issue;
  for c in "12a3".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_issue, "123");
}

#[test]
fn create_push_accepts_desc_chars() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_field = Field::Desc;
  for c in "foo-bar".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_desc, "foo-bar");
  app.create_pop_char();
  assert_eq!(app.create_desc, "foo-ba");
}

#[test]
fn enter_confirm_delete_refuses_main() {
  let (_dir, mut app) = make_app();
  app.enter_confirm_delete();
  assert_eq!(app.view, View::List, "main worktree should not allow delete view");
}

#[test]
fn toggle_delete_branch_flips() {
  let (_dir, mut app) = make_app();
  assert!(!app.delete_branch_on_remove);
  app.toggle_delete_branch();
  assert!(app.delete_branch_on_remove);
}

#[test]
fn next_prev_with_single_entry_stays_put() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.next();
  assert_eq!(app.list_state.selected(), Some(0));
  app.prev();
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn refresh_keeps_selection_in_bounds() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(5));
  app.refresh().unwrap();
  assert_eq!(app.list_state.selected(), Some(0));
}

// ---- sidebar / focus / vim motions ---------------------------------------

#[test]
fn sidebar_open_by_default() {
  let (_dir, app) = make_app();
  assert!(
    app.sidebar_open,
    "sidebar should default to open (will be hidden when narrow)"
  );
  assert!(!app.sidebar_focused, "focus defaults to the worktree list");
}

#[test]
fn toggle_sidebar_flips_open_flag() {
  let (_dir, mut app) = make_app();
  let before = app.sidebar_open;
  app.toggle_sidebar();
  assert_eq!(app.sidebar_open, !before);
  app.toggle_sidebar();
  assert_eq!(app.sidebar_open, before);
}

#[test]
fn toggle_sidebar_when_closed_resets_focus_to_list() {
  let (_dir, mut app) = make_app();
  app.sidebar_focused = true;
  app.sidebar_open = true;
  app.toggle_sidebar(); // close
  assert!(!app.sidebar_open);
  assert!(
    !app.sidebar_focused,
    "closing the sidebar must drop focus back to the list"
  );
}

#[test]
fn toggle_focus_only_works_when_sidebar_open() {
  let (_dir, mut app) = make_app();
  app.sidebar_open = false;
  app.toggle_focus();
  assert!(!app.sidebar_focused, "focus cannot move to a hidden sidebar");

  app.sidebar_open = true;
  app.toggle_focus();
  assert!(app.sidebar_focused);
  app.toggle_focus();
  assert!(!app.sidebar_focused);
}

#[test]
fn first_selects_first_worktree() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.first();
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn last_selects_last_worktree() {
  let (_dir, mut app) = make_app();
  app.last();
  let expected = app.worktrees.len().saturating_sub(1);
  assert_eq!(app.list_state.selected(), Some(expected));
}

#[test]
fn handle_g_motion_tracks_pending_then_jumps_to_first() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  // First `g` arms the motion but does not move.
  assert!(!app.pending_g);
  app.handle_g();
  assert!(app.pending_g, "first 'g' must arm the gg sequence");
  // Second `g` jumps to first and disarms.
  app.handle_g();
  assert!(!app.pending_g, "second 'g' completes gg and disarms");
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn pending_g_resets_on_other_key() {
  let (_dir, mut app) = make_app();
  app.handle_g();
  assert!(app.pending_g);
  app.cancel_pending_motion();
  assert!(!app.pending_g, "any non-g keypress must drop the pending motion");
}

#[test]
fn sidebar_scroll_clamps_to_zero() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.sidebar_scroll, 0);
  app.sidebar_scroll_up();
  assert_eq!(app.sidebar_scroll, 0, "scrolling up from 0 stays at 0");

  // The renderer normally publishes a max bound; simulate enough room for scroll.
  app.sidebar_max_scroll = 5;
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar_scroll, 1);
  app.sidebar_scroll_up();
  assert_eq!(app.sidebar_scroll, 0);
}

#[test]
fn sidebar_scroll_clamps_at_max() {
  // The renderer sets `sidebar_max_scroll`. Scrolling past it must stop there
  // so the user can't push the panel content entirely off-screen.
  let (_dir, mut app) = make_app();
  app.sidebar_max_scroll = 3;
  app.sidebar_scroll_down();
  app.sidebar_scroll_down();
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar_scroll, 3);
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar_scroll, 3, "scrolling beyond max must clamp");
}

#[test]
fn focus_routes_navigation_to_sidebar() {
  // When sidebar is focused, next()/prev() should NOT move the list selection.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.sidebar_open = true;
  app.sidebar_focused = true;
  app.sidebar_max_scroll = 5; // pretend the renderer has populated this

  app.next();
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "list must stay put when sidebar has focus"
  );
  assert!(
    app.sidebar_scroll >= 1,
    "next() must scroll the sidebar when it has focus"
  );

  app.prev();
  assert_eq!(app.list_state.selected(), Some(0));
  assert_eq!(app.sidebar_scroll, 0, "prev() scrolled back up");
}

#[test]
fn next_prev_invalidate_sidebar_cache() {
  // Moving selection must drop any cached sidebar content so the new
  // worktree's preview is recomputed on the next frame.
  let (_dir, mut app) = make_app();
  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), vec![]));
  app.next();
  assert!(app.sidebar_cache.is_none(), "next() must invalidate the sidebar cache");

  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), vec![]));
  app.prev();
  assert!(app.sidebar_cache.is_none(), "prev() must invalidate the sidebar cache");
}

#[test]
fn refresh_invalidates_sidebar_cache() {
  let (_dir, mut app) = make_app();
  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), vec![]));
  app.refresh().unwrap();
  assert!(app.sidebar_cache.is_none());
}

// ---- fuzzy filter (issue #21) -------------------------------------------

#[test]
fn filter_state_defaults_to_inactive_and_empty() {
  let (_dir, app) = make_app();
  assert!(!app.filter_active, "filter must default to inactive");
  assert!(app.filter_query.is_empty(), "filter query must default to empty");
}

#[test]
fn enter_filter_activates_capture_and_disarms_gg() {
  let (_dir, mut app) = make_app();
  app.handle_g(); // arm `gg`
  assert!(app.pending_g);

  app.enter_filter();
  assert!(app.filter_active);
  assert!(
    !app.pending_g,
    "opening the filter bar must drop any half-typed gg motion"
  );
}

#[test]
fn enter_filter_drops_sidebar_focus() {
  // regression: PR #44 Copilot review — sidebar focus survived `/` and broke
  // j/k navigation after Enter committed the filter.
  // Regression for the Copilot review on PR #44: if the sidebar held focus
  // when the user hit `/`, after `exit_filter_keep` (Enter) the focus would
  // still be on the sidebar, so `j` / `k` would scroll it instead of walking
  // the filtered worktrees — contradicting the documented "navigation
  // returns to the table" contract. Opening the filter bar must therefore
  // pre-emptively pull focus back to the list.
  let (_dir, mut app) = make_app();
  app.sidebar_open = true;
  app.sidebar_focused = true;

  app.enter_filter();
  assert!(
    !app.sidebar_focused,
    "opening the filter bar must hand focus back to the list"
  );
}

#[test]
fn enter_filter_preserves_existing_query() {
  // Hitting `/` on a sticky filter re-opens the bar so the user can refine
  // it; only Esc clears.
  let (_dir, mut app) = make_app();
  app.filter_query = "auth".into();
  app.enter_filter();
  assert_eq!(app.filter_query, "auth");
  assert!(app.filter_active);
}

#[test]
fn filter_push_char_appends_to_query() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  for c in "tui".chars() {
    app.filter_push_char(c);
  }
  assert_eq!(app.filter_query, "tui");
}

#[test]
fn filter_pop_char_removes_last_char() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter_query = "tuix".into();
  app.filter_pop_char();
  assert_eq!(app.filter_query, "tui");
}

#[test]
fn filter_pop_char_on_empty_is_noop() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter_pop_char();
  assert_eq!(app.filter_query, "");
  assert!(app.filter_active, "popping an empty query must not exit filter mode");
}

#[test]
fn exit_filter_keep_disables_capture_keeps_query() {
  // Enter behaviour: filter sticks, navigation returns to the list.
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter_query = "auth".into();
  app.exit_filter_keep();
  assert!(!app.filter_active);
  assert_eq!(app.filter_query, "auth", "Enter must not wipe the query");
}

#[test]
fn exit_filter_cancel_clears_query() {
  // Esc behaviour: full list back, query gone.
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter_query = "auth".into();
  app.exit_filter_cancel();
  assert!(!app.filter_active);
  assert!(app.filter_query.is_empty(), "Esc must clear the query");
}

#[test]
fn filtered_indices_returns_all_when_query_empty() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("beta"),
    worktree_fixture("gamma"),
  ];
  let idx = app.filtered_indices();
  assert_eq!(idx, vec![0, 1, 2], "empty query is the identity over worktrees");
}

#[test]
fn filtered_indices_keeps_only_matching_worktrees() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("feat-1-tui-search"),
    worktree_fixture("feat-2-cli-completions"),
    worktree_fixture("fix-3-locked-worktree"),
  ];
  app.filter_query = "tui".into();

  let idx = app.filtered_indices();
  let names: Vec<&str> = idx.iter().map(|&i| app.worktrees[i].name.as_str()).collect();
  assert_eq!(
    names,
    vec!["feat-1-tui-search"],
    "only the worktree whose name contains 'tui' should match"
  );
}

#[test]
fn filtered_indices_supports_subsequence_match() {
  // The candidate must NOT contain the query as a contiguous substring —
  // otherwise a regression that downgrades the fuzzy matcher to plain
  // substring matching would still pass. Spread the query characters across
  // the haystack so only the subsequence path can score it.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    // 'a'-'u'-'t'-'h' appear in order but separated by other characters;
    // there is no literal `auth` substring anywhere in this name.
    worktree_fixture("a-foo-u-bar-t-baz-h-qux"),
    worktree_fixture("chore-1-bump-deps"),
  ];
  app.filter_query = "auth".into();
  // Sanity-guard the precondition: if a future refactor introduces an `auth`
  // substring into the fixture, the test would silently degrade to a
  // substring check again.
  assert!(
    !app.worktrees[0].name.contains("auth"),
    "fixture must not contain 'auth' as a substring or the test stops covering subsequence"
  );

  let idx = app.filtered_indices();
  assert_eq!(idx.len(), 1);
  assert_eq!(app.worktrees[idx[0]].name, "a-foo-u-bar-t-baz-h-qux");
}

#[test]
fn filtered_indices_ranks_substring_above_subsequence() {
  // Issue #21 contract: "exact substring match > prefix match > sub-sequence
  // match". Verify by feeding a candidate that contains the literal needle
  // and another that only matches via gaps; the contiguous one must rank
  // first in the returned index list.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    // subsequence: a-u-t-h spread out across the name
    worktree_fixture("a-zzz-u-yyy-t-xxx-h"),
    // direct substring of "auth"
    worktree_fixture("auth-service"),
  ];
  app.filter_query = "auth".into();

  let idx = app.filtered_indices();
  assert!(!idx.is_empty(), "at least the substring candidate must match");
  assert_eq!(
    app.worktrees[idx[0]].name, "auth-service",
    "contiguous substring must outrank a spread subsequence"
  );
}

#[test]
fn filtered_indices_skips_when_no_match() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("alpha"), worktree_fixture("beta")];
  app.filter_query = "zzzz".into();
  assert!(app.filtered_indices().is_empty());
}

#[test]
fn selected_returns_filtered_worktree() {
  // `selected()` must resolve the table state's index through the filter map
  // back to the underlying worktree, not blindly index into `worktrees`.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("authentication"),
    worktree_fixture("beta"),
  ];
  app.filter_query = "auth".into();
  app.list_state.select(Some(0));

  let sel = app.selected().expect("filtered selection must resolve");
  assert_eq!(sel.name, "authentication");
}

#[test]
fn selected_returns_none_when_filter_matches_nothing() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("alpha"), worktree_fixture("beta")];
  app.filter_query = "zzzz".into();
  app.list_state.select(Some(0));
  assert!(app.selected().is_none());
}

#[test]
fn next_navigates_within_filtered_subset_and_wraps() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo-a"),
    worktree_fixture("foo-b"),
  ];
  app.filter_query = "foo".into();
  app.list_state.select(Some(0));

  app.next();
  assert_eq!(app.list_state.selected(), Some(1));
  app.next();
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "wrap-around to start of filtered subset"
  );
}

#[test]
fn prev_navigates_within_filtered_subset_and_wraps() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo-a"),
    worktree_fixture("foo-b"),
  ];
  app.filter_query = "foo".into();
  app.list_state.select(Some(0));

  app.prev();
  assert_eq!(app.list_state.selected(), Some(1), "wrap-around backwards");
}

#[test]
fn first_and_last_jump_inside_filtered_subset() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo-1"),
    worktree_fixture("beta"),
    worktree_fixture("foo-2"),
    worktree_fixture("gamma"),
  ];
  app.filter_query = "foo".into();
  app.list_state.select(Some(1));

  app.first();
  assert_eq!(app.list_state.selected(), Some(0));
  app.last();
  assert_eq!(
    app.list_state.selected(),
    Some(1),
    "last must use the filtered length (2 matches → index 1)"
  );
}

#[test]
fn filter_push_clamps_selection_when_subset_shrinks() {
  // User starts on the second match, then types more so only the first stays;
  // selection must clamp instead of dangling past the new end.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("foo-bar"), worktree_fixture("foo-baz-xx")];
  app.filter_query = "foo".into();
  app.list_state.select(Some(1));

  // Typing more reduces the match set to just "foo-bar".
  for c in "-bar".chars() {
    app.filter_push_char(c);
  }
  let filtered = app.filtered_indices();
  assert_eq!(filtered.len(), 1, "only foo-bar should still match foo-bar");
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "selection must clamp to inside the new filtered subset"
  );
}

#[test]
fn exit_filter_cancel_restores_full_list_selection() {
  // After clearing the filter, the original full list comes back and a
  // selection past the filtered len must remain valid for the larger set.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo"),
    worktree_fixture("beta"),
  ];
  app.filter_query = "foo".into();
  app.list_state.select(Some(0));

  app.exit_filter_cancel();
  assert!(app.filter_query.is_empty());
  assert_eq!(app.filtered_indices(), vec![0, 1, 2]);
  // Selection from the filtered view (index 0) remains a valid index in the
  // full list (index 0). The clamp logic doesn't move it forward, only back.
  assert_eq!(app.list_state.selected(), Some(0));
}

// ---- picker mode (issue #22) --------------------------------------------

#[test]
fn picker_mode_defaults_to_false() {
  // The regular `gwm` TUI entry point must not behave like a picker. The
  // create / delete / bootstrap actions stay reachable, and `picker_result`
  // is unset until an explicit picker session asks for it.
  let (_dir, app) = make_app();
  assert!(!app.picker_mode, "default App must not be in picker mode");
  assert!(
    app.picker_result.is_none(),
    "no path is selected until the user confirms"
  );
}

#[test]
fn new_picker_at_enables_picker_mode() {
  // `gwm switch` enters the TUI through this constructor; the picker flag
  // is what drives the event loop into "Enter confirms, n/d/b are inert".
  let (dir, _) = init_repo();
  let app = App::new_picker_at(Some(dir.path())).unwrap();
  assert!(app.picker_mode, "new_picker_at must set picker_mode=true");
}

#[test]
fn new_picker_at_opens_filter_bar() {
  // Per issue #22: "switch could open the filter bar immediately on
  // startup". A user invoking `gwm switch` already knows they want to
  // narrow the list; opening the bar saves one keystroke.
  let (dir, _) = init_repo();
  let app = App::new_picker_at(Some(dir.path())).unwrap();
  assert!(app.filter_active, "picker mode must open with the filter bar active");
}

#[test]
fn picker_confirm_records_selected_path() {
  // Enter in picker mode commits the highlighted worktree path so the
  // event loop can return it to the caller (which prints it to stdout
  // for the `cd "$(gwm switch)"` flow).
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.list_state.select(Some(0));
  let expected = app.selected().expect("test fixture must have a worktree").path.clone();

  app.picker_confirm();
  assert_eq!(
    app.picker_result,
    Some(expected),
    "picker_confirm must record the selected worktree's path"
  );
}

#[test]
fn picker_confirm_with_no_selection_keeps_result_none() {
  // If the filter wipes the list down to zero matches, hitting Enter must
  // not crash and must not record a bogus path. The event loop is then
  // free to keep the TUI open (or break with None, which is the caller's
  // call).
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.worktrees.clear();
  app.list_state.select(None);

  app.picker_confirm();
  assert!(
    app.picker_result.is_none(),
    "picker_confirm with no selection must leave picker_result unset"
  );
}

#[test]
fn picker_confirm_outside_picker_mode_is_inert() {
  // Defensive: `picker_confirm` shouldn't poison the regular TUI flow if
  // it's ever wired into the wrong event branch. Only picker mode reacts
  // to Enter by recording a path; the normal `Enter = copy path to status
  // bar` behaviour is left to its own handler.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  assert!(!app.picker_mode);

  app.picker_confirm();
  assert!(
    app.picker_result.is_none(),
    "picker_confirm outside picker mode must not record a path"
  );
}

// Copilot review (PR #53): the event loop unconditionally `break`s after
// `picker_confirm()` in picker mode, even when no worktree is selected.
// That turns Enter-on-an-empty-filter-result into an exit-1 surprise.
// The fix surfaces a `picker_should_exit` flag so the loop can keep
// running until the user actually picks something.

#[test]
fn picker_should_exit_defaults_false() {
  let (_dir, app) = make_app();
  assert!(!app.picker_should_exit, "newly-built App must not signal a picker exit");
}

#[test]
fn picker_confirm_with_selection_signals_exit() {
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.list_state.select(Some(0));
  app.picker_confirm();
  assert!(
    app.picker_should_exit,
    "successful picker_confirm must signal the event loop to exit"
  );
  assert!(app.picker_result.is_some());
}

#[test]
fn picker_confirm_without_selection_does_not_signal_exit() {
  // regression: PR #53 Copilot review — picker_confirm with an empty filter
  // result unconditionally broke the event loop and exited 1.
  // When the fuzzy filter narrows the list down to zero matches, Enter
  // must keep the TUI open so the user can back-space and try again,
  // not exit with code 1.
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.worktrees.clear();
  app.list_state.select(None);

  app.picker_confirm();
  assert!(
    !app.picker_should_exit,
    "picker_confirm with no selection must NOT signal exit"
  );
  assert!(app.picker_result.is_none());
}

#[test]
fn picker_confirm_without_selection_reports_status() {
  // regression: PR #53 — Enter on an empty filter result was silently
  // swallowed; no status-bar hint surfaced the no-match state.
  // The user needs feedback explaining why Enter was inert. Surface a
  // status-bar hint so the no-match case isn't silently swallowed.
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.worktrees.clear();
  app.list_state.select(None);

  app.picker_confirm();
  assert!(
    app.status.to_lowercase().contains("no") || app.status.to_lowercase().contains("nothing"),
    "picker_confirm with no selection must update the status bar (got: {:?})",
    app.status
  );
}

// Copilot review (PR #53): in picker mode, `Esc` during an active filter
// only clears the filter — but the picker footer reads `esc:cancel`, so
// the documented contract is "Esc cancels the picker". Add an explicit
// `picker_cancel` that signals exit without recording a path so the
// event loop can route filter-mode Esc to the picker contract.

#[test]
fn picker_cancel_signals_exit_without_path() {
  // regression: PR #53 — picker footer reads `esc:cancel` but Esc-in-filter
  // only cleared the query; the picker contract wasn't honored.
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.list_state.select(Some(0));

  app.picker_cancel();
  assert!(
    app.picker_should_exit,
    "picker_cancel must signal the event loop to exit"
  );
  assert!(
    app.picker_result.is_none(),
    "picker_cancel must NOT record a path (Esc is the no-pick exit)"
  );
}

#[test]
fn picker_cancel_outside_picker_mode_is_inert() {
  // Defensive: like `picker_confirm`, `picker_cancel` should be a no-op
  // when wired into the wrong branch — the regular TUI's Esc semantics
  // are handled elsewhere.
  let (_dir, mut app) = make_app();
  assert!(!app.picker_mode);

  app.picker_cancel();
  assert!(
    !app.picker_should_exit,
    "picker_cancel outside picker mode must not flip the exit flag"
  );
}

// ---- Confirm countdown state machine (issue #30) -------------------------
//
// The countdown only arms when `delete_branch_on_remove` is ON AND the
// configured `confirm_countdown_secs` is non-zero. In every other case
// (off, or countdown_secs = 0) the modal stays single-keystroke. These
// tests pin every branch of that dispatch so a regression on either knob
// fails loudly.
//
// Time injection: `confirm_press_y`, `tick_confirm_countdown`, and the
// progress / remaining getters all take an `Instant` parameter so the
// tests can step through the countdown without sleeping. The event loop
// passes `Instant::now()` at the real call sites.

#[test]
fn countdown_total_zero_when_delete_branch_off() {
  let (_dir, app) = make_app();
  assert!(!app.delete_branch_on_remove);
  assert_eq!(app.confirm_countdown_total(), Duration::ZERO);
  assert!(!app.confirm_is_countdown_mode());
}

#[test]
fn countdown_total_matches_config_when_delete_branch_on() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  assert!(app.delete_branch_on_remove);
  assert_eq!(app.confirm_countdown_total(), Duration::from_secs(3));
  assert!(app.confirm_is_countdown_mode());
}

#[test]
fn countdown_total_zero_when_config_says_zero() {
  let (_dir, mut app) = make_app();
  app.config.tui.confirm_countdown_secs = 0;
  app.toggle_delete_branch();
  assert_eq!(app.confirm_countdown_total(), Duration::ZERO);
  assert!(
    !app.confirm_is_countdown_mode(),
    "countdown_secs=0 must fall back to the classic modal even when delete_branch is armed"
  );
}

#[test]
fn confirm_press_y_in_classic_mode_fires_immediately() {
  let (_dir, mut app) = make_app();
  // delete_branch OFF → classic flow
  let action = app.confirm_press_y(Instant::now());
  assert_eq!(action, ConfirmKeyAction::FireNow);
  assert!(
    app.confirm_countdown_started_at.is_none(),
    "classic mode must never set the timer"
  );
}

#[test]
fn confirm_press_y_in_countdown_mode_arms_the_timer() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let now = Instant::now();
  let action = app.confirm_press_y(now);
  assert_eq!(action, ConfirmKeyAction::Armed);
  assert_eq!(app.confirm_countdown_started_at, Some(now));
}

#[test]
fn confirm_press_y_a_second_time_disarms_the_timer() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let t1 = t0 + Duration::from_millis(500);
  let action = app.confirm_press_y(t1);
  assert_eq!(action, ConfirmKeyAction::Disarmed);
  assert!(app.confirm_countdown_started_at.is_none());
}

#[test]
fn confirm_dismiss_resets_timer_and_returns_to_list() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  app.view = View::Confirm;
  app.confirm_press_y(Instant::now());
  assert!(app.confirm_countdown_started_at.is_some());
  app.confirm_dismiss();
  assert_eq!(app.view, View::List);
  assert!(
    app.confirm_countdown_started_at.is_none(),
    "Esc/n must always disarm the countdown"
  );
}

#[test]
fn tick_unarmed_is_noop() {
  let (_dir, mut app) = make_app();
  let outcome = app.tick_confirm_countdown(Instant::now());
  assert_eq!(outcome, CountdownTickOutcome::NotArmed);
}

#[test]
fn tick_before_duration_is_pending() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let outcome = app.tick_confirm_countdown(t0 + Duration::from_millis(1500));
  assert_eq!(outcome, CountdownTickOutcome::Pending);
  assert!(
    app.confirm_countdown_started_at.is_some(),
    "pending tick must not clear the timer"
  );
}

#[test]
fn tick_at_duration_signals_ready_to_fire() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let outcome = app.tick_confirm_countdown(t0 + Duration::from_secs(3));
  assert_eq!(outcome, CountdownTickOutcome::ReadyToFire);
}

#[test]
fn tick_past_duration_signals_ready_to_fire() {
  // Tick rate is 200ms (mod.rs), so the elapsed value handed to the App
  // can overshoot the configured duration slightly. The state machine
  // must still report ReadyToFire, not Pending.
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let outcome = app.tick_confirm_countdown(t0 + Duration::from_millis(3500));
  assert_eq!(outcome, CountdownTickOutcome::ReadyToFire);
}

#[test]
fn countdown_progress_grows_with_elapsed() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  assert!((app.confirm_countdown_progress(t0) - 0.0).abs() < 1e-9);
  let mid = app.confirm_countdown_progress(t0 + Duration::from_millis(1500));
  assert!((0.49..=0.51).contains(&mid), "got progress = {mid}");
  let done = app.confirm_countdown_progress(t0 + Duration::from_secs(10));
  assert!((done - 1.0).abs() < 1e-9, "progress clamps to 1.0; got {done}");
}

#[test]
fn countdown_remaining_secs_counts_down_to_zero() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  assert_eq!(app.confirm_countdown_remaining_secs(t0), 3);
  // Anywhere strictly inside [2s, 3s) still rounds up to "1s left".
  assert_eq!(
    app.confirm_countdown_remaining_secs(t0 + Duration::from_millis(2500)),
    1
  );
  assert_eq!(app.confirm_countdown_remaining_secs(t0 + Duration::from_secs(3)), 0);
}

// Regression for Copilot review on PR #66: the gauge previously used
// `round()`, which rendered a fully-filled 10-cell bar at progress 0.95
// even though `confirm_delete` only fires at progress 1.0. The user saw
// "bar full" but the action hadn't happened yet — a false-positive
// signal on the destructive path. The fix: floor the cell count and
// keep the last cell empty until progress actually hits 1.0.

#[test]
fn filled_cells_zero_at_progress_zero() {
  assert_eq!(filled_cells_for_progress(0.0, 10), 0);
}

#[test]
fn filled_cells_full_only_at_progress_one() {
  assert_eq!(filled_cells_for_progress(1.0, 10), 10);
}

#[test]
fn filled_cells_below_one_keeps_last_cell_empty() {
  // 0.95 used to round() up to 10 cells (full bar) — that's the bug.
  // It must now floor to 9 (or less), reserving the last cell for the
  // "action fires" moment.
  assert_eq!(filled_cells_for_progress(0.95, 10), 9);
  // 0.99 even closer to full — still must not paint the last cell.
  assert!(filled_cells_for_progress(0.99, 10) < 10);
  // 0.999_999 same story — float weirdness must not flip the last cell.
  assert!(filled_cells_for_progress(0.999_999, 10) < 10);
}

#[test]
fn filled_cells_clamps_above_one() {
  // Float drift on an overshooting tick (200ms poll past N×1000ms) can
  // hand a progress > 1.0; the bar must clamp at the cell count.
  assert_eq!(filled_cells_for_progress(1.5, 10), 10);
}

#[test]
fn filled_cells_floors_partial_progress() {
  // 0.55 with 10 cells → 5.5 floor → 5 (not 6 from rounding).
  assert_eq!(filled_cells_for_progress(0.55, 10), 5);
  // 0.5 exact → 5 cells.
  assert_eq!(filled_cells_for_progress(0.5, 10), 5);
}

// ---- Issue / PR linking (issue #67) -------------------------------------

use gwm::github::{IssueState, IssueStatus, LinkSource, PrState, PrStatus};
use gwm::tui::{GitHubFetchState, LinkPromptStage, LinkTarget};

fn make_app_on_branch(name: &str) -> (tempfile::TempDir, git2::Repository, App) {
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch(name, &head, false).unwrap();
  }
  repo.set_head(&format!("refs/heads/{}", name)).unwrap();
  let app = App::new_at(Some(dir.path())).unwrap();
  (dir, repo, app)
}

#[test]
fn current_link_reflects_branch_name_auto_detect() {
  let (_dir, _repo, app) = make_app_on_branch("feat/#42-tui-search");
  let link = app.current_link();
  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_source, LinkSource::BranchName);
  assert_eq!(link.pr, None);
}

#[test]
fn enter_open_menu_transitions_view() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.enter_open_menu();
  assert_eq!(app.view, View::OpenMenu);
}

#[test]
fn open_menu_choose_issue_returns_url_when_linked_and_slug_available() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  app.enter_open_menu();
  let url = app.open_menu_pick(LinkTarget::Issue).unwrap();
  assert_eq!(url, "https://github.com/kbrdn1/gwm-cli/issues/42");
  // After picking, the view is back to the list.
  assert_eq!(app.view, View::List);
}

#[test]
fn open_menu_pick_returns_none_when_no_link() {
  let (_dir, repo, mut app) = make_app_on_branch("random-branch");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  app.enter_open_menu();
  let url = app.open_menu_pick(LinkTarget::Pr);
  assert!(url.is_none());
  // Status bar should hint why.
  assert!(
    app.status.to_lowercase().contains("no pr"),
    "status should mention missing PR link: {}",
    app.status
  );
}

#[test]
fn enter_link_prompt_starts_at_choose_target() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  assert_eq!(app.view, View::LinkPrompt);
  assert_eq!(app.link_prompt_stage(), LinkPromptStage::ChooseTarget);
  assert!(app.link_prompt_number_input().is_empty());
}

#[test]
fn link_prompt_choose_issue_advances_to_input() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  assert_eq!(app.link_prompt_stage(), LinkPromptStage::InputNumber);
}

#[test]
fn link_prompt_only_accepts_digits() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  for c in "12a3".chars() {
    app.link_prompt_push_char(c);
  }
  assert_eq!(app.link_prompt_number_input(), "123");
}

#[test]
fn link_prompt_submit_writes_branch_config() {
  let (_dir, repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  for c in "42".chars() {
    app.link_prompt_push_char(c);
  }
  app.link_prompt_submit().unwrap();
  // After submit, view returns to list and the link is persisted.
  assert_eq!(app.view, View::List);
  let cfg = repo.config().unwrap();
  let v = cfg.get_string("branch.random-branch.gwm-issue").unwrap();
  assert_eq!(v, "42");
}

#[test]
fn link_prompt_cancel_returns_to_list() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_cancel();
  assert_eq!(app.view, View::List);
}

#[test]
fn github_fetch_state_default_is_idle() {
  let (_dir, _repo, app) = make_app_on_branch("feat/#42-tui-search");
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn apply_fetch_results_loads_issue_and_pr_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let issue = IssueStatus {
    number: 42,
    title: "TUI search".into(),
    state: IssueState::Open,
    url: "https://example.test".into(),
    labels: vec!["feature".into()],
    updated_at: "2026-05-19T00:00:00Z".into(),
  };
  let pr = PrStatus {
    number: 61,
    title: "feat(tui): search".into(),
    state: PrState::Draft,
    url: "https://example.test/pr".into(),
    updated_at: "2026-05-19T00:00:00Z".into(),
    checks_passed: 2,
    checks_total: 3,
  };
  app.apply_issue_fetch_result(Ok(issue.clone()));
  app.apply_pr_fetch_result(Ok(pr.clone()));
  match app.issue_fetch_state() {
    GitHubFetchState::Loaded(_) => {}
    other => panic!("expected Loaded, got {:?}", other),
  }
  match app.pr_fetch_state() {
    GitHubFetchState::Loaded(_) => {}
    other => panic!("expected Loaded, got {:?}", other),
  }
}

#[test]
fn apply_fetch_error_stores_error_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_issue_fetch_result(Err("gh not found".into()));
  match app.issue_fetch_state() {
    GitHubFetchState::Error(msg) => assert!(msg.contains("gh"), "msg = {}", msg),
    other => panic!("expected Error, got {:?}", other),
  }
}

#[test]
fn refresh_link_invalidates_fetch_state() {
  // After the user changes selection or the branch link changes, any
  // previously fetched status no longer applies. The state must reset.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_issue_fetch_result(Err("e".into()));
  app.refresh_link();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn next_resets_fetch_state_on_selection_change() {
  // PR #68 Copilot review: selection change must invalidate the cached
  // GitHub fetch state, otherwise the right-panel Issue/PR block shows
  // stale data from the previously selected worktree.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(0));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.next();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn prev_resets_fetch_state_on_selection_change() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(0));
  app.apply_pr_fetch_result(Err("stale".into()));
  app.prev();
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn first_resets_fetch_state_on_selection_change() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(1));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.first();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn last_resets_fetch_state_on_selection_change() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(0));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.last();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn filter_clamping_resets_fetch_state_when_selection_moves() {
  // When typing into the filter narrows the visible set so much that the
  // current selection no longer points at the same worktree, the link
  // cache must invalidate too. Otherwise the right-panel block lies.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("zzz-unique"));
  app.list_state.select(Some(1));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.enter_filter();
  app.filter_push_char('z'); // only the second fixture matches
                             // The selection survives but the link cache should still reset because
                             // the filter operation can drop selection back to index 0 on the
                             // filtered subset. The contract: any selection-state mutation refreshes.
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn refresh_worktrees_resets_fetch_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_pr_fetch_result(Err("stale".into()));
  app.refresh().unwrap();
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn refresh_github_status_message_reflects_partial_failure() {
  // PR #68 Copilot review: when one of the fetches fails the status line
  // must not claim "refreshed" — the user should see something hinting
  // the refresh was incomplete.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  // Simulate the result of a refresh where the issue fetch failed but
  // (for the sake of this test) the PR fetch went through. Apply the
  // failure directly — we can't shell out to `gh` in unit tests.
  app.apply_issue_fetch_result(Err("gh: connection refused".into()));
  let pr = gwm::github::PrStatus {
    number: 1,
    title: "x".into(),
    state: gwm::github::PrState::Open,
    url: "https://example.test/pr".into(),
    updated_at: "".into(),
    checks_passed: 0,
    checks_total: 0,
  };
  app.apply_pr_fetch_result(Ok(pr));
  // Now call the same status-rendering logic the refresh would have run.
  app.report_github_refresh_status();
  assert!(
    !app.status.contains("refreshed"),
    "status must not claim 'refreshed' on partial failure: {}",
    app.status
  );
  assert!(
    app.status.to_lowercase().contains("error") || app.status.to_lowercase().contains("fail"),
    "status should mention failure: {}",
    app.status
  );
}

#[test]
fn refresh_github_status_message_celebrates_full_success() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let issue = gwm::github::IssueStatus {
    number: 42,
    title: "x".into(),
    state: gwm::github::IssueState::Open,
    url: "https://example.test".into(),
    labels: vec![],
    updated_at: "".into(),
  };
  app.apply_issue_fetch_result(Ok(issue));
  app.report_github_refresh_status();
  assert!(
    app.status.to_lowercase().contains("refreshed") || app.status.to_lowercase().contains("ok"),
    "all-green refresh should signal success: {}",
    app.status
  );
}

// ---- Issue #73: lazygit-style color helpers --------------------------------
// Three pure functions live in `tui/ui.rs` and are re-exported through
// `tui/mod.rs` so the table-driven tests below can pin the contract.
// Visual regressions land here first, before showing up in screenshots.

#[test]
fn branch_name_color_codes_synced_branch_as_green() {
  let synced = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  assert_eq!(branch_name_color(&synced), Color::Green);
}

#[test]
fn branch_name_color_codes_dirty_branch_as_red() {
  // Dirty = local working copy has uncommitted work. Lazygit doesn't surface
  // dirty in its branches view (it has a dedicated files view), but for a
  // worktree manager the most important signal is "this worktree has work
  // not yet captured anywhere" — red flags that hard.
  let dirty = BranchStatus {
    is_dirty: true,
    has_upstream: true,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  assert_eq!(branch_name_color(&dirty), Color::Red);
}

#[test]
fn branch_name_color_codes_ahead_or_behind_as_yellow() {
  let ahead = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 3,
    behind: 0,
    unknown: false,
  };
  let behind = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 0,
    behind: 2,
    unknown: false,
  };
  assert_eq!(branch_name_color(&ahead), Color::Yellow);
  assert_eq!(branch_name_color(&behind), Color::Yellow);
}

#[test]
fn branch_name_color_codes_unpublished_branch_as_magenta() {
  // No upstream + nothing dirty = branch exists locally only. Mirrors
  // lazygit's "?" magenta marker for RemoteBranchNotStoredLocally —
  // distinct from "synced" (green) and from "behind" (yellow) so the user
  // can tell at a glance whether they've pushed yet.
  let unpublished = BranchStatus {
    is_dirty: false,
    has_upstream: false,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  assert_eq!(branch_name_color(&unpublished), Color::Magenta);
}

#[test]
fn branch_name_color_codes_unknown_status_as_darkgray() {
  let unknown = BranchStatus {
    unknown: true,
    ..BranchStatus::default()
  };
  assert_eq!(branch_name_color(&unknown), Color::DarkGray);
}

#[test]
fn freshness_color_picks_green_for_recent_branches() {
  assert_eq!(freshness_color(Duration::from_secs(0)), Color::Green);
  assert_eq!(freshness_color(Duration::from_secs(86_400 * 3)), Color::Green);
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 6 + 3600 * 23)),
    Color::Green
  );
}

#[test]
fn freshness_color_picks_yellow_for_one_to_four_week_branches() {
  assert_eq!(freshness_color(Duration::from_secs(86_400 * 7)), Color::Yellow);
  assert_eq!(freshness_color(Duration::from_secs(86_400 * 15)), Color::Yellow);
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 29 + 3600 * 23)),
    Color::Yellow
  );
}

#[test]
fn freshness_color_picks_darkgray_for_stale_branches() {
  // Branches older than a month read as "stale" — gwm encourages cleanup
  // via `gwm doctor`, so the colour reinforces the prompt.
  assert_eq!(freshness_color(Duration::from_secs(86_400 * 30)), Color::DarkGray);
  assert_eq!(freshness_color(Duration::from_secs(86_400 * 365)), Color::DarkGray);
}

#[test]
fn pr_badge_color_maps_each_state_to_its_lazygit_palette() {
  // Mirrors `pkg/gui/presentation/branches.go::WithPrColor` — open=green,
  // draft=darkgray, merged=magenta, closed=red. The actual lazygit RGB
  // shades are slightly off-palette for terminal themes; we use the
  // 16-color names so the dots respect the user's colour scheme.
  assert_eq!(pr_badge_color(PrState::Open), Color::Green);
  assert_eq!(pr_badge_color(PrState::Draft), Color::DarkGray);
  assert_eq!(pr_badge_color(PrState::Merged), Color::Magenta);
  assert_eq!(pr_badge_color(PrState::Closed), Color::Red);
}

// Ensure the IssueState variants stay accessible — once `branch_name_color`
// and the rest land, the sidebar's badge function will need to fall back to
// an issue-derived colour when no PR is linked. The compile-time check below
// catches a stale import without polluting the runtime tests.
#[test]
fn issue_state_variants_compile() {
  let _ = IssueState::Open;
  let _ = IssueState::Closed;
}

// ---- Issue #73: configurable `o:` open key ---------------------------------
// `App::resolve_open_target` returns an `OpenTarget` that the event loop
// dispatches on (suspend-and-spawn for shell/editor, OS opener for finder).
// Pure resolution — no side effects, no spawn — so the test can pin the
// command resolution under every config / env combination.

use gwm::config::{TuiOpenConfig, TuiOpenMode};
use gwm::tui::OpenTarget;

#[test]
fn resolve_open_target_returns_none_when_nothing_selected() {
  let (_dir, mut app) = make_app();
  app.list_state.select(None);
  assert!(app.resolve_open_target().is_none());
}

#[test]
fn resolve_open_target_defaults_to_shell_mode() {
  // Default config (`mode = "shell"`) + a worktree selected → Shell variant
  // carrying the worktree path. The exact command depends on env / fallback;
  // here we only pin the variant + path so the test isn't $SHELL-dependent.
  let (_dir, app) = make_app();
  let target = app
    .resolve_open_target()
    .expect("main worktree should always be selectable");
  match target {
    OpenTarget::Shell { path, command } => {
      assert_eq!(path, app.worktrees[0].path);
      assert!(!command.is_empty(), "shell command must never be empty");
    }
    other => panic!("expected Shell variant, got {:?}", other),
  }
}

#[test]
fn resolve_open_target_honours_shell_cmd_override() {
  // `shell_cmd = "/usr/bin/fish"` must beat `$SHELL` — that's the whole
  // point of the override. Use a sentinel that's unlikely to be the
  // ambient shell to make the assertion deterministic.
  let (_dir, mut app) = make_app();
  app.config.tui.open = TuiOpenConfig {
    mode: TuiOpenMode::Shell,
    shell_cmd: Some("/sentinel/shell".into()),
    editor_cmd: None,
  };
  match app.resolve_open_target().unwrap() {
    OpenTarget::Shell { command, .. } => assert_eq!(command, "/sentinel/shell"),
    other => panic!("expected Shell, got {:?}", other),
  }
}

#[test]
fn resolve_open_target_uses_editor_mode_when_configured() {
  let (_dir, mut app) = make_app();
  app.config.tui.open = TuiOpenConfig {
    mode: TuiOpenMode::Editor,
    shell_cmd: None,
    editor_cmd: Some("hx".into()),
  };
  match app.resolve_open_target().unwrap() {
    OpenTarget::Editor { path, command } => {
      assert_eq!(path, app.worktrees[0].path);
      assert_eq!(command, "hx");
    }
    other => panic!("expected Editor, got {:?}", other),
  }
}

// ---- Issue #73: `y: yank` clipboard support -------------------------------
// `App::yank_selected_path` returns the path to push into the clipboard,
// or None when nothing's selected. The actual shell-out (pbcopy / wl-copy
// / xclip / clip) is tested manually — CI machines don't necessarily
// have any of these installed, so we keep the test scope to the pure
// resolution step and rely on the smoke test in the TUI for the rest.

#[test]
fn yank_selected_path_returns_path_for_selected_worktree() {
  let (_dir, app) = make_app();
  let path = app.yank_selected_path().expect("main worktree must be yankable");
  assert_eq!(path, app.worktrees[0].path);
}

#[test]
fn yank_selected_path_returns_none_when_nothing_selected() {
  let (_dir, mut app) = make_app();
  app.list_state.select(None);
  assert!(app.yank_selected_path().is_none());
}

#[test]
fn yank_candidates_for_current_platform_is_non_empty() {
  // Whatever the host OS, the candidate list must offer at least one
  // tool to try — empty would mean `y` could never succeed even with a
  // working pbcopy / xclip installed.
  assert!(
    !gwm::tui::clipboard_candidates().is_empty(),
    "clipboard candidates must include at least one tool for this OS"
  );
}

#[test]
fn resolve_open_target_uses_finder_mode_for_legacy_behaviour() {
  let (_dir, mut app) = make_app();
  app.config.tui.open = TuiOpenConfig {
    mode: TuiOpenMode::Finder,
    shell_cmd: None,
    editor_cmd: None,
  };
  match app.resolve_open_target().unwrap() {
    OpenTarget::Finder { path } => assert_eq!(path, app.worktrees[0].path),
    other => panic!("expected Finder, got {:?}", other),
  }
}
