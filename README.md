# gwm — git worktree manager

[![ci](https://github.com/kbrdn1/gwm-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/kbrdn1/gwm-cli/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/kbrdn1/gwm-cli?display_name=tag&sort=semver)](https://github.com/kbrdn1/gwm-cli/releases)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.md)
[![rust](https://img.shields.io/badge/rust-1.86%2B-orange?logo=rust)](https://www.rust-lang.org/)

Rust CLI + ratatui TUI to manage git worktrees across projects. Native `libgit2` (vendored — no `gwq` / `git` CLI dependency), per-repo + user-level configurable bootstrap (file copies, regex guards, lifecycle hooks), single binary, portable.

![gwm TUI — worktree table and details sidebar](docs/2.tui/_assets/hero.png)

> **Full documentation lives in [`docs/`](docs/).** This README is the landing page; every feature has a dedicated section in the doc tree.

## install

| Channel          | Command                                                              |
|:-----------------|:---------------------------------------------------------------------|
| Cargo (crates.io) | `cargo install gwm-cli`                                             |
| Cargo (source)   | `cargo install --path .`                                             |
| cargo-binstall   | `cargo binstall gwm-cli`                                             |
| Homebrew (macOS) | `brew tap kbrdn1/tap && brew install gwm`                            |
| Nix flake        | `nix profile install github:kbrdn1/gwm-cli`                          |
| Prebuilt         | <https://github.com/kbrdn1/gwm-cli/releases> (Linux / macOS / Windows) |

The crate is published as **`gwm-cli`** (the bare `gwm` name on crates.io belongs to an unrelated project) — the installed command is still `gwm`. `cargo binstall gwm-cli` grabs the prebuilt binary from the matching GitHub Release instead of compiling `git2`/vendored-libgit2 from source — no Rust toolchain needed at install time.

Full install matrix and verification steps: [`docs/getting-started/install.md`](docs/1.getting-started/1.install.md).

## the 30-second tour

```bash
cd /path/to/your/repo
gwm init                                          # write a default .gwm.toml
gwm init --preset laravel                          # …or seed a stack preset (laravel/node/rust/go/python-uv)
gwm init --list-presets                            # list the built-in presets
gwm create feat 42 user-authentication            # → ~/cc-worktree/<repo>/feat-42-user-authentication
                                                  # → branch feat/#42-user-authentication
gwm                                               # opens the TUI on the current repo
gcd auth                                          # fuzzy-jump into the worktree (needs `gwm shell-init`)
```

Step-by-step walkthrough: [`docs/getting-started/first-worktree.md`](docs/1.getting-started/2.first-worktree.md).

## what gwm does

- **Native worktree ops** via vendored `libgit2` — `git worktree add/list/remove/prune` without shelling out.
- **CLI + ratatui TUI** — `gwm <subcommand>` for scripts, bare `gwm` opens the interactive interface.
- **JSON API + daemon** ([#38](https://github.com/kbrdn1/gwm-cli/issues/38)) — `--format=json` on `gwm list` / `doctor` / `path` (stable schemas under [`docs/schema/`](docs/schema/)), and `gwm daemon`, a JSON-RPC 2.0 server over a unix socket (`list` / `doctor` / `path` + a `subscribe` push stream) so editors and statusbars connect once instead of shelling out per query.
- **First daemon consumer — `gwm statusline`** ([#309](https://github.com/kbrdn1/gwm-cli/issues/309)) — a thin, dependency-free client that renders a compact one-line worktree summary (active branch · count · dirty/ahead/behind · issue/PR) for tmux / starship / zsh prompts off the daemon; `--watch` rides the `subscribe` stream, and with no daemon it degrades to a blank line. See [Integrations → Daemon consumers](docs/5.integrations/4.daemon-consumers.md).
- **Multi-repo workspace mode** ([#36](https://github.com/kbrdn1/gwm-cli/issues/36)) — `gwm --workspace ~/Projects` opens the TUI across every git repo one level below a root (a REPO column tags each row; the active repo follows the selection); `gwm list --workspace ~/Projects` prints the merged table; `gwm create --repo <name>` picks the target. Bare `gwm` in a repo-free dir that holds child repos offers to open it as a workspace.
- **Per-repo `.gwm.toml` + user-level global config** — branch / path conventions, file copies, regex guards, no-symlink invariants. A `~/.config/gwm/config.toml` deep-merges underneath each repo's `.gwm.toml`. Edit it git-config-style with `gwm config get / set / list / validate`.
- **Config presets for `gwm init`** ([#37](https://github.com/kbrdn1/gwm-cli/issues/37)) — `gwm init --preset <name>` seeds an opinionated `.gwm.toml` for a known stack (`laravel` / `node` / `nuxt` / `rust` / `go` / `python-uv` / `generic`) instead of the generic template; `--list-presets` enumerates them, `--show` prints the resolved TOML without writing.
- **Lifecycle hooks `[hooks.*]`** — `pre_create` / `post_create` / `pre_bootstrap` / `post_bootstrap` / `pre_remove` / `post_remove` phases, each with `when:` predicates and per-step `on_fail = abort|warn|ignore`.
- **CLI aliases + Gitmoji convention** — `[aliases]` expand `gwm <alias>` to argv before parsing; `gwm commit-prefix`, `gwm types --gitmoji`, and an opt-in `gwm hooks install commit-msg` hook enforce the repo's Gitmoji + Conventional Commits style.
- **GitHub workflow** — branches matching `<type>/#<N>-<slug>` auto-link to their issue (with ephemeral PR auto-detection); `gwm new` opens an issue from a template then spins up the worktree, `gwm pr` renders the PR body; `gwm review <PR#>` ([#308](https://github.com/kbrdn1/gwm-cli/issues/308)) pulls an existing PR — including one from a fork — into an isolated worktree (fetch + link), the inbound counterpart to `gwm create` (safe-by-default: bootstrap/hooks are opt-in via `--bootstrap`, since a fork PR's setup commands are arbitrary code); live status surfaces in the TUI sidebar via `gh`.
- **Safety daily** — `--dry-run` on `gwm remove` / `gwm prune` to preview, `gwm undo` + `gwm history` to recover a misfired removal, a confirm-overlay countdown on armed branch-deletion, and deny-list regexes on copied files (the original "no AWS RDS in `.env`" incident, generalised).
- **`gwm sync`** — fetch a worktree's upstream and rebase (or `--merge`) its branch onto it, conflict-safe.
- **Fleet chores across worktrees** ([#313](https://github.com/kbrdn1/gwm-cli/issues/313)) — `gwm exec [<slug>...] -- <cmd>` runs a command in each worktree sequentially (everything after `--` forwarded verbatim) and prints a `✓ / ✗` rollup, exiting non-zero if any failed; `gwm clean [<slug>...]` reports reclaimable build artifacts (`target/`, `node_modules/`, `dist/`, `build/`) per worktree, deleting them only with `--yes`.
- **Configurable launchers** — drive the TUI's `l` (git TUI) and `r` / `R` (review) keybindings through `[git_tui]` and `[review]` sections in `.gwm.toml`.
- **TUI personalisation** — role-based `[theme]` presets (`catppuccin` / `gruvbox` / `tokyo-night` / `claude-dark`), a remappable `[tui.keys]` keymap with multi-key chords (plus rebindable per-context modal keys under `[tui.keys.modal.<context>]`, all editable live from the Settings panel's Keys tab), a `:` command palette, and a sidebar stashes mode — all responsive down to a narrow terminal.
- **Embedded PTY overlays** ([#35](https://github.com/kbrdn1/gwm-cli/issues/35)) — `l` / `L` open lazygit and `o` / `O` open a native `$SHELL` session inside the TUI (no alternate-screen swap); `Esc` closes the overlay.
- **Richer Status sidebar** — the Working Tree pane renders `git status` as a nerd-font file-explorer tree ([#300](https://github.com/kbrdn1/gwm-cli/issues/300)) with git-coloured rows, and the Issue/PR section surfaces the linked PR's overall CI state (` CI passing 9/9` / ` CI failing 7/9` / ` CI running 8/9`) derived from the already-fetched rollup ([#299](https://github.com/kbrdn1/gwm-cli/issues/299)).
- **TOFU trust ledger on `.gwm.toml`** ([#95](https://github.com/kbrdn1/gwm-cli/issues/95)) — first `gwm create` / `gwm bootstrap` against a repo prints the bootstrap surface (copies, guards, commands) and prompts before running anything. Recorded in `~/.config/gwm/trust.toml` keyed on `(origin URL, sha256 of .gwm.toml)`; any byte change re-prompts. CI bypass: `--allow-bootstrap` or `GWM_ALLOW_BOOTSTRAP=1`. Manage with `gwm trust list / revoke / show`.

## documentation

The full tree lives under [`docs/`](docs/) — structured for [Nuxt Content](https://content.nuxt.com/) (numeric prefixes for sidebar order, frontmatter on every page) and ready to drop into the future static site.

| Section                                                         | Read this when …                                                              |
|:----------------------------------------------------------------|:------------------------------------------------------------------------------|
| [Getting Started](docs/1.getting-started/index.md)              | you want to install gwm and create your first worktree                        |
| [TUI](docs/2.tui/index.md)                                      | you live in the ratatui interface — keymap, sidebar, launchers, filter        |
| [CLI](docs/3.cli/index.md)                                      | you script gwm from shells, CI jobs, or `gh` aliases                          |
| [Configuration](docs/4.configuration/index.md)                  | you're writing or extending `.gwm.toml` — bootstrap, guards, predicates       |
| [Integrations](docs/5.integrations/index.md)                    | you wire gwm with `gh`, `lazygit`, AI reviewers, Homebrew, Nix, or `gwm doctor` in CI |
| [Development](docs/6.development/index.md)                      | you're contributing — test layout, conventions, dev shell                     |
| [Roadmap](docs/7.roadmap.md)                                    | you want to know what shipped and what comes next                             |

The [`docs/README.md`](docs/README.md) page documents the authoring conventions (frontmatter contract, numeric-prefix routing, link semantics) for anyone editing the tree.

## history

gwm started as a Rust rewrite of `tools/worktree-manager.sh` — a bash script tied to one team's Laravel stack and one repo's incident history. The Rust version keeps the lessons, makes them configurable per repo, and ships as a single binary so it works in every repo without per-project shell-script copies. Full background under [Development → Contributing → history](docs/6.development/2.contributing.md#history).

## license

MIT — see [LICENSE.md](LICENSE.md).

## related docs

- [`CHANGELOG.md`](CHANGELOG.md) — release index (root = `[Unreleased]`; per-version archives under [`changelogs/`](changelogs/))
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — branch / commit / PR conventions
- [Stability & compatibility](docs/6.development/3.stability.md) — the 1.0 SemVer contract: what's covered, what's free to change, MSRV, deprecations
- [`ROADMAP.md`](ROADMAP.md) — long-form roadmap with grouped categories
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- [`.github/LABELS.md`](.github/LABELS.md)
- [`examples/gwm.toml.example`](examples/gwm.toml.example) — annotated full config
- [`skills/SKILL.md`](skills/SKILL.md) — the bundled Claude Code skill manifest
