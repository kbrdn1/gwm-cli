---
title: Configuration
description: The .gwm.toml schema — worktree conventions, bootstrap pipeline, regex guards, when-predicates, and the v0.6 launcher / open-dispatch sections.
navigation:
  title: Configuration
---

# Configuration

gwm reads `.gwm.toml` from the repo root. Without a config, it falls back to sensible defaults (`~/cc-worktree/<repo>/<type>-<issue>-<desc>`, no bootstrap). With one, it can copy files, run shell hooks, refuse to inherit dangerous secrets, and configure the TUI launchers.

- **[`.gwm.toml` schema](/configuration/gwm-toml)** — every section: `[worktree]`, `[[bootstrap.copy]]`, `[[bootstrap.guard]]`, `[bootstrap.fallback.*]`, `[[bootstrap.no_symlink]]`, `[[bootstrap.command]]`, `[git_tui]`, `[review]`, `[tui]`, `[tui.open]`, `[doctor]`.
- **[Bootstrap pipeline](/configuration/bootstrap)** — execution order: copies → guards → fallbacks → no-symlink check → commands.
- **[Regex guards](/configuration/guards)** — deny-list patterns on copied files (the original "no AWS RDS in `.env`" incident).
- **[`when` predicates](/configuration/when-predicates)** — `file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!`, `&&`, `||` composition.

Run `gwm init` in a fresh repo to write a default `.gwm.toml`. For the full annotated example with every field commented, see [`examples/gwm.toml.example`](https://github.com/kbrdn1/gwm-cli/blob/main/examples/gwm.toml.example) in the repo.
