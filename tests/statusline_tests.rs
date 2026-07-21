//! Unit tests for the pure statusline render core (issue #309). No socket,
//! no git: every input is a hand-built `JsonWorktree`, every output an
//! exact string. The socket transport is covered in
//! `daemon_integration.rs`; the NDJSON parsers in `daemon_tests.rs`.

use gwm::json_api::{JsonStatus, JsonWorktree};
use gwm::statusline::{active_index, active_index_with, render, render_for_cwd};
use std::path::{Path, PathBuf};

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
    agents: None,
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
fn detached_head_literal_branch_falls_back_to_name() {
  // Real daemon data for a detached checkout carries branch: Some("HEAD")
  // (libgit2 `shorthand()` yields the literal "HEAD"), not None. The render
  // must treat it like the detached case and fall back to the worktree name
  // instead of printing the useless literal "HEAD · 1 wt".
  let wts = vec![wt("/wt/x", "detached-wt", Some("HEAD"))];
  assert_eq!(render(&wts, Some(0)), "detached-wt · 1 wt");
}

#[test]
fn watch_emits_a_trailing_blank_when_the_stream_ends() {
  // A --watch consumer must see a final blank render once the daemon stream
  // ends after pushing snapshots (daemon stopped / restarted), so a tail
  // clears the stale line instead of freezing on the last render (#309).
  let wts = vec![wt("/repo", "repo", Some("main"))];
  let mut emitted: Vec<usize> = Vec::new();
  gwm::statusline::watch(
    |cb| {
      cb(&wts); // one snapshot, then the stream ends cleanly (Ok)
      Ok(())
    },
    |worktrees| emitted.push(worktrees.len()),
  );
  assert_eq!(emitted, vec![1, 0], "one real snapshot (1) then the trailing blank (0)");
}

#[test]
fn watch_emits_a_blank_even_when_the_stream_never_connects() {
  // The error path (daemon unreachable, no snapshot ever) must also land on
  // the trailing blank — same graceful degradation as a clean close.
  let mut emitted: Vec<usize> = Vec::new();
  gwm::statusline::watch(
    |_cb| Err(gwm::error::GwmError::Other("no daemon".into())),
    |worktrees| emitted.push(worktrees.len()),
  );
  assert_eq!(emitted, vec![0], "only the trailing blank when nothing streamed");
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
fn active_index_with_canonicalizes_both_sides() {
  // The daemon reports a raw `/var/...` path (libgit2 keeps it as stored);
  // the shell's cwd resolves through the symlink to `/private/var/...`.
  // A naive `starts_with` that canonicalizes only one side misses the
  // match — the prompt would collapse to `N wt` from inside the worktree
  // (Codex review #311). Canonicalizing BOTH sides recovers it.
  let wts = vec![wt("/var/wt/feat", "feat", Some("feat/#9-f"))];

  // Simulate macOS `/var` -> `/private/var`.
  let canon = |p: &Path| -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix("/var/") {
      Some(rest) => PathBuf::from(format!("/private/var/{rest}")),
      None => p.to_path_buf(),
    }
  };

  // Identity matching (the old behaviour) misses the symlinked path.
  assert_eq!(active_index(&wts, Path::new("/private/var/wt/feat/src")), None);
  // Canonicalizing both the cwd and the worktree path makes it match.
  assert_eq!(
    active_index_with(&wts, Path::new("/private/var/wt/feat/src"), canon),
    Some(0)
  );
}

#[test]
fn active_index_with_identity_matches_active_index() {
  // `active_index` is just `active_index_with` under the identity
  // canonicaliser; the longest-enclosing-path rule must be preserved.
  let wts = vec![
    wt("/repo", "repo", Some("main")),
    wt("/repo/nested", "nested", Some("feat/#3-n")),
  ];
  let id = |p: &Path| p.to_path_buf();
  assert_eq!(
    active_index_with(&wts, Path::new("/repo/nested/src"), id),
    active_index(&wts, Path::new("/repo/nested/src"))
  );
  assert_eq!(active_index_with(&wts, Path::new("/repo/nested/src"), id), Some(1));
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

// -- Agent indicator (issue #408) ------------------------------------------

mod agent_indicator {
  use super::*;
  use gwm::json_api::{JsonAgentSession, JsonWorktreeAgents};

  fn agents(kind: &str, freshness: &str) -> JsonWorktreeAgents {
    let s = JsonAgentSession {
      kind: kind.into(),
      freshness: freshness.into(),
      last_activity: 1_784_480_000,
      id: "s1".into(),
    };
    JsonWorktreeAgents {
      top: s.clone(),
      sessions: vec![s],
    }
  }

  #[test]
  fn active_session_on_the_active_worktree_shows_a_compact_hint() {
    let mut w = wt("/wt/feat", "feat", Some("feat/#408-x"));
    w.agents = Some(agents("claude", "active"));
    assert_eq!(render(&[w], Some(0)), "feat/#408-x · 1 wt · claude");
  }

  #[test]
  fn idle_session_is_not_advertised() {
    // The statusline is the most compact surface — only a live pair is
    // worth a segment; idle leftovers stay in the TUI overlay.
    let mut w = wt("/wt/feat", "feat", Some("feat/#408-x"));
    w.agents = Some(agents("codex", "idle"));
    assert_eq!(render(&[w], Some(0)), "feat/#408-x · 1 wt");
  }

  #[test]
  fn no_agents_field_renders_byte_identical_to_before() {
    let w = wt("/wt/feat", "feat", Some("feat/#408-x"));
    assert_eq!(render(&[w], Some(0)), "feat/#408-x · 1 wt");
  }

  #[test]
  fn inactive_worktree_sessions_do_not_leak_into_the_line() {
    let mut other = wt("/wt/other", "other", Some("feat/#1-y"));
    other.agents = Some(agents("vibe", "active"));
    let active = wt("/wt/feat", "feat", Some("feat/#408-x"));
    assert_eq!(render(&[active, other], Some(0)), "feat/#408-x · 2 wt");
  }
}
