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
- **Bootstrap hooks**: shell commands gated by composable `when` predicates (`file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!`, `&&`, `||`) and arbitrary `env` injection.
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

### via Homebrew (macOS)

```bash
brew tap kbrdn1/tap
brew install gwm
```

The formula lives at [`kbrdn1/homebrew-tap`](https://github.com/kbrdn1/homebrew-tap) (`Formula/gwm.rb`) and is refreshed automatically on every stable release of `gwm-cli` by the `homebrew-tap-update` job in [`release.yml`](.github/workflows/release.yml). The canonical formula source lives at [`packaging/homebrew/gwm.rb.template`](packaging/homebrew/gwm.rb.template) — pre-release tags (`-rc.N`, `-alpha.N`, `-beta.N`) are filtered out so `brew install gwm` always points at a stable build.

### prebuilt binaries

Releases at <https://github.com/kbrdn1/gwm-cli/releases> ship Linux (x86_64 + aarch64), macOS (Intel + Apple Silicon), and Windows binaries with `.sha256` sidecars.

### via Nix flake

A `flake.nix` lives at the repo root. With flakes enabled:

```bash
# one-shot run, no clone
nix run github:kbrdn1/gwm-cli -- list

# install into your profile
nix profile install github:kbrdn1/gwm-cli

# in a NixOS / nix-darwin config, via the overlay
nixpkgs.overlays = [ inputs.gwm.overlays.default ];
environment.systemPackages = [ pkgs.gwm ];
```

The package is built via `rustPlatform.buildRustPackage` and pins `Cargo.lock`; `git2`'s `vendored-libgit2` feature keeps the closure free of system libgit2.

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
| `d`         | delete selected (confirm `y` · countdown when `p` is armed — see below)         |
| `b`         | re-run bootstrap on the selected worktree                                       |
| `o`         | open the worktree per [`[tui.open]`](#open-dispatch) — `shell` (default) / `editor` / `finder` |
| `y`         | yank the selected worktree's path to the system clipboard (pbcopy / wl-copy / xclip / clip) |
| `l`         | launch `lazygit -p <selected-worktree>` fullscreen; resume the TUI on exit      |
| `v`         | toggle the git details sidebar (auto-hidden when terminal width < 120 cols)     |
| `Tab`       | swap focus between the worktree list and the sidebar                            |
| `/`         | open the fuzzy filter bar (`Enter` confirms sticky filter · `Esc` clears)       |
| `r`         | refresh                                                                         |
| `p`         | toggle "delete branch on remove"                                                |
| `Enter`     | show selected path in status bar                                                |
| `?`         | help overlay                                                                    |
| `q`         | quit                                                                            |
| `Esc`       | clear a sticky filter if any, otherwise quit                                    |

### details sidebar

When the terminal is at least **120 columns wide** and the sidebar is enabled (default ON, toggle with `v`), the right pane shows a lazygit-style details panel for the currently selected worktree:

- **Header** — `● <worktree-name>`, with the `●` colour tracking the linked PR / issue state (open=green, draft=gray, merged=magenta, closed=red, neutral=darkgray when nothing's linked).
- **Basic Settings** — branch (coloured by status: synced=green, ahead/behind=yellow, dirty=red, unpublished=magenta, unknown=darkgray), path, head (short OID), **Created** (branch age in compact `2d` / `3w` / `1M` form, coloured by freshness), main / locked / prunable flags, branch status.
- **Recent commits** — `git log --oneline -n 10`.
- **Working tree** — `git status --short` (`✓ clean` when empty).
- **Commands** — keybindings cheat-sheet.

Press `Tab` to focus the sidebar; `j` / `k` (or arrow keys) then scroll it instead of moving the worktree selection. The focused panel's border turns cyan.

#### open dispatch

The `o` key dispatches on `[tui.open]` in `.gwm.toml`:

```toml
[tui.open]
mode = "shell"          # "shell" (default) | "editor" | "finder"
shell_cmd = ""           # override $SHELL; empty = unset
editor_cmd = "hx"        # override $EDITOR; empty = unset
```

- `shell` (default, **changed in v0.6**): suspend the TUI and spawn `$SHELL` with `cwd` set to the worktree — same lifecycle as `l: lazygit`. Exit the shell, the TUI restores.
- `editor`: suspend the TUI and run `$EDITOR <worktree-path>`.
- `finder`: pre-v0.6 behaviour — hand off to the OS file manager (`open` / `xdg-open` / `explorer`) without suspending the TUI.

Unknown `mode` values are a hard config error at load time, surfaced before the TUI opens.

### fuzzy filter

Press `/` to open an inline filter bar at the bottom of the worktree table. As you type, the table narrows in real time using [`nucleo-matcher`](https://docs.rs/nucleo-matcher) — the same fuzzy engine used by Helix and Zellij. Matches are ranked by how tight the hit is (contiguous substring beats spread-out subsequence), so the most likely candidate sits on top.

```
/                            → filter bar opens
auth                         → table now shows feat-99-user-authentication only
<Enter>                       → filter sticks, navigation back on the table
<Esc>                         → clears filter, full list back
```

The filter is sticky between Enter and Esc: `j` / `k` / `gg` / `G` continue to work on the filtered subset, and the table title shows `worktrees (N/M)` (visible / total). Hit `/` again to re-open the bar and refine the query. `Esc` from the list view clears the sticky filter before it considers quitting, so you can't accidentally quit when you meant to drop the filter.

### confirm-overlay countdown ([#30](https://github.com/kbrdn1/gwm-cli/issues/30))

The `d` confirm overlay has two modes, picked automatically based on whether `p` was pressed earlier in the session:

- **Classic** (`delete branch on remove` is OFF): single keystroke. `y` / `Enter` fires the delete; `n` / `Esc` cancels. Same as the pre-#30 behaviour.
- **Countdown** (`delete branch on remove` is ON): the overlay shows the branch about to disappear plus an `arm` step. `y` / `Enter` *arms* a safety countdown (default 3s, visualised by a progress bar); the actual delete only fires once the bar fills. `Esc` / `n` cancels at any time; pressing `y` again during the countdown disarms it without firing.

The countdown duration is configurable via `[tui].confirm_countdown_secs` in `.gwm.toml`. Accepted range: `0..=5`. Setting it to `0` keeps the classic single-keystroke modal even when `delete branch on remove` is armed; values above `5` are clamped to `5` on read.

```toml
[tui]
# 3s default. Set to 0 to disable the countdown (classic modal even when p is armed).
confirm_countdown_secs = 3
```

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
gwm switch                                  # interactive picker — prints chosen path on stdout
gwm s                                       # short alias for `switch`
gwm bootstrap                               # re-run bootstrap on the CWD worktree
gwm bootstrap auth                          # ...or on a named one
gwm remove auth                             # remove (fuzzy match) — keeps the branch
gwm remove auth --delete-branch             # remove + drop the branch
gwm prune                                   # clean stale .git/worktrees entries
gwm completions zsh                         # print a zsh / bash / fish / powershell / elvish script
gwm shell-init zsh                          # print a `gcd <pattern>` shell wrapper (one-line cd)
gwm tmux auth                               # open the matched worktree in a new tmux window
gwm tmux auth -p                            # ...or in a split of the current pane
gwm zellij auth                             # same, but for zellij — new tab via `--cwd`
gwm zellij auth -p                          # ...or in a new pane of the current tab
gwm doctor                                  # diagnose config + env + worktree state (exit 0/1/2)
gwm link issue 42                           # link an issue to the current worktree
gwm link pr 61                              # link a PR
gwm unlink issue                            # remove the issue link (auto-detect resurfaces)
gwm open issue                              # open the linked issue in the browser
gwm open pr --print-url                     # print the URL on stdout instead of spawning
gwm status                                  # show link + live state via `gh`
gwm status --json                           # machine-readable JSON of the status
```

### Issue / PR linking ([#67](https://github.com/kbrdn1/gwm-cli/issues/67))

Every worktree can be linked to a GitHub issue and / or pull request:

- Branches following `<type>/#<N>-<slug>` are auto-linked to issue `#N` — zero setup.
- Explicit overrides land in `git config branch.<name>.gwm-issue` / `gwm-pr` (local, per-branch, no extra file).
- `gwm status` shells out to `gh issue view` / `gh pr view` to fetch the live state, title, labels, and CI rollup. Without `gh` (or outside a GitHub remote), only the local link is shown.

In the TUI:

- `O` — open menu (`i` open issue, `p` open PR) in the browser.
- `L` — link prompt (`i` issue, `p` pr, then type the number).
- `R` — refresh the GitHub status (synchronous fetch).
- The right details panel renders a live block: `Issue #42 [open] TUI: fuzzy search` / `PR #61 [draft] · checks 2/3`.

### multiplexer integration (`gwm tmux` / `gwm zellij`)

Inside an already-running tmux session, `gwm tmux <pattern>` shells out to `tmux new-window -n <name> -c <path>` so the new window's shell lands directly inside the matched worktree. `-p` (or `--split`) swaps the verb for `split-window` — same `-c`, current window's layout. `gwm zellij <pattern>` does the same for zellij via `zellij action new-tab --name <name> --cwd <path>` (or `new-pane --cwd` with `-p`); the `--cwd` flag on `new-tab` needs zellij ≥ 0.40.

Both require the corresponding multiplexer to actually be running (i.e. `$TMUX` / `$ZELLIJ` set in the calling environment). Outside a session the command refuses with a clear error rather than spawning a stray server.

### diagnose your setup

`gwm doctor` runs a series of cheap checks and reports each with `✓ / ! / ✗`, then exits `0` (all green), `1` (any warning), or `2` (any failure) — so it can be wired into CI or a pre-commit hook.

```bash
$ gwm doctor
✓ .gwm.toml parses
    /path/to/repo/.gwm.toml parses cleanly
✓ guard references resolve
    2 guard reference(s) resolve
✓ `when` predicates supported
    1 predicate(s) recognised
✓ external binaries on PATH
    3/3 binaries found
✓ no prunable worktrees
    5 worktree(s) tracked, none prunable
✓ no orphan gwm branches
    7 merged gwm-style branch(es) preserved per CONTRIBUTING, no unmerged orphans
✓ base directory writable
    /home/you/cc-worktree/myrepo is writable
```

If a real orphan (unmerged feature branch with no worktree) or a prunable entry exists, the doctor surfaces them as Warning with a remediation hint:

```bash
! no prunable worktrees
    1 prunable entry: feat-12-old
    → run `gwm prune` to clear them
! no orphan gwm branches
    1 unmerged orphan branch(es): feat/#99-wip-experiment
    → git branch -d feat/#99-wip-experiment
```

Checks performed:

1. **`.gwm.toml` parses** — Ok if it parses (or absent, defaults assumed); Failed if the TOML is broken.
2. **guard references resolve** — every `[[bootstrap.copy]].guards = [...]` points at an existing `[[bootstrap.guard]]`.
3. **`when` predicates supported** — every `[[bootstrap.command]].when` uses one of the known keyword prefixes (`file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`). Boolean composition via `!`, `&&`, `||` (precedence `!` > `&&` > `||`) is allowed inside the expression.
4. **external binaries on PATH** — `lazygit` (TUI `l` keybinding), `direnv` (only if `.envrc` exists), and the first executable token of every `[[bootstrap.command]].run`.
5. **no prunable worktrees** — `.git/worktrees/` entries whose working dir was removed manually.
6. **no orphan gwm branches** — local branches matching `<type>/#<issue>-<desc>` (created by `gwm create`) with no worktree. User-managed branches (`main`, `release-*`, `dependabot/...`) are ignored.
7. **base directory writable** — the configured `[worktree].base` exists and is writable, or its parent is (gwm creates the base lazily on first `gwm create`).

### one-line cd into a worktree

The binary itself cannot change the parent shell's directory. `gwm shell-init <shell>` prints a function (`gcd`) that bridges two flows in one wrapper:

- **`gcd <pattern>`** → `gwm cd <pattern>` (fuzzy resolve, exits `0` on a hit, `1` on miss / ambiguous / not in a repo).
- **`gcd`** (no argument) → `gwm switch` (interactive picker — `Enter` to commit, `Esc` / `Ctrl-C` / `q` to cancel with exit code `1`).

In both cases the wrapper only performs the `cd` after a successful exit code, so a cancelled picker or a missed pattern never strands you in `$HOME`.

`eval` the wrapper in your rc file:

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
gcd        # → cd $(gwm switch)  → opens the picker, cd's into the chosen worktree
```

### interactive picker (`gwm switch`)

When you can't remember the exact pattern, `gwm switch` opens the worktree TUI in picker mode — same table, fuzzy filter bar pre-open, create / delete / bootstrap disabled. `Enter` confirms the highlighted row and prints its path on stdout; `Esc` / `q` / `Ctrl-C` cancels with exit code `1`.

The recommended invocation is via the `gcd` wrapper from [one-line cd into a worktree](#one-line-cd-into-a-worktree) — bare `gcd` (no argument) routes to `gwm switch` and cd's into the chosen worktree in one keystroke. If you haven't installed the wrapper, the raw form is:

```bash
cd "$(gwm switch)"   # open picker, type to narrow, Enter to commit
gwm s                # same, via the alias
```

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

# composable when predicates: pick `bun install` if bun is on PATH,
# fall back to `npm ci` otherwise, and skip the noisy step in CI.
[[bootstrap.command]]
name = "install (bun)"
run  = "bun install"
when = "file_exists:package.json && cmd_exists:bun"

[[bootstrap.command]]
name = "install (npm fallback)"
run  = "npm ci"
when = "file_exists:package.json && !cmd_exists:bun"

[[bootstrap.command]]
name = "build docs"
run  = "./scripts/full-build.sh"
when = "glob_exists:docs/**/*.md && !env_set:CI"
```

`when` predicates recognised by the evaluator:

| Predicate                    | True when …                                                  |
|:-----------------------------|:-------------------------------------------------------------|
| `file_exists:<path>`         | `<worktree>/<path>` resolves on disk                          |
| `cmd_exists:<binary>`        | `<binary>` resolves on `$PATH` (`which` lookup)              |
| `env_set:<NAME>`             | `std::env::var(NAME)` returns `Ok`                            |
| `env_eq:<NAME>=<value>`      | `NAME` is set and its value matches `<value>` exactly         |
| `glob_exists:<pattern>`      | at least one path under the worktree matches `<pattern>` (supports `**`) |

Combine atoms with `!` (NOT), `&&` (AND), `||` (OR), conventional precedence `!` > `&&` > `||`. Whitespace around operators is tolerated. Unknown keywords default to `true` so old configs keep running while `gwm doctor` surfaces them.

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
| tests                               | none                 | 190 tests (config / naming / bootstrap / doctor / flake / worktree / TUI / CLI) |

## development

```bash
cargo build              # debug build
cargo test               # 190 tests
cargo fmt && cargo clippy -- -D warnings
cargo run                # opens TUI in the current repo
cargo install --path .   # install locally
```

Nix users can drop into a pinned dev shell — Rust toolchain, `rust-analyzer`, `clippy`, `rustfmt`, `cargo-watch`, `cargo-edit`, and the libgit2 build deps — without touching the host system:

```bash
nix develop              # bundled toolchain + LSP + linter + formatter
```

All tests live under `tests/`:

```
tests/
├── common/                       # shared helpers (init_repo, paths_equal)
├── config_tests.rs               # .gwm.toml parsing + write_default
├── naming_tests.rs               # kebab, branch validation, parse roundtrip
├── bootstrap_tests.rs            # copies / guards / no-symlink / commands
├── bootstrap_when_tests.rs       # `when:` predicate grammar (file/cmd/env/glob + boolean ops)
├── doctor_tests.rs               # `gwm doctor` checks + severity arithmetic
├── flake_tests.rs                # Nix flake structure (build, devShell, app)
├── worktree_integration.rs       # git2 add/list/remove/prune
├── tui_app_tests.rs              # state transitions (ratatui-free)
└── cli_binary.rs                 # assert_cmd end-to-end
```

Sentinel tests (pinned to catch a specific regression) are prefixed with a
`// regression: <one-line>` tag inside the test body so the target incident
is discoverable without `git blame`. Suite hygiene was last audited in
[`claudedocs/test-audit-0.4.0.md`](claudedocs/test-audit-0.4.0.md).

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
