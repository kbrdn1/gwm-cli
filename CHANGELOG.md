# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **TUI keymap redesign** (issue #290): unified, consistent key bindings across
  the worktree list. New actions and default bindings:
  - `p` → `pull` — git pull on the selected worktree's branch (async,
    progress shown in status bar).
  - `P` → `push` — git push on the selected worktree's branch (async).
  - `c` → `edit_worktree` — rename modal mirroring New Worktree (Type / Issue /
    Desc), pre-filled by parsing the current branch. Submitting renames the
    local branch (`git branch -m`), the remote branch when it exists on origin
    (`git push origin :<old> <new>:<new>` + upstream re-track), and moves the
    worktree directory on disk (`git worktree move`) so the slug stays in sync
    — all off-thread.
  - `e` → `exit_to_worktree` — quit TUI and print the selected path to stdout,
    enabling `cd "$(gwm)"` shell patterns.
  - `y` → `yank_branch_name` — copy the selected branch name to clipboard.
  - `w` → `yank_worktree_name` — copy the selected worktree name to clipboard.
  - `t` → `mux_pane` — open the selected worktree in a new tmux/zellij pane.
  - `h` / `H` → `macro_one` / `macro_two` — run user-configured commands from
    `[tui.macro1]` / `[tui.macro2]` in the project `.gwm.toml`.
  - `s` (was `S`) → `sync`; `D` (was `p`) → `toggle_delete_branch`;
    `l` → `lazygit_pty`; `r` → `review_pty`.
  - Sidebar keys: `V` → `toggle_sidebar` (show/hide), `S` → `toggle_sidebar_mode`
    (Commits ↔ Stashes), `Space` → `cycle_sidebar_layout` (auto / side-by-side /
    stacked), `v` → `toggle_sidebar_position` (left ↔ right).
  - Action slugs aligned: `lazygit_fullscreen`, `terminal_pty`,
    `terminal_fullscreen`, `review_fullscreen`, `yank_path`.
  - Pre-#290 `[tui.keys]` slugs (`git_tui`, `review`, `yank`, `open`,
    `open_menu`, …) still load via backward-compat aliases.
- **`[tui.macro1]` / `[tui.macro2]` config** (issue #290): user-defined
  commands launched from the TUI. Each entry accepts a `command` string and an
  optional `open_in` field (`"pty"` — default; `"mux_pane"` for a new
  tmux/zellij pane).

- **PTY overlay for lazygit and native terminal** (issue #35): press `l` to
  open lazygit inside a ~90 % fullscreen embedded PTY overlay; press `L` to
  open it fullscreen; `o` / `O` do the same for a native `$SHELL` session. Both
  overlays stay inside the TUI — no alternate screen swap. `Esc` closes the
  overlay; `q` inside lazygit quits lazygit and auto-closes. The keybindings
  (`lazygit_pty`, `lazygit_fullscreen`, `terminal_pty`, `terminal_fullscreen`)
  are fully rebindable in `[tui.keys]`. Powered by `portable-pty 0.9` +
  `tui-term 0.3` (`tui_term::vt100` — bundled vt100 0.16).

### Dependencies

- Added `portable-pty 0.9` (cross-platform PTY pair) and `tui-term 0.3`
  (ratatui widget rendering a vt100 parser buffer).

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
