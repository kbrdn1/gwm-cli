---
title: Configuration
description: The .gwm.toml schema — worktree conventions, bootstrap pipeline, lifecycle hooks, regex guards, when-predicates, theme, keymap, templates, aliases, and the user-level global config.
navigation:
  title: Configuration
---

# Configuration

gwm reads `.gwm.toml` from the repo root. Without a config, it falls back to sensible defaults (`~/cc-worktree/<repo>/<type>-<issue>-<desc>`, no bootstrap). With one, it can copy files, run lifecycle hooks, refuse to inherit dangerous secrets, configure the TUI launchers / keymap / theme, and declare GitHub labels, milestones, and issue / PR templates. The same schema can also live at a [user-level global config](/configuration/global-config) merged underneath every repo.

- **[`.gwm.toml` schema](/configuration/gwm-toml)** — every section: `[worktree]`, `[[bootstrap.copy]]`, `[[bootstrap.guard]]`, `[bootstrap.fallback.*]`, `[[bootstrap.no_symlink]]`, `[[hooks.*]]` (+ legacy `[[bootstrap.command]]`), `[theme]`, `[tui]`, `[tui.keys]`, `[tui.open]`, `[git_tui]`, `[review]`, `[gitmoji]`, `[[labels]]`, `[[milestones]]`, `[issue_template]`, `[pr_template]`, `[aliases]`, `[doctor]`.
- **[User-level global config](/configuration/global-config)** — `~/.config/gwm/config.toml` merged underneath each repo's `.gwm.toml`; deep-overlay semantics and the `GWM_NO_GLOBAL_CONFIG=1` opt-out.
- **[Bootstrap pipeline](/configuration/bootstrap)** — execution order: lifecycle hooks around copies → guards → fallbacks → no-symlink check.
- **[Regex guards](/configuration/guards)** — deny-list patterns on copied files (the original "no AWS RDS in `.env`" incident).
- **[`when` predicates](/configuration/when-predicates)** — `file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!`, `&&`, `||` composition.
- **[TOFU trust ledger](/configuration/trust-ledger)** — the gate that fires before the bootstrap pipeline (issue #95). Threat model, CLI surface (`gwm trust list / revoke / show`), TUI behaviour, ledger format, CI bypass.
- **[Config presets](/configuration/presets)** — `gwm init --preset <stack>` seeds an opinionated `.gwm.toml` for a known stack (`laravel`, `node`/`nuxt`, `rust`, `go`, `python-uv`, `generic`) instead of the generic template; `--list-presets` and `--show`.

Run `gwm init` in a fresh repo to write a default `.gwm.toml`. For the full annotated example with every field commented, see [`examples/gwm.toml.example`](https://github.com/kbrdn1/gwm-cli/blob/main/examples/gwm.toml.example) in the repo.
