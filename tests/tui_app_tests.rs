mod common;

use common::init_repo;
use gwm::naming::BRANCH_TYPES;
use gwm::tui::{filled_cells_for_progress, App, ConfirmKeyAction, CountdownTickOutcome, Field, View};
use gwm::worktree::{BranchStatus, WorktreeInfo};
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
  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), Default::default()));
  app.next();
  assert!(app.sidebar_cache.is_none(), "next() must invalidate the sidebar cache");

  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), Default::default()));
  app.prev();
  assert!(app.sidebar_cache.is_none(), "prev() must invalidate the sidebar cache");
}

#[test]
fn refresh_invalidates_sidebar_cache() {
  let (_dir, mut app) = make_app();
  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), Default::default()));
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

// ---- Sidebar sections (Option C — bordered subsections, no Commands block) ----

use gwm::tui::build_sidebar_sections;

fn detailed_worktree_fixture() -> WorktreeInfo {
  WorktreeInfo {
    name: "api-rest".into(),
    path: PathBuf::from("/Users/test/cc-worktree/api-rest"),
    branch: Some("feat/#42-api-rest".into()),
    head: Some("08d1029f1234567890abcdef".into()),
    is_main: true,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus {
      is_dirty: false,
      has_upstream: true,
      ahead: 0,
      behind: 0,
      unknown: false,
    },
  }
}

fn section_text(lines: &[ratatui::text::Line<'static>]) -> String {
  lines
    .iter()
    .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
    .collect::<Vec<_>>()
    .join("")
}

#[test]
fn sidebar_sections_omit_commands_block() {
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(&w);
  let all = format!(
    "{}\n{}\n{}",
    section_text(&sections.worktree),
    section_text(&sections.working_tree),
    section_text(&sections.recent_commits),
  );
  assert!(
    !all.contains("Commands"),
    "the Commands cheat-sheet block must be removed (lives in ? help); got: {}",
    all
  );
  assert!(
    !all.contains("Bootstrap worktree"),
    "help-overlay phrasing must not leak into the sidebar: {}",
    all
  );
  assert!(
    !all.contains("Toggle this sidebar"),
    "help-overlay phrasing must not leak into the sidebar: {}",
    all
  );
}

#[test]
fn sidebar_sections_omit_inline_section_headers() {
  // The new layout puts section titles on the Block borders, so the inline
  // `Basic Settings:` / `Recent commits:` / `Working tree:` headers must
  // disappear from the content lines.
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(&w);
  let all = format!(
    "{}\n{}\n{}",
    section_text(&sections.worktree),
    section_text(&sections.working_tree),
    section_text(&sections.recent_commits),
  );
  assert!(!all.contains("Basic Settings:"), "got: {}", all);
  assert!(!all.contains("Recent commits:"), "got: {}", all);
  assert!(!all.contains("Working tree:"), "got: {}", all);
}

#[test]
fn sidebar_worktree_section_is_compact_identity() {
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(&w);
  let text = section_text(&sections.worktree);

  assert!(text.contains("api-rest"), "name on top line: {}", text);
  assert!(text.contains("feat/#42-api-rest"), "branch shown: {}", text);
  assert!(text.contains("08d1029"), "short head shown: {}", text);
  assert!(
    text.contains("synced") || text.contains("✓"),
    "synced state badge shown: {}",
    text
  );
  assert!(
    text.contains("main") || text.contains("★"),
    "main badge shown: {}",
    text
  );
}

#[test]
fn sidebar_worktree_section_short_enough_for_compact_layout() {
  // Compact identity block: name, branch · head, badges, path → 4 lines target.
  // Allow ≤5 to leave headroom for variable badges.
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(&w);
  assert!(
    sections.worktree.len() <= 5,
    "compact worktree block must stay ≤5 lines (target 4), got {}: {:?}",
    sections.worktree.len(),
    sections.worktree.iter().map(section_text_single).collect::<Vec<_>>()
  );
}

fn section_text_single(l: &ratatui::text::Line<'static>) -> String {
  l.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn sidebar_worktree_section_skips_irrelevant_badges() {
  // A non-main, unlocked, non-prunable worktree should NOT advertise
  // those flags — only the ones that are true add visual noise.
  let mut w = detailed_worktree_fixture();
  w.is_main = false;
  let sections = build_sidebar_sections(&w);
  let text = section_text(&sections.worktree);
  assert!(
    !text.contains("★ main"),
    "non-main worktree must not show ★ main: {}",
    text
  );
  assert!(
    !text.contains("locked"),
    "unlocked worktree must not show locked badge: {}",
    text
  );
  assert!(
    !text.contains("prunable"),
    "non-prunable worktree must not show prunable badge: {}",
    text
  );
}

#[test]
fn sidebar_worktree_badge_uses_divergence_sigil_when_ahead() {
  // Regression: PR #70 review (Copilot) flagged that a non-dirty/non-unknown
  // status always rendered `✓ <label>`, which produced misleading badges like
  // `✓ ↑2` for branches that were *ahead* of upstream. The `✓` is reserved
  // for synced / clean — divergence must use `↑` / `↓` / `⇅` (or no sigil).
  let mut w = detailed_worktree_fixture();
  w.status = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 2,
    behind: 0,
    unknown: false,
  };
  let sections = build_sidebar_sections(&w);
  let badge = section_text_single(&sections.worktree[2]);
  assert!(
    !badge.contains("✓"),
    "ahead-only branch must not display the synced/clean ✓ sigil: {}",
    badge
  );
  assert!(badge.contains("↑2"), "ahead label must still be visible: {}", badge);
}

#[test]
fn sidebar_worktree_badge_uses_divergence_sigil_when_behind() {
  let mut w = detailed_worktree_fixture();
  w.status = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 0,
    behind: 3,
    unknown: false,
  };
  let sections = build_sidebar_sections(&w);
  let badge = section_text_single(&sections.worktree[2]);
  assert!(
    !badge.contains("✓"),
    "behind-only branch must not display the synced/clean ✓ sigil: {}",
    badge
  );
  assert!(badge.contains("↓3"), "behind label must still be visible: {}", badge);
}

#[test]
fn sidebar_worktree_badge_keeps_check_sigil_when_synced() {
  // Sanity: the fixture has `has_upstream=true, ahead=0, behind=0` so the
  // synced label *should* still display `✓`. Guards against an over-eager
  // fix that would drop the sigil everywhere.
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(&w);
  let badge = section_text_single(&sections.worktree[2]);
  assert!(badge.contains("✓"), "synced branch must keep the ✓ sigil: {}", badge);
  assert!(badge.contains("synced"), "label must still say synced: {}", badge);
}

// ---- tilde_compress path-boundary safety (PR #70 review, Copilot) -------

use gwm::tui::tilde_compress_with_home;

#[test]
fn tilde_compress_does_not_slice_across_path_boundaries() {
  // Regression: PR #70 review (Copilot) flagged that string `strip_prefix`
  // on the rendered home dir would slice into longer directory names that
  // merely *start with* the same characters — e.g. home `/home/al` would
  // turn `/home/alice/repo` into `~ice/repo`. The compression must only
  // fire when the prefix ends at a path separator boundary.
  let home = std::path::Path::new("/home/al");
  assert_eq!(
    tilde_compress_with_home("/home/alice/repo", home),
    "/home/alice/repo",
    "must not slice across the `alice` directory name"
  );
}

#[test]
fn tilde_compress_compresses_exact_home_match() {
  let home = std::path::Path::new("/home/alice");
  assert_eq!(tilde_compress_with_home("/home/alice", home), "~");
  assert_eq!(tilde_compress_with_home("/home/alice/repo", home), "~/repo");
  assert_eq!(tilde_compress_with_home("/home/alice/repo/sub", home), "~/repo/sub");
}

#[test]
fn tilde_compress_falls_back_when_path_outside_home() {
  let home = std::path::Path::new("/home/alice");
  assert_eq!(tilde_compress_with_home("/var/log/x", home), "/var/log/x");
  // Sibling directory starting with the same letters → no match.
  assert_eq!(tilde_compress_with_home("/home/alicent/x", home), "/home/alicent/x");
}

// ---- Issue / PR summary line width budgeting ----------------------------

use gwm::tui::{issue_summary_line, pr_summary_line};

fn line_visible_width(line: &ratatui::text::Line<'static>) -> usize {
  line.spans.iter().map(|s| s.content.chars().count()).sum()
}

#[test]
fn issue_summary_line_truncates_loaded_state_to_budget() {
  // Regression: with a 48-column sidebar, a fully-loaded issue line was
  // `#67 (auto) [open] <40-char title>` ≈ 58 chars, overflowing the
  // Issue/PR block and forcing ratatui's `Wrap` to push the title onto a
  // second visual row that the layout's `Constraint::Length` didn't
  // budget for. The Loaded variant must keep the head + badge prefix
  // intact and trim the title so the total ≤ max_width.
  let status = gwm::github::IssueStatus {
    number: 828,
    title:
      "Stats: subscriptions distribution across schools and individual customers (very long title to force truncation)"
        .into(),
    state: gwm::github::IssueState::Open,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let line = issue_summary_line(
    828,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(status),
    30,
  );
  let width = line_visible_width(&line);
  assert!(
    width <= 30,
    "loaded issue line must fit in 30 cols, got {}: {:?}",
    width,
    line.spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>()
  );
  // Ellipsis confirms the title was actually trimmed (not just clipped).
  let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(joined.ends_with('…'), "expected trailing ellipsis: {}", joined);
}

#[test]
fn pr_summary_line_truncates_loaded_state_to_budget() {
  let status = gwm::github::PrStatus {
    number: 70,
    title: "feat(tui): redesign Details sidebar with bordered subsections and four cards".into(),
    state: gwm::github::PrState::Open,
    url: String::new(),
    checks_passed: 3,
    checks_total: 3,
    updated_at: String::new(),
  };
  let line = pr_summary_line(
    70,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(status),
    35,
  );
  let width = line_visible_width(&line);
  assert!(
    width <= 35,
    "loaded PR line must fit in 35 cols, got {}: {:?}",
    width,
    line.spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>()
  );
}

#[test]
fn issue_summary_line_keeps_short_title_intact() {
  // Sanity: budget large enough → no truncation, no spurious ellipsis.
  let status = gwm::github::IssueStatus {
    number: 1,
    title: "short".into(),
    state: gwm::github::IssueState::Open,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let line = issue_summary_line(
    1,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Loaded(status),
    80,
  );
  let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains("short"),
    "short title must not be truncated: {}",
    joined
  );
  assert!(
    !joined.contains('…'),
    "no ellipsis when budget exceeds content: {}",
    joined
  );
}

#[test]
fn issue_summary_line_truncates_error_state_to_budget() {
  let line = issue_summary_line(
    42,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Error(
      "gh: API rate limit exceeded for user, retry after 60s with exponential backoff please".into(),
    ),
    30,
  );
  let width = line_visible_width(&line);
  assert!(width <= 30, "error line must fit in 30 cols, got {}", width);
}

// ---- Recent Commits panel: lazygit-style fill + clip (issue #71) ---------

use gwm::tui::{recent_commits_lines, RECENT_COMMITS_LIMIT};

fn add_commits(repo: &git2::Repository, count: usize) {
  use git2::Signature;
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  for i in 0..count {
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo
      .commit(Some("HEAD"), &sig, &sig, &format!("commit-{}", i), &tree, &[&parent])
      .unwrap();
  }
}

fn worktree_pointing_at_dir(dir: &std::path::Path) -> WorktreeInfo {
  WorktreeInfo {
    name: "test".into(),
    path: dir.to_path_buf(),
    branch: Some("main".into()),
    head: None,
    is_main: true,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
  }
}

#[test]
fn recent_commits_lines_respects_limit_when_repo_has_more() {
  let (dir, repo) = init_repo();
  add_commits(&repo, 14); // 15 total commits (1 seed + 14)
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 5);
  assert_eq!(
    lines.len(),
    5,
    "limit=5 must produce exactly 5 lines, got {}",
    lines.len()
  );
}

#[test]
fn recent_commits_lines_returns_all_when_under_limit() {
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 100);
  assert_eq!(
    lines.len(),
    1,
    "init_repo has 1 commit, asking for 100 should still return 1, got {}",
    lines.len()
  );
}

#[test]
#[allow(clippy::assertions_on_constants)] // intentional const pin
fn recent_commits_default_limit_fills_modern_terminal_heights() {
  // Regression: the previous hardcoded limit of 10 left the bottom of tall
  // sidebars empty. On a 50-line terminal, the Recent Commits block gets
  // ~12–18 rows after the small fixed sections take their slice; on a
  // 100-line terminal it gets ~70+. Keep the default generous so the
  // block fills the panel without re-shelling git on scroll. A future
  // contributor that lowers the constant will trip this test.
  assert!(
    RECENT_COMMITS_LIMIT >= 50,
    "RECENT_COMMITS_LIMIT must be ≥ 50 to fill a typical sidebar, got {}",
    RECENT_COMMITS_LIMIT
  );
}

#[test]
fn build_sidebar_sections_fetches_up_to_default_recent_commits_limit() {
  // Wire-up smoke: build_sidebar_sections must use RECENT_COMMITS_LIMIT (or
  // higher) so the cached section is dense enough to fill a tall panel.
  let (dir, repo) = init_repo();
  add_commits(&repo, 30); // 31 total commits
  let w = worktree_pointing_at_dir(dir.path());
  let sections = build_sidebar_sections(&w);
  assert_eq!(
    sections.recent_commits.len(),
    31,
    "expected all 31 commits to be cached (default limit ≥ 50 ≥ 31), got {}",
    sections.recent_commits.len()
  );
}

// ---- lazygit-style row format (hash + initials + subject) ----------------

use gwm::tui::{author_initials, COMMIT_HASH_DISPLAY_LEN};

#[test]
fn author_initials_two_word_name_picks_first_letters_of_each() {
  assert_eq!(author_initials("Kylian Bardini"), "KB");
  assert_eq!(author_initials("Jesse Duffield"), "JD");
}

#[test]
fn author_initials_single_word_takes_first_two_chars() {
  assert_eq!(author_initials("Linus"), "Li");
  assert_eq!(author_initials("kb"), "kb");
}

#[test]
fn author_initials_three_or_more_words_only_uses_first_two() {
  // Lazygit caps the result at 2 chars regardless of token count.
  assert_eq!(author_initials("Jean-Paul Marie Dupont"), "JM");
}

#[test]
fn author_initials_strips_leading_whitespace() {
  assert_eq!(author_initials("  Kylian Bardini"), "KB");
}

#[test]
fn author_initials_empty_returns_empty() {
  assert_eq!(author_initials(""), "");
  assert_eq!(author_initials("   "), "");
}

#[test]
fn author_initials_takes_first_unicode_scalar_per_token() {
  // Documented divergence from lazygit (PR #72 Copilot review): gwm's
  // `author_initials` uses `str::chars()` and slices on Unicode scalar
  // values, NOT grapheme clusters. Single-scalar emoji ("🦀") survive
  // intact, but multi-scalar grapheme clusters like the French flag
  // "🇫🇷" (two regional indicators) are split — only the first scalar
  // makes it into the initials. Pinning this so a future contributor
  // doesn't quietly break it by adopting a grapheme-aware crate.
  assert_eq!(author_initials("🦀 Crab"), "🦀C");
  assert_eq!(
    author_initials("🇫🇷 Bardini"),
    "🇫B",
    "two-scalar grapheme cluster (flag) is intentionally split per scalar"
  );
}

// ---- Graph node markers (○ commit, ◎ merge) -----------------------------

fn add_merge_commit(repo: &git2::Repository) -> git2::Oid {
  use git2::Signature;
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();

  // Start from current HEAD. Create a sibling branch with one commit, then
  // merge it back into HEAD with a true 2-parent merge commit.
  let base = repo.head().unwrap().peel_to_commit().unwrap();
  let branch_name = "tmp-side";
  repo.branch(branch_name, &base, false).unwrap();
  // Side commit.
  let side_oid = {
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo
      .commit(
        Some(&format!("refs/heads/{}", branch_name)),
        &sig,
        &sig,
        "side",
        &tree,
        &[&base],
      )
      .unwrap()
  };
  let side = repo.find_commit(side_oid).unwrap();
  // Merge commit on HEAD with two parents.
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo
    .commit(
      Some("HEAD"),
      &sig,
      &sig,
      "merge: side into trunk",
      &tree,
      &[&base, &side],
    )
    .unwrap()
}

#[test]
fn commit_row_carries_parent_hashes() {
  // Regression: the renderer needs the parent count to pick ○ vs ◎.
  // git_log_with_author must surface the parent list, not flatten it.
  let (dir, repo) = init_repo();
  add_merge_commit(&repo); // adds two extra commits (side + merge)
  let rows = gwm::worktree::git_log_with_author(dir.path(), 10).unwrap();
  // First row = merge commit (HEAD), should have 2 parents.
  assert!(!rows.is_empty(), "expected at least 1 commit");
  assert_eq!(
    rows[0].parents.len(),
    2,
    "HEAD is a merge commit and must surface both parents, got {:?}",
    rows[0].parents
  );
  // The seed `init` commit has no parent.
  let seed = rows
    .iter()
    .find(|r| r.subject == "init")
    .expect("seed commit must be in log");
  assert!(
    seed.parents.is_empty(),
    "seed commit has no parents, got {:?}",
    seed.parents
  );
}

#[test]
fn recent_commits_line_marks_merge_commit_with_bullseye() {
  // U+25CE ◎ is named "BULLSEYE" in Unicode — it is what lazygit uses
  // for `MergeSymbol`. The previous test name said "diamond" which
  // was geometrically wrong; this name pins the actual glyph.
  let (dir, repo) = init_repo();
  add_merge_commit(&repo);
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 10);
  // Find the merge commit row by subject; assert it carries ◎ somewhere.
  let merge = lines
    .iter()
    .find(|l| {
      let joined: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
      joined.contains("merge: side into trunk")
    })
    .expect("merge row must be present");
  let joined: String = merge.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains('◎'),
    "merge row must carry the ◎ bullseye marker, got: {}",
    joined
  );
}

#[test]
fn recent_commits_line_marks_normal_commit_with_open_circle() {
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1);
  let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains('○'),
    "non-merge row must carry the ○ marker, got: {}",
    joined
  );
  assert!(
    !joined.contains('◎'),
    "non-merge row must NOT carry the ◎ marker, got: {}",
    joined
  );
}

#[test]
fn recent_commits_line_starts_with_short_hash() {
  // The first span of every row must be a hash of exactly
  // COMMIT_HASH_DISPLAY_LEN hex chars (8 by default, matching lazygit).
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1);
  assert_eq!(lines.len(), 1, "init_repo should produce 1 commit");
  let head_span = lines[0]
    .spans
    .first()
    .expect("commit row must carry at least the hash span");
  assert_eq!(
    head_span.content.chars().count(),
    COMMIT_HASH_DISPLAY_LEN,
    "expected hash span of {} chars, got {:?}",
    COMMIT_HASH_DISPLAY_LEN,
    head_span.content
  );
  // Must be all-hex.
  assert!(
    head_span.content.chars().all(|c| c.is_ascii_hexdigit()),
    "expected hex hash, got {:?}",
    head_span.content
  );
}

#[test]
fn recent_commits_line_includes_author_initials_after_hash() {
  // init_repo signs commits as "gwm-test" — a single token → first 2 chars.
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1);
  let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
  // Initials live as a styled span after the hash + double space.
  assert!(
    joined.contains("gw"),
    "expected 'gw' initials from 'gwm-test', got {:?}",
    joined
  );
}

#[test]
fn recent_commits_line_carries_subject_unclipped() {
  // The renderer relies on ratatui's view-level clip — `recent_commits_lines`
  // itself must NOT pre-truncate the subject (otherwise scrollback / wider
  // sidebars would lose information). Verify the full subject is preserved.
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1);
  let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains("init"),
    "expected the seed 'init' subject, got {:?}",
    joined
  );
  assert!(
    !joined.contains('…'),
    "must not pre-emptively truncate with ellipsis: {:?}",
    joined
  );
}
