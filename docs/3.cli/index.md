---
title: CLI
description: The scriptable side of gwm, from the subcommand reference to the shell completions and the tmux / zellij / herdr integration.
navigation:
  title: CLI
---

# CLI

`gwm <subcommand>` is the scriptable side of gwm, designed to be safe in pipelines, shell completions, and pre-commit hooks. Every subcommand exits with a meaningful code (`0` ok, `1` warning, `2` failure) so you can wire `gwm doctor` into CI without parsing stdout.

- **[Reference](/cli/reference)**: every subcommand, exhaustive (`init`, `config`, `create`, `new`, `pr`, `review`, `list`, `path`, `cd`, `switch`, `bootstrap`, `sync`, `remove`, `prune`, `exec`, `clean`, `undo`, `history`, `link`, `unlink`, `open`, `status`, `labels`, `milestones`, `doctor`, `daemon`, `statusline`, `trust`, `types`, `commit-prefix`, `hooks`, `theme`, `tui keys`, `aliases`, `tmux`, `zellij`, `herdr`, `completions`, `shell-init`).
- **[Shell completions](/cli/completions)**: generate completion scripts for zsh / bash / fish / PowerShell / elvish, plus dynamic worktree-name completion via `gwm list --format=names`.
- **[Multiplexer integration](/cli/multiplexer)**: `gwm tmux` / `gwm zellij` / `gwm herdr` to open a worktree in a new tab or pane of the current session.

The CLI never spawns a TUI of its own: every subcommand is non-interactive and prints to stdout/stderr. The only TUI surface is bare `gwm` (or `gwm switch` in picker mode), covered in the [TUI section](/tui).
