---
title: TUI
description: The ratatui interface — keybindings, sidebar layout, configurable launchers, fuzzy filter, and confirm-overlay countdown.
navigation:
  title: TUI
---

# TUI

Bare `gwm` (no arguments) opens the ratatui interface on the current repo. From there you can create, delete, bootstrap, and jump between worktrees without leaving the terminal.

- **[Keybindings](/tui/keybindings)** — the full key map, including the v0.6 additions (`R`, `F`, `O`, `L`, `f`, `y`).
- **[Details sidebar](/tui/sidebar)** — the four-bordered subsections on the right pane, the lazygit-style commit graph, the live Issue / PR block.
- **[Fuzzy filter](/tui/filter)** — `/` opens the inline filter bar; nucleo-matcher under the hood.
- **[Confirm-overlay countdown](/tui/confirm-countdown)** — the safety countdown that prevents accidental branch deletions when `p` is armed.
- **[Configurable launchers](/tui/launchers)** — `[git_tui]` (`l`) and `[review]` (`R`), with `{base} {head} {path} {diff}` placeholders.
- **[Open dispatch](/tui/open-dispatch)** — `[tui.open]` controls what `o` does (`shell` / `editor` / `finder`).

`n` (new worktree) and `b` (re-bootstrap) are gated by the [TOFU trust ledger](/configuration/trust-ledger) — an untrusted `.gwm.toml` lands a refuse message in the status bar rather than running bootstrap. The picker variant (`gwm switch`, alias `gwm s`) reuses the same TUI but disables create / delete / bootstrap, then prints the chosen worktree's path on stdout — meant to be `eval`d by the `gcd` shell wrapper.
