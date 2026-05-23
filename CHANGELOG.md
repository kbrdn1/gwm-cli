# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- 🐛 **`GitHubFetch` cache keyed by issue/PR number + drops late results after `invalidate()`** ([#138](https://github.com/kbrdn1/gwm-cli/issues/138)). Closes two correctness bugs flagged by Copilot review on #137 (the #128 extraction PR), both in the new `GitHubFetch` sub-struct:
  - **Cache identity collision.** `is_cached` looked only at the per-target `issue_state` / `pr_state` enum, so any terminal variant for `Issue(_)` made every `Issue(*)` key falsely hit the cache. After `request(Issue(42)) → complete_issue(42, Ok(...))`, a subsequent `request(Issue(43))` wrongly returned `HitCache` — even though Issue 43 was never fetched. The dedupe contract promised "(target, number) is the identity" — it held on the inflight path but broke on the cache path.
  - **Late-result race with `invalidate()`.** `complete_issue` / `complete_pr` stamped results unconditionally, even when an intervening `invalidate()` had cleared the inflight slot. A shell-out that resolved after the user navigated away (and `invalidate()` cleared the slot) would stamp stale `Loaded(IssueStatus)` into the now-active worktree's cache.
  - Fix: replace the single `issue_state` / `pr_state` slots with `issue_cache: HashMap<u64, GitHubFetchState<IssueStatus>>` + `pr_cache: HashMap<u64, GitHubFetchState<PrStatus>>` keyed by number; `complete_*` early-returns when the inflight slot for the key is gone (the "still authoritative" check). `invalidate()` clears both maps AND the inflight set. New keyed accessors `GitHubFetch::issue_fetch_state(number)` / `pr_fetch_state(number)` return a `&'static GitHubFetchState::Idle` for absent keys (no per-call allocation). App-level `App::issue_fetch_state()` / `pr_fetch_state()` resolve via the current `link.issue` / `link.pr` so the renderer call sites in `tui/ui.rs` work unchanged.
  - Pinned by `tests/tui_state_github_fetch_tests.rs::{request_after_complete_for_different_number_returns_spawn_not_hit_cache, request_after_complete_for_different_pr_number_returns_spawn_not_hit_cache, complete_after_invalidate_drops_the_stale_result, complete_pr_after_invalidate_drops_the_stale_result}` — all four flipped RED → GREEN. No CLI surface or `.gwm.toml` schema change.

## Past releases

In reverse chronological order:

- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md) — 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md) — 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md) — 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
