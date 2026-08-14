# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The Settings panel sizes to its active tab**
  ([#569](https://github.com/kbrdn1/gwm-cli/issues/569)). #550 gave every
  bounded overlay one width policy; height stayed a flat percentage of the
  frame, so the panel took 60% of the terminal whether the tab under it had 3
  rows or 173. On a 40-row terminal that is a 24-row box for the Worktree tab's
  three fields, roughly six rows of it blank.

  The box is now the header, the body, the footer hint, the border and the
  padding, clamped between a floor of 11 rows (the shortest tab that carries a
  real form) and a ceiling that leaves about 25 rows of body. It therefore
  changes size as tabs are cycled, which is the deliberate trade: the tabs are
  genuinely different lengths, and the alternative was blank rows on three tabs
  out of five.

  The `?` overlay and the command palette keep their percentage. Measured, they
  carry about 220 rows and 52 commands, so sizing to content would resolve to
  the ceiling in every ordinary state and only ever engage on a heavily
  filtered palette, where it becomes a live resize while typing.

  A modal that outgrew a short terminal used to repaint it edge to edge, with
  no margin at all: the create form drew its border on the first and last rows
  of a 14-row terminal. Every content-sized overlay now keeps two rows above
  and below.

- **One spelling for a worktree path, everywhere it is printed in full**
  ([#568](https://github.com/kbrdn1/gwm-cli/issues/568)). The header rendered
  `$HOME` as `~` and the table printed the same value raw, so one path appeared
  twice on one screen in two spellings. The column paying for it is the one that
  can least afford it: `PATH` is `Fill(1)`, it takes whatever the other columns
  leave and by design vanishes first, and it spent 13 of its columns on the home
  directory on *every* row. Measured on the demo fixture at 103 columns the
  column gets about 22 cells, and all five rows read `/Users/kbrdn1/gwm-demo`,
  identical, hard-clipped mid-path with no ellipsis. They now start at
  `~/gwm-demo/`, so what survives the clip is the part that differs per row.

  Compression runs before the terminal sanitiser, not after, and that order is
  load-bearing: the prefix is matched byte for byte against `dirs::home_dir()`,
  so sanitising first would rewrite whatever `$HOME` itself carries and
  compression would silently stop firing for exactly the users whose home is
  hostile. The tail is still sanitised, so a worktree directory name cannot ride
  the tilde into the cell.

  The sidebar's `Path` row had the other half of the problem and now shares the
  same helper: it compressed but never sanitised. Nothing leaked, because
  ratatui drops a zero-width formatting character rather than painting it, but
  the row silently showed a path the filesystem does not have.

  `gwm path --format=json` is unaffected and stays absolute: this is a rendering
  change in the TUI only.

### Fixed

- **Tilde compression fires on Windows, and with a trailing separator on
  `$HOME`** ([#568](https://github.com/kbrdn1/gwm-cli/issues/568)). The home
  prefix was matched byte for byte, which failed in two ways that had been
  silent since the helper was written, so the header and the sidebar rendered
  absolute paths for the affected users rather than `~`.

  On Windows the two sources spell the same path differently: a worktree path
  comes from libgit2, which emits `/` there, while the home directory comes back
  with `\`, so `C:/Users/alice/repo` never matched `C:\Users\alice`. Separator
  spellings are now compared as equivalent, on Windows only, since a backslash
  is an ordinary character in a Unix directory name.

  `HOME=/home/alice/` is legal and is handed back with the separator intact,
  which left the match ending mid-boundary and refused. Trailing separators are
  now trimmed before the comparison.

## Past releases

In reverse chronological order:

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
