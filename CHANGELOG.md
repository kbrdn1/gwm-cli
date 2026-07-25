# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A CI checks overlay. `C` (rebindable `ci_checks`) — or `c` while the
  status pane holds the focus, the same contextual dispatch as the
  `j` / `k` sidebar scroll — lists every `statusCheckRollup` entry of
  the linked PR: one row per check with the state icon coloured like
  the sidebar CI indicator, `j` / `k` to move, `Enter` to open the
  check's details URL in the browser, `/` to filter, `f` to re-fetch
  the PR and refresh the rows in place, `Esc` to close
  (verbs rebindable under `[tui.keys.modal.ci_checks]`). Each row
  carries a right-aligned muted detail column with the owning workflow
  and the run duration (elapsed time with an ellipsis while the check
  is in flight), and the PR line's CI indicator ends with the resolved
  key that opens the overlay (`… CI passing 10/10 [c]`). The per-check
  name, URL, workflow and timestamps are now kept when the rollup is
  classified, additively on `PrStatus`. (#436)

- Responsive sidebar heights. The Agents, `Working Tree` and `Recent
  Commits` sections now share the column through a pure layout solver:
  natural heights while everything fits (`Recent Commits` absorbs the
  slack, as before), and on a short terminal every visible scrollable
  section keeps a guaranteed floor (7 lines for `Working Tree`, border
  plus 5 content rows; 5 lines for `Recent Commits`, border plus 3) with
  the remaining height split proportionally to content size — the
  non-scrollable Agents pane always keeps its full (bounded) height so
  its `+N more` indicator can never be clipped away. Empty sections
  keep their collapse behaviour; overflowing content stays reachable
  through the existing section scrolls, and an overflowing `Working
  Tree` paints a scrollbar on its inner right edge. (#438)
- The `Working Tree` pane scrolls. With the status pane focused, `J` / `K`
  (rebindable as `wt_scroll_down` / `wt_scroll_up` under `[tui.keys]`) move
  an independent scroll offset over the file tree, clamped against the
  viewport the layout actually granted the section — on a large change set
  the entries beyond the pane height were simply unreachable before. The
  offset resets on worktree navigation and on the commits ↔ stashes mode
  toggle, mirroring the Recent Commits scroll contract. (#437)

- A project logo. `docs/_assets/logo.svg` (dark) and
  `docs/_assets/logo-light.svg` (light) draw what the tool operates on —
  a trunk, a root node and two worktree nodes — on a 24 grid with square
  corners, in the palette the rest of the project already uses. It heads
  the README title through the same `<picture>` dark/light swap as the
  promo banner, and the banner now carries it beside the wordmark.

### Changed

- No workflow checkout persists the auto-injected token any more: the
  read-only checkouts of `ci.yml` (all six) and `pre-release.yml` (both) now
  set `persist-credentials: false`, extending the `release.yml` split from
  #429 to every workflow. The invariant is pinned per file by a YAML-parsing
  test, with a stricter rule than release.yml: these workflows never push, so
  no checkout may pass a token at all. (#433)

### Removed

- The `winget-publish` job. `WINGET_TOKEN` was never provisioned, so the
  guard step painted a red "publish kbrdn1.gwm to winget" job on every
  stable release run, and the channel is blocked upstream anyway: the
  initial manifest PR (microsoft/winget-pkgs#403295) sits on Needs-CLA and
  `komac update` can only update a package that already exists. winget
  joins the AUR, Nixpkgs and aqua as a channel fed by hand; the manual
  `komac update` recipe lives in CONTRIBUTING, and the job's absence is
  pinned by a test. (#448)

## Past releases

In reverse chronological order:

- [`1.3.0`](changelogs/1.3.0.md) — 2026-07-24
- [`1.2.0`](changelogs/1.2.0.md) — 2026-07-21
- [`1.1.1`](changelogs/1.1.1.md) — 2026-07-16
- [`1.1.0`](changelogs/1.1.0.md) — 2026-07-15
- [`1.0.3`](changelogs/1.0.3.md) — 2026-07-09
- [`1.0.2`](changelogs/1.0.2.md) — 2026-07-06
- [`1.0.1`](changelogs/1.0.1.md) — 2026-07-01
- [`1.0.0`](changelogs/1.0.0.md) — 2026-06-26
- [`0.9.0`](changelogs/0.9.0.md) — 2026-06-07
- [`0.8.0`](changelogs/0.8.0.md) — 2026-06-01
- [`0.7.0`](changelogs/0.7.0.md) — 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.10.0-rc.4`](changelogs/pre-releases/0.10.0-rc.4.md) — 2026-06-17
- [`0.10.0-rc.3`](changelogs/pre-releases/0.10.0-rc.3.md) — 2026-06-17
- [`0.10.0-rc.2`](changelogs/pre-releases/0.10.0-rc.2.md) — 2026-06-16
- [`0.10.0-rc.1`](changelogs/pre-releases/0.10.0-rc.1.md) — 2026-06-10
- [`0.9.0-rc.3`](changelogs/pre-releases/0.9.0-rc.3.md) — 2026-06-07
- [`0.9.0-rc.2`](changelogs/pre-releases/0.9.0-rc.2.md) — 2026-06-06
- [`0.9.0-rc.1`](changelogs/pre-releases/0.9.0-rc.1.md) — 2026-06-02
- [`0.8.0-rc.5`](changelogs/pre-releases/0.8.0-rc.5.md) — 2026-06-01
- [`0.8.0-rc.4`](changelogs/pre-releases/0.8.0-rc.4.md) — 2026-05-29
- [`0.8.0-rc.3`](changelogs/pre-releases/0.8.0-rc.3.md) — 2026-05-29
- [`0.8.0-rc.2`](changelogs/pre-releases/0.8.0-rc.2.md) — 2026-05-23
- [`0.8.0-rc.1`](changelogs/pre-releases/0.8.0-rc.1.md) — 2026-05-23
- [`0.7.0-rc.3`](changelogs/pre-releases/0.7.0-rc.3.md) — 2026-05-23
- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md) — 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md) — 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md) — 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
