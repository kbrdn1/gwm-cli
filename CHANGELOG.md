# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`gwm review <PR#>` — materialise an existing PR into a worktree** (#308).
  The inbound counterpart to `gwm create`: resolves the PR head via `gh` and
  fetches origin's universal `refs/pull/<N>/head` ref — cross-fork aware, and
  valid for PRs in any state (open / draft / closed / merged) — into a local
  `review/pr-<N>-<author>-<slug>` branch, attaches a worktree, links the PR so
  the sidebar / CI indicator light up immediately, and points the diff base at
  the PR's base ref (`origin/<base>`). Tear down with
  `gwm remove <dir> --delete-branch` like any worktree. `--name` overrides the
  branch. **Safe-by-default**: bootstrap and lifecycle hooks are *not* run,
  because a review worktree holds a contributor's (possibly fork) code and
  those steps execute commands against it (`npm install`, `composer install`,
  `direnv allow`, `post_create` hooks …) — i.e. arbitrary code; pass
  `--bootstrap` to opt in once you trust the PR. This closes the loop the
  v0.10.0-rc.2 CI indicator opened — a failing PR can now be pulled into a
  worktree to act on. (The branch-level `gwm checkout <remote-branch>` /
  `gwm create --from` primitives noted in #308 are deferred to a follow-up.)
- **First daemon consumer — `gwm statusline`** ([#309](https://github.com/kbrdn1/gwm-cli/issues/309)): the first real consumer of the `gwm daemon` JSON-RPC surface (#38). A thin, dependency-free client that connects to the daemon socket and renders a compact one-line worktree summary for tmux / starship / zsh prompts — active branch, worktree count, dirty / ahead / behind, linked issue / PR. `--watch` rides the `subscribe` stream and reprints on every `worktrees.changed` push; without it, a single `list` round-trip prints once and exits. When no daemon is reachable it prints an empty line and exits `0`, so a prompt substitution degrades to nothing. A CI rollup is intentionally omitted (not part of the daemon's stable schema). New `docs/5.integrations/4.daemon-consumers.md` (EN + FR) documents the statusline, the raw `socat` / `nc` protocol one-liner, and an editor recipe (Zed / VS Code). `JsonWorktree` / `JsonStatus` gained `Deserialize` so a client can decode the lines the server serialises.

## Past releases

In reverse chronological order:

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
