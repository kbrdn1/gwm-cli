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
gwm list --format=names                     # one worktree name per line (for shell completion)
gwm path auth                               # print the resolved path (use $(gwm path ...))
gwm cd auth                                 # same — primitive for the `gcd` wrapper
gwm bootstrap                               # re-run bootstrap on the CWD worktree
gwm bootstrap auth                          # ...or on a named one
gwm remove auth                             # remove (fuzzy match) — keeps the branch
gwm remove auth --delete-branch             # remove + drop the branch
gwm prune                                   # clean stale .git/worktrees entries
gwm completions zsh                         # print a zsh / bash / fish / powershell / elvish script
gwm shell-init zsh                          # print a `gcd <pattern>` shell wrapper (one-line cd)
gwm doctor                                  # diagnose config + env + worktree state (exit 0/1/2)
```

### diagnose your setup

`gwm doctor` runs a series of cheap checks and reports each with `✓ / ! / ✗`, then exits `0` (all green), `1` (any warning), or `2` (any failure) — so it can be wired into CI or a pre-commit hook.

```bash
$ gwm doctor
✓ .gwm.toml parses
    /path/to/repo/.gwm.toml parses cleanly
✓ guard references resolve
    2 guard reference(s) resolve
✓ `when` predicates supported
✓ external binaries on PATH
    3/3 binaries found
! no prunable worktrees
    1 prunable entry: feat-12-old
    → run `gwm prune` to clear them
! no orphan gwm branches
    1 orphan branch(es): feat/#23-stale
    → git branch -d feat/#23-stale
✓ base directory writable
```

Checks performed:

1. **`.gwm.toml` parses** — Ok if it parses (or absent, defaults assumed); Failed if the TOML is broken.
2. **guard references resolve** — every `[[bootstrap.copy]].guards = [...]` points at an existing `[[bootstrap.guard]]`.
3. **`when` predicates supported** — every `[[bootstrap.command]].when` uses a known keyword prefix (currently only `file_exists:`).
4. **external binaries on PATH** — `lazygit` (TUI `l` keybinding), `direnv` (only if `.envrc` exists), and the first executable token of every `[[bootstrap.command]].run`.
5. **no prunable worktrees** — `.git/worktrees/` entries whose working dir was removed manually.
6. **no orphan gwm branches** — local branches matching `<type>/#<issue>-<desc>` (created by `gwm create`) with no worktree. User-managed branches (`main`, `release-*`, `dependabot/...`) are ignored.
7. **base directory writable** — the configured `[worktree].base` exists and is writable, or its parent is (gwm creates the base lazily on first `gwm create`).

### one-line cd into a worktree

The binary itself cannot change the parent shell's directory. `gwm shell-init <shell>` prints a function (`gcd`) that wraps `gwm cd` — `eval` it in your rc file and use `gcd <pattern>` as a one-liner:

```bash
# zsh
echo 'eval "$(gwm shell-init zsh)"' >> ~/.zshrc
# bash
echo 'eval "$(gwm shell-init bash)"' >> ~/.bashrc
# fish (also persist by adding to ~/.config/fish/config.fish)
gwm shell-init fish | source
# PowerShell (current session)
Invoke-Expression (& gwm shell-init powershell | Out-String)
# PowerShell (persist via $PROFILE)
gwm shell-init powershell | Out-File -Append -Encoding utf8 $PROFILE
```

Then:

```bash
gcd auth   # → cd $(gwm cd auth) → e.g. ~/cc-worktree/myrepo/feat-99-user-authentication
```

`gcd` propagates the exit code from `gwm cd` and never attempts the `cd` if the lookup failed (no match, ambiguous pattern, not in a git repo).

### shell completions

`gwm completions <shell>` prints a static completion script (generated from the live clap argument tree, so it never drifts from the actual subcommands). Supported shells: `zsh`, `bash`, `fish`, `powershell`, `elvish`.

```bash
# zsh — drop into the first writable fpath entry
gwm completions zsh > "${fpath[1]}/_gwm"

# bash — system-wide
gwm completions bash | sudo tee /etc/bash_completion.d/gwm > /dev/null
# bash — per-user
gwm completions bash > ~/.local/share/bash-completion/completions/gwm

# fish
gwm completions fish > ~/.config/fish/completions/gwm.fish

# PowerShell — load into the current session (ephemeral)
gwm completions powershell | Out-String | Invoke-Expression
# PowerShell — persist by appending to $PROFILE
gwm completions powershell | Out-File -Append -Encoding utf8 $PROFILE
```

For dynamic completion of worktree names (the `<pattern>` arg of `path` / `remove` / `bootstrap`), wire a custom completer to `gwm list --format=names` — e.g. in zsh:

```zsh
_gwm_worktrees() { compadd $(gwm list --format=names 2>/dev/null) }
compdef _gwm_worktrees gwm-path gwm-remove gwm-bootstrap
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
| tests                               | none                 | 81 tests (config / naming / bootstrap / worktree / TUI / CLI) |

## development

```bash
cargo build              # debug build
cargo test               # 81 tests
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

The full roadmap (with grouped categories and per-item issue links) lives in [`ROADMAP.md`](ROADMAP.md). Highlights for the next minor:

- Shell completions ([#18](https://github.com/kbrdn1/gwm-cli/issues/18)), `gwm cd` + `shell-init` ([#19](https://github.com/kbrdn1/gwm-cli/issues/19)), `gwm doctor` ([#20](https://github.com/kbrdn1/gwm-cli/issues/20)), TUI fuzzy filter ([#21](https://github.com/kbrdn1/gwm-cli/issues/21)).

Contributions welcome — open a [feature request issue](.github/ISSUE_TEMPLATE/feature_request.yml) or pick an item from [`ROADMAP.md`](ROADMAP.md).

## license

MIT — see [LICENSE.md](LICENSE.md).

## related docs

- [`CHANGELOG.md`](CHANGELOG.md)
- [`CONTRIBUTING.md`](CONTRIBUTING.md)
- [`ROADMAP.md`](ROADMAP.md)
- [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- [`.github/LABELS.md`](.github/LABELS.md)
- [`examples/gwm.toml.example`](examples/gwm.toml.example)
