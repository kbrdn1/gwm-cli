# gwm — git worktree manager

[![ci](https://github.com/kbrdn1/gwm-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/kbrdn1/gwm-cli/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/kbrdn1/gwm-cli?display_name=tag&sort=semver)](https://github.com/kbrdn1/gwm-cli/releases)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.md)
[![rust](https://img.shields.io/badge/rust-1.80%2B-orange?logo=rust)](https://www.rust-lang.org/)

Rust CLI + ratatui TUI to manage git worktrees across projects. Native `libgit2` (no `gwq` / `git` CLI dependency), per-repo configurable bootstrap (file copies, regex guards, shell hooks), single binary, portable.

Born as a rewrite of a project-specific `tools/worktree-manager.sh` script — the bash version was tied to Laravel/PHP and one repo's incident history. `gwm` keeps the lessons, makes them configurable, and works the same way in every repo.

## features

- **Native worktree ops** via `libgit2` (vendored). `git worktree add/list/remove/prune` without shelling out.
- **CLI + TUI**: `gwm <subcommand>` for scripts and hooks, `gwm` alone opens a ratatui interface.
- **Per-repo config** in `.gwm.toml`: branch / path conventions, file copies, regex guards, shell hooks, no-symlink invariants.
- **Branch convention**: `<type>/#<issue>-<description>` by default (`feat`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`). Overridable per repo.
- **Safety guards**: deny-list regexes on copied files (e.g. refuse to inherit a `.env` pointing at AWS RDS). Pluggable actions: `abort` or `seed-from-example`.
- **Bootstrap hooks**: shell commands gated by `file_exists:` predicates and arbitrary `env` injection.
- **Fuzzy lookup**: `gwm remove auth` matches `feat-123-user-authentication` if unambiguous.
- **Branch status column**: dirty / clean / `↑N ↓M` vs upstream, color-coded in TUI and CLI output.

## install

### from source

```bash
git clone https://github.com/kbrdn1/gwm-cli.git
cd gwm-cli
cargo install --path .
```

The binary lands in `~/.cargo/bin/gwm`.

### prebuilt binaries

Releases at <https://github.com/kbrdn1/gwm-cli/releases> ship Linux (x86_64 + aarch64), macOS (Intel + Apple Silicon), and Windows binaries with `.sha256` sidecars.

## usage

### TUI (interactive)

```bash
cd <a-git-repo>
gwm                # opens the TUI on the current repo
```

Key bindings:

| Key         | Action                                                                          |
|:------------|:--------------------------------------------------------------------------------|
| `↑` / `k`   | previous worktree (scrolls the sidebar when it has focus)                       |
| `↓` / `j`   | next worktree (scrolls the sidebar when it has focus)                           |
| `gg`        | jump to the first worktree                                                      |
| `G`         | jump to the last worktree                                                       |
| `n`         | new worktree (form: type → issue → description)                                 |
| `d`         | delete selected (confirm `y`)                                                   |
| `b`         | re-run bootstrap on the selected worktree                                       |
| `o`         | open the worktree dir in the OS file manager (`open` / `xdg-open` / `explorer`) |
| `l`         | launch `lazygit -p <selected-worktree>` fullscreen; resume the TUI on exit      |
| `v`         | toggle the git details sidebar (auto-hidden when terminal width < 120 cols)     |
| `Tab`       | swap focus between the worktree list and the sidebar                            |
| `r`         | refresh                                                                         |
| `p`         | toggle "delete branch on remove"                                                |
| `Enter`     | show selected path in status bar                                                |
| `?`         | help overlay                                                                    |
| `q` / `Esc` | quit                                                                            |

### details sidebar

When the terminal is at least **120 columns wide** and the sidebar is enabled (default ON, toggle with `v`), the right pane shows a lazyssh-style details panel for the currently selected worktree:

- **Basic Settings** — branch, path, head (short OID), main / locked / prunable flags, branch status.
- **Recent commits** — `git log --oneline -n 10`.
- **Working tree** — `git status --short` (`✓ clean` when empty).
- **Commands** — keybindings cheat-sheet.

Press `Tab` to focus the sidebar; `j` / `k` (or arrow keys) then scroll it instead of moving the worktree selection. The focused panel's border turns cyan.

### lazygit integration

Press `l` on any worktree to suspend the TUI and open [`lazygit`](https://github.com/jesseduffield/lazygit) fullscreen on that worktree (`lazygit -p <path>`). When you quit lazygit, the gwm TUI is restored exactly where you left it. If `lazygit` is not on `$PATH`, the status bar reports it without crashing.

### CLI

```bash
gwm init                                    # write .gwm.toml in the current repo
gwm types                                   # list valid branch types
gwm create feat 123 "user-authentication"   # → feat/#123-user-authentication
gwm create feat 123 foo --no-bootstrap      # skip the .gwm.toml bootstrap stages
gwm list                                    # list all worktrees of the current repo
gwm path auth                               # print the resolved path (use $(gwm path ...))
gwm bootstrap                               # re-run bootstrap on the CWD worktree
gwm bootstrap auth                          # ...or on a named one
gwm remove auth                             # remove (fuzzy match) — keeps the branch
gwm remove auth --delete-branch             # remove + drop the branch
gwm prune                                   # clean stale .git/worktrees entries
```

## configuration

`.gwm.toml` at the repo root, see `examples/gwm.toml.example` for the annotated full version.

```toml
[worktree]
base         = "{home}/cc-worktree/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"

# file copies main → worktree
[[bootstrap.copy]]
from = ".env.testing"
to   = ".env.testing"
required = true
fallback = "inline"

[[bootstrap.copy]]
from = ".env"
to   = ".env"
required = false
guards = ["no-aws-rds"]

# regex guards on copied files
[[bootstrap.guard]]
name = "no-aws-rds"
deny_patterns = ["amazonaws\\.com", "\\.rds\\."]
on_match      = "seed-from-example"
example_file  = ".env.example"

# inline fallback when a required source is missing
[bootstrap.fallback.env_testing]
target  = ".env.testing"
content = """
APP_ENV=testing
DB_CONNECTION=sqlite
DB_DATABASE=:memory:
"""

# refuse to inherit symlinks (vendor/, node_modules/)
[[bootstrap.no_symlink]]
path = "vendor"

# post-copy commands
[[bootstrap.command]]
name = "composer install"
run  = "composer install --no-interaction --prefer-dist"
when = "file_exists:composer.json"
env  = { COMPOSER_IGNORE_PLATFORM_REQ = "ext-imagick" }
```

Available placeholders: `{home}`, `{repo}`, `{type}`, `{issue}`, `{desc}`. Tilde (`~/...`) is also expanded.

### defaults without `.gwm.toml`

| Setting          | Default                          |
|:-----------------|:---------------------------------|
| `base`           | `{home}/cc-worktree/{repo}`      |
| `path_pattern`   | `{type}-{issue}-{desc}`          |
| `branch_pattern` | `{type}/#{issue}-{desc}`         |
| bootstrap        | none — just `git worktree add`   |

### supported branch types

`feat`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`.

## differences vs. the original bash script

| Capability                          | bash + gwq           | gwm                              |
|:------------------------------------|:---------------------|:---------------------------------|
| worktree engine                     | `gwq` (CLI external) | `libgit2` (vendored)             |
| bootstrap                           | hardcoded in bash    | declarative in `.gwm.toml`       |
| multi-repo portability              | per-project script   | one binary, per-repo config      |
| TUI                                 | linear bash menu     | full ratatui screen              |
| anti-RDS guard                      | hardcoded            | configurable regex deny-list     |
| tests                               | none                 | 71 tests (config / naming / bootstrap / worktree / TUI / CLI) |

## development

```bash
cargo build              # debug build
cargo test               # 71 tests
cargo fmt && cargo clippy -- -D warnings
cargo run                # opens TUI in the current repo
cargo install --path .   # install locally
```

All tests live under `tests/`:

```
tests/
├── common/                       # shared helpers (init_repo, paths_equal)
├── config_tests.rs               # .gwm.toml parsing + write_default
├── naming_tests.rs               # kebab, branch validation, parse roundtrip
├── bootstrap_tests.rs            # copies / guards / no-symlink / commands
├── worktree_integration.rs       # git2 add/list/remove/prune
├── tui_app_tests.rs              # state transitions (ratatui-free)
└── cli_binary.rs                 # assert_cmd end-to-end
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the branch / commit / PR conventions.

## roadmap

- `--watch` mode (gwq parity)
- TUI fuzzy filter on the worktree list
- Pluggable `when` predicates beyond `file_exists:`
- Optional per-worktree env file (`.gwm.env`) sourced before commands
- Per-OS path overrides in `.gwm.toml`

Contributions welcome — open a [feature request issue](.github/ISSUE_TEMPLATE/feature_request.yml).

## license

MIT — see [LICENSE.md](LICENSE.md).

## related docs

- [`CHANGELOG.md`](CHANGELOG.md)
- [`CONTRIBUTING.md`](CONTRIBUTING.md)
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- [`.github/LABELS.md`](.github/LABELS.md)
- [`examples/gwm.toml.example`](examples/gwm.toml.example)
