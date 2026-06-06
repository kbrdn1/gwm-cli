# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- TUI: direct pane-focus keys — `1` focuses the worktrees pane,
  `2` opens (if hidden) and focuses the status pane. Both are
  rebindable as `focus_worktrees` / `focus_status` under `[tui.keys]`;
  `Tab` keeps its toggle. (#217)
- TUI: contextual statusbar — a context chip (`worktrees` / `status` /
  `switch`) on the left, an animated loading spinner while a GitHub
  fetch is inflight, contextual hints in the middle, and the action log
  pinned right with absolute priority. (#217)
- TUI: a bottom-right `selected of visible` counter on the worktrees
  pane, lazygit-style. (#217)
- TUI: GitHub issue/PR status (`F`) is fetched off-thread, so the
  statusbar spinner animates during the `gh` shell-out instead of the
  event loop blocking. (#217)

### Changed

- TUI: the sidebar now defaults to the stacked layout (status pane under
  the worktrees table), and the split ratios are tuned per axis —
  42% / 58% stacked, 55% / 45% side-by-side. The orientation cycle
  (`V`) is unchanged. (#217)
- TUI: panes are labelled `[1] Worktrees (N)` and `[2] Status` (the focus
  mnemonics for the `1` / `2` keys), and the help overlay is titled
  `Keybindings` with a centred context subtitle in a distinct colour. Its
  section headers are Title-Cased (`Global`, `List View`, `Issue / PR`,
  `Create Form`, `Confirm Delete`), and the overlay now scrolls
  (`j`/`k`, `g`/`G`) when it outgrows the modal. (#217)
- TUI: modal overlays lift their title off the border into the frame as a
  centred line and gain interior padding on every side, so no content hugs
  the edge; the confirm-delete buttons render as flat coloured chips
  (` Confirm ` / ` Cancel `) instead of `[ Confirm ]` / `[ Cancel ]`. (#217)
- TUI: the create-worktree modal is reworked — the branch type is a
  horizontal `‹ name ›` selector (Left/Right or Up/Down) whose focused
  value reads as a highlighted chip, the issue and description fields are
  single-row inputs with a background surface (was 3-row bordered boxes)
  with vertical gaps between rows, the live branch/dir preview sits above
  the inputs, the modal width is capped so it doesn't span a wide
  terminal, and it grows a ` Create ` / ` Cancel ` button row plus a
  control hint. The branch type also cycles with `h` / `l` (vim), and the
  issue/description fields are length-capped so the resolved branch name
  stays within git's ref limit. (#217)
- TUI: the create-worktree modal opens focused on the Issue field rather
  than the cycle-only Type field, so the first keypress edits text
  instead of being a silent no-op (the branch type keeps its default and
  stays reachable via Shift-Tab). (#217)
- TUI: the link prompt's target step is reworked into a titled (`Link`)
  vertical selectable list — `j`/`k` (or arrows) move the highlight and
  `Enter` links the highlighted row, while `i`/`p` stay direct picks. The
  picker state machine and its key handler are extracted into a testable
  `App::handle_link_prompt_key` (mirroring the create modal). (#217)

### Fixed

- TUI: polish modal follow-ups after the statusbar pass — compact Link
  prompt with chip selection, clearer create Issue feedback, distinct
  Keybindings section colour, and aligned Confirm Delete details. (#220)
- TUI: the sidebar Issue/PR block prompted `press R to fetch status`,
  but `R` runs the review launcher — it now resolves the live
  `fetch_github` binding (`F` by default). (#217)

## Past releases

In reverse chronological order:

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
