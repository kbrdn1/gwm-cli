---
title: gwm — git worktree manager
description: Rust CLI + ratatui TUI to manage git worktrees across projects. Native libgit2, per-repo configurable bootstrap, single binary.
---

# gwm

Rust CLI + ratatui TUI to manage git worktrees across projects.

- Native `libgit2` (vendored) — no `gwq` / `git` CLI dependency.
- `gwm <subcommand>` for scripts and hooks; bare `gwm` opens a ratatui interface.
- Per-repo `.gwm.toml`: branch / path conventions, file copies, regex guards, shell hooks, no-symlink invariants.
- Branch convention `<type>/#<issue>-<description>` by default; overridable per repo.
- Configurable launchers for the `l` (git TUI) and `R` (review) keybindings.
- First-class GitHub issue / PR linking — branches matching the naming convention auto-link to their issue.

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
cargo install --path .
# or: brew tap kbrdn1/tap && brew install gwm

# bootstrap a per-repo config (optional but recommended)
cd /path/to/your/repo
gwm init

# create a worktree on a feature branch
gwm create feat 42 user-authentication
# → ~/cc-worktree/<repo>/feat-42-user-authentication
# → branch feat/#42-user-authentication

# open the TUI on the current repo
gwm

# fuzzy-jump back into an existing worktree (with `gwm shell-init` wired up)
gcd auth
```

## why gwm

The bash version (`tools/worktree-manager.sh` in some of our repos) was tied to one project's stack and one team's incident history. `gwm` keeps the lessons — anti-RDS guards, `.env.testing` copies, post-create hooks — and makes them configurable per repo. One binary, same behaviour everywhere.

The full background lives in [the changelog](/development#changelog) and in the issue-tracker history at [github.com/kbrdn1/gwm-cli](https://github.com/kbrdn1/gwm-cli).
