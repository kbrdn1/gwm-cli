# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **One width policy for every modal** ([#550](https://github.com/kbrdn1/gwm-cli/issues/550)).
  Overlays used to size themselves with four different rules, two of which
  branched on `term_width <= 80` to spend a bigger percentage on a small
  terminal. Width was therefore **not monotonic**: dragging a pane past 80
  columns made the link prompt 16 columns *narrower* and the exec / clean /
  detail overlay 22. Four others sized on a bare percentage with no ceiling, so
  on a 200-column terminal the delete confirmation reached 124 columns for a
  four-row detail grid, and the help, config and command-palette overlays 120.

  Every bounded overlay now resolves its width through a single
  `modal_width(term_width, pct, min, max)`: its own knobs, one rule. Never
  narrower as the terminal widens, never past its ceiling, always two columns
  of margin per side. The floor also makes 80 columns, the width the docs
  advertise, a size the surfaces were actually sized for. The confirm modal was
  49 columns wide there and its hint row read `Enter activa`, cut mid-word with
  no ellipsis and the `n cancel` hint entirely off screen.

  The PTY overlay, the command-log transcript, the note editor and the
  bootstrap report keep spending a percentage of the frame: they are text
  canvases, and the width is content. `render_section` hard-clips by design
  (one logical row, one visual row), so on those surfaces every column the
  frame gives up is a column of a hook's error nobody can reach.

- **The TUI is compact by default** ([#545](https://github.com/kbrdn1/gwm-cli/issues/545)).
  Panes and sidebar sections no longer draw box rules; each is delimited by a
  filled one-line header instead, which buys back two rows and two columns per
  section. The title keeps its bracketed keybinding and goes uppercase
  (`[1] WORKTREES`, `ISSUE / PR [F]`), the counter moves to the right of that
  same line rather than into a bottom rule, a muted rule marks the boundary
  between the two panes, and the worktrees pane sizes itself to its row count
  instead of reserving its share of the stacked split. Focus reads on the
  header — the active pane takes the `focus` role and the `selection_bg` fill.
  Modal titles moved into the top rule as part of the same pass
  ([#549](https://github.com/kbrdn1/gwm-cli/issues/549)), two rows back per
  overlay.

  `[tui] layout = "bordered"` restores the previous lazygit-style boxes. That
  mode is left untouched by the compact refinements (no dimming, no separator
  rule) so it stays a faithful restore rather than a third look. Overlays and
  modals keep their border under either value.

### Added

- **`[tui] status_one_line`** ([#547](https://github.com/kbrdn1/gwm-cli/issues/547)):
  folds the sidebar's Status block onto a single row — branch, head, state
  badges, diff and age joined by ` · ` — where it used to spend one labelled
  row per value. **On by default**, which frees three rows for the panes below;
  `status_one_line = false` restores the labelled block.

  A knob rather than a compact-mode behaviour, so it applies under `bordered`
  too. The `Path` row never folds in: a path is the one value long enough that
  sharing a row would clip both halves. Segment order is the width policy — the
  sidebar does not wrap, so a row wider than the pane is clipped on the right,
  and the fold puts identity first and the age last. Reachable from the
  Settings panel's **TUI** tab, where the edit now drops the sidebar cache so
  the new shape is visible immediately (a live theme change had the same
  staleness and is fixed with it).

- **`[tui] dim_unfocused`** ([#545](https://github.com/kbrdn1/gwm-cli/issues/545)):
  dims the body of whichever pane does not hold focus, in either layout. Off by
  default — it trades contrast for a stronger focus cue, and the inactive
  pane's content is still readable information. Uses the terminal's `DIM`
  attribute, so semantic colours survive.
- **`section_bg` theme role** ([#545](https://github.com/kbrdn1/gwm-cli/issues/545)):
  the compact header fill. An indexed colour rather than a translucent white,
  so the mode stays readable on a terminal without truecolor; each preset takes
  the tone its own palette reserves for chrome bands, and keeps it distinct
  from `selection_bg` — which is also what separates a focused header from an
  unfocused one.

### Fixed

- **A path of wide glyphs is ellipsized by the room it actually takes**
  ([#554](https://github.com/kbrdn1/gwm-cli/issues/554)). `ellipsize_middle`
  counted characters while all eleven of its callers hand it a budget in
  terminal cells, the width of a ratatui rect. For anything but narrow Latin
  the two disagree: a path of 40 CJK glyphs is 40 characters and 80 columns, so
  the helper called it short and returned it whole. In the delete confirmation
  the row then overflowed its frame and wrapped, dropping the path off its
  `Path` label and breaking the aligned grid; in a table cell ratatui simply
  clipped the tail, which is the half a middle ellipsis exists to keep.

  The budget, the head/tail split and the padding that fills a picker or
  reclaim row are all measured in cells now, and the measure is ratatui's own
  `CellWidth` summed per extended grapheme, which is exactly what
  `Buffer::set_stringn` walks. Anything less agrees with the renderer on CJK
  and disagrees elsewhere: `unicode-width` reads `لالالا` as 3 columns where 6
  get painted, because lam-alef counts as a ligature, and `ｶﾞｶﾞｶﾞ` as 3 where 6
  get painted, because a halfwidth dakuten is `Grapheme_Extend` yet terminals
  give it a cell. Walking graphemes also puts the cut where a glyph ends
  rather than between a base and its combining mark, and skips control
  characters the way the renderer does. A glyph that would straddle the last
  column is dropped whole rather than half-drawn, so a result is at most the
  budget rather than exactly it. `unicode-segmentation`, already in the tree
  through crossterm, is now a direct dependency.

- **The create and rename forms keep their focused field on screen**
  ([#553](https://github.com/kbrdn1/gwm-cli/issues/553)). Both modals size
  themselves to their content, and a content box taller than the terminal is
  clamped to the frame: ratatui then cut the tail off with no indicator. The
  rename form wants 18 rows, so a 16-row terminal lost its `Desc` row — the
  field the modal opens focused on. It stayed reachable by `Tab` and typed into
  blind; free-form mode lost its only input the same way at 12 rows.

  The body now scrolls to whichever field has focus, derived from the focus
  itself rather than kept as scroll state — the forms have no scroll cursor,
  focus is the only thing that moves. A form that fits renders exactly as
  before; one that does not gets the Settings panel's scrollbar, so the
  overflow is visible instead of silent.

- **Returning from a fullscreen surface no longer ends the session when the
  terminal is slow to answer** ([#548](https://github.com/kbrdn1/gwm-cli/issues/548)).
  Coming back from the PTY overlay, an `exec` run or a review launch, gwm could
  exit with `error: io error: The cursor position could not be read within a
  normal duration`. That message is crossterm giving up on a DSR report:
  `Terminal::clear` snapshots the cursor with `ESC [ 6 n` before wiping the
  screen, and the return path from a fullscreen child is exactly when the
  terminal is least likely to answer in time.

  The three call sites now clear without asking. The snapshot was dead weight
  here — each one repaints the whole frame on the next loop iteration, so the
  position it restored was overwritten before anyone could see it. What the
  callers needed was the other half of `clear`, wiping the screen *and*
  resetting the back buffer so the next `draw` is a full repaint rather than a
  diff against stale content; a fix that only wiped would have traded the crash
  for a blank TUI.

## Past releases

In reverse chronological order:

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
