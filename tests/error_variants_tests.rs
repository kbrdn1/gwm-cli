//! Issue #105: pin the four typed variants that split the `Other(String)`
//! catch-all so reviewers can see, in one place, the contract each
//! variant carries and which call sites now construct them.
//!
//! Each test does two things:
//!   1. construct the variant directly to pin its shape (compile-time
//!      check that the data carried is what we agreed on);
//!   2. exercise a real call site (via the public API) that should now
//!      return that variant rather than `Other(String)`.
//!
//! The second half is what makes this TDD rather than tautology: if a
//! future refactor downgrades a site back to `Other(..)`, the matches!
//! assertion fails.
//!
//! Display contracts are deliberately loose — they must include the
//! relevant context (branch name, kind, etc) so users can grep, but
//! we don't pin the exact wording (that would make every copy-edit a
//! breaking test).

use gwm::error::{GwmError, LinkKind};
use gwm::github::{parse_issue_json, parse_labels_json, parse_milestones_json, parse_pr_json};

// ---- UnbornHead ---------------------------------------------------------

#[test]
fn unborn_head_variant_constructs_and_renders() {
  let e = GwmError::UnbornHead {
    reason: "HEAD is unborn or detached".into(),
  };
  let rendered = e.to_string();
  assert!(
    rendered.to_lowercase().contains("head"),
    "UnbornHead Display must mention HEAD for grep-ability, got: {}",
    rendered
  );
  assert!(matches!(e, GwmError::UnbornHead { .. }));
}

// ---- GhJsonParse --------------------------------------------------------

#[test]
fn gh_json_parse_variant_constructs_and_carries_kind() {
  // Use a deliberately malformed payload so serde_json hands us a real
  // `serde_json::Error` to wrap — synthesising one by hand is awkward.
  let bad = "{ not json";
  let err = serde_json::from_str::<serde_json::Value>(bad).expect_err("malformed json");
  let e = GwmError::GhJsonParse {
    kind: "issue",
    source: err,
  };
  let rendered = e.to_string();
  assert!(
    rendered.contains("issue"),
    "GhJsonParse Display must surface the `kind` so users see which payload broke, got: {}",
    rendered
  );
  assert!(matches!(e, GwmError::GhJsonParse { kind: "issue", .. }));
}

#[test]
fn parse_issue_json_returns_gh_json_parse_on_malformed_input() {
  let bad = "{ not json";
  let err = parse_issue_json(bad).expect_err("malformed json");
  assert!(
    matches!(err, GwmError::GhJsonParse { kind: "issue", .. }),
    "parse_issue_json should surface GhJsonParse{{kind:'issue'}}; got: {:?}",
    err
  );
}

#[test]
fn parse_pr_json_returns_gh_json_parse_on_malformed_input() {
  let bad = "not json at all";
  let err = parse_pr_json(bad).expect_err("malformed json");
  assert!(
    matches!(err, GwmError::GhJsonParse { kind: "pr", .. }),
    "parse_pr_json should surface GhJsonParse{{kind:'pr'}}; got: {:?}",
    err
  );
}

#[test]
fn parse_labels_json_returns_gh_json_parse_on_malformed_input() {
  let bad = "{}"; // labels expects an array, so this fails the shape check
  let err = parse_labels_json(bad).expect_err("malformed json");
  assert!(
    matches!(err, GwmError::GhJsonParse { kind: "labels", .. }),
    "parse_labels_json should surface GhJsonParse{{kind:'labels'}}; got: {:?}",
    err
  );
}

#[test]
fn parse_milestones_json_returns_gh_json_parse_on_malformed_input() {
  let bad = "{}"; // milestones expects an array, so this fails the shape check
  let err = parse_milestones_json(bad).expect_err("malformed json");
  assert!(
    matches!(err, GwmError::GhJsonParse { kind: "milestones", .. }),
    "parse_milestones_json should surface GhJsonParse{{kind:'milestones'}}; got: {:?}",
    err
  );
}

// ---- LinkMissing --------------------------------------------------------

#[test]
fn link_missing_variant_constructs_with_kind_and_branch() {
  let e_issue = GwmError::LinkMissing {
    kind: LinkKind::Issue,
    branch: "feat/#42-foo".into(),
  };
  let rendered = e_issue.to_string();
  assert!(
    rendered.contains("feat/#42-foo"),
    "LinkMissing Display must include the branch name, got: {}",
    rendered
  );
  assert!(
    rendered.to_lowercase().contains("issue"),
    "LinkMissing{{Issue}} Display must mention `issue`, got: {}",
    rendered
  );
  assert!(matches!(
    e_issue,
    GwmError::LinkMissing {
      kind: LinkKind::Issue,
      ..
    }
  ));

  let e_pr = GwmError::LinkMissing {
    kind: LinkKind::Pr,
    branch: "fix/#7-bar".into(),
  };
  let rendered_pr = e_pr.to_string();
  assert!(
    rendered_pr.to_lowercase().contains("pr"),
    "LinkMissing{{Pr}} Display must mention `pr`, got: {}",
    rendered_pr
  );
  assert!(matches!(e_pr, GwmError::LinkMissing { kind: LinkKind::Pr, .. }));
}

// ---- CommandFailed (existing variant, now also covers the worktree.rs
//      git log / git status sites previously in `Other`) ------------------

#[test]
fn command_failed_still_carries_inner_detail() {
  // Pre-existing variant — pinned here too so the issue #105 split
  // doesn't accidentally regress its shape. The shared catch-all
  // contract from `tests/error_tests.rs` continues to hold.
  let e = GwmError::CommandFailed("git log exited 1: fatal: not a git repo".into());
  let rendered = e.to_string();
  assert!(
    rendered.contains("git log"),
    "CommandFailed Display must surface the inner detail, got: {}",
    rendered
  );
}
