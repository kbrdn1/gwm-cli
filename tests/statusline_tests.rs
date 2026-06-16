//! Unit tests for the pure statusline render core (issue #309). No socket,
//! no git: every input is a hand-built `JsonWorktree`, every output an
//! exact string. The socket transport is covered in
//! `daemon_integration.rs`; the NDJSON parsers in `daemon_tests.rs`.

use gwm::json_api::{JsonStatus, JsonWorktree};
use gwm::statusline::{active_index, render, render_for_cwd};
use std::path::Path;

/// Build a worktree with the given path / name / branch and a clean,
/// upstream-tracking status. Tests tweak the fields they care about.
fn wt(path: &str, name: &str, branch: Option<&str>) -> JsonWorktree {
  JsonWorktree {
    name: name.to_string(),
    id: name.to_string(),
    path: path.to_string(),
    branch: branch.map(String::from),
    head: Some("0".repeat(40)),
    is_main: branch == Some("main"),
    is_locked: false,
    is_prunable: false,
    status: JsonStatus {
      is_dirty: false,
      has_upstream: true,
      ahead: 0,
      behind: 0,
      unknown: false,
    },
    age_seconds: None,
    issue: None,
    pr: None,
  }
}

#[test]
fn empty_set_renders_blank() {
  assert_eq!(render(&[], None), "");
}

#[test]
fn single_clean_main_shows_branch_and_count() {
  let wts = vec![wt("/repo", "repo", Some("main"))];
  assert_eq!(render(&wts, Some(0)), "main · 1 wt");
}

#[test]
fn dirty_ahead_behind_issue_and_pr_are_rendered() {
  let mut active = wt(
    "/wt/feat",
    "feat-309-daemon-consumer",
    Some("feat/#309-daemon-consumer"),
  );
  active.status = JsonStatus {
    is_dirty: true,
    has_upstream: true,
    ahead: 2,
    behind: 1,
    unknown: false,
  };
  active.issue = Some(309);
  active.pr = Some(310);
  let wts = vec![
    wt("/repo", "repo", Some("main")),
    wt("/wt/a", "a", Some("feat/#1-a")),
    wt("/wt/b", "b", Some("feat/#2-b")),
    active,
  ];
  assert_eq!(
    render(&wts, Some(3)),
    "feat/#309-daemon-consumer · 4 wt · * ↑2 ↓1 · #309 · PR #310"
  );
}

#[test]
fn detached_head_falls_back_to_name() {
  let wts = vec![wt("/wt/x", "detached-wt", None)];
  assert_eq!(render(&wts, Some(0)), "detached-wt · 1 wt");
}

#[test]
fn no_active_worktree_shows_count_only() {
  let wts = vec![
    wt("/repo", "repo", Some("main")),
    wt("/wt/a", "a", Some("feat/#1-a")),
    wt("/wt/b", "b", Some("feat/#2-b")),
  ];
  assert_eq!(render(&wts, None), "3 wt");
}

#[test]
fn unknown_status_omits_local_state_flags() {
  let mut active = wt("/wt/x", "x", Some("feat/#9-x"));
  // Even with is_dirty set, an `unknown` status (detached / unborn) must
  // not emit `*`/arrows: ahead/behind/dirty are meaningless then.
  active.status = JsonStatus {
    is_dirty: true,
    has_upstream: false,
    ahead: 0,
    behind: 0,
    unknown: true,
  };
  let wts = vec![active];
  assert_eq!(render(&wts, Some(0)), "feat/#9-x · 1 wt");
}

#[test]
fn no_upstream_suppresses_ahead_behind_but_keeps_dirty() {
  let mut active = wt("/wt/x", "x", Some("feat/#9-x"));
  active.status = JsonStatus {
    is_dirty: true,
    has_upstream: false,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  let wts = vec![active];
  assert_eq!(render(&wts, Some(0)), "feat/#9-x · 1 wt · *");
}

#[test]
fn active_index_picks_the_enclosing_worktree() {
  let wts = vec![
    wt("/repo", "repo", Some("main")),
    wt("/repo/nested", "nested", Some("feat/#3-n")),
  ];
  // cwd inside the nested worktree resolves to the *longest* matching path.
  assert_eq!(active_index(&wts, Path::new("/repo/nested/src")), Some(1));
  // cwd in the outer repo (but not the nested dir) resolves to the outer.
  assert_eq!(active_index(&wts, Path::new("/repo/docs")), Some(0));
  // cwd outside every worktree resolves to None.
  assert_eq!(active_index(&wts, Path::new("/elsewhere")), None);
}

#[test]
fn render_for_cwd_resolves_then_renders() {
  let wts = vec![
    wt("/repo", "repo", Some("main")),
    wt("/wt/feat", "feat", Some("feat/#7-f")),
  ];
  assert_eq!(render_for_cwd(&wts, Path::new("/wt/feat")), "feat/#7-f · 2 wt");
  // Outside any worktree: count-only, still useful in a prompt.
  assert_eq!(render_for_cwd(&wts, Path::new("/tmp")), "2 wt");
}
