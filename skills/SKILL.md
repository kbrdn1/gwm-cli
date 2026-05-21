---
name: gwm
description: Manage git worktrees across any repository with the `gwm` Rust binary (CLI + ratatui TUI). Use when the user asks to create / list / remove / bootstrap worktrees, mentions `gwm`, `gwq`, `git worktree`, or asks about per-repo `.gwm.toml` config (branch conventions, file copies, regex guards on `.env`, shell hooks like composer/npm install, no-symlink invariants). Replaces project-specific `tools/worktree-manager.sh` bash wrappers. Triggers on "worktree", "gwm", "gwm create", "gwm list", "gwm bootstrap", ".gwm.toml", "bootstrap worktree", "feat/#", "fix/#".
allowed-tools: Bash, Read, Edit, Write
---

# gwm — git worktree manager (Rust CLI + TUI)

Single-binary Rust tool that manages git worktrees with `libgit2`, a ratatui TUI, and a declarative per-repo bootstrap (`.gwm.toml`). Replaces project-specific bash wrappers (`tools/worktree-manager.sh`-style) with one portable binary that works in any git repo.

Source: https://github.com/kbrdn1/gwm-cli

## When to use this skill

- User runs or asks about `gwm <subcommand>` (init / list / create / remove / path / bootstrap / prune / types).
- User opens the TUI by running `gwm` alone in a repo.
- User mentions `.gwm.toml` (per-repo config) or any of its sections: `[worktree]`, `[[bootstrap.copy]]`, `[[bootstrap.guard]]`, `[[bootstrap.no_symlink]]`, `[[bootstrap.command]]`.
- User wants to migrate a `tools/worktree-manager.sh` or `gwq`-based workflow to `gwm`.
- User asks how to set up the AWS RDS guard, the safe `.env.testing` fallback, or the no-symlink invariant for `vendor/` / `node_modules/`.
- User asks about the branch convention `<type>/#<issue>-<desc>` or its overrides.

## Prerequisites

```bash
command -v gwm           # required — installed by `cargo install --path .` from the gwm-cli repo
command -v cargo         # required at install time (1.80+ recommended)
command -v git           # required at runtime
```

`gwm` vendors `libgit2`, so no system git2 lib is needed. The binary is self-contained once compiled.

## Install (from source)

```bash
git clone https://github.com/kbrdn1/gwm-cli.git
cd gwm-cli
cargo install --path .         # → ~/.cargo/bin/gwm
gwm --version
```

Prebuilt releases (Linux x86_64/aarch64, macOS Intel/Apple Silicon, Windows): https://github.com/kbrdn1/gwm-cli/releases.

## Default conventions

| What                | Default                              | Override                       |
|:--------------------|:-------------------------------------|:-------------------------------|
| Branch name         | `<type>/#<issue>-<desc>`             | `.gwm.toml` `branch_pattern`   |
| Worktree dir name   | `<type>-<issue>-<desc>`              | `.gwm.toml` `path_pattern`     |
| Worktree base       | `~/cc-worktree/<repo>/`              | `.gwm.toml` `base`             |
| Bootstrap           | none (just `git worktree add`)       | `.gwm.toml` `[bootstrap.*]`    |

Branch types: `feat`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`.

Placeholders in patterns: `{home}`, `{repo}`, `{type}`, `{issue}`, `{desc}`.

## CLI reference

```bash
gwm                          # opens the TUI in the current repo
gwm init                     # write .gwm.toml in the repo root (refuses overwrite)
gwm types                    # list supported branch types

gwm create <type> <issue> <desc>          # create branch + worktree + bootstrap
gwm create feat 123 "user-authentication"
gwm create feat 123 foo --no-bootstrap    # skip the .gwm.toml stages

gwm list                                  # list worktrees of the current repo
gwm path <pattern>                        # print path (fuzzy match) → use $(gwm path auth)
gwm bootstrap                             # re-run bootstrap on cwd worktree
gwm bootstrap <pattern>                   # ...or on a named worktree
gwm remove <pattern>                      # remove (fuzzy). Keeps the branch.
gwm remove <pattern> --delete-branch      # also drop the local branch
gwm prune                                 # clean stale .git/worktrees entries
```

## Status column

The TUI table and `gwm list` both expose a `STATUS` column:

| label              | meaning                                                         | colour       |
|:-------------------|:----------------------------------------------------------------|:-------------|
| `clean`            | no upstream, no changes                                          | green        |
| `✓ synced`         | upstream set, no ahead/behind, no local changes                  | green        |
| `● dirty`          | uncommitted changes (working tree or index)                      | yellow       |
| `↑N`               | N commits ahead of upstream                                      | cyan         |
| `↓M`               | M commits behind upstream                                        | yellow       |
| `↑N ↓M`            | both                                                             | yellow       |
| `● dirty ↑N`       | combined indicators                                              | yellow       |
| `locked`           | linked worktree is locked (git2 reports it)                      | magenta      |
| `prunable`         | working tree dir is missing — run `gwm prune`                    | red          |
| `unknown`          | status couldn't be computed (detached HEAD, IO error, etc.)      | dark gray    |

## TUI key map

| Key         | Action                                                                          |
|:------------|:--------------------------------------------------------------------------------|
| `↑` / `k`   | previous worktree (scrolls the sidebar when it has focus)                       |
| `↓` / `j`   | next worktree (scrolls the sidebar when it has focus)                           |
| `gg`        | jump to the first worktree                                                      |
| `G`         | jump to the last worktree                                                       |
| `n`         | new worktree form (type ↑/↓, Tab between fields, Enter on desc submits)         |
| `d`         | delete (confirm `y`)                                                            |
| `b`         | re-run bootstrap on selected                                                    |
| `o`         | open worktree dir in OS file manager (`open` / `xdg-open` / `explorer`)         |
| `l`         | run the configured `[git_tui]` launcher (default `lazygit -p <selected-worktree>` fullscreen) |
| `R`         | run the configured `[review]` launcher against the resolved base (issue #75)    |
| `v`         | toggle the git details sidebar (auto-hidden when terminal width < 120 cols)    |
| `Tab`       | swap focus between the worktree list and the sidebar                            |
| `f`         | refresh worktree list (also accepts `r` for muscle memory)                      |
| `F`         | refresh GitHub issue/PR status via `gh`                                         |
| `p`         | toggle "delete branch on remove"                                                |
| `Enter`     | show path in status bar                                                         |
| `?`         | help overlay                                                                    |
| `q` / `Esc` | quit                                                                            |
| `Ctrl-C`    | force quit                                                                      |

## Details sidebar

When the terminal width is ≥ 120 columns and the sidebar is open (default ON, toggle with `v`), the right pane shows a lazyssh-style details panel for the selected worktree:

- **Basic Settings**: branch, path, head (short OID), main / locked / prunable flags, branch status.
- **Recent commits**: `git log --oneline -n 10` (shells out to `git`).
- **Working tree**: `git status --short` (`✓ clean` when empty).
- **Commands**: keybindings cheat-sheet inside the panel.

`Tab` swaps focus between the worktree list and the sidebar. `j` / `k` (and arrows) scroll the focused panel. The focused panel's border turns cyan.

## Configurable launchers (`l` git_tui · `R` review) — issue #75

Two TUI keybindings share the same mini-API: take a `command` template from `.gwm.toml`, substitute placeholders, split with `shell-words`, and exec it with `cwd = <selected-worktree>`.

| Key | Section     | Default                       | Placeholders                          | Default `fullscreen` |
|:----|:------------|:------------------------------|:--------------------------------------|:---------------------|
| `l` | `[git_tui]` | `lazygit -p {path}`           | `{path}`                              | `true`               |
| `R` | `[review]`  | _(inert until configured)_    | `{base} {head} {path} {diff}`         | `false`              |

`fullscreen = true` suspends the gwm TUI for a TUI-style takeover (same recipe as the pre-issue-#75 `l` → lazygit flow); `fullscreen = false` runs the command in the background, captures stderr's first line, and lands it on the status bar. The `{diff}` placeholder is **lazy** — gwm only shells out to `git diff {base}..{head}` (into a tempfile) when the template references it.

### `[review]` base resolution chain (for `{base}`)

1. `branch.<name>.merge` (the branch's upstream, if any).
2. `branch.<name>.gwm-base` (recorded by `gwm create` so the parent ref survives `git push -u`).
3. `[review].default_base` from `.gwm.toml`.
4. `"dev"` (gwm's project convention).
5. `"main"` (universal git default).

### `[review].tool` built-in presets

Sugar over `command + fullscreen`. Setting both `command` and `tool` makes `command` win (the TUI surfaces the shadow on next render).

| `tool = "X"` | Resolves to                                              | `fullscreen` default |
|:-------------|:---------------------------------------------------------|:---------------------|
| `lumen`      | `lumen diff {base}..{head}`                              | true (TUI)           |
| `claude`     | `claude --print 'review the diff {base}..{head}'`        | false                |
| `codex`      | `codex review {base}..{head}`                            | false                |
| `aider`      | `aider --message 'review {base}..{head}'`                | true (TUI)           |
| `gh`         | `gh pr view --web`                                       | false                |

### Worked snippets

```toml
# Switch the `l` key to gitui.
[git_tui]
command = "gitui -d {path}"

# Review with lumen (TUI), skip when nothing to review.
[review]
tool = "lumen"
skip_when_no_changes = true
# default_base = "dev"   # optional pin overriding the auto chain

# Or a free-form shell line — `command` always wins over `tool`.
[review]
command = "my-review-bot --diff-file {diff} --owner kbrdn1"
fullscreen = false
```

### `gwm doctor` integration

A configured `[review]` / `[git_tui]` binary that is not on `$PATH` surfaces as **Warning** (exit code `1`), never **Failed** (exit code `2`) — both launchers are opt-in, so a CI pre-commit hook gated on `gwm doctor` keeps passing when the only red flag is a missing local-only tool.

## `.gwm.toml` schema

Drop this at the repo root (or use `gwm init` for the annotated example).

```toml
[worktree]
base           = "{home}/cc-worktree/{repo}"
path_pattern   = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"

# File copies main → worktree (run in order).
[[bootstrap.copy]]
from = ".env.testing"
to   = ".env.testing"
required = true               # if source missing AND no fallback → fail
fallback = "inline"           # "inline" (use [bootstrap.fallback.<key>]) | "abort" | "skip" (default)

[[bootstrap.copy]]
from = ".env"
to   = ".env"
required = false
guards = ["no-aws-rds"]       # references guard names

# Regex guards on copied files.
[[bootstrap.guard]]
name           = "no-aws-rds"
deny_patterns  = ["amazonaws\\.com", "\\.rds\\."]
on_match       = "seed-from-example"   # or "abort"
example_file   = ".env.example"

# Inline fallback when a required source is missing.
# Key is the destination filename normalized: ".env.testing" → "env_testing".
[bootstrap.fallback.env_testing]
target  = ".env.testing"
content = """
APP_ENV=testing
DB_CONNECTION=sqlite
DB_DATABASE=:memory:
"""

# Symlinks to refuse (inherited from main).
[[bootstrap.no_symlink]]
path = "vendor"

[[bootstrap.no_symlink]]
path = "node_modules"

# Shell commands after copies.
[[bootstrap.command]]
name = "composer install"
run  = "composer install --no-interaction --prefer-dist"
when = "file_exists:composer.json"     # only predicate supported today
env  = { COMPOSER_IGNORE_PLATFORM_REQ = "ext-imagick" }

[[bootstrap.command]]
name = "direnv allow"
run  = "direnv allow ."
when = "file_exists:.envrc"
```

## Bootstrap report

Every create / bootstrap run prints (or shows in the TUI) a per-step report:

| Sigil | Status   | Meaning                                                    |
|:------|:---------|:-----------------------------------------------------------|
| ✓     | Ok       | step ran cleanly                                           |
| ·     | Skipped  | conditional not met / dest already exists / optional miss  |
| !     | Warning  | guard fired with fallback, or symlink removed              |
| ✗     | Failed   | required step couldn't proceed                             |

A run with any ✗ should be inspected before testing inside the worktree.

## Common workflows

### Migrating from `tools/worktree-manager.sh` + gwq

1. Install: `cargo install --path /path/to/gwm-cli`
2. In each repo: `gwm init` → edit `.gwm.toml` with the project-specific copies / guards / commands.
3. Replace `./tools/worktree-manager.sh create feat 123 foo` with `gwm create feat 123 foo`.
4. Replace `gwq list` / `gwq remove` / `gwq prune` with `gwm list` / `gwm remove` / `gwm prune`.
5. Drop `gwq` from the repo's prerequisites (gwm is self-contained).

### Setting up the AWS RDS guard (Laravel / production-safe)

`.gwm.toml`:

```toml
[[bootstrap.copy]]
from = ".env.testing"
to   = ".env.testing"
required = true
fallback = "inline"

[bootstrap.fallback.env_testing]
target  = ".env.testing"
content = """
APP_ENV=testing
APP_KEY=
DB_CONNECTION=sqlite
DB_DATABASE=:memory:
CACHE_STORE=array
QUEUE_CONNECTION=sync
MAIL_MAILER=array
SESSION_DRIVER=array
BCRYPT_ROUNDS=4
"""

[[bootstrap.copy]]
from = ".env"
to   = ".env"
required = false
guards = ["no-prod-rds"]

[[bootstrap.guard]]
name = "no-prod-rds"
deny_patterns = ["amazonaws\\.com", "\\.rds\\.", "prod\\.flippad\\.com"]
on_match = "seed-from-example"
example_file = ".env.example"

[[bootstrap.no_symlink]]
path = "vendor"

[[bootstrap.command]]
name = "composer install"
run  = "composer install --no-interaction --prefer-dist"
when = "file_exists:composer.json"
env  = { COMPOSER_IGNORE_PLATFORM_REQ = "ext-imagick" }
```

### Node project (npm / pnpm / bun)

```toml
[[bootstrap.copy]]
from = ".env"
to   = ".env"
required = false

[[bootstrap.no_symlink]]
path = "node_modules"

[[bootstrap.command]]
name = "install deps"
run  = "npm ci"
when = "file_exists:package-lock.json"
```

### Quick create + cd

```bash
gwm create feat 42 cool-thing
cd "$(gwm path cool-thing)"
```

Wrap as a shell function:

```bash
gwmcd() { cd "$(gwm path "$1")"; }
gwmcd cool-thing
```

## Architecture (for skill agents asked to extend gwm)

```
src/
├── lib.rs               # public re-exports — tests import these
├── main.rs              # bin entry, dispatches to cli::run
├── error.rs             # GwmError (thiserror) + Result alias
├── config.rs            # serde TOML → Config (worktree, bootstrap)
├── naming.rs            # BranchSpec, kebab(), parse_branch()
├── worktree.rs          # discover_repo, list, add, remove, prune, find_fuzzy (all via git2)
├── bootstrap.rs         # run(BootstrapCtx) → BootstrapReport
├── cli.rs               # clap subcommands + handlers
└── tui/
    ├── mod.rs           # crossterm event loop
    ├── app.rs           # App state, transitions
    └── ui.rs            # ratatui drawing
```

Tests under `tests/` mirror this layout. TDD bar: any new behaviour ships with a matching test file or new assertions in an existing one.

## Differences vs. the bash + gwq stack

| Capability                  | bash + gwq            | gwm                            |
|:----------------------------|:----------------------|:-------------------------------|
| worktree engine             | `gwq` CLI external    | `libgit2` vendored             |
| bootstrap                   | hardcoded shell       | declarative TOML + hooks       |
| portability across repos    | per-project script    | one binary + per-repo config   |
| TUI                         | linear bash menu      | full ratatui screen            |
| anti-RDS guard              | hardcoded `grep`      | configurable regex deny-list   |
| tests                       | 0                     | 71 (config / naming / bootstrap / worktree / TUI / CLI) |
| install                     | `chmod +x` per repo   | `cargo install --path .`       |

## Troubleshooting

**`error: not inside a git repository`** — run `gwm` from inside a repo or pass a path explicitly.

**`gwm create` fails with "branch ... already exists"** — the branch was created in a previous run that didn't finish. `git branch -D <branch>` or pick another issue number, then retry.

**`gwm remove` reports "pattern '...' is ambiguous"** — multiple worktrees match the substring. Pass a more specific pattern or the exact dir name from `gwm list`.

**Bootstrap step shows ✗ on a `.env` copy with guard match + no example_file** — either set `example_file` in the guard, or change `on_match` to `"abort"` and rely on `.env.example`. Either way, the source `.env` is never copied past a guard match.

**TUI shows `(prunable)` next to a worktree** — its working dir was deleted out-of-band. Run `gwm prune` (or hit `r` in the TUI after manual cleanup).

**`cargo install --path .` fails to build libgit2** — install a C toolchain (`xcode-select --install` on macOS, `build-essential` on Debian/Ubuntu). The `git2` crate uses `vendored-libgit2` so it builds from source.

**`.env` was copied even though it points to prod** — the guard's regex didn't match. Test it with `echo $YOUR_HOST | grep -E '<pattern>'`. Regex syntax is Rust `regex` crate (PCRE-like, no lookaround).

## Quick reference card

```
gwm                        # TUI
gwm init                   # scaffold .gwm.toml
gwm create <t> <#> <desc>  # create + bootstrap
gwm list                   # list worktrees
gwm path <pat>             # print path
gwm bootstrap [pat]        # re-run bootstrap
gwm remove <pat> [-b]      # remove (-b drops branch)
gwm prune                  # clean stale refs
gwm types                  # show branch types
```

## Related

- Repo: https://github.com/kbrdn1/gwm-cli
- Bash predecessor: `tools/worktree-manager.sh` (skill: `worktree-wrapper`) — `gwm` is the multi-repo replacement.
- Naming convention: `CONTRIBUTING.md` (per repo) — matches `gwm` defaults.
