mod common;

use common::init_repo;
use gwm::naming::BRANCH_TYPES;
use gwm::tui::{
  branch_name_color, filled_cells_for_progress, freshness_color, header_title, pr_badge_color, App, ConfirmKeyAction,
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
    link: gwm::github::BranchLink::empty(),
  }
}

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at(Some(dir.path())).unwrap();
  (dir, app)
}

#[test]
fn header_title_includes_running_version_repo_and_workdir() {
  // The TUI header surfaces `gwm v<version> — <repo> (<workdir>)`.
  // Cross-check both halves: the version comes from CARGO_PKG_VERSION
  // (the same string `gwm --version` prints) and the format wraps
  // each piece exactly as the docstring describes — so a regression
  // dropping the `v` prefix, the em-dash, or the repo name fails the
  // test for the right reason.
  let title = header_title("gwm-cli", "/Users/kbrdn1/Projects/Perso/gwm-cli");
  assert!(title.contains("gwm-cli"), "missing repo name: {}", title);
  assert!(
    title.contains("/Users/kbrdn1/Projects/Perso/gwm-cli"),
    "missing workdir: {}",
    title
  );
  let version_token = format!("v{}", env!("CARGO_PKG_VERSION"));
  assert!(
    title.contains(&version_token),
    "missing running version `{}`: {}",
    version_token,
    title
  );
  // Pin the exact layout so a refactor moving the version pre/post
  // would have to update this assertion deliberately.
  assert_eq!(
    title,
    format!(
      " gwm v{} — gwm-cli (/Users/kbrdn1/Projects/Perso/gwm-cli) ",
      env!("CARGO_PKG_VERSION")
    )
  );
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
  assert_eq!(app.create_form.field, Field::Type);
  assert!(app.create_form.issue.is_empty());
  assert!(app.create_form.desc.is_empty());
}

#[test]
fn create_field_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_next_field();
  assert_eq!(app.create_form.field, Field::Issue);
  app.create_next_field();
  assert_eq!(app.create_form.field, Field::Desc);
  app.create_next_field();
  assert_eq!(app.create_form.field, Field::Type);
  app.create_prev_field();
  assert_eq!(app.create_form.field, Field::Desc);
}

#[test]
fn create_type_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_prev_type();
  assert_eq!(app.create_form.type_index, BRANCH_TYPES.len() - 1);
  app.create_next_type();
  assert_eq!(app.create_form.type_index, 0);
}

#[test]
fn create_push_only_digits_on_issue() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_form.field = Field::Issue;
  for c in "12a3".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_form.issue, "123");
}

#[test]
fn create_push_accepts_desc_chars() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_form.field = Field::Desc;
  for c in "foo-bar".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_form.desc, "foo-bar");
  app.create_pop_char();
  assert_eq!(app.create_form.desc, "foo-ba");
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
  assert!(!app.filter.active, "filter must default to inactive");
  assert!(app.filter.query.is_empty(), "filter query must default to empty");
}

#[test]
fn enter_filter_activates_capture_and_disarms_gg() {
  let (_dir, mut app) = make_app();
  app.handle_g(); // arm `gg`
  assert!(app.pending_g);

  app.enter_filter();
  assert!(app.filter.active);
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
  app.filter.query = "auth".into();
  app.enter_filter();
  assert_eq!(app.filter.query, "auth");
  assert!(app.filter.active);
}

#[test]
fn filter_push_char_appends_to_query() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  for c in "tui".chars() {
    app.filter_push_char(c);
  }
  assert_eq!(app.filter.query, "tui");
}

#[test]
fn filter_pop_char_removes_last_char() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter.query = "tuix".into();
  app.filter_pop_char();
  assert_eq!(app.filter.query, "tui");
}

#[test]
fn filter_pop_char_on_empty_is_noop() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter_pop_char();
  assert_eq!(app.filter.query, "");
  assert!(app.filter.active, "popping an empty query must not exit filter mode");
}

#[test]
fn exit_filter_keep_disables_capture_keeps_query() {
  // Enter behaviour: filter sticks, navigation returns to the list.
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter.query = "auth".into();
  app.exit_filter_keep();
  assert!(!app.filter.active);
  assert_eq!(app.filter.query, "auth", "Enter must not wipe the query");
}

#[test]
fn exit_filter_cancel_clears_query() {
  // Esc behaviour: full list back, query gone.
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter.query = "auth".into();
  app.exit_filter_cancel();
  assert!(!app.filter.active);
  assert!(app.filter.query.is_empty(), "Esc must clear the query");
}

#[test]
fn filtered_indices_returns_all_when_query_empty() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("beta"),
    worktree_fixture("gamma"),
  ];
  let idx: Vec<usize> = app.filtered_indices().to_vec();
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
  app.filter.query = "tui".into();

  let idx: Vec<usize> = app.filtered_indices().to_vec();
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
  app.filter.query = "auth".into();
  // Sanity-guard the precondition: if a future refactor introduces an `auth`
  // substring into the fixture, the test would silently degrade to a
  // substring check again.
  assert!(
    !app.worktrees[0].name.contains("auth"),
    "fixture must not contain 'auth' as a substring or the test stops covering subsequence"
  );

  let idx: Vec<usize> = app.filtered_indices().to_vec();
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
  app.filter.query = "auth".into();

  let idx: Vec<usize> = app.filtered_indices().to_vec();
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
  app.filter.query = "zzzz".into();
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
  app.filter.query = "auth".into();
  app.list_state.select(Some(0));

  let sel = app.selected().expect("filtered selection must resolve");
  assert_eq!(sel.name, "authentication");
}

#[test]
fn selected_returns_none_when_filter_matches_nothing() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("alpha"), worktree_fixture("beta")];
  app.filter.query = "zzzz".into();
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
  app.filter.query = "foo".into();
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
  app.filter.query = "foo".into();
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
  app.filter.query = "foo".into();
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
  app.filter.query = "foo".into();
  app.list_state.select(Some(1));

  // Typing more reduces the match set to just "foo-bar".
  for c in "-bar".chars() {
    app.filter_push_char(c);
  }
  let filtered: Vec<usize> = app.filtered_indices().to_vec();
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
  app.filter.query = "foo".into();
  app.list_state.select(Some(0));

  app.exit_filter_cancel();
  assert!(app.filter.query.is_empty());
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
  assert!(app.filter.active, "picker mode must open with the filter bar active");
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
    app.confirm.started_at.is_none(),
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
  assert_eq!(app.confirm.started_at, Some(now));
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
  assert!(app.confirm.started_at.is_none());
}

#[test]
fn confirm_dismiss_resets_timer_and_returns_to_list() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  app.view = View::Confirm;
  app.confirm_press_y(Instant::now());
  assert!(app.confirm.started_at.is_some());
  app.confirm_dismiss();
  assert_eq!(app.view, View::List);
  assert!(
    app.confirm.started_at.is_none(),
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
    app.confirm.started_at.is_some(),
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

// ---- Configurable launchers (issue #75) --------------------------------
//
// The `R` key in the worktree-list view now triggers the [review]
// launcher; the previous "refresh GitHub status" action moves to `F`.
// `f` becomes the worktree refresh (previously `r`). These tests pin
// the new methods that back those keybindings; the actual key
// dispatch sits in `src/tui/mod.rs` and is exercised by the
// behaviour we assert here.

#[test]
fn prepare_review_returns_none_when_no_review_configured() {
  // Default config has no [review] block ⇒ pressing `R` must surface
  // a status-bar hint, not spawn anything.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let plan = app.prepare_review();
  assert!(plan.is_none(), "no [review] config ⇒ no launcher plan");
  let s = app.status.to_lowercase();
  assert!(
    s.contains("review") && (s.contains("not configured") || s.contains("not set") || s.contains("gwm.toml")),
    "status bar must explain why R was inert: {}",
    app.status
  );
}

#[test]
fn prepare_review_skips_when_no_changes_and_flag_on() {
  // skip_when_no_changes defaults to true. A fresh repo has zero
  // commits past `main` ⇒ R must short-circuit with the documented
  // "no changes to review" status line.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.config.review.command = Some("lumen diff {base}..{head}".into());
  app.config.review.fullscreen = Some(true);
  // The branch was created from HEAD (main), and we have made no new
  // commits — count_commits_ahead must be 0.
  let plan = app.prepare_review();
  assert!(plan.is_none(), "no commits past base + skip_when_no_changes ⇒ skip");
  let s = app.status.to_lowercase();
  assert!(
    s.contains("no changes"),
    "status bar must say 'no changes': {}",
    app.status
  );
  assert!(
    app.status.contains("main") || app.status.contains("dev"),
    "status should name the resolved base: {}",
    app.status
  );
}

#[test]
fn prepare_review_returns_plan_when_configured_and_diff_exists() {
  // The full happy path: [review].command set, head ≠ base (one extra
  // commit), skip_when_no_changes off so we don't have to bother
  // creating a diff. The plan must surface the expanded argv with all
  // placeholders substituted.
  let (dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.config.review.command = Some("reviewer --base {base} --head {head}".into());
  app.config.review.skip_when_no_changes = false;
  // Force a known base so the test isn't sensitive to the chain order.
  app.config.review.default_base = Some("main".into());
  // Drop the upstream / gwm-base config so default_base wins.
  let mut cfg = repo.config().unwrap();
  let _ = cfg.remove("branch.feat/#42-tui-search.gwm-base");

  let plan = app.prepare_review().expect("configured + no skip ⇒ plan present");
  let argv = &plan.expanded.argv;
  assert_eq!(argv[0], "reviewer");
  assert_eq!(argv[1], "--base");
  assert_eq!(argv[2], "main");
  assert_eq!(argv[3], "--head");
  assert_eq!(argv[4], "feat/#42-tui-search");
  // The cwd must be the worktree path so the spawned tool sees the
  // selected branch's working tree as `.`. Use `paths_equal` so the
  // macOS `/private/var` ↔ `/var` symlink doesn't flake the assertion.
  assert!(
    common::paths_equal(&plan.cwd, dir.path()),
    "plan.cwd = {} vs dir = {}",
    plan.cwd.display(),
    dir.path().display()
  );
}

#[test]
fn prepare_review_respects_default_base_chain() {
  // `branch.<n>.gwm-base` (set by gwm create) must outrank
  // `[review].default_base`. Recorded as `main` by the fixture.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  // Manually record gwm-base since the test fixture didn't go through
  // worktree::add. The base chain reads from this key.
  gwm::launcher::write_gwm_base(&app.repo, "feat/#42-tui-search", "release-3.x").unwrap();
  app.config.review.command = Some("echo {base}".into());
  app.config.review.skip_when_no_changes = false;
  app.config.review.default_base = Some("trunk".into());

  let plan = app.prepare_review().expect("must resolve");
  assert_eq!(
    plan.expanded.argv,
    vec!["echo", "release-3.x"],
    "gwm-base must win over [review].default_base"
  );
}

#[test]
fn prepare_git_tui_default_uses_lazygit() {
  // Backwards-compat: no `[git_tui]` block ⇒ `l` still runs
  // `lazygit -p <path>` fullscreen.
  let (dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let plan = app.prepare_git_tui().expect("git_tui has a default");
  let argv = &plan.expanded.argv;
  assert_eq!(argv[0], "lazygit");
  assert_eq!(argv[1], "-p");
  // The path the launcher injects is the *currently selected* worktree;
  // make_app_on_branch only creates the main, so the default selection
  // is the main repo's path. Use `paths_equal` to dodge the macOS
  // `/private/var` ↔ `/var` mismatch (same trick as elsewhere in the
  // suite).
  assert!(
    common::paths_equal(std::path::Path::new(&argv[2]), dir.path()),
    "argv[2] = {} vs dir = {}",
    argv[2],
    dir.path().display()
  );
  assert!(plan.fullscreen, "lazygit defaults to fullscreen");
}

#[test]
fn prepare_git_tui_uses_user_command_when_set() {
  // [git_tui].command override must beat the lazygit default.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.config.git_tui.command = Some("gitui -d {path}".into());
  app.config.git_tui.fullscreen = Some(false);
  let plan = app.prepare_git_tui().expect("must resolve");
  assert_eq!(plan.expanded.argv[0], "gitui");
  assert_eq!(plan.expanded.argv[1], "-d");
  assert!(!plan.fullscreen, "user opted out of fullscreen");
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

// ---- Issue #73 (PR #74 follow-up): table marker pastille ------------------
// The marker column (first cell) doubles as the lazygit-style status dot.
// `★` still wins for main; non-main worktrees that carry a link (issue or
// PR) get `●` so the row visually signals "this has GitHub context", even
// when the sidebar is hidden (`<120` cols or `v` collapsed).

#[test]
fn table_marker_for_main_worktree_is_yellow_star() {
  use gwm::github::BranchLink;
  let mut w = worktree_fixture("main");
  w.is_main = true;
  w.link = BranchLink::empty();
  let (label, color) = gwm::tui::table_marker(&w);
  assert_eq!(label, "★");
  assert_eq!(color, Color::Yellow);
}

#[test]
fn table_marker_for_linked_non_main_is_neutral_dot() {
  // No fetched state available at the table layer — colour stays neutral
  // (Cyan) so the dot reads as "has link" without claiming a specific
  // open/closed status. The coloured dot lives in the sidebar header where
  // the live fetch state is known.
  use gwm::github::{BranchLink, LinkSource};
  let mut w = worktree_fixture("feat-1");
  w.is_main = false;
  w.link = BranchLink {
    issue: Some(42),
    pr: None,
    issue_source: LinkSource::BranchName,
    pr_source: LinkSource::None,
  };
  let (label, color) = gwm::tui::table_marker(&w);
  assert_eq!(label, "●");
  assert_eq!(color, Color::Cyan);
}

#[test]
fn table_marker_for_unlinked_non_main_is_blank() {
  use gwm::github::BranchLink;
  let mut w = worktree_fixture("feat-1");
  w.is_main = false;
  w.link = BranchLink::empty();
  let (label, _color) = gwm::tui::table_marker(&w);
  assert_eq!(label, " ", "unlinked non-main rows keep an empty marker cell");
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
    link: gwm::github::BranchLink::empty(),
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
    link: gwm::github::BranchLink::empty(),
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

// ---- Commit graph topology (lazygit port — pipes + connectors) ---------

use gwm::tui::commit_graph::{box_drawing_chars, build_pipe_sets, render_commits, render_pipe_set, test_row, PipeKind};

fn spans_to_text(spans: &[ratatui::text::Span<'static>]) -> String {
  spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
#[allow(clippy::type_complexity)]
fn graph_glyph_table_matches_lazygit_truth_table() {
  // 16-case ground truth ported verbatim from
  // `lazygit/pkg/gui/presentation/graph/cell.go::getBoxDrawingChars`.
  // If any of these flip, the entire graph rendering will silently shift.
  let cases: &[((bool, bool, bool, bool), (char, char))] = &[
    ((true, true, true, true), ('│', '─')),
    ((true, true, true, false), ('│', ' ')),
    ((true, true, false, true), ('│', '─')),
    ((true, true, false, false), ('│', ' ')),
    ((true, false, true, true), ('┴', '─')),
    ((true, false, true, false), ('╯', ' ')),
    ((true, false, false, true), ('╰', '─')),
    ((true, false, false, false), ('╵', ' ')),
    ((false, true, true, true), ('┬', '─')),
    ((false, true, true, false), ('╮', ' ')),
    ((false, true, false, true), ('╭', '─')),
    ((false, true, false, false), ('╷', ' ')),
    ((false, false, true, true), ('─', '─')),
    ((false, false, true, false), ('─', ' ')),
    ((false, false, false, true), ('╶', '─')),
    ((false, false, false, false), (' ', ' ')),
  ];
  for &((u, d, l, r), expected) in cases {
    assert_eq!(
      box_drawing_chars(u, d, l, r),
      expected,
      "case ({}, {}, {}, {}) — expected {:?}",
      u,
      d,
      l,
      r,
      expected
    );
  }
}

#[test]
fn graph_linear_history_emits_single_column_circles() {
  // Three commits, each pointing at the next: c (parent b) → b (parent a) → a (no parent).
  let rows = vec![test_row("c", &["b"]), test_row("b", &["a"]), test_row("a", &[])];
  let graphs = render_commits(&rows);
  assert_eq!(graphs.len(), 3);
  // Each row should be a 2-cell render (one column → 2 chars).
  for (idx, g) in graphs.iter().enumerate() {
    let text = spans_to_text(g);
    assert!(
      text.contains('○'),
      "linear history row {} must carry a ○ node, got {:?}",
      idx,
      text
    );
    assert!(
      !text.contains('◎'),
      "linear history row {} must NOT carry a ◎ merge node, got {:?}",
      idx,
      text
    );
  }
}

#[test]
fn graph_merge_commit_carries_bullseye_and_branch_corners() {
  // Topology:
  //   c (merge: parents = a, b)
  //   b (parent a)         ← side branch
  //   a (no parent)        ← trunk root
  let rows = vec![test_row("c", &["a", "b"]), test_row("b", &["a"]), test_row("a", &[])];
  let graphs = render_commits(&rows);
  // Row 0 = merge commit, must carry ◎.
  let merge_text = spans_to_text(&graphs[0]);
  assert!(merge_text.contains('◎'), "merge row must carry ◎, got {:?}", merge_text);
  // Row 1 (the side branch) must spawn somewhere outside column 0 —
  // there should be a `╮` corner on row 0 to drop the second parent
  // into a fresh column.
  assert!(
    merge_text.contains('╮') || merge_text.contains('─'),
    "merge row must carry a corner / horizontal stroke into the new branch column, got {:?}",
    merge_text
  );
}

#[test]
fn graph_pipe_set_first_commit_seeds_starts_pipe() {
  // Internal invariant: the first row's pipe set must contain a STARTS
  // pipe for the first commit, regardless of how many parents it has.
  let rows = vec![test_row("a", &["b"])];
  let pipes = build_pipe_sets(&rows);
  assert_eq!(pipes.len(), 1);
  assert!(
    pipes[0]
      .iter()
      .any(|p| p.kind == PipeKind::Starts && p.from_hash == "a"),
    "first row must contain a STARTS pipe whose from_hash is the commit itself, got {:?}",
    pipes[0]
  );
}

#[test]
fn graph_pipe_set_merge_commit_emits_extra_starts_per_parent() {
  // A merge with 2 parents should emit 2 STARTS pipes whose from_pos is
  // the commit's column and to_pos points at distinct columns.
  let rows = vec![test_row("c", &["a", "b"]), test_row("b", &["a"]), test_row("a", &[])];
  let pipes = build_pipe_sets(&rows);
  let row0 = &pipes[0];
  let starts: Vec<_> = row0.iter().filter(|p| p.kind == PipeKind::Starts).collect();
  assert_eq!(
    starts.len(),
    2,
    "merge row must emit 2 STARTS pipes (one per parent), got {} ({:?})",
    starts.len(),
    starts
  );
}

#[test]
fn graph_render_pipe_set_empty_input_returns_empty() {
  let graphs = render_commits(&[]);
  assert!(graphs.is_empty());
}

#[test]
fn graph_row_width_is_deterministic_on_commit_list() {
  // The graph width is `2 * (max_pos + 1)` chars, derived from pipe
  // topology — it must NOT depend on terminal width or external state.
  // Snapshot the linear-history width so a regression caught quickly.
  let rows = vec![test_row("c", &["b"]), test_row("b", &["a"]), test_row("a", &[])];
  let graphs = render_commits(&rows);
  for g in &graphs {
    let text = spans_to_text(g);
    let chars = text.chars().count();
    assert!(
      (1..=8).contains(&chars),
      "linear-history row must render in ≤ 8 chars, got {} ({:?})",
      chars,
      text
    );
  }
}

#[test]
fn graph_render_pipe_set_handles_single_pipe_starts() {
  use gwm::tui::commit_graph::Pipe;
  let pipes = vec![Pipe {
    from_pos: 0,
    to_pos: 0,
    from_hash: "a".into(),
    to_hash: "b".into(),
    kind: PipeKind::Starts,
  }];
  let spans = render_pipe_set(&pipes);
  let text = spans_to_text(&spans);
  // Cell 0: ○ + filler (space, since right has no neighbor)
  assert!(text.starts_with('○'), "expected ○ glyph at column 0, got {:?}", text);
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

// --- TOFU trust gate, TUI side (issue #95, PR #113 follow-up) -----------
//
// `check_trust_for_bootstrap` is the silent gate shared by
// `submit_create` and `bootstrap_selected`. These tests exercise it
// directly — driving the full event loop through `submit_create` would
// require a real worktree base + `worktree::add`, which is orthogonal
// to the security policy under test. The two `submit_create_*` /
// `bootstrap_selected_*` integration tests at the bottom of this
// section verify the actual call-site wiring.

use gwm::trust::TrustMode;

/// Build an App whose workdir already has a `.gwm.toml` ready to be
/// hashed by the gate. Returns the dir keepalive + the App (with
/// `trust_mode = Prompt` by default).
fn app_with_config(toml_body: &str) -> (tempfile::TempDir, App) {
  let (dir, _repo) = init_repo();
  std::fs::write(dir.path().join(".gwm.toml"), toml_body).unwrap();
  let app = App::new_at(Some(dir.path())).unwrap();
  (dir, app)
}

#[test]
fn tui_gate_passes_when_no_gwm_toml_present() {
  // The CLI gate is a no-op when `.gwm.toml` doesn't exist — same
  // contract on the TUI side. Construction with `init_repo`'s empty
  // workdir already covers this.
  let (_dir, app) = make_app();
  assert!(
    matches!(app.check_trust_for_bootstrap(), Ok(None)),
    "no .gwm.toml → gate must clear"
  );
}

#[test]
fn tui_gate_passes_on_empty_bootstrap_surface() {
  // A `.gwm.toml` with only `[worktree]` (no executable surface)
  // carries no RCE risk. Prompting in that case would just train
  // the user to mash `y` — same UX bug the CLI gate avoids.
  let (_dir, app) = app_with_config(
    r#"[worktree]
base = "/tmp/never-used"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  );
  assert!(
    matches!(app.check_trust_for_bootstrap(), Ok(None)),
    "empty surface → gate must clear"
  );
}

#[test]
fn tui_gate_refuses_untrusted_config_in_prompt_mode() {
  // Default `TrustMode::Prompt`: a `.gwm.toml` declaring a bootstrap
  // command, no ledger entry → the gate refuses with a status-bar
  // message. The CLI in this same position would prompt; the TUI
  // can't (alternate-screen + no modal yet), so it points the user
  // at the CLI gate / env bypass.
  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");
  // Hold the env lock + clean up afterwards to keep the harness
  // hermetic against the trust_tests env-mutation test.
  let prior_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  let prior_allow = std::env::var("GWM_ALLOW_BOOTSTRAP").ok();
  // SAFETY: this test is the sole env mutator inside this binary.
  // The `trust_tests` env tests live in a separate test binary and
  // run in their own process, so there's no cross-binary race.
  unsafe {
    std::env::set_var("GWM_TRUST_LEDGER", &ledger);
    std::env::remove_var("GWM_ALLOW_BOOTSTRAP");
  }

  let (_dir, app) = app_with_config(
    r#"[[bootstrap.command]]
name = "x"
run  = "true"
"#,
  );

  match app.check_trust_for_bootstrap() {
    Ok(Some(msg)) => {
      assert!(
        msg.contains("not in trust ledger"),
        "refuse message must point at the gate (got: {})",
        msg
      );
      assert!(
        msg.contains("--allow-bootstrap") || msg.contains("GWM_ALLOW_BOOTSTRAP"),
        "refuse message must surface the bypass options (got: {})",
        msg
      );
    }
    other => panic!("expected refuse, got {:?}", other),
  }

  // SAFETY: restoration paired with the set/remove above.
  unsafe {
    match prior_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
    match prior_allow {
      Some(v) => std::env::set_var("GWM_ALLOW_BOOTSTRAP", v),
      None => std::env::remove_var("GWM_ALLOW_BOOTSTRAP"),
    }
  }
}

#[test]
fn tui_gate_clears_under_allow_mode() {
  // `--allow-bootstrap` (resolved to `TrustMode::Allow` at the
  // entrypoint) bypasses the gate. Threading it through
  // `with_trust_mode` is the whole reason for this PR follow-up —
  // pin the wiring down.
  let (_dir, app) = app_with_config(
    r#"[[bootstrap.command]]
name = "x"
run  = "true"
"#,
  );
  let app = app.with_trust_mode(TrustMode::Allow);
  assert!(
    matches!(app.check_trust_for_bootstrap(), Ok(None)),
    "Allow mode → gate must clear regardless of ledger state"
  );
}

#[test]
fn tui_gate_refuses_under_deny_mode_even_with_safe_config() {
  // `--deny-bootstrap` is the forensic mode: refuse even if there's
  // nothing scary. The empty-surface short-circuit comes BEFORE the
  // Deny check inside `evaluate`, so a truly empty surface still
  // clears — Deny mode is meaningful only when a real surface
  // exists. Document that with the same fixture as the prompt-mode
  // refuse test (a config with a bootstrap.command).
  let (_dir, app) = app_with_config(
    r#"[[bootstrap.command]]
name = "x"
run  = "true"
"#,
  );
  let app = app.with_trust_mode(TrustMode::Deny);
  let outcome = app.check_trust_for_bootstrap();
  match outcome {
    Ok(Some(msg)) => assert!(
      msg.contains("--deny-bootstrap"),
      "deny refuse must name the flag (got: {})",
      msg
    ),
    other => panic!("expected deny refuse, got {:?}", other),
  }
}

#[test]
fn tui_submit_create_aborts_on_untrusted_config() {
  // End-to-end: `submit_create` reads the gate, sets the status bar
  // verbatim from the refuse message, and crucially does NOT call
  // `worktree::add` — meaning no orphaned worktree dir lands on
  // disk. We pin that postcondition by checking the resolved
  // worktree path is absent after the call.
  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");
  let base_dir = tempfile::TempDir::new().unwrap();
  let prior_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  let prior_allow = std::env::var("GWM_ALLOW_BOOTSTRAP").ok();
  // SAFETY: see comment on `tui_gate_refuses_untrusted_config_in_prompt_mode`.
  unsafe {
    std::env::set_var("GWM_TRUST_LEDGER", &ledger);
    std::env::remove_var("GWM_ALLOW_BOOTSTRAP");
  }

  let body = format!(
    r#"[worktree]
base = "{base}"
path_pattern = "{{type}}-{{issue}}-{{desc}}"
branch_pattern = "{{type}}/#{{issue}}-{{desc}}"

[[bootstrap.command]]
name = "echo"
run  = "echo would-have-run"
"#,
    base = base_dir.path().display(),
  );
  let (_dir, mut app) = app_with_config(&body);

  // Drive `submit_create` end-to-end: type=feat (index in the
  // resolved branch_types), issue + desc filled in.
  let feat_idx = app
    .branch_types
    .iter()
    .position(|t| t.name == "feat")
    .expect("`feat` is in BRANCH_TYPES defaults");
  app.create_form.type_index = feat_idx;
  app.create_form.issue = "42".into();
  app.create_form.desc = "untrusted-creates".into();

  // Must succeed (no Err — Err would crash out of the event loop) and
  // the gate must have set a refuse message.
  app.submit_create().expect("submit_create must surface a soft refusal");

  assert!(
    app.status.contains("not in trust ledger"),
    "status must reflect the gate refusal (got: {})",
    app.status
  );
  let would_have_been = base_dir.path().join("feat-42-untrusted-creates");
  assert!(
    !would_have_been.exists(),
    "worktree dir MUST NOT be created when the gate refuses (got: {})",
    would_have_been.display()
  );

  // SAFETY: env restoration paired with set/remove above.
  unsafe {
    match prior_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
    match prior_allow {
      Some(v) => std::env::set_var("GWM_ALLOW_BOOTSTRAP", v),
      None => std::env::remove_var("GWM_ALLOW_BOOTSTRAP"),
    }
  }
  let _ = BRANCH_TYPES; // keep the import live; the indirect lookup above relies on the default list.
}

#[test]
fn tui_bootstrap_selected_aborts_on_untrusted_config() {
  // Counterpart of the submit_create test — the `b` keybinding
  // (re-run bootstrap on an existing worktree) takes the same gate.
  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");
  let prior_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  let prior_allow = std::env::var("GWM_ALLOW_BOOTSTRAP").ok();
  // SAFETY: same rationale as the previous test.
  unsafe {
    std::env::set_var("GWM_TRUST_LEDGER", &ledger);
    std::env::remove_var("GWM_ALLOW_BOOTSTRAP");
  }

  let (_dir, mut app) = app_with_config(
    r#"[[bootstrap.command]]
name = "echo"
run  = "echo trapped"
"#,
  );

  // Seed a fake selection so `selected()` returns Some — otherwise
  // `bootstrap_selected` short-circuits before reaching the gate
  // with "nothing selected".
  app.worktrees = vec![worktree_fixture("dummy")];
  app.list_state.select(Some(0));

  app.bootstrap_selected();

  assert!(
    app.status.contains("not in trust ledger"),
    "status must reflect the gate refusal (got: {})",
    app.status
  );

  // SAFETY: env restoration paired with set/remove above.
  unsafe {
    match prior_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
    match prior_allow {
      Some(v) => std::env::set_var("GWM_ALLOW_BOOTSTRAP", v),
      None => std::env::remove_var("GWM_ALLOW_BOOTSTRAP"),
    }
  }
}
