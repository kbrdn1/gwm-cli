# gwm — git worktree manager

[![ci](https://github.com/kbrdn1/gwm-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/kbrdn1/gwm-cli/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/kbrdn1/gwm-cli?display_name=tag&sort=semver)](https://github.com/kbrdn1/gwm-cli/releases)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.md)
[![rust](https://img.shields.io/badge/rust-1.80%2B-orange?logo=rust)](https://www.rust-lang.org/)

Rust CLI + ratatui TUI to manage git worktrees across projects. Native `libgit2` (vendored — no `gwq` / `git` CLI dependency), per-repo configurable bootstrap (file copies, regex guards, shell hooks), single binary, portable.

> **Full documentation lives in [`docs/`](docs/).** This README is the landing page; every feature has a dedicated section in the doc tree.

## install

| Channel        | Command                                                              |
|:---------------|:---------------------------------------------------------------------|
| Cargo (source) | `cargo install --path .`                                              |
| Homebrew (macOS) | `brew tap kbrdn1/tap && brew install gwm`                          |
| Nix flake      | `nix profile install github:kbrdn1/gwm-cli`                          |
| Prebuilt       | <https://github.com/kbrdn1/gwm-cli/releases> (Linux / macOS / Windows) |

Full install matrix and verification steps: [`docs/getting-started/install.md`](docs/1.getting-started/1.install.md).

## the 30-second tour

```bash
cd /path/to/your/repo
gwm init                                          # write a default .gwm.toml
gwm create feat 42 user-authentication            # → ~/cc-worktree/<repo>/feat-42-user-authentication
                                                  # → branch feat/#42-user-authentication
gwm                                               # opens the TUI on the current repo
gcd auth                                          # fuzzy-jump into the worktree (needs `gwm shell-init`)
```

Step-by-step walkthrough: [`docs/getting-started/first-worktree.md`](docs/1.getting-started/2.first-worktree.md).

## what gwm does

- **Native worktree ops** via vendored `libgit2` — `git worktree add/list/remove/prune` without shelling out.
- **CLI + ratatui TUI** — `gwm <subcommand>` for scripts, bare `gwm` opens the interactive interface.
- **Per-repo `.gwm.toml`** — branch / path conventions, file copies, regex guards, shell hooks, no-symlink invariants.
- **Configurable launchers** — drive the TUI's `l` (git TUI) and `R` (review) keybindings through `[git_tui]` and `[review]` sections in `.gwm.toml`.
- **GitHub issue / PR linking** — branches matching `<type>/#<N>-<slug>` auto-link to their issue; live status surfaces in the TUI sidebar via `gh`.
- **Safety guards** — deny-list regexes on copied files (the original "no AWS RDS in `.env`" incident, generalised), plus a confirm-overlay countdown when destructive branch-deletion is armed.
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
- [`ROADMAP.md`](ROADMAP.md) — long-form roadmap with grouped categories
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- [`.github/LABELS.md`](.github/LABELS.md)
- [`examples/gwm.toml.example`](examples/gwm.toml.example) — annotated full config
- [`skills/SKILL.md`](skills/SKILL.md) — the bundled Claude Code skill manifest
