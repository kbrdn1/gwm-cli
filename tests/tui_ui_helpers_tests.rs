//! Unit tests for pure layout helpers in `tui::ui` (issue #187 review
//! follow-up): the middle-ellipsizer that keeps a long path readable in
//! the confirm modal, and the badge-group width used to align the help
//! overlay's per-chord key badges.

use gwm::tui::{badge_group_width, ellipsize_middle, pane_counter, worktrees_pane_title};

#[test]
fn ellipsize_middle_returns_input_when_it_fits() {
  assert_eq!(ellipsize_middle("short", 10), "short");
  // Exactly `max` is left untouched (no ellipsis when nothing is cut).
  assert_eq!(ellipsize_middle("exactly10!", 10), "exactly10!");
}

#[test]
fn ellipsize_middle_keeps_head_and_tail_around_the_ellipsis() {
  // A long path keeps both its root and the worktree name at the end.
  let s = "/Users/kb/Projects/Flippad/worktrees/chore-880-drop-deprecated";
  let out = ellipsize_middle(s, 20);
  assert_eq!(out.chars().count(), 20, "must fit exactly within max");
  assert!(out.contains('…'), "must carry the middle ellipsis: {out}");
  assert!(out.starts_with("/Users"), "keeps the head: {out}");
  // Tail length is `max - 1 - ceil((max-1)/2)` chars, so a suffix that
  // fits inside it survives (the head/tail split is the contract, not a
  // specific cut point).
  assert!(out.ends_with("ecated"), "keeps the tail: {out}");
}

#[test]
fn ellipsize_middle_degrades_to_a_single_ellipsis_when_too_narrow() {
  assert_eq!(ellipsize_middle("anything", 1), "…");
  assert_eq!(ellipsize_middle("anything", 0), "…");
}

#[test]
fn ellipsize_middle_counts_chars_not_bytes() {
  // Multi-byte segments must not be sliced mid-codepoint, and the budget
  // is measured in chars.
  let s = "~/Projets/dépôt-très-long/branche-accentuée-éàü";
  let out = ellipsize_middle(s, 15);
  assert_eq!(out.chars().count(), 15);
  assert!(out.contains('…'));
}

#[test]
fn badge_group_width_single_chord_is_chord_plus_two_pad() {
  // ` q ` → 1 + 2.
  assert_eq!(badge_group_width("q"), 3);
  // ` Ctrl-C ` → 6 + 2.
  assert_eq!(badge_group_width("Ctrl-C"), 8);
}

#[test]
fn badge_group_width_splits_comma_chords_into_separate_badges() {
  // `j, Down` renders as `[ j ] [ Down ]`:
  //   ` j ` = 3, one separator space, ` Down ` = 6  → 10.
  assert_eq!(badge_group_width("j, Down"), 3 + 1 + 6);
  // `g g` is a *single* sequential chord (space inside, no comma) → one
  // badge ` g g ` = 5.
  assert_eq!(badge_group_width("g g"), 5);
}

#[test]
fn badge_group_width_unbound_renders_one_muted_badge() {
  let expected = "(unbound)".chars().count() + 2;
  assert_eq!(badge_group_width("(unbound)"), expected);
  assert_eq!(badge_group_width(""), expected);
}

// ---------------------------------------------------------------------------
// Worktrees pane title + counter (issue #217)
// ---------------------------------------------------------------------------

#[test]
fn worktrees_pane_title_unfiltered_shows_total_with_focus_index() {
  // No active filter → the `(N)` counter is the full worktree count, and the
  // pane carries the `[1]` focus mnemonic (focusable with the `1` key). The
  // casing is fixed to `Worktrees` (was lowercase `worktrees`).
  assert_eq!(worktrees_pane_title(true, 5, 5), " [1] Worktrees (5) ");
}

#[test]
fn worktrees_pane_title_filtered_shows_visible_over_total() {
  // Active filter → `(visible/total)` so the user sees how much the filter
  // narrowed the list.
  assert_eq!(worktrees_pane_title(false, 3, 5), " [1] Worktrees (3/5) ");
}

#[test]
fn pane_counter_is_blank_when_nothing_visible() {
  // Empty list → no `N of M` footer at all (mirrors the Recent Commits
  // section, which drops its counter when there is nothing to scroll).
  assert_eq!(pane_counter(0, 0), None);
}

#[test]
fn pane_counter_formats_selected_of_visible() {
  // Bottom-right footer of the worktrees pane, lazygit-style: the 1-based
  // selected position over the visible count (e.g. `3 of 12`).
  assert_eq!(pane_counter(3, 12).as_deref(), Some(" 3 of 12 "));
}
