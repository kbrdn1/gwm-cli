# gwm — roadmap

This document tracks where `gwm` is heading. It complements [CHANGELOG.md](CHANGELOG.md) (what already shipped) and the [open issues](https://github.com/kbrdn1/gwm-cli/issues) (the source of truth for scope details).

Each item below links to its GitHub issue. The scope, alternatives considered, and acceptance criteria live there — this file is the map, not the spec.

## Current state — v0.6.0

The 0.6.x line ships:

- **Native worktree ops via libgit2 (vendored)** — single binary, no `gwq` / `git` CLI dependency.
- **CLI + ratatui TUI** — `gwm <subcommand>` for scripts, `gwm` alone opens the interactive interface.
- **Per-repo `.gwm.toml`** — branch / path conventions, file copies, regex guards (`abort` or `seed-from-example`), shell hooks gated by `when:` predicates (`file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!`, `&&`, `||` composition), no-symlink invariants.
- **Lazygit-style details sidebar** — four bordered subsections (Worktree / Issue · PR / Working Tree / Recent Commits), status-coloured branch names, header status dot tracking linked PR / issue state, 300-commit Recent Commits buffer with the full topology renderer (`○ ◎ │ ╮ ╭ ╯ ╰ ┴ ┬ ─`).
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
- **Release pipeline** — `release.yml` on `vX.Y.Z` tags, `pre-release.yml` on `-rc.N` / `-alpha.N` / `-beta.N` tags, 5-target build matrix (Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64), automatic Homebrew tap update on stable releases, Nix flake at the repo root.
- **433 tests** — 15 integration files + colocated unit tests covering config, naming, bootstrap, doctor, github linking, launcher, multiplexer, homebrew formula, pre-commit hook, TUI state, worktree (libgit2 integration), and CLI end-to-end.

See [`changelogs/0.6.0.md`](changelogs/0.6.0.md) for the full v0.6.0 release notes, and [`changelogs/`](changelogs/) for the per-version archive.

## Shipped — pre-v0.6.0

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

If an issue still shows `open` on GitHub even though its work shipped, it's a tracking issue waiting for a follow-up audit — check the CHANGELOG and the linked PR before reopening scope work on it.

## Quick wins

Small, well-scoped items with high daily-usage payoff. Likely picks for the next minor.

- [#24](https://github.com/kbrdn1/gwm-cli/issues/24) — **`gwm sync`** — fetch + rebase (or merge) the selected worktree's branch against its upstream, with conflict detection.
- [#27](https://github.com/kbrdn1/gwm-cli/issues/27) — **`cargo-binstall` support** via `[package.metadata.binstall]` so `cargo binstall gwm` pulls the prebuilt archive instead of compiling from source.
- [#31](https://github.com/kbrdn1/gwm-cli/issues/31) — **`--dry-run` on `gwm remove` and `gwm prune`** — show the resolved target / planned actions, no side effects. Pairs nicely with the safety stance of [#29](https://github.com/kbrdn1/gwm-cli/issues/29) below.
- [#86](https://github.com/kbrdn1/gwm-cli/issues/86) — **`[aliases]` in `.gwm.toml`** — git-config-style aliases (`wip = "create feat 0 wip"`, `ll = "list --format names"`), plus a user-level `~/.config/gwm/aliases.toml` fallback. Lowest-cost item on the configurability axis (pre-clap argv expansion).

## Configurability

A coherent batch of items that move hardcoded conventions and one-off shell scripts into `.gwm.toml`. Theme: every team-portable convention should live in the config that's already checked in, not in tribal knowledge.

### Repo conventions

- [#80](https://github.com/kbrdn1/gwm-cli/issues/80) — **`[[branch_types]]` configurable** — promote the hardcoded `BRANCH_TYPES` const (`src/naming.rs:5`) to a `.gwm.toml` block. `gwm types` reads from config when present, defaults otherwise. Validation in `BranchSpec::validate()` follows.
- [#85](https://github.com/kbrdn1/gwm-cli/issues/85) — **`[gitmoji]` mapping** — `branch_type → emoji` table with sensible defaults; new `gwm commit-prefix` subcommand prints the resolved prefix for the current branch; opt-in `commit-msg` hook auto-prepends it. Pairs naturally with `[[branch_types]]`.

### GitHub publish (declarative repo state)

- [#81](https://github.com/kbrdn1/gwm-cli/issues/81) — **`[[labels]]` + `gwm labels push`** — declare labels in `.gwm.toml` (with optional `color`, deterministic pastel hash when absent), publish to the remote via `gh label create --force`. `--dry-run` and `--prune` for the destructive bits. Same plumbing extracted to `src/github_publish.rs` for #82 to reuse.
- [#82](https://github.com/kbrdn1/gwm-cli/issues/82) — **`[[milestones]]` + `gwm milestones push`** — same pattern as labels, for milestones (REST API since `gh milestone` doesn't exist natively).
- [#83](https://github.com/kbrdn1/gwm-cli/issues/83) — **`[issue_template]` defaults** — map branch types to `.github/ISSUE_TEMPLATE/*.yml` templates with per-type defaults (surface, title prefix, labels). New `gwm new <type> <desc>` creates the issue *and* the worktree in one go.
- [#84](https://github.com/kbrdn1/gwm-cli/issues/84) — **`[pr_template]` per branch type** — body templates with placeholders (`{commits}` is the killer feature). New `gwm pr [--draft]` subcommand. Shared template renderer with #83 (`src/templating.rs`).

### Lifecycle & control surface

- [#88](https://github.com/kbrdn1/gwm-cli/issues/88) — **`[hooks.*]` lifecycle hooks** — six phases (`pre_create`, `post_create`, `pre_bootstrap`, `post_bootstrap`, `pre_remove`, `post_remove`) with `on_fail = "abort" | "warn" | "ignore"`. Existing `[[bootstrap.command]]` aliased to `[[hooks.post_create]]` for compat.
- [#89](https://github.com/kbrdn1/gwm-cli/issues/89) — **`gwm config get/set/list/validate/path/edit`** — git-config-style CLI over `.gwm.toml` with `toml_edit` for comment-preserving round-tripping. Includes dot-path notation (`worktree.base`) and array-table indexing (`labels[+].name = "bug"`).
- [#87](https://github.com/kbrdn1/gwm-cli/issues/87) — **`[tui.keys]` keymap** — remap any TUI action through `.gwm.toml`, chord support (`g g`), with `gwm tui keys` introspection. Sits alongside themes (#33) and command palette (#32) as the "TUI personalisation" trio.

## Safety & UX

Defensive features for a tool that performs destructive operations.

- [#29](https://github.com/kbrdn1/gwm-cli/issues/29) — **`gwm undo` + `gwm history`** — operation journal at `$XDG_DATA_HOME/gwm/history.toml` with branch-OID recovery so a fat-finger `gwm remove --delete-branch` is recoverable beyond `git reflog`.

## TUI polish

Refinements that make the interface more discoverable and customisable.

- [#32](https://github.com/kbrdn1/gwm-cli/issues/32) — **Command palette `:`** — Helix / Vim-style command bar with fuzzy completion across every TUI action, complementing the existing `?` overlay and `/` filter.
- [#33](https://github.com/kbrdn1/gwm-cli/issues/33) — **Themes** — configurable colour scheme via `.gwm.toml` `[theme]`, with built-in presets (Catppuccin, Gruvbox, Tokyo Night, Solarized).
- [#34](https://github.com/kbrdn1/gwm-cli/issues/34) — **Sidebar stash mode** — press `s` to cycle the Details panel between the current view and a stashes view.

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
