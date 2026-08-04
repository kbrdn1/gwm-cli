# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Multi-row selection in the TUI, and a batch delete on top of it**
  ([#484](https://github.com/kbrdn1/gwm-cli/issues/484)). `Space` marks the
  highlighted worktree, `d` then deletes every marked row in one batch; with
  nothing marked it stays the single-row delete it has always been. Only `d`
  reads the mark set, so the worktrees footer carries the count
  (` 3 of 12 · 2 marked `) rather than letting a live selection go invisible
  under `b` / `s` / `p`. Marks are keyed by on-disk path, which is what makes
  them survive the fuzzy reranking and stay unambiguous in workspace mode,
  where two repos can hold the same worktree id. Opening the filter and the
  manual `f` refresh clear them; the background auto-refresh only prunes rows
  that no longer exist, otherwise a 60s timer would eat a selection still
  being built. The confirm overlay snapshots its targets when it opens, so a
  refresh landing during the safety countdown cannot retarget the deletion,
  and for a batch it reports the size and how many targets carry a branch
  instead of listing rows, with `D` arming the branch deletion batch-wide. A
  batch never stops at the first error: every target is attempted through its
  own repo handle and only after re-checking that its id still resolves to the
  path the overlay named (a worktree removed and recreated from another shell
  during the countdown gets the same id back, and removing by id alone would
  have deleted it), the confirm stays open narrowed to what failed (narrowed,
  never recomputed: `worktree::remove` prunes the admin entry before deleting
  the directory, so a removal that fails on the filesystem drops its own row,
  and recomputing would have fallen back to the cursor row), and the status
  line names the failures.
- **`gwm remove` takes several patterns**
  ([#484](https://github.com/kbrdn1/gwm-cli/issues/484)). `gwm remove a b c`
  removes the batch in one command and `--dry-run` prints one plan per
  pattern. Every pattern is resolved before anything is touched, so an unknown
  or ambiguous one fails the whole command with nothing removed, which is what
  `gwm list --format json | ... | xargs -n1 gwm remove` could not do: it
  removed the first half of the batch and then reported the typo. Patterns
  naming the same worktree collapse to a single removal.

### Changed

- **`cycle_sidebar_layout` moved from `Space` to `z`**
  ([#484](https://github.com/kbrdn1/gwm-cli/issues/484)), to make room for the
  row mark. Space-to-mark is the convention in lazygit, k9s and fzf, so the
  default was picked on merit rather than on which verb was there first. Both
  pre-#484 defaults are one `[tui.keys]` line away
  (`cycle_sidebar_layout = ["Space"]`, `toggle_select = ["z"]`), and
  `gwm tui keys` prints the resolved set with a per-row source. One upgrade
  note: `z` is now a shipped default, so a `.gwm.toml` that binds a chord
  *starting* with `z` (say `top = ["z z"]`) is a prefix conflict and is
  refused at load time, the same way any chord/prefix pair has always been.
  Rebind that chord, or move `cycle_sidebar_layout` elsewhere.

### Docs

- **Retired the em dash across the whole `docs/` tree**
  ([#516](https://github.com/kbrdn1/gwm-cli/issues/516)). 1586 occurrences in
  78 of the 79 pages, English and French, replaced by whatever connector the
  dash was standing in for: a colon where it introduced a list or an
  explanation, a full stop where it joined two independent clauses, a comma or
  parentheses around an aside. Fenced code blocks are untouched, since they
  reproduce shell comments and program output. Schema and reference tables
  used a bare dash as a cell value for two different things, "no default" and
  "this preset adds nothing here"; those now read `_(required)_` and
  `_(none)_`. 45 headings change shape, so their generated anchors change with
  them; none of them was the target of an internal link, and the 194 internal
  anchors in the tree resolve exactly as they did before.

## Past releases

In reverse chronological order:

- [`1.6.1`](changelogs/1.6.1.md), 2026-08-04
- [`1.6.0`](changelogs/1.6.0.md), 2026-08-03
- [`1.5.0`](changelogs/1.5.0.md), 2026-07-26
- [`1.4.0`](changelogs/1.4.0.md), 2026-07-25
- [`1.3.0`](changelogs/1.3.0.md), 2026-07-24
- [`1.2.0`](changelogs/1.2.0.md), 2026-07-21
- [`1.1.1`](changelogs/1.1.1.md), 2026-07-16
- [`1.1.0`](changelogs/1.1.0.md), 2026-07-15
- [`1.0.3`](changelogs/1.0.3.md), 2026-07-09
- [`1.0.2`](changelogs/1.0.2.md), 2026-07-06
- [`1.0.1`](changelogs/1.0.1.md), 2026-07-01
- [`1.0.0`](changelogs/1.0.0.md), 2026-06-26
- [`0.9.0`](changelogs/0.9.0.md), 2026-06-07
- [`0.8.0`](changelogs/0.8.0.md), 2026-06-01
- [`0.7.0`](changelogs/0.7.0.md), 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md), 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md), 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md), 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md), 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md), 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md), 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.10.0-rc.4`](changelogs/pre-releases/0.10.0-rc.4.md), 2026-06-17
- [`0.10.0-rc.3`](changelogs/pre-releases/0.10.0-rc.3.md), 2026-06-17
- [`0.10.0-rc.2`](changelogs/pre-releases/0.10.0-rc.2.md), 2026-06-16
- [`0.10.0-rc.1`](changelogs/pre-releases/0.10.0-rc.1.md), 2026-06-10
- [`0.9.0-rc.3`](changelogs/pre-releases/0.9.0-rc.3.md), 2026-06-07
- [`0.9.0-rc.2`](changelogs/pre-releases/0.9.0-rc.2.md), 2026-06-06
- [`0.9.0-rc.1`](changelogs/pre-releases/0.9.0-rc.1.md), 2026-06-02
- [`0.8.0-rc.5`](changelogs/pre-releases/0.8.0-rc.5.md), 2026-06-01
- [`0.8.0-rc.4`](changelogs/pre-releases/0.8.0-rc.4.md), 2026-05-29
- [`0.8.0-rc.3`](changelogs/pre-releases/0.8.0-rc.3.md), 2026-05-29
- [`0.8.0-rc.2`](changelogs/pre-releases/0.8.0-rc.2.md), 2026-05-23
- [`0.8.0-rc.1`](changelogs/pre-releases/0.8.0-rc.1.md), 2026-05-23
- [`0.7.0-rc.3`](changelogs/pre-releases/0.7.0-rc.3.md), 2026-05-23
- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md), 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md), 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md), 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md), 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md), 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md), 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md), 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md), 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md), 2026-05-18
