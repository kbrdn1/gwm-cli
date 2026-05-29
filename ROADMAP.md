# gwm — roadmap

This document tracks where `gwm` is heading. It complements [CHANGELOG.md](CHANGELOG.md) (what already shipped) and the [open issues](https://github.com/kbrdn1/gwm-cli/issues) (the source of truth for scope details).

Each item below links to its GitHub issue. The scope, alternatives considered, and acceptance criteria live there — this file is the map, not the spec.

## Current state — v0.7.0

The 0.7.x line ships:

- **Native worktree ops via libgit2 (vendored)** — single binary, no `gwq` / `git` CLI dependency.
- **CLI + ratatui TUI** — `gwm <subcommand>` for scripts, `gwm` alone opens the interactive interface.
- **Per-repo `.gwm.toml`** — branch / path conventions, configurable branch types, declarative GitHub labels / milestones, file copies, regex guards (`abort` or `seed-from-example`), shell hooks gated by `when:` predicates (`file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!`, `&&`, `||` composition), no-symlink invariants.
- **Bootstrap hardening for hostile clones** — TOFU trust ledger on `.gwm.toml`, `--allow-bootstrap` / `--deny-bootstrap`, path-traversal rejection, symlink-safe copy/write primitives, load-time regex validation for deny patterns.
- **Lazygit-style details sidebar** — four bordered subsections (Worktree / Issue · PR / Working Tree / Recent Commits), status-coloured branch names, header status dot tracking linked PR / issue state, 300-commit Recent Commits buffer with the full topology renderer (`○ ◎ │ ╮ ╭ ╯ ╰ ┴ ┬ ─`).
- **Measured TUI sidebar perf pass** — branch age is cached on `WorktreeInfo`, `filtered_indices` is memoised on `FilterState`, Recent Commits uses a cached libgit2 revwalk keyed by `(worktree path, head OID, limit)`, and commit-graph pipes store `git2::Oid` instead of heap-allocated hash strings.
- **Configurable launchers** — `[git_tui]` drives `l` (default `lazygit -p {path}`), `[review]` drives `R` (presets: `lumen` / `claude` / `codex` / `aider` / `gh`, plus free-form `command =`). Placeholders `{base} {head} {path} {diff}` with lazy diff materialisation.
- **GitHub issue / PR linking** — branches matching `<type>/#<N>-<slug>` auto-link to their issue; CLI `link / unlink / open / status` for explicit overrides; live state surfaces in the TUI sidebar via `gh`.
- **`[tui.open]` dispatch** — `o` key now spawns `$SHELL` in the worktree by default; opt back to OS file manager via `mode = "finder"`.
- **`y: yank`** — copy the selected worktree's path to the clipboard (pbcopy / wl-copy / xclip / xsel / clip).
- **Vim motions** — `gg` / `G` jump to first / last; `Tab` swaps focus between the list and the sidebar; `j` / `k` / `↑` / `↓` move selection or scroll the focused panel.
- **Fuzzy filter (`/`)** — sticky `nucleo-matcher` filter on the worktree list; smart-case, AND on spaces, contiguous beats spread-out; same engine powers `gwm switch` (picker mode), `gwm path / cd / remove / bootstrap` (fuzzy CLI lookup).
- **One-line `cd`** — `gwm shell-init <shell>` wires up a `gcd <pattern>` (resolve + cd) and bare `gcd` (picker + cd) for zsh / bash / fish / PowerShell.
- **Shell completions** — `gwm completions <shell>` for zsh / bash / fish / PowerShell / elvish (static script generated from the live clap argument tree).
- **Multiplexer integration** — `gwm tmux <pattern> [-p]` and `gwm zellij <pattern> [-p]` open the worktree in a new window / pane / tab; refuse to spawn outside an active session.
- **`gwm doctor`** — 7 checks (parses / guard refs / `when` predicates / external binaries / prunable / orphan branches / base writable), exit codes `0/1/2` for CI.
- **Confirm-overlay countdown** — safety countdown on the delete-confirm overlay when `p` (delete-branch-on-remove) is armed; duration tunable via `[tui].confirm_countdown_secs` (0..=5, clamped).
- **State-sliced TUI internals** — `tui::app::App` is decomposed into `tui/state/{create_form,filter,confirm,link_prompt,sidebar,github_fetch}.rs`, with dedicated tests for each state slice.
- **Release pipeline** — `release.yml` on `vX.Y.Z` tags, `pre-release.yml` on `-rc.N` / `-alpha.N` / `-beta.N` tags, 5-target build matrix (Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64), GitHub Release assets, Homebrew tap update job on stable releases, Nix flake at the repo root. Publish reliability follow-up: [#146](https://github.com/kbrdn1/gwm-cli/issues/146).
- **600+ tests** — integration and state-machine tests covering config, naming, bootstrap, doctor, GitHub linking, launcher, multiplexer, homebrew formula, pre-commit hook, TUI state slices, worktree libgit2 integration, release workflow guards, and CLI end-to-end.

See [`changelogs/0.7.0.md`](changelogs/0.7.0.md) for the full v0.7.0 release notes, and [`changelogs/`](changelogs/) for the per-version archive.

## Shipped highlights

For reference (each linked to its closing PR):

| Issue | Shipped in | Feature                                                                         |
|:------|:-----------|:--------------------------------------------------------------------------------|
| [#18](https://github.com/kbrdn1/gwm-cli/issues/18) | v0.3.0 | Shell completions (zsh / bash / fish / powershell / elvish)                     |
| [#19](https://github.com/kbrdn1/gwm-cli/issues/19) | v0.3.0 | `gwm cd <pattern>` + `gwm shell-init <shell>` (the `gcd` wrapper)               |
| [#20](https://github.com/kbrdn1/gwm-cli/issues/20) | v0.3.0 | `gwm doctor` (initial check set)                                                |
| [#21](https://github.com/kbrdn1/gwm-cli/issues/21) | v0.3.0 | TUI fuzzy filter (`/`)                                                          |
| [#22](https://github.com/kbrdn1/gwm-cli/issues/22) | v0.4.0 | `gwm switch` (picker UI printing the chosen path on stdout)                     |
| [#23](https://github.com/kbrdn1/gwm-cli/issues/23) | v0.4.0 | Tmux / Zellij integration (`gwm tmux` / `gwm zellij`)                           |
| [#25](https://github.com/kbrdn1/gwm-cli/issues/25) | v0.4.0 | Extended `when:` predicates (`cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!` / `&&` / `\|\|`) |
| [#26](https://github.com/kbrdn1/gwm-cli/issues/26) | v0.5.0 | Homebrew tap (`brew tap kbrdn1/tap && brew install gwm`)                        |
| [#28](https://github.com/kbrdn1/gwm-cli/issues/28) | v0.5.0 | Nix flake (`nix profile install github:kbrdn1/gwm-cli`)                         |
| [#30](https://github.com/kbrdn1/gwm-cli/issues/30) | v0.5.0 | TUI confirm-overlay countdown                                                   |
| [#47](https://github.com/kbrdn1/gwm-cli/issues/47) | v0.5.0 | `gwm doctor`: skip merged gwm-style branches in the orphan check                |
| [#59](https://github.com/kbrdn1/gwm-cli/issues/59) | v0.5.0 | `[doctor].trunks` config knob                                                   |
| [#67](https://github.com/kbrdn1/gwm-cli/issues/67) ([PR #68](https://github.com/kbrdn1/gwm-cli/pull/68)) | v0.6.0-rc.1 | Issue / PR linking — CLI + TUI controls, `gh`-backed live status     |
| [#69](https://github.com/kbrdn1/gwm-cli/issues/69) ([PR #70](https://github.com/kbrdn1/gwm-cli/pull/70)) | v0.6.0 | TUI Details sidebar redesign (four bordered subsections)            |
| [#71](https://github.com/kbrdn1/gwm-cli/issues/71) ([PR #72](https://github.com/kbrdn1/gwm-cli/pull/72)) | v0.6.0 | TUI Recent Commits panel — lazygit-style layout + full topology renderer |
| [#73](https://github.com/kbrdn1/gwm-cli/issues/73) ([PR #74](https://github.com/kbrdn1/gwm-cli/pull/74)) | v0.6.0 | Lazygit-style sidebar facelift (`Created` row, status colours, `[tui.open]`, `y: yank`) |
| [#75](https://github.com/kbrdn1/gwm-cli/issues/75) ([PR #76](https://github.com/kbrdn1/gwm-cli/pull/76)) | v0.6.0 | Configurable launchers (`[git_tui]` + `[review]`) — keymap reshuffle `r/R → f/F`, new `R` |
| [#77](https://github.com/kbrdn1/gwm-cli/issues/77) | v0.6.0 | Docs restructure into `docs/` tree (Nuxt Content conventions) + README shrunk to landing |
| [#80](https://github.com/kbrdn1/gwm-cli/issues/80) / [#81](https://github.com/kbrdn1/gwm-cli/issues/81) / [#82](https://github.com/kbrdn1/gwm-cli/issues/82) | v0.7.0-rc.1 | Configurable branch types, declarative GitHub labels, declarative GitHub milestones |
| [#93](https://github.com/kbrdn1/gwm-cli/issues/93) / [#94](https://github.com/kbrdn1/gwm-cli/issues/94) / [#95](https://github.com/kbrdn1/gwm-cli/issues/95) / [#96](https://github.com/kbrdn1/gwm-cli/issues/96) | v0.7.0-rc.1 | Bootstrap hardening: symlink-safe copies, path traversal rejection, TOFU trust ledger, guard regex load validation |
| [#97](https://github.com/kbrdn1/gwm-cli/issues/97) / [#98](https://github.com/kbrdn1/gwm-cli/issues/98) / [#99](https://github.com/kbrdn1/gwm-cli/issues/99) / [#100](https://github.com/kbrdn1/gwm-cli/issues/100) / [#101](https://github.com/kbrdn1/gwm-cli/issues/101) | v0.7.0-rc.2 | Static regex lifting, worktree removal ordering fix, stale-branch refusal, argv-injection guards, E2E create/remove/init tests |
| [#102](https://github.com/kbrdn1/gwm-cli/issues/102) / [#123](https://github.com/kbrdn1/gwm-cli/issues/123) / [#124](https://github.com/kbrdn1/gwm-cli/issues/124) / [#125](https://github.com/kbrdn1/gwm-cli/issues/125) / [#126](https://github.com/kbrdn1/gwm-cli/issues/126) / [#127](https://github.com/kbrdn1/gwm-cli/issues/127) / [#128](https://github.com/kbrdn1/gwm-cli/issues/128) | v0.7.0-rc.2 | `tui::app::App` decomposed into focused `tui/state/` sub-structs |
| [#103](https://github.com/kbrdn1/gwm-cli/issues/103) / [#104](https://github.com/kbrdn1/gwm-cli/issues/104) | v0.7.0-rc.2 | TUI render-loop perf: cached branch age and memoised `filtered_indices` |
| [#105](https://github.com/kbrdn1/gwm-cli/issues/105) / [#106](https://github.com/kbrdn1/gwm-cli/issues/106) | v0.7.0-rc.2 | Typed error variants and shared constructors/render helpers |
| [#138](https://github.com/kbrdn1/gwm-cli/issues/138) | v0.7.0-rc.3 | `GitHubFetch` cache keyed by issue/PR number; late results dropped after `invalidate()` |
| [#131](https://github.com/kbrdn1/gwm-cli/pull/131) / [#134](https://github.com/kbrdn1/gwm-cli/pull/134) | v0.7.0-rc.3 | TUI state encapsulation polish for `ConfirmModal` and `FilterState` |
| [#107](https://github.com/kbrdn1/gwm-cli/issues/107) / [#108](https://github.com/kbrdn1/gwm-cli/issues/108) | v0.7.0 | Measured P3 TUI sidebar perf: cached libgit2 Recent Commits and `Oid` commit graph pipes |
| [#146](https://github.com/kbrdn1/gwm-cli/issues/146) / [#147](https://github.com/kbrdn1/gwm-cli/issues/147) / [#112](https://github.com/kbrdn1/gwm-cli/issues/112) | v0.8.0-rc.1 | Release hardening: `gh`-CLI publish + workflow token, pre-release `[Unreleased]` dupe guard, Windows in the test matrix |
| [#86](https://github.com/kbrdn1/gwm-cli/issues/86) / [#85](https://github.com/kbrdn1/gwm-cli/issues/85) | v0.8.0-rc.1 | CLI aliases (`[aliases]` in `.gwm.toml` + user fallback, pre-clap expansion), gitmoji mapping + `gwm commit-prefix` + opt-in `commit-msg` hook |
| [#31](https://github.com/kbrdn1/gwm-cli/issues/31) / [#29](https://github.com/kbrdn1/gwm-cli/issues/29) | v0.8.0-rc.2 | Safety daily: `--dry-run` on `gwm remove` / `gwm prune`, `gwm undo` + `gwm history` operation journal at `$XDG_DATA_HOME/gwm/history.toml` |
| [#89](https://github.com/kbrdn1/gwm-cli/issues/89) / [#88](https://github.com/kbrdn1/gwm-cli/issues/88) | v0.8.0-rc.3 | Config CLI (`gwm config get/set/unset/list/validate/path/edit`, comment-preserving `toml_edit`) + `[hooks.*]` lifecycle hooks (six phases, `on_fail`, `[[bootstrap.command]]` compat) |
| [#83](https://github.com/kbrdn1/gwm-cli/issues/83) / [#84](https://github.com/kbrdn1/gwm-cli/issues/84) | v0.8.0-rc.3 | GitHub templates: `[issue_template]` + `gwm new`, `[pr_template]` + `gwm pr` with `{commits}` / `{files_changed}` placeholders |
| [#87](https://github.com/kbrdn1/gwm-cli/issues/87) / [#32](https://github.com/kbrdn1/gwm-cli/issues/32) / [#33](https://github.com/kbrdn1/gwm-cli/issues/33) / [#34](https://github.com/kbrdn1/gwm-cli/issues/34) | v0.8.0-rc.3 | TUI personalisation: `[tui.keys]` remappable keymap with chords + `gwm tui keys`, command palette (`:`), `[theme]` role-based presets, sidebar stashes mode (`s`) |
| [#24](https://github.com/kbrdn1/gwm-cli/issues/24) / [#27](https://github.com/kbrdn1/gwm-cli/issues/27) | v0.8.0-rc.4 | Quick wins: `gwm sync [<pattern>] [--merge]` (fetch + rebase/merge onto upstream, conflict-safe) and `cargo-binstall` support via `[package.metadata.binstall]` |

If an issue still shows `open` on GitHub even though its work shipped, it's a tracking issue waiting for a follow-up audit — check the CHANGELOG and the linked PR before reopening scope work on it.

## Ambitious

Larger investments with strategic payoff. Gated by user demand or a concrete first consumer.

- [#35](https://github.com/kbrdn1/gwm-cli/issues/35) — **PTY-embedded lazygit panel** — render lazygit live in a right-hand pane (`portable-pty` + `tui-term`), beside the worktree list and the Details sidebar. Distinct from the existing `l` launcher: this one would render lazygit **inside** gwm rather than handing the alt-screen over.
- [#36](https://github.com/kbrdn1/gwm-cli/issues/36) — **Multi-repo workspace mode** — `gwm --workspace ~/Projects` shows worktrees across every child repo in one TUI.
- [#37](https://github.com/kbrdn1/gwm-cli/issues/37) — **Configuration presets** — `gwm init --preset laravel / nuxt / rust / go / python-uv` seeds an opinionated `.gwm.toml` for known stacks instead of the generic default.
- [#38](https://github.com/kbrdn1/gwm-cli/issues/38) — **JSON-RPC / gRPC API + daemon mode** — `--format=json` on key commands, then a long-running daemon over `$XDG_RUNTIME_DIR/gwm.sock` for editor / statusbar integration.

## How to contribute

1. Pick an item that interests you and read its issue for scope details.
2. Comment on the issue if you intend to work on it (avoids parallel duplication).
3. `gwm create <type> <issue> <slug>` to spin up an isolated worktree (the issue auto-links itself — see [`docs/integrations/github-linking.md`](docs/5.integrations/1.github-linking.md)).
4. Open a PR targeting `dev` following the conventions in [CONTRIBUTING.md](CONTRIBUTING.md) (Gitmoji + Conventional Commits, tests required, never squash; full docs version at [`docs/development/contributing.md`](docs/6.development/2.contributing.md)).
5. The issue is the source of truth — this roadmap is updated to reflect what ships in each release.

Items marked `good first issue` (when applicable) are intentionally scoped so a newcomer can land them without a deep dive into the codebase.

## Out of scope (for now)

A few directions the project deliberately steers clear of:

- **Replacing lazygit / gitui in scope** — `gwm` is a worktree manager. Git history surgery stays with the dedicated tools that already do it well; `gwm` integrates with them rather than competing.
- **GUI front-end** — the terminal is the target. A GUI app would split focus and dilute the design.
- **Worktree synchronisation across machines** — too much surface (state, conflict, networking) for a tool whose value is local responsiveness.

That can change if a concrete use case shows up. Open a feature-request issue with the rationale.
