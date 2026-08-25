# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The rich PR / issue view (`I`) had its design pass** ([#551](https://github.com/kbrdn1/gwm-cli/issues/551)). It was
  built to get the data on screen and had never been laid out; the compact
  layout of 1.8 made its own density the next thing that read as unpolished.
  Six things changed:
  - **The issue and the PR are two tabs**, switched with `Tab`. The view
    still opens on the PR, which left the issue unreachable from a worktree
    in review. A PR landing while the view is open still replaces an issue
    that was only standing in for it, and does not replace one you tabbed
    to.
  - **Bodies render as Markdown** rather than as their source. Headings,
    emphasis, inline code, fenced blocks, lists, task lists, block quotes,
    GitHub alerts, links by their text, and HTML comments not shown at all.
  - **Nothing is capped.** The view scrolls, so the window is the terminal
    and the row count costs only the rows. Descriptions, reviews and the
    whole conversation render in full; a `… N more` row now only reports
    what the fetch itself did not return.
  - **The metadata block wears the Status pane's colours**, resolved through
    the pane's own helpers rather than a second set of rules.
  - **A width policy of its own**, 80% of the terminal capped at 120 columns
    against the shared overlay's 62% capped at 88. That ceiling was chosen
    for the clean report, whose rows stop earning columns; prose does not.
    A label-less row also spans the whole inner width now, which the view
    was paying for twice.
  - **Code and diff lines are kept whole and scroll sideways** with `h` /
    `l`. In YAML or Python the indentation is the program, and a wrapped
    `+` line's continuation carries no sigil and reads as context.
  - **`y` copies the active tab's URL, `Y` its description.**

### Added

- **Merge a PR from the TUI** ([#551](https://github.com/kbrdn1/gwm-cli/issues/551)).
  `M` from the worktree table merges the selected row's linked PR; `M` inside
  the PR / issue view merges the active tab's. Both go through the same
  confirmation the delete flow uses, whose summary names the PR, `head →
  base`, the resolved method and what it does to the history, and the CI
  rollup.

  The check state is shown rather than enforced: a forge refuses a merge for
  reasons gwm does not model, and its own error says which. **The source
  branch is never deleted**: neither backend is ever asked to.

  The method comes from the new `merge_method` key and defaults to `merge`,
  the least destructive of the three:

  ```toml
  merge_method = "merge"   # or "squash", "rebase"
  ```

## Past releases

In reverse chronological order:

- [`1.9.0`](changelogs/1.9.0.md), 2026-08-16
- [`1.8.0`](changelogs/1.8.0.md), 2026-08-13
- [`1.7.1`](changelogs/1.7.1.md), 2026-08-12
- [`1.7.0`](changelogs/1.7.0.md), 2026-08-12
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
