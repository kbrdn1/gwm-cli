---
title: gwm, the git worktree manager
description: Rust CLI + ratatui TUI to manage git worktrees across projects. Native libgit2, per-repo configurable bootstrap, single binary.
---

# gwm

Rust CLI + ratatui TUI to manage git worktrees across projects.

- Worktree operations run on vendored `libgit2`, with no `gwq` dependency; only a few features (`gwm sync`, the review-diff launcher, the sidebar's `git status` / `git log`) shell out to your own `git`.
- `gwm <subcommand>` for scripts and hooks; bare `gwm` opens a ratatui interface.
- Per-repo `.gwm.toml`: branch / path conventions, file copies, regex guards, `[hooks.*]` lifecycle commands, no-symlink invariants, plus a user-level global config at `~/.config/gwm/config.toml` merged underneath it so a preference set once applies to every repo. `gwm init --preset <name>` seeds an opinionated config for a known stack (`laravel` / `symfony` / `node` / `nuxt` / `rust` / `go` / `python-uv` / `generic`).
- Multi-repo workspace mode: `gwm --workspace ~/Projects` (and bare `gwm` auto-detect) opens the TUI across every git repo one level below a root with a REPO column; `gwm list --workspace` prints the merged table; `gwm create --repo <name>` picks the target.
- JSON API + daemon: `--format=json` on `gwm list` / `doctor` / `path` (stable schemas), and `gwm daemon`, a JSON-RPC 2.0 server over a unix socket with a `subscribe` push stream, for editor / statusbar integration.
- `gwm statusline`: a dependency-free daemon consumer that renders a one-line worktree summary (active branch · worktree count · dirty / ahead / behind · linked issue / PR) for a tmux / starship / zsh prompt; `--watch` rides the daemon's `subscribe` stream and reprints on every change. Degrades to an empty line (exit 0) when no daemon is reachable.
- Branch convention `<type>/#<issue>-<description>` by default; overridable per repo. `[aliases]` mirror `git config` aliases; `gwm commit-prefix` + an opt-in `commit-msg` hook drive the Gitmoji + Conventional Commits convention.
- Configurable launchers for the `l` (git TUI) and `r` / `R` (review) keybindings.
- TUI personalisation: role-based `[theme]` presets (`catppuccin`, `gruvbox`, `tokyo-night`, `claude-dark`), a remappable `[tui.keys]` keymap with multi-key chords and rebindable per-context modal keys (all editable live from the Settings panel's Keys tab), a `:` command palette, and a responsive sidebar.
- Embedded PTY overlays: `l` / `L` open lazygit and `o` / `O` open a native `$SHELL` session inside the TUI; the Working Tree pane renders `git status` as a nerd-font file tree, and the Issue/PR section surfaces the linked PR's overall CI state.
- Safety nets: `--dry-run` on `gwm remove` / `gwm prune`, plus `gwm undo` / `gwm history` backed by an operation journal.
- First-class GitHub issue / PR linking: branches matching the naming convention auto-link to their issue; PRs are auto-detected from `gh` when not explicitly linked. Declarative `[[labels]]` / `[[milestones]]`, `gwm new` (issue → worktree), `gwm pr` (templated PR body), and `gwm review <PR#>`, which materialises an existing (or fork) GitHub PR into an isolated worktree (fetch + link; bootstrap and lifecycle hooks are opt-in via `--bootstrap`, off by default since the PR's code is untrusted).
- `gwm sync` fetches a worktree's upstream and rebases (or merges) onto it, conflict-safe.
- Fleet chores across worktrees: `gwm exec [<slug>…] -- <cmd>` runs a command in each worktree sequentially (everything after `--` forwarded verbatim, `✓` / `✗` per-worktree rollup, non-zero exit if any failed), and `gwm clean [<slug>…]` reports reclaimable `target/` / `node_modules/` / `dist/` / `build/` directories, report-only until you pass `--yes` (which only deletes git-ignored dirs).
- [TOFU trust ledger](/configuration/trust-ledger) on `.gwm.toml`: the first `gwm create` / `gwm bootstrap` on a repo prompts before executing any bootstrap command line. `--allow-bootstrap` / `GWM_ALLOW_BOOTSTRAP=1` for CI bypass.
- Install via `cargo install gwm-cli`, `cargo binstall gwm-cli` (prebuilt archives, no toolchain), Homebrew, or Nix.

## documentation map

| Section                                            | Read this when …                                                              |
|:---------------------------------------------------|:------------------------------------------------------------------------------|
| [Getting Started](/getting-started)                | you want to install gwm and create your first worktree                        |
| [TUI](/tui)                                        | you live in the ratatui interface: keymap, sidebar, launchers, filter        |
| [CLI](/cli)                                        | you script gwm from shells, CI jobs, or `gh` aliases                          |
| [Configuration](/configuration)                    | you're writing or extending `.gwm.toml`: bootstrap, guards, predicates       |
| [Integrations](/integrations)                      | you wire gwm with `gh`, `lazygit`, Homebrew, Nix, or `gwm doctor` in CI       |
| [Development](/development)                        | you're contributing: test layout, conventions, dev shell                     |
| [Roadmap](/roadmap)                                | you want to know what ships next                                              |

## the 30-second tour

```bash
# install
cargo install gwm-cli
# or: cargo binstall gwm-cli        # prebuilt archive, no Rust toolchain
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

The bash version (`tools/worktree-manager.sh` in some of our repos) was tied to one project's stack and one team's incident history. `gwm` keeps the lessons (anti-RDS guards, `.env.testing` copies, post-create hooks) and makes them configurable per repo. One binary, same behaviour everywhere.

The full background lives in [the changelog](/development#changelog) and in the issue-tracker history at [github.com/kbrdn1/gwm-cli](https://github.com/kbrdn1/gwm-cli).
