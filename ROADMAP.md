# gwm — roadmap

This document tracks where `gwm` is heading. It complements [CHANGELOG.md](CHANGELOG.md) (what already shipped) and the [open issues](https://github.com/kbrdn1/gwm-cli/issues) (the source of truth for scope details).

Each item below links to its GitHub issue. The scope, alternatives considered, and acceptance criteria live there — this file is the map, not the spec.

## Current state — v0.2.0

The 0.2.x line ships:

- **Native worktree ops via libgit2 (vendored)** — single binary, no `gwq` / `git` CLI dependency.
- **CLI + ratatui TUI** — `gwm <subcommand>` for scripts, `gwm` alone opens the interactive interface.
- **Per-repo `.gwm.toml`** — branch / path conventions, file copies, regex guards, shell hooks, no-symlink invariants.
- **Responsive details sidebar** — auto-shown when terminal width ≥ 120 cols, lazyssh-style with branch / path / head / status / recent commits / working-tree summary / commands cheat-sheet.
- **Lazygit fullscreen launch** (`l`) — suspends the gwm TUI, runs `lazygit -p <path>`, restores the TUI on exit.
- **Vim motions** — `gg` / `G` jump to first / last; `Tab` swaps focus between the list and the sidebar.
- **Release pipeline** — `release.yml` on `vX.Y.Z` tags, `pre-release.yml` on `vX.Y.Z-rc.N` / `-alpha.N` / `-beta.N` tags, 5-target build matrix (Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64).
- **74 tests** across config, naming, bootstrap, worktree (libgit2 integration), TUI state, and CLI end-to-end.

See [CHANGELOG.md](CHANGELOG.md#020---2026-05-18) for the full release notes.

## Quick wins

Small, well-scoped items with high daily-usage payoff. Likely picks for the next minor.

- [#18](https://github.com/kbrdn1/gwm-cli/issues/18) — **Shell completions** (zsh / bash / fish / powershell) via `clap_complete`, with dynamic completion of worktree names for `gwm path / remove / bootstrap`.
- [#19](https://github.com/kbrdn1/gwm-cli/issues/19) — **`gwm cd <pattern>` + `gwm shell-init <shell>`** to officialise the `cd "$(gwm path ...)"` workflow as a one-liner sourced from the rc file.
- [#20](https://github.com/kbrdn1/gwm-cli/issues/20) — **`gwm doctor`** to validate `.gwm.toml`, surface missing dependencies (`lazygit`, `direnv`, command binaries), and catch orphan branches / prunable worktrees.
- [#21](https://github.com/kbrdn1/gwm-cli/issues/21) — **TUI fuzzy filter** — press `/` in the list view to filter worktrees in real time (already on the README roadmap, lifted into the issue tracker).

## Power user

Workflow accelerators for daily use.

- [#22](https://github.com/kbrdn1/gwm-cli/issues/22) — **`gwm switch`** — a stripped-down picker UI that prints the selected worktree's path on `Enter` (paired with the `gwm cd` flow).
- [#23](https://github.com/kbrdn1/gwm-cli/issues/23) — **Tmux / Zellij integration** — `gwm tmux <pattern>` / `gwm zellij <pattern>` open the worktree in a new window or pane.
- [#24](https://github.com/kbrdn1/gwm-cli/issues/24) — **`gwm sync`** — fetch + rebase (or merge) the selected worktree's branch against its upstream, with conflict detection.
- [#25](https://github.com/kbrdn1/gwm-cli/issues/25) — **Extended bootstrap `when` predicates** — `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `&&` / `||` / `!` composition.

## Distribution

Lower the install friction by meeting users on their preferred channel.

- [#26](https://github.com/kbrdn1/gwm-cli/issues/26) — **Homebrew tap** (`brew tap kbrdn1/tap && brew install gwm`), auto-updated by `release.yml`.
- [#27](https://github.com/kbrdn1/gwm-cli/issues/27) — **`cargo-binstall` support** via `[package.metadata.binstall]` so `cargo binstall gwm` pulls the prebuilt archive.
- [#28](https://github.com/kbrdn1/gwm-cli/issues/28) — **Nix flake** — `nix profile install github:kbrdn1/gwm-cli` + a `devShell` for contributors.

## Safety & UX

Defensive features for a tool that performs destructive operations.

- [#29](https://github.com/kbrdn1/gwm-cli/issues/29) — **`gwm undo` + `gwm history`** — operation journal at `$XDG_DATA_HOME/gwm/history.toml` with branch-OID recovery.
- [#30](https://github.com/kbrdn1/gwm-cli/issues/30) — **TUI confirm overlay with a countdown** when `delete_branch_on_remove` is armed.
- [#31](https://github.com/kbrdn1/gwm-cli/issues/31) — **`--dry-run` on `gwm remove` and `gwm prune`** — show the resolved target / planned actions, no side effects.

## TUI polish

Refinements that make the interface more discoverable and customisable.

- [#32](https://github.com/kbrdn1/gwm-cli/issues/32) — **Command palette `:`** — Helix / Vim-style command bar with fuzzy completion across every TUI action.
- [#33](https://github.com/kbrdn1/gwm-cli/issues/33) — **Themes** — configurable colour scheme via `.gwm.toml` `[theme]`, with built-in presets (Catppuccin, Gruvbox, Tokyo Night, Solarized).
- [#34](https://github.com/kbrdn1/gwm-cli/issues/34) — **Sidebar stash mode** — press `s` to cycle the Details panel between commits-and-status (current) and stashes.

## Ambitious

Larger investments with strategic payoff. Gated by user demand or a concrete first consumer.

- [#35](https://github.com/kbrdn1/gwm-cli/issues/35) — **PTY-embedded lazygit panel** — render lazygit live in a right-hand pane (`portable-pty` + `tui-term`), beside the worktree list and the Details sidebar.
- [#36](https://github.com/kbrdn1/gwm-cli/issues/36) — **Multi-repo workspace mode** — `gwm --workspace ~/Projects` shows worktrees across every child repo in one TUI.
- [#37](https://github.com/kbrdn1/gwm-cli/issues/37) — **Configuration presets** — `gwm init --preset laravel / nuxt / rust / go / python-uv` seeds an opinionated `.gwm.toml` for known stacks.
- [#38](https://github.com/kbrdn1/gwm-cli/issues/38) — **JSON-RPC API + daemon mode** — `--format=json` on key commands, then a long-running daemon over `$XDG_RUNTIME_DIR/gwm.sock` for editor / statusbar integration.

## How to contribute

1. Pick an item that interests you and read its issue for scope details.
2. Comment on the issue if you intend to work on it (avoids parallel duplication).
3. Open a PR targeting `dev` following the conventions in [CONTRIBUTING.md](CONTRIBUTING.md) (Gitmoji + Conventional Commits, tests required, never squash).
4. The issue is the source of truth — this roadmap is updated to reflect what ships in each release.

Items marked `good first issue` (when applicable) are intentionally scoped so a newcomer can land them without a deep dive into the codebase.

## Out of scope (for now)

A few directions the project deliberately steers clear of:

- **Replacing lazygit / gitui in scope** — `gwm` is a worktree manager. Git history surgery stays with the dedicated tools that already do it well; `gwm` integrates with them rather than competing.
- **GUI front-end** — the terminal is the target. A GUI app would split focus and dilute the design.
- **Worktree synchronisation across machines** — too much surface (state, conflict, networking) for a tool whose value is local responsiveness.

That can change if a concrete use case shows up. Open a feature-request issue with the rationale.
