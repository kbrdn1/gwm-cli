# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Agent session pane** (#408). gwm now detects AI-agent coding sessions
  (Claude Code, Codex, opencode, Mistral Vibe) per worktree by reading each
  tool's on-disk session artefacts — no process scanning, `std::fs` only, so
  detection behaves identically on Linux, macOS and Windows. Surfaced as:
  an **AGENT** column in the worktree table (most recently active agent,
  coloured by freshness), an `Agent:` summary line in the sidebar's Worktree
  block, a generic detail overlay on `a` (rebindable; scroll/close keys under
  `[tui.keys.modal.detail]`) listing every matched session, an **additive,
  experimental-tier** `agents` field on the `list --format=json` / daemon
  row (`SCHEMA_VERSION` stays 1; omitted when no session matched), and an
  active-agent segment in `gwm statusline`. Detection runs off-thread with a
  30 s re-check and a 30-day artefact scan window; missing or malformed
  artefact stores degrade silently to "no sessions".
- **`gwm agents` + manual pinning** (#408). A dedicated CLI surface for the
  same detection: `gwm agents` lists sessions per worktree (human or
  `--format=json`), the plain `gwm list` table gains an AGENT column, and
  `gwm agents attach <worktree> <session-id>` / `detach` pin a session to a
  worktree when the recorded directory is not enough — auto-detection stays
  the default, the pin (git branch config `gwm-agent-pin`, one per worktree)
  only adds, and every surface honours it. `GWM_AGENTS_HOME` overrides the
  scanned home for deterministic tests/CI. Sessions carry a human-readable
  **name** when their artefacts have one (first prompt, or Vibe's title) —
  shown in the overlay and `gwm agents`, and exposed as an optional `name`
  on the wire. The TUI overlay is interactive: `j`/`k` select a session
  (highlight + scrollbar, stable frame), `a` pins it, `d` unpins. Daemon
  detection is cached for 30 s per poll loop; a live session always beats a
  freshly-ended one for the compact indicator.

### Removed

- The `aur-publish` job. `gwm-cli-bin` is maintained on the AUR by a third
  party, so the job never had push rights: being advisory, it failed silently
  on every stable tag while the release reported success. The AUR is now fed
  by hand, like Nixpkgs and aqua. The `PKGBUILD` template, its render script
  and their tests are unchanged, so the handover stays a one-liner. (#430)

### Changed

- The release workflow's read-only checkouts no longer persist the
  auto-injected token in `.git/config`. Only the two checkouts that push
  (the Homebrew tap and the Scoop bucket) keep a credential, and both sides
  of that split are pinned by a test. (#429)

### Fixed

- `gwm clean --yes` no longer fails wholesale when a concurrent writer (a
  watcher such as rust-analyzer regenerating files inside `target/`) races
  the removal: a transient ENOTEMPTY is retried with a bounded budget, and a
  directory already reclaimed by someone else counts as success instead of
  aborting the command. (#440)

## Past releases

In reverse chronological order:

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
