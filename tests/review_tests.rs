//! Unit tests for the pure naming helpers behind `gwm review` (issue #308).
//! These pin the `review/pr-<N>-<author>-<slug>` branch / dir contract
//! without touching git or `gh`.

use gwm::review::{dirname_from_branch, head_slug, review_branch_name, review_dirname};

#[test]
fn head_slug_keeps_last_segment_kebabed() {
  assert_eq!(head_slug("alice/feat/Spike X"), "spike-x");
  assert_eq!(head_slug("fix-typo"), "fix-typo");
  assert_eq!(head_slug("feature/ADD_Thing"), "add-thing");
}

#[test]
fn review_branch_name_is_review_pr_author_slug() {
  assert_eq!(
    review_branch_name(312, "alice", "spike-x"),
    "review/pr-312-alice-spike-x"
  );
}

#[test]
fn review_dirname_mirrors_branch_tail_with_dashes() {
  assert_eq!(review_dirname(312, "alice", "spike-x"), "review-pr-312-alice-spike-x");
}

#[test]
fn naming_kebabs_a_bot_author() {
  // `dependabot[bot]` would carry git-ref-illegal characters; kebab
  // neutralises the brackets so the branch / dir stay valid.
  assert_eq!(
    review_branch_name(7, "dependabot[bot]", "bump-serde"),
    "review/pr-7-dependabot-bot-bump-serde"
  );
  assert_eq!(
    review_dirname(7, "dependabot[bot]", "bump-serde"),
    "review-pr-7-dependabot-bot-bump-serde"
  );
}

#[test]
fn naming_skips_empty_author_and_slug_segments() {
  // No author and no slug must not leave a dangling `--` run.
  assert_eq!(review_branch_name(9, "", ""), "review/pr-9");
  assert_eq!(review_dirname(9, "", ""), "review-pr-9");
  // Slug present, author missing.
  assert_eq!(review_branch_name(9, "", "fix"), "review/pr-9-fix");
}

#[test]
fn dirname_from_branch_flattens_slashes() {
  assert_eq!(dirname_from_branch("review/pr-9-x"), "review-pr-9-x");
  assert_eq!(dirname_from_branch("my-flat-name"), "my-flat-name");
}
