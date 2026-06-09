//! Theme colour audit (issue #170, follow-up to #33 / #168).
//!
//! The `[theme]` pipeline lands a resolved [`Theme`] on `App.theme`, but
//! before this audit several `draw_*` / render helpers still painted with
//! hard-coded `Color::*` literals, so a `[theme]` role override (`focus`,
//! `branch`, `dirty`, …) did not visually apply at those sites.
//!
//! These tests pin the **resolved style, not the literal**: each helper is
//! driven with a synthetic theme whose every role is a unique `Rgb` value,
//! and we assert the previously-hard-coded site now returns the matching
//! role colour. Because every role holds a distinct value, a regression
//! that re-hardcodes a literal — or wires the wrong role — fails here.
//!
//! The mapping rule under test (literal → role, preserving the pre-theme
//! default appearance since each role's default equals the old literal):
//!   green → `branch` (branch identity) / `clean` (ok/synced status)
//!   cyan  → `focus` (focus border) / `accent` (highlight, ahead)
//!   yellow→ `main` (trunk badge) / `dirty` (warning, hash, prompt)
//!   dark gray → `selection_bg` (selection background) / `muted` (dim text)
//!   magenta → `locked`   red → `prunable`
//! Since #210 the worktree-name chrome (`Color::White`) maps to the `name`
//! role and the table path column (`Color::Gray`, a brighter mid-grey than
//! the dark-gray `selection_bg` / `muted`) to the `path` role. The
//! remaining structural `Color::White` (help/step labels, the not-yet-fetched
//! dot) and `Color::Reset` (unlinked marker) still carry no semantic role.
//! Since #211 the git-status families have dedicated roles `staged`
//! (default cyan) / `modified` (yellow) / `untracked` (green) rather
//! than borrowing accent / dirty / clean.

mod common;

use common::init_repo;
use gwm::github::{BranchLink, IssueState, PrState};
use gwm::tui::commit_graph::{render_pipe_set, test_row, Pipe, PipeKind};
use gwm::tui::state::sidebar::SidebarMode;
use gwm::tui::theme::Theme;
use gwm::tui::{
  branch_name_color, branch_status_color, build_sidebar_sections, footer_line, format_status, freshness_color,
  header_line, help_label_style, issue_badge_color, palette_name_style, pr_badge_color, table_marker,
  working_tree_status_line, worktree_name_style, worktree_path_style,
};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use std::path::PathBuf;
use std::time::Duration;

/// A theme where every role holds a unique, recognisable `Rgb` value, so a
/// site reading the *wrong* role (or a re-hardcoded literal) is caught.
fn audit_theme() -> Theme {
  Theme {
    focus: Color::Rgb(1, 1, 1),
    accent: Color::Rgb(2, 2, 2),
    branch: Color::Rgb(3, 3, 3),
    clean: Color::Rgb(4, 4, 4),
    dirty: Color::Rgb(5, 5, 5),
    main: Color::Rgb(6, 6, 6),
    locked: Color::Rgb(7, 7, 7),
    prunable: Color::Rgb(8, 8, 8),
    muted: Color::Rgb(9, 9, 9),
    selection_bg: Color::Rgb(10, 10, 10),
    name: Color::Rgb(11, 11, 11),
    path: Color::Rgb(12, 12, 12),
    staged: Color::Rgb(13, 13, 13),
    modified: Color::Rgb(14, 14, 14),
    untracked: Color::Rgb(15, 15, 15),
  }
}

/// `fg` of the first span whose content contains `needle`.
fn fg_containing(line: &Line<'_>, needle: &str) -> Option<Color> {
  line
    .spans
    .iter()
    .find(|s| s.content.contains(needle))
    .and_then(|s| s.style.fg)
}

fn dirty_status() -> BranchStatus {
  BranchStatus {
    is_dirty: true,
    has_upstream: true,
    ahead: 0,
    behind: 0,
    unknown: false,
  }
}

// ---------------------------------------------------------------------------
// Pure colour helpers
// ---------------------------------------------------------------------------

#[test]
fn branch_name_color_resolves_every_state_through_theme_roles() {
  let t = audit_theme();

  let synced = BranchStatus {
    has_upstream: true,
    ..BranchStatus::default()
  };
  assert_eq!(branch_name_color(&synced, &t), t.branch, "synced → branch");

  assert_eq!(branch_name_color(&dirty_status(), &t), t.prunable, "dirty → prunable");

  let ahead = BranchStatus {
    has_upstream: true,
    ahead: 2,
    ..BranchStatus::default()
  };
  assert_eq!(branch_name_color(&ahead, &t), t.dirty, "ahead → dirty");

  let unpublished = BranchStatus::default(); // no upstream, not dirty, not unknown
  assert_eq!(branch_name_color(&unpublished, &t), t.locked, "no upstream → locked");

  let unknown = BranchStatus {
    unknown: true,
    ..BranchStatus::default()
  };
  assert_eq!(branch_name_color(&unknown, &t), t.muted, "unknown → muted");
}

/// Representative `BranchStatus` values spanning every colour branch of the
/// worst-status chain: unknown, dirty, behind-only, ahead-only, synced,
/// unpublished/clean.
fn status_color_cases() -> Vec<(&'static str, BranchStatus)> {
  vec![
    (
      "unknown",
      BranchStatus {
        unknown: true,
        ..BranchStatus::default()
      },
    ),
    ("dirty", dirty_status()),
    (
      "behind-only",
      BranchStatus {
        has_upstream: true,
        behind: 3,
        ..BranchStatus::default()
      },
    ),
    (
      "ahead-only",
      BranchStatus {
        has_upstream: true,
        ahead: 2,
        ..BranchStatus::default()
      },
    ),
    (
      "synced",
      BranchStatus {
        has_upstream: true,
        ..BranchStatus::default()
      },
    ),
    ("unpublished-clean", BranchStatus::default()),
  ]
}

#[test]
fn branch_status_color_resolves_worst_status_through_theme_roles() {
  let t = audit_theme();
  let role = |label: &str| match label {
    "unknown" => t.muted,
    "dirty" | "behind-only" => t.dirty,
    "ahead-only" => t.accent,
    _ => t.clean,
  };
  for (label, s) in status_color_cases() {
    assert_eq!(branch_status_color(&s, &t), role(label), "{label} → expected role");
  }
}

#[test]
fn format_status_colour_routes_through_branch_status_color() {
  // Issue #241: the table status cell (`format_status`) and the sidebar status
  // share one colour derivation. Pin the dedup so a future re-inline that
  // diverges from `branch_status_color` is caught.
  let t = audit_theme();
  for (label, s) in status_color_cases() {
    assert_eq!(
      format_status(&s, 16, &t).1,
      branch_status_color(&s, &t),
      "{label}: format_status colour must equal branch_status_color"
    );
  }
}

#[test]
fn freshness_color_resolves_through_theme_roles() {
  let t = audit_theme();
  assert_eq!(freshness_color(Duration::from_secs(0), &t), t.clean, "fresh → clean");
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 14), &t),
    t.dirty,
    "ageing → dirty"
  );
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 90), &t),
    t.muted,
    "stale → muted"
  );
}

#[test]
fn pr_badge_color_resolves_through_theme_roles() {
  let t = audit_theme();
  assert_eq!(pr_badge_color(PrState::Open, &t), t.clean, "open → clean");
  assert_eq!(pr_badge_color(PrState::Draft, &t), t.muted, "draft → muted");
  assert_eq!(pr_badge_color(PrState::Merged, &t), t.locked, "merged → locked");
  assert_eq!(pr_badge_color(PrState::Closed, &t), t.prunable, "closed → prunable");
}

#[test]
fn issue_badge_color_resolves_through_theme_roles() {
  let t = audit_theme();
  assert_eq!(issue_badge_color(IssueState::Open, &t), t.clean, "open → clean");
  assert_eq!(issue_badge_color(IssueState::Closed, &t), t.locked, "closed → locked");
}

#[test]
fn table_marker_resolves_through_theme_roles() {
  let t = audit_theme();

  // Main keeps its single `★` painted with the `main` role.
  let mut main = base_worktree("main");
  main.is_main = true;
  assert_eq!(
    table_marker(&main, &t).spans[0].style.fg,
    Some(t.main),
    "main marker → main role"
  );

  // Issue linked, PR empty: issue dot → `clean`, separator → `muted`, the
  // empty PR dot → `name` (the neutral white slot).
  let mut issue_only = base_worktree("issue");
  issue_only.link = BranchLink {
    issue: Some(7),
    ..BranchLink::empty()
  };
  let line = table_marker(&issue_only, &t);
  assert_eq!(line.spans[0].style.fg, Some(t.clean), "issue dot → clean role");
  assert_eq!(line.spans[1].style.fg, Some(t.muted), "separator → muted role");
  assert_eq!(line.spans[2].style.fg, Some(t.name), "empty pr dot → name role");

  // Once an issue status is loaded, the Issue dot follows the same state role
  // as the Issue/PR pane badge.
  let mut closed_issue = issue_only.clone();
  closed_issue.issue_state = Some(IssueState::Closed);
  let line = table_marker(&closed_issue, &t);
  assert_eq!(
    line.spans[0].style.fg,
    Some(issue_badge_color(IssueState::Closed, &t)),
    "closed issue dot → issue_badge_color closed role"
  );

  // PR linked, issue empty: mirror — empty issue dot → `name`, PR → `locked`.
  let mut pr_only = base_worktree("pr");
  pr_only.link = BranchLink {
    pr: Some(8),
    ..BranchLink::empty()
  };
  let line = table_marker(&pr_only, &t);
  assert_eq!(line.spans[0].style.fg, Some(t.name), "empty issue dot → name role");
  assert_eq!(line.spans[2].style.fg, Some(t.locked), "pr dot → locked role");

  // Nothing linked: two `name`-white slots.
  let unlinked = base_worktree("plain");
  let line = table_marker(&unlinked, &t);
  assert_eq!(line.spans[0].style.fg, Some(t.name), "empty issue dot → name role");
  assert_eq!(line.spans[2].style.fg, Some(t.name), "empty pr dot → name role");
}

// ---------------------------------------------------------------------------
// working_tree_status_line — #211: git-status families have dedicated roles
// ---------------------------------------------------------------------------

#[test]
fn working_tree_status_line_resolves_status_families_through_dedicated_roles() {
  let t = audit_theme();

  // The dedicated roles are distinct from the accent/dirty/clean they used
  // to borrow, so this fixture proves the families decoupled (a regression
  // back to the borrow would resolve to accent/dirty/clean and fail here).
  assert_ne!(t.staged, t.accent, "fixture: staged must differ from accent");
  assert_ne!(t.modified, t.dirty, "fixture: modified must differ from dirty");
  assert_ne!(t.untracked, t.clean, "fixture: untracked must differ from clean");

  // Staged change (X column set, Y blank) → `staged` on both the status
  // code column and the file name.
  let staged = working_tree_status_line("A  staged.rs", &t);
  assert_eq!(staged.spans[0].style.fg, Some(t.staged), "staged code → staged role");
  assert_eq!(
    fg_containing(&staged, "staged.rs"),
    Some(t.staged),
    "staged name → staged role"
  );

  // Worktree modification (Y column set) → `modified`.
  let modified = working_tree_status_line(" M tracked.rs", &t);
  assert_eq!(
    fg_containing(&modified, "tracked.rs"),
    Some(t.modified),
    "modified name → modified role"
  );

  // Untracked (`??`) → `untracked`.
  let untracked = working_tree_status_line("?? new.rs", &t);
  assert_eq!(
    fg_containing(&untracked, "new.rs"),
    Some(t.untracked),
    "untracked name → untracked role"
  );
}

#[test]
fn working_tree_status_families_default_to_legacy_cyan_yellow_green() {
  // Guard the "no visible change without [theme]" contract: with the
  // default theme the families keep their pre-#211 cyan/yellow/green.
  let d = Theme::default();
  assert_eq!(
    working_tree_status_line("A  s.rs", &d).spans[0].style.fg,
    Some(Color::Cyan),
    "default staged → Cyan"
  );
  assert_eq!(
    fg_containing(&working_tree_status_line(" M t.rs", &d), "t.rs"),
    Some(Color::Yellow),
    "default modified → Yellow"
  );
  assert_eq!(
    fg_containing(&working_tree_status_line("?? n.rs", &d), "n.rs"),
    Some(Color::Green),
    "default untracked → Green"
  );
}

#[test]
fn issue_summary_line_closed_badge_agrees_with_the_header_dot() {
  // Copilot review #209: the summary line must not paint a closed issue
  // with a different role than `issue_badge_color` (the sidebar dot).
  // Both resolve closed → `locked` ("moved on"), never `prunable`.
  let t = audit_theme();
  let status = gwm::github::IssueStatus {
    number: 42,
    title: "done".into(),
    state: IssueState::Closed,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let line = gwm::tui::issue_summary_line(
    42,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Loaded(status),
    80,
    &t,
  );
  assert_eq!(
    fg_containing(&line, "closed"),
    Some(t.locked),
    "closed issue badge → locked, matching issue_badge_color"
  );
  assert_eq!(
    fg_containing(&line, "closed"),
    Some(issue_badge_color(IssueState::Closed, &t)),
    "summary line and the header dot must use the same role for closed issues"
  );
}

#[test]
fn pr_summary_line_merged_badge_routes_through_pr_badge_color() {
  // #239 / Copilot review #209 follow-up: the PR Loaded arm used to inline
  // its own badge-colour map. After the dedup it must resolve through
  // `pr_badge_color`, exactly as the issue side calls `issue_badge_color`.
  // `audit_theme()` gives `locked` a value distinct from clean/muted/prunable
  // so a Merged PR's badge can only match if the route is honoured.
  let t = audit_theme();
  let status = gwm::github::PrStatus {
    number: 42,
    title: "shipped".into(),
    state: PrState::Merged,
    url: String::new(),
    checks_passed: 0,
    checks_total: 0,
    updated_at: String::new(),
  };
  let line = gwm::tui::pr_summary_line(
    42,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Loaded(status),
    80,
    &t,
  );
  assert_eq!(
    fg_containing(&line, "merged"),
    Some(t.locked),
    "merged PR badge → locked, matching pr_badge_color"
  );
  assert_eq!(
    fg_containing(&line, "merged"),
    Some(pr_badge_color(PrState::Merged, &t)),
    "summary line and the header dot must use the same role for merged PRs"
  );
}

// ---------------------------------------------------------------------------
// #210: `name` / `path` chrome roles
// ---------------------------------------------------------------------------

#[test]
fn name_and_path_styles_resolve_through_theme_roles() {
  // The worktree-name style (table name cell + sidebar header) reads the
  // `name` role and stays bold; the table path cell reads `path`.
  let t = audit_theme();
  let name = worktree_name_style(&t);
  assert_eq!(name.fg, Some(t.name), "worktree name → name role");
  assert!(
    name.add_modifier.contains(Modifier::BOLD),
    "the worktree name stays bold so it anchors each row"
  );
  assert_eq!(worktree_path_style(&t).fg, Some(t.path), "table path cell → path role");
}

#[test]
fn name_and_path_styles_default_to_legacy_white_and_gray() {
  // Guard the "no visible change without [theme]" contract for the new
  // chrome roles: the default theme keeps the pre-#170 White name / Gray
  // path literals.
  let d = Theme::default();
  assert_eq!(worktree_name_style(&d).fg, Some(Color::White), "default name → White");
  assert_eq!(worktree_path_style(&d).fg, Some(Color::Gray), "default path → Gray");
}

#[test]
fn palette_and_help_labels_resolve_through_name_role() {
  // #240(b): the command-palette non-highlighted command name and the
  // Keybindings-overlay entry label used to paint with a hard-coded
  // `Color::White` that bypassed `theme.name`, so a light / non-default
  // preset could not recolour them (#210). They now route through the
  // `name` role. `audit_theme()` gives `name` a value distinct from
  // White, so a regression back to the literal is caught here.
  let t = audit_theme();
  assert_eq!(
    palette_name_style(&t).fg,
    Some(t.name),
    "palette command name → name role"
  );
  assert_eq!(help_label_style(&t).fg, Some(t.name), "help entry label → name role");
}

#[test]
fn palette_and_help_labels_default_to_legacy_white() {
  // Guard the "no visible change without [theme]" contract for #240(b):
  // the default `name` role is White, so the default palette / help output
  // is byte-identical to the pre-#240 hard-coded literal.
  let d = Theme::default();
  assert_eq!(
    palette_name_style(&d).fg,
    Some(Color::White),
    "default palette command name → White"
  );
  assert_eq!(
    help_label_style(&d).fg,
    Some(Color::White),
    "default help entry label → White"
  );
}

#[test]
fn summary_line_heads_resolve_through_name_role() {
  // The `Issue #N` / `PR #N` heads are identity text, not a status signal,
  // so they paint with the `name` role (the badge keeps its state colour).
  let t = audit_theme();
  let issue = gwm::tui::issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Idle,
    80,
    &t,
  );
  assert_eq!(
    fg_containing(&issue, "Issue #7"),
    Some(t.name),
    "issue summary head → name role"
  );
  let pr = gwm::tui::pr_summary_line(
    9,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Idle,
    80,
    &t,
  );
  assert_eq!(
    fg_containing(&pr, "PR    #9"),
    Some(t.name),
    "pr summary head → name role"
  );
}

#[test]
fn summary_line_loaded_icons_resolve_through_state_roles() {
  let t = audit_theme();
  let issue_status = gwm::github::IssueStatus {
    number: 7,
    title: "closed".into(),
    state: IssueState::Closed,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let issue = gwm::tui::issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Loaded(issue_status),
    80,
    &t,
  );
  assert_eq!(
    issue.spans[0].style.fg,
    Some(issue_badge_color(IssueState::Closed, &t)),
    "loaded issue icon → issue state role"
  );

  let pr_status = gwm::github::PrStatus {
    number: 9,
    title: "merged".into(),
    state: PrState::Merged,
    url: String::new(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 0,
  };
  let pr = gwm::tui::pr_summary_line(
    9,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Loaded(pr_status),
    80,
    &t,
  );
  assert_eq!(
    pr.spans[0].style.fg,
    Some(pr_badge_color(PrState::Merged, &t)),
    "loaded PR icon → PR state role"
  );
}

#[test]
fn summary_line_non_loaded_icons_stay_muted() {
  let t = audit_theme();
  let issue = gwm::tui::issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Idle,
    80,
    &t,
  );
  assert_eq!(issue.spans[0].style.fg, Some(t.muted), "idle issue icon → muted");
  let pr = gwm::tui::pr_summary_line(
    9,
    gwm::github::LinkSource::Explicit,
    &gwm::tui::GitHubFetchState::Error("offline".into()),
    80,
    &t,
  );
  assert_eq!(pr.spans[0].style.fg, Some(t.muted), "error PR icon → muted");
}

// ---------------------------------------------------------------------------
// Header / footer chrome
// ---------------------------------------------------------------------------

#[test]
fn header_line_resolves_chrome_through_theme_roles() {
  let t = audit_theme();
  let line = header_line("repo", "/home/u/repo", true, 120, &t);
  assert_eq!(fg_containing(&line, "gwm "), Some(t.accent), "version chip → accent");
  assert_eq!(fg_containing(&line, "repo"), Some(t.name), "current-dir badge → name");
  assert_eq!(fg_containing(&line, "picker"), Some(t.dirty), "picker chip → dirty");
  assert_eq!(fg_containing(&line, "/home/u/repo"), Some(t.muted), "path → muted");
}

#[test]
fn footer_line_resolves_chrome_through_theme_roles() {
  let t = audit_theme();
  let hints = &[("n", "new")];
  let line = footer_line(hints, "opened foo", 120, &t);
  // Key chip → accent.
  assert_eq!(fg_containing(&line, "n"), Some(t.accent), "key chip → accent");
  // Dim hint label → muted.
  assert_eq!(fg_containing(&line, "new"), Some(t.muted), "hint label → muted");
  // Status log → dirty.
  assert_eq!(fg_containing(&line, "opened foo"), Some(t.dirty), "status → dirty");
}

// ---------------------------------------------------------------------------
// Commit graph
// ---------------------------------------------------------------------------

#[test]
fn commit_graph_node_and_connectors_resolve_through_branch_role() {
  let t = audit_theme();
  let from = test_row("a", &[]);
  let to = test_row("b", &[]);
  let pipes = vec![Pipe {
    from_pos: 0,
    to_pos: 0,
    from_hash: from.hash,
    to_hash: to.hash,
    kind: PipeKind::Starts,
  }];
  let spans = render_pipe_set(&pipes, &t);
  assert!(
    spans.iter().any(|s| s.style.fg == Some(t.branch)),
    "graph spans must paint with the branch role, got: {:?}",
    spans
      .iter()
      .map(|s| (s.content.as_ref(), s.style.fg))
      .collect::<Vec<_>>()
  );
  assert!(
    spans.iter().all(|s| s.style.fg == Some(t.branch)),
    "every graph span (node + connectors) must use the branch role"
  );
}

// ---------------------------------------------------------------------------
// Sidebar identity card (build_sidebar_sections) — badges + branch line
// ---------------------------------------------------------------------------

#[test]
fn sidebar_identity_badges_resolve_through_theme_roles() {
  let t = audit_theme();
  let mut w = base_worktree("feat-170");
  w.is_main = true;
  w.is_locked = true;
  w.is_prunable = true;
  w.status = dirty_status();

  let sections = build_sidebar_sections(&w, SidebarMode::Commits, &t);

  // The badges_line lives in the worktree identity section. Flatten its
  // spans and assert each flag badge wears its role colour.
  let badge_fg = |needle: &str| -> Option<Color> {
    sections
      .worktree
      .iter()
      .flat_map(|l| l.spans.iter())
      .find(|s| s.content.contains(needle))
      .and_then(|s| s.style.fg)
  };

  assert_eq!(badge_fg("★ main"), Some(t.main), "★ main badge → main role");
  assert_eq!(badge_fg("🔒 locked"), Some(t.locked), "locked badge → locked role");
  assert_eq!(
    badge_fg("⚠ prunable"),
    Some(t.prunable),
    "prunable badge → prunable role"
  );

  // The dirty branch line uses branch_name_color → prunable for a dirty tree.
  assert_eq!(
    badge_fg(w.branch.as_deref().unwrap()),
    Some(t.prunable),
    "dirty branch name → prunable role"
  );
}

#[test]
fn sidebar_identity_default_theme_preserves_legacy_palette() {
  // Guard the "no visible change for users who omit [theme]" contract:
  // with the default theme the badges keep their pre-#170 literals.
  let d = Theme::default();
  let mut w = base_worktree("feat-170");
  w.is_main = true;
  w.is_locked = true;
  w.is_prunable = true;
  let sections = build_sidebar_sections(&w, SidebarMode::Commits, &d);
  let badge_fg = |needle: &str| -> Option<Color> {
    sections
      .worktree
      .iter()
      .flat_map(|l| l.spans.iter())
      .find(|s| s.content.contains(needle))
      .and_then(|s| s.style.fg)
  };
  assert_eq!(badge_fg("★ main"), Some(Color::Yellow));
  assert_eq!(badge_fg("🔒 locked"), Some(Color::Magenta));
  assert_eq!(badge_fg("⚠ prunable"), Some(Color::Red));
}

// ---------------------------------------------------------------------------
// End-to-end: a `[theme]` override in `.gwm.toml` reaches a render helper
// ---------------------------------------------------------------------------

#[test]
fn gwm_toml_role_override_reaches_a_render_helper() {
  // Closes the loop the integration test (#33) left open: a per-role
  // override in `.gwm.toml` must actually change a previously-hard-coded
  // render site, not just `App.theme`.
  let (dir, _) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r##"
[theme]
branch = "#abcdef"
"##,
  )
  .unwrap();
  let app = gwm::tui::App::new_at_layered(Some(dir.path()), None).unwrap();
  let expected = Color::Rgb(0xab, 0xcd, 0xef);
  assert_eq!(app.theme.branch, expected, "override must land on App.theme");

  let synced = BranchStatus {
    has_upstream: true,
    ..BranchStatus::default()
  };
  assert_eq!(
    branch_name_color(&synced, &app.theme),
    expected,
    "the `branch` override must reach the branch-name render helper"
  );
}

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

fn base_worktree(name: &str) -> WorktreeInfo {
  WorktreeInfo {
    name: name.into(),
    path: PathBuf::from(format!("/tmp/gwm-theme-audit/{}", name)),
    branch: Some(format!("feat/#170-{}", name)),
    head: Some("0123456789abcdef0123456789abcdef01234567".into()),
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: BranchLink::empty(),
    issue_state: None,
    age: Some(Duration::from_secs(3600)),
  }
}
