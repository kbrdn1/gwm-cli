---
title: gwm — git worktree manager
description: Rust CLI + ratatui TUI to manage git worktrees across projects. Native libgit2, per-repo configurable bootstrap, single binary.
---

# gwm

Rust CLI + ratatui TUI to manage git worktrees across projects.

- Native `libgit2` (vendored) — no `gwq` / `git` CLI dependency.
- `gwm <subcommand>` for scripts and hooks; bare `gwm` opens a ratatui interface.
- Per-repo `.gwm.toml`: branch / path conventions, file copies, regex guards, `[hooks.*]` lifecycle commands, no-symlink invariants — plus a user-level global config at `~/.config/gwm/config.toml` merged underneath it so a preference set once applies to every repo.
- Branch convention `<type>/#<issue>-<description>` by default; overridable per repo. `[aliases]` mirror `git config` aliases; `gwm commit-prefix` + an opt-in `commit-msg` hook drive the Gitmoji + Conventional Commits convention.
- Configurable launchers for the `l` (git TUI) and `R` (review) keybindings.
- TUI personalisation: role-based `[theme]` presets (`catppuccin`, `gruvbox`, `tokyo-night`, `claude-dark`), a remappable `[tui.keys]` keymap with multi-key chords, a `:` command palette, a responsive sidebar, and a single-line statusline.
- Safety nets: `--dry-run` on `gwm remove` / `gwm prune`, plus `gwm undo` / `gwm history` backed by an operation journal.
- First-class GitHub issue / PR linking — branches matching the naming convention auto-link to their issue; PRs are auto-detected from `gh` when not explicitly linked. Declarative `[[labels]]` / `[[milestones]]`, `gwm new` (issue → worktree), and `gwm pr` (templated PR body).
- `gwm sync` fetches a worktree's upstream and rebases (or merges) onto it, conflict-safe.
- [TOFU trust ledger](/configuration/trust-ledger) on `.gwm.toml` — first `gwm create` / `gwm bootstrap` on a repo prompts before executing any bootstrap command line. `--allow-bootstrap` / `GWM_ALLOW_BOOTSTRAP=1` for CI bypass.
- Install via `cargo install gwm`, `cargo binstall gwm` (prebuilt archives, no toolchain), Homebrew, or Nix.

## documentation map

| Section                                            | Read this when …                                                              |
|:---------------------------------------------------|:------------------------------------------------------------------------------|
| [Getting Started](/getting-started)                | you want to install gwm and create your first worktree                        |
| [TUI](/tui)                                        | you live in the ratatui interface — keymap, sidebar, launchers, filter        |
| [CLI](/cli)                                        | you script gwm from shells, CI jobs, or `gh` aliases                          |
| [Configuration](/configuration)                    | you're writing or extending `.gwm.toml` — bootstrap, guards, predicates       |
| [Integrations](/integrations)                      | you wire gwm with `gh`, `lazygit`, Homebrew, Nix, or `gwm doctor` in CI       |
| [Development](/development)                        | you're contributing — test layout, conventions, dev shell                     |
| [Roadmap](/roadmap)                                | you want to know what ships next                                              |

## the 30-second tour

```bash
# install
cargo install gwm
# or: cargo binstall gwm        # prebuilt archive, no Rust toolchain
# or: brew tap kbrdn1/tap && brew install gwm

# bootstrap a per-repo config (optional but recommended)
cd /path/to/your/repo
gwm init

# create a worktree on a feature branch
gwm create feat 42 user-authentication
# → ~/cc-worktree/<repo>/feat-42-user-authentication
# → branch feat/#42-user-authentication

# open the TUI on the current repo (themes, command palette, remappable keys)
gwm

# fuzzy-jump back into an existing worktree (with `gwm shell-init` wired up)
gcd auth

# misfired a remove? bring it back
gwm undo
```

## why gwm

The bash version (`tools/worktree-manager.sh` in some of our repos) was tied to one project's stack and one team's incident history. `gwm` keeps the lessons — anti-RDS guards, `.env.testing` copies, post-create hooks — and makes them configurable per repo. One binary, same behaviour everywhere.

The full background lives in [the changelog](/development#changelog) and in the issue-tracker history at [github.com/kbrdn1/gwm-cli](https://github.com/kbrdn1/gwm-cli).
