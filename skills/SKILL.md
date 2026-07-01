---
name: gwm
description: Manage git worktrees across any repository with the `gwm` Rust binary (CLI + ratatui TUI). Use when the user asks to create / list / remove / bootstrap / switch / link worktrees, run a command across worktrees (`gwm exec`) or reclaim build artifacts (`gwm clean`), materialise a PR into a worktree (`gwm review`), drive a multi-repo workspace (`--workspace`), run the JSON daemon / statusline (`gwm daemon`, `gwm statusline`), seed config from a preset (`gwm init --preset`), diagnose with `gwm doctor`, drive tmux/zellij, or wire `gcd` via `gwm shell-init`. Triggers on `gwm`, `gwq`, `git worktree`, `.gwm.toml`, `gwm create`, `gwm list`, `gwm exec`, `gwm clean`, `gwm review`, `gwm daemon`, `gwm statusline`, `gwm bootstrap`, `gwm doctor`, `gwm switch`, `gwm tmux`, `gwm zellij`, `gwm link`, `gwm status`, `gwm shell-init`, `gcd`, `feat/#`, `fix/#`, GitHub issue/PR linking on a worktree.
allowed-tools: Bash, Read, Edit, Write
---

# gwm — git worktree manager (Rust CLI + TUI)

Single-binary Rust tool that manages git worktrees with `libgit2`, a ratatui TUI, a declarative per-repo bootstrap (`.gwm.toml`), GitHub issue/PR linking, multiplexer hand-off (tmux / zellij), and a doctor command. Replaces project-specific bash wrappers with one portable binary that works in any git repo.

Source: https://github.com/kbrdn1/gwm-cli — latest stable: `1.0.0` (the SemVer milestone; machine contracts frozen — MSRV 1.86). 1.0.0 ships: `gwm exec` / `gwm clean` (fleet command fan-out + build-artifact reclaim, #313), `gwm review <PR#>` (materialise a PR into a worktree, #308), the JSON API + `gwm daemon` + `gwm statusline` (#38 / #309), `gwm init --preset` stack templates (#37), multi-repo `--workspace` mode (#36), embedded PTY overlays for lazygit / shell (#35), a redesigned & fully-rebindable keymap incl. contextual modal keys + a live Settings panel with a Keys tab (#290 / #219 / #294), a Working Tree file-explorer pane and a current-PR CI indicator (#300 / #299).

## When to use this skill

- User runs or asks about any `gwm <subcommand>`: `init` (incl. `--preset`), `list` (incl. `--format json`), `create`, `remove`, `path` / `cd`, `bootstrap`, `sync`, `prune`, `doctor`, `types`, `completions`, `shell-init`, `switch` (alias `s`), `tmux`, `zellij`, `link`, `unlink`, `open`, `status`, `exec`, `clean`, `review`, `daemon`, `statusline`, `new`, `pr`, `config`, `history`, `undo`, `trust`, `labels`, `milestones`, `hooks`, `commit-prefix`, `aliases`, `theme`, `tui keys`.
- User wants to **fan a command out across worktrees** (`gwm exec -- <cmd>`) or **reclaim build artifacts** (`gwm clean`).
- User wants to **materialise a GitHub PR into a worktree** to review/test it (`gwm review <PR#>`, cross-fork, safe-by-default).
- User wants a **multi-repo workspace** (`gwm --workspace <dir>`), the **JSON API / daemon** (`--format=json`, `gwm daemon`, `gwm statusline` for shell prompts), or **stack presets** (`gwm init --preset laravel|node|rust|go|python-uv|generic`).
- User opens the TUI by running `gwm` alone in a repo, or the picker via `gwm switch` / `gwm s`.
- User asks about the redesigned TUI: PTY overlays (`l`/`L` lazygit, `r`/`R` review, `o`/`O` shell, #35), the Settings panel (`4`) and its live-editable Keys tab (#294), the Command Logs modal (`3`), the Working Tree file-explorer pane (#300), the current-PR CI indicator (#299), `[tui.keys.modal.<context>]` rebindable overlay keys (#219), and `[tui.macro1]`/`[tui.macro2]` (`h`/`H`).
- User mentions `.gwm.toml` (per-repo config) or any of its sections: `[worktree]`, `[doctor]`, `[tui]`, `[tui.open]`, `[git_tui]`, `[review]`, `[[bootstrap.copy]]`, `[[bootstrap.guard]]`, `[[bootstrap.no_symlink]]`, `[[bootstrap.command]]`, `[bootstrap.fallback.*]`.
- User asks about composable `when` predicates (`file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`) and the `!` / `&&` / `||` operators.
- User wants to migrate a `tools/worktree-manager.sh` or `gwq`-based workflow to `gwm`.
- User asks how to set up the AWS RDS guard, the safe `.env.testing` fallback, or the no-symlink invariant for `vendor/` / `node_modules/`.
- User asks about the branch convention `<type>/#<issue>-<desc>` or its overrides.
- User wants to link a worktree to a GitHub issue / PR, refresh GitHub status from inside the TUI, or run `gwm doctor` to validate setup before pushing.
- User mentions `gcd <pattern>` (shell wrapper from `gwm shell-init <shell>`).
- User wants tmux / zellij hand-off (`gwm tmux <pat>`, `gwm zellij <pat>` with optional `--split`).
- User wants to configure the TUI `l` (git_tui) / `R` (review) launchers, the `o` (open dispatch: shell/editor/finder), or `y` (yank path to clipboard) keys.
- User asks about the `branch.<n>.gwm-base` key (review base-resolution anchor) or any of the `lumen` / `claude` / `codex` / `aider` / `gh` review presets.

## Prerequisites

```bash
command -v gwm           # required — installed by `cargo install --path .` from the gwm-cli repo
command -v cargo         # required at install time (1.86+ — the crate MSRV)
command -v git           # required at runtime
command -v gh            # OPTIONAL — needed for live `gwm status` / TUI GitHub state / `R: review` preset
command -v tmux          # OPTIONAL — needed by `gwm tmux`
command -v zellij        # OPTIONAL — needed by `gwm zellij` (≥ 0.40 for `--cwd` on new-tab)
command -v lazygit       # OPTIONAL — default `[git_tui]` binary backing the TUI `l` key
command -v lumen         # OPTIONAL — `[review] tool = "lumen"` preset (default review tool)
command -v claude        # OPTIONAL — `[review] tool = "claude"` preset
command -v codex         # OPTIONAL — `[review] tool = "codex"` preset
command -v aider         # OPTIONAL — `[review] tool = "aider"` preset
command -v pbcopy        # OPTIONAL — TUI `y: yank` on macOS (wl-copy/xclip/xsel on Linux, clip on Windows)
```

`gwm` vendors `libgit2`, so no system git2 lib is needed. The binary is self-contained once compiled.

## Install (from source)

```bash
git clone https://github.com/kbrdn1/gwm-cli.git
cd gwm-cli
cargo install --path .         # → ~/.cargo/bin/gwm
gwm --version
```

No Rust toolchain at hand? `cargo binstall gwm-cli` pulls the prebuilt binary from the matching GitHub Release (via `[package.metadata.binstall]`) and drops it in `~/.cargo/bin/` without compiling `git2`/vendored-libgit2 from source — much faster on first install.

Prebuilt releases (Linux x86_64/aarch64, macOS Intel/Apple Silicon, Windows): https://github.com/kbrdn1/gwm-cli/releases. A Homebrew formula ships under `packaging/homebrew/` and a Nix `flake.nix` is at the repo root.

## Default conventions

| What                | Default                              | Override                       |
|:--------------------|:-------------------------------------|:-------------------------------|
| Branch name         | `<type>/#<issue>-<desc>`             | `.gwm.toml` `branch_pattern`   |
| Worktree dir name   | `<type>-<issue>-<desc>`              | `.gwm.toml` `path_pattern`     |
| Worktree base       | `~/cc-worktree/<repo>/`              | `.gwm.toml` `base`             |
| Bootstrap           | none (just `git worktree add`)       | `.gwm.toml` `[bootstrap.*]`    |
| Doctor trunks       | `["dev", "main"]`                    | `.gwm.toml` `[doctor] trunks`  |
| TUI confirm timer   | `3` (clamped 0..=5)                  | `.gwm.toml` `[tui] confirm_countdown_secs` |

Branch types: `feat`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`.

Placeholders in patterns: `{home}`, `{repo}` (repo name), `{repo_path}` (main repo's absolute workdir), `{repo_parent}` (its parent dir), `{type}`, `{issue}`, `{desc}`. Tilde (`~/…`) is also expanded.

## CLI reference

```bash
gwm                          # opens the TUI in the current repo
gwm --workspace <dir>        # TUI / list / create across every git repo one level under <dir> (#36)
gwm init [--preset <stack>]  # write .gwm.toml (refuses overwrite); seed from a stack preset (#37)
gwm init --list-presets      # built-in presets: laravel, node/nuxt, rust, go, python-uv, generic
gwm types                    # list supported branch types

gwm create <type> <issue> <desc>          # create branch + worktree + bootstrap
gwm create feat 123 "user-authentication"
gwm create feat 123 foo --no-bootstrap    # skip the .gwm.toml stages
gwm create feat 123 foo --reuse-branch    # attach to an existing local branch instead of erroring
gwm create feat 123 foo --repo <name>     # workspace mode: which child repo gets the worktree

gwm list [--format table|names|json] [--detect-pr]   # `json` = stable schema (#38); names = completion
gwm path <pattern> [--format text|json]   # print path (fuzzy match) → use $(gwm path auth)
gwm cd   <pattern>                        # alias of `gwm path`
gwm bootstrap                             # re-run bootstrap on cwd worktree
gwm bootstrap <pattern>                   # ...or on a named worktree
gwm sync                                  # fetch + rebase the cwd worktree onto its upstream
gwm sync <pattern>                        # ...or a fuzzy-matched worktree
gwm sync <pattern> --merge                # merge the upstream instead of rebasing
gwm remove <pattern> [--delete-branch] [--dry-run] [--force]   # remove (fuzzy); -b drops the branch
gwm prune [--dry-run]                     # clean stale .git/worktrees entries

gwm doctor [--format text|json]           # diagnose setup. Exit: 0=green, 1=warn, 2=fail
gwm completions <bash|elvish|fish|powershell|zsh>   # emit a shell-completion script on stdout
gwm shell-init  <bash|fish|powershell|zsh>          # emit a `gcd <pattern>` wrapper to eval/source

gwm switch                                # interactive picker → prints chosen path to stdout (alias: s)

gwm tmux   <pattern> [-p|--split]         # open matched worktree in new tmux window (or split)
gwm zellij <pattern> [-p|--split]         # open matched worktree in new zellij tab (or pane)

# --- fan-out & disk hygiene (#313) ---
gwm exec [<slug>...] -- <cmd>             # run <cmd> in each worktree (sequential); ✓/✗ rollup, non-zero on any fail
                                          #   default = all non-main worktrees; slugs before `--`; cmd verbatim after `--`
gwm clean [<slug>...] [--yes]             # report (or with --yes reclaim) target/ node_modules/ dist/ build/
                                          #   --yes deletes ONLY git-ignored dirs holding no tracked files; never follows symlinks

# --- GitHub (needs `gh`) ---
gwm new <type> <desc>                     # create issue from a repo template, then its worktree
gwm pr [--draft] [--base <b>] [--render]  # render [pr_template] body, then `gh pr create`
gwm review <PR#> [--name <b>] [--bootstrap]   # materialise a PR into a worktree (cross-fork; safe-by-default) (#308)
gwm link   issue|pr <N> [--worktree PAT]  # bind a worktree to a GitHub issue or PR
gwm unlink issue|pr      [--worktree PAT] # remove the explicit link
gwm open   [issue|pr]    [--worktree PAT] # open the linked URL in $BROWSER
gwm status [--worktree PAT] [--json]      # show link + live GitHub state (needs `gh`)
gwm labels   list|push [--dry-run] [--prune]      # sync the declarative [[labels]] set to origin
gwm milestones list|push [--dry-run] [--prune]    # sync the declarative [[milestones]] set to origin

# --- daemon & shell consumers (#38 / #309) ---
gwm daemon [--socket <path>] [--poll-ms <n>]   # long-running JSON-RPC 2.0 over a unix socket (list/doctor/path/subscribe)
gwm statusline [--socket <path>] [--watch]     # one-line prompt summary off the daemon (degrades to blank when none)

# --- config / convention / introspection ---
gwm config get|set|unset|list|validate|path|edit   # git-config-style editing of .gwm.toml (comment-preserving)
gwm history [--all]                       # recent destructive ops journal (newest first)
gwm undo [--bootstrap]                    # reverse the last destructive op for this repo
gwm trust list|show|revoke                # TOFU trust ledger for .gwm.toml bootstrap
gwm aliases                               # resolved CLI aliases (built-in + repo + user)
gwm commit-prefix [<branch>] [--unicode]  # Gitmoji + Conventional commit prefix for the branch
gwm hooks install commit-msg [--force]    # opt-in git hook that auto-prepends the commit prefix
gwm theme list|show [<name>]              # role-based [theme] presets
gwm tui keys                              # resolved keymap (defaults + [tui.keys[.modal.*]] overrides)
```

### `gwm doctor`

Runs a structured set of checks across config, environment, and worktree state. Designed for CI / pre-commit hooks:

| Exit | Meaning                                                                  |
|:-----|:-------------------------------------------------------------------------|
| `0`  | All checks green                                                         |
| `1`  | At least one **warning** (advisory, e.g. orphan gwm-style branch)        |
| `2`  | At least one **failure** (broken config, prunable worktree, etc.)        |

Trunk branches the orphan-branch check treats as merge destinations come from `[doctor] trunks = [...]` (default `["dev", "main"]`). Setting `trunks = []` disables the filter (every unclaimed gwm-style branch is flagged).

### `gwm shell-init` → `gcd <pattern>`

The emitted wrapper defines a `gcd` function that resolves a worktree by fuzzy pattern and `cd`s into it in one keystroke. Install per shell:

```bash
# zsh
echo 'eval "$(gwm shell-init zsh)"'  >> ~/.zshrc
# bash
echo 'eval "$(gwm shell-init bash)"' >> ~/.bashrc
# fish
gwm shell-init fish | source        # also add the eval to ~/.config/fish/config.fish
# powershell
Invoke-Expression (& gwm shell-init powershell | Out-String)
```

`gcd` (no arg) launches `gwm switch` (the picker) and `cd`s into the chosen entry.

### GitHub linking (`link` / `unlink` / `open` / `status`)

Links live in **per-branch git config**:

- `branch.<name>.gwm-issue` ← `gwm link issue <N>`
- `branch.<name>.gwm-pr`    ← `gwm link pr <N>`
- `branch.<name>.gwm-base`  ← _written by `gwm create`_ — anchors the `[review].{base}` resolution chain so the parent ref survives even when the branch has no upstream yet. Not user-facing; surfaces only via the `R: review` launcher.

Local, per-branch, survives worktree moves. Issue numbers are auto-detected from the `<type>/#<N>-<slug>` convention when no explicit override is set; PR numbers are **not** auto-detected and must be linked explicitly.

`gwm status` shells out to `gh issue view` / `gh pr view` to fetch state, title, labels, and the CI rollup. Without `gh` (or outside a GitHub repo), it prints only the local link. `--json` emits a stable schema for scripting.

### Multiplexer hand-off (`tmux` / `zellij`)

- `gwm tmux <pat>` requires `$TMUX` to be set (i.e. you are already inside a tmux session) — otherwise it exits non-zero with a clear error rather than spawning a stray server. `--split` opens a horizontal split of the current pane instead of a new window.
- `gwm zellij <pat>` requires `$ZELLIJ`. `--cwd` on `zellij action new-tab` needs zellij ≥ 0.40. `--split` opens a new pane in the current tab instead of a new tab.

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

Built-in defaults after the **v0.10 keymap redesign (#290)**. Every binding is
rebindable via `[tui.keys]` (list view) and `[tui.keys.modal.<context>]`
(overlays, #219). Run `gwm tui keys` for the full resolved map incl. every modal
context.

| Key             | Action                                                                      |
|:----------------|:----------------------------------------------------------------------------|
| `j`/`↓` `k`/`↑` | move selection (scrolls the focused pane)                                    |
| `gg` / `G`·End  | jump to first / last worktree                                               |
| `Tab`           | swap focus between the worktree list and the Status sidebar                  |
| `1` / `2`       | focus the worktrees pane / the Status pane                                  |
| `3` / `4`       | open the Command Logs modal / the Settings (config) panel (#290 / #294)     |
| `n` / `d` / `b` | new worktree modal / delete selected / re-run bootstrap (TOFU gate)         |
| `D`             | toggle "delete branch on remove" (arms the safety countdown when ON)        |
| `p` / `P`       | pull / push the selected worktree's branch                                  |
| `s` / `f`       | sync (fetch + rebase) / refresh the worktree list                           |
| `c` / `e`       | edit-worktree / exit-to-worktree (sets the picker's cd target)             |
| `l` / `L`       | lazygit — PTY overlay / fullscreen `[git_tui]` (#35)                        |
| `r` / `R`       | `[review]` launcher — PTY overlay / fullscreen (#35 / #75)                  |
| `o` / `O`       | terminal (`$SHELL`) — PTY overlay / fullscreen honouring `[tui.open]` (#35) |
| `t`             | open the worktree in a tmux/zellij pane (mux)                              |
| `h` / `H`       | run `[tui.macro1]` / `[tui.macro2]` (#290)                                  |
| `y` / `w` / `Y` | yank branch name / worktree name / absolute path to the clipboard           |
| `i` / `B`       | link prompt (issue/PR) / browse-links menu (open linked issue·PR in browser)|
| `F`             | refresh GitHub issue/PR status via `gh` (off-thread; statusbar spinner)     |
| `V` / `v`       | toggle the sidebar / flip its position left ↔ right                         |
| `S` / `Space`   | sidebar Details mode (commits ↔ stashes) / cycle layout (auto→side→stacked) |
| `.` / `?`       | open the docs in `$BROWSER` / help overlay                                  |
| `:` / `/`       | command palette (fuzzy-fire any action) / fuzzy filter bar (`Esc` clears)   |
| `Enter`         | show path in status bar (picker mode: print path to stdout + exit)          |
| `q` / `Esc`     | quit (`Esc` closes an overlay / clears a sticky filter first)               |
| `Ctrl-C`        | force quit                                                                  |

PTY overlays (`l`/`r`/`o`) run the tool **inside** the TUI (no alt-screen swap;
`Esc` closes); the uppercase variants (`L`/`R`/`O`) suspend the TUI for a
fullscreen takeover. The Settings panel (`4`) edits `.gwm.toml` live across
category tabs (Theme / Worktree / TUI / Keys / All) with a per-layer selector
(`L`); the **Keys** tab captures a keystroke and writes it back validated (#294).

## Picker mode (`gwm switch` / `gcd`)

`gwm switch` opens the same TUI minus the create / delete / bootstrap actions. The fuzzy filter bar opens immediately so typing narrows the list right away. `Enter` commits the highlighted pick (path → stdout). `Esc` / `Ctrl-C` / `q` exit non-zero without printing.

## Details sidebar

When the sidebar is open (default ON, toggle with `v`), it shows a details panel for the selected worktree. The layout is responsive (issue #188): at ≥ 120 columns it sits **side-by-side** with the table; below that it **stacks** under the table (it is no longer hidden). `V` cycles `auto → side-by-side → stacked → auto`; `H` flips the side-by-side position left ↔ right, with the default set by `[tui] sidebar_position = "left" | "right"` (default `right`). Since the lazygit-style redesign (issues #69 / #71 / #73) the panel is **four independent rounded-border subsections** stacked vertically — no outer `Details` frame, section titles ride the block borders, no inline `Label:` content headers.

```
╭─ Worktree ──────────────────────╮      ●  status dot tracks the linked PR / issue
│ ● api-rest                      │         state (open=green, draft=darkgray,
│ feat/#42-api-rest · 08d1029     │         merged=magenta, closed=red, white=link
│ Created: 2d                     │         not yet fetched, darkgray=no link).
│ ✓ synced  ★ main                │         Rebuilt fresh every frame so it tracks
│ ~/Projects/Flippad/…/api-rest   │         live `gh` fetches without invalidating
╰─────────────────────────────────╯         the cached git preview.

╭─ Issue / PR ────────────────────╮      Live `gh issue view` / `gh pr view` data:
│ #42 · open · 3 labels           │         state + checks rollup. Refresh with `F`.
│ checks 7/8                      │         Empty block hints "press L to link" when
╰─────────────────────────────────╯         the worktree has no link.

╭─ Working Tree ──────────────────╮      `git status --short` (`✓ clean` when empty).
│ ✓ clean                         │
╰─────────────────────────────────╯

╭─ Recent Commits ────────────────╮      Full lazygit-style topology graph (issue #71):
│ 08d1029  KB  ○  feat: …         │         per-row format `<hash>  <initials>  <node>
│ 4d874e7  KB  ○  fix: …          │         <subject>`, `○` for commit, `◎` for merge,
│ 2d1d3ae  KB  ◎  merge: …        │         vertical pipes `│`, corners `╮ ╭ ╯ ╰`,
│ … (300 commits, scrollable)     │         junctions `┴ ┬`, horizontal strokes `─`.
│                          7 of 14│         Subjects hard-clipped (no Wrap). Buffer =
╰─────────────────────────────────╯         300 commits (matches lazygit's `log -300`).
                                            Bottom-right footer: `<bottom> of <total>`.
```

Worktree block: name (bold) prefixed by the `●` dot · `branch · short-head` (branch coloured by `BranchStatus` — worst-state wins: dirty=red, ahead/behind=yellow, unpublished=magenta, synced=green, unknown=darkgray) · `Created: <age>` with freshness colour (green < 7d, yellow < 30d, darkgray ≥ 30d; `-` when undeterminable, e.g. trunk / detached HEAD) · status + flag badges (only the relevant ones — false flags stay invisible: `★ main`, `🔒 locked`, `⚠ prunable`) · tilde-compressed path.

GitHub fetch state machine per worktree: `Idle → Loading → Loaded(T) | Error(String)`. Manual refresh = `F` (the legacy `R` was rebound to `R: review` in #75).

`Tab` swaps focus between the worktree list and the sidebar. `j` / `k` (and arrows) scroll the Recent Commits block when the sidebar is focused — the small blocks above stay pinned. The focused panel's border turns cyan.

## Terminal open: `o` (PTY overlay) / `O` (fullscreen) — issues #73 / #35

Since the keymap redesign, `o` opens an **embedded PTY terminal overlay** inside
the TUI (no alt-screen swap; `Esc` closes), while `O` is the **fullscreen**
variant that suspends the TUI and honours the `[tui.open]` dispatch below. Three
fullscreen modes:

| `mode = ` | Behaviour                                                                                  |
|:----------|:-------------------------------------------------------------------------------------------|
| `"shell"` _(default)_ | Suspend the TUI and spawn `$SHELL` with `cwd = <worktree>` — lazygit-style. Exiting the shell restores the TUI. |
| `"editor"` | Suspend the TUI and run `$EDITOR <worktree-path>`.                                        |
| `"finder"` | Hand off to the OS file manager (`open` / `xdg-open` / `explorer`).                       |

```toml
[tui.open]
mode       = "shell"     # "shell" (default) | "editor" | "finder"
shell_cmd  = ""          # override $SHELL when set ("" = read $SHELL)
editor_cmd = "hx"        # override $EDITOR when set ("" = read $EDITOR)
```

`shell_cmd` and `editor_cmd` win over the env var when non-empty. An unknown `mode` is a hard config-load error.

## Yank: `y` branch · `w` worktree name · `Y` path — issues #73 / #290

The yank keys copy to the system clipboard: `y` = the branch name, `w` = the
worktree (dir) name, `Y` = the selected worktree's absolute path. Probe order
(first hit wins on `$PATH`):

| OS         | Candidates (in order)                                                              |
|:-----------|:-----------------------------------------------------------------------------------|
| macOS      | `pbcopy`                                                                           |
| Linux      | `wl-copy`, `xclip -selection clipboard`, `xsel --clipboard --input`                |
| Windows    | `clip`                                                                             |

Missing tool surfaces a status-bar hint, never a panic. No config knob — the probe list is built per-platform.

## Configurable launchers (`l`/`L` git_tui · `r`/`R` review) — issues #75 / #35

Two configurable launchers share the same mini-API: take a `command` template from `.gwm.toml`, substitute placeholders, split with `shell-words`, and exec it with `cwd = <selected-worktree>`. Each has a **PTY-overlay** binding (lowercase — runs inside the TUI, no alt-screen swap, `Esc` closes) and a **fullscreen** binding (uppercase — suspends the TUI):

| Keys (PTY / full) | Section     | Default                       | Placeholders                          | Default `fullscreen` |
|:------------------|:------------|:------------------------------|:--------------------------------------|:---------------------|
| `l` / `L`         | `[git_tui]` | `lazygit -p {path}`           | `{path}`                              | `true`               |
| `r` / `R`         | `[review]`  | _(inert until configured)_    | `{base} {head} {path} {diff}`         | `false`              |

`fullscreen = true` suspends the gwm TUI for a TUI-style takeover (same recipe as the pre-issue-#75 `l` → lazygit flow); `fullscreen = false` runs the command **synchronously in-place** — gwm stays in the alt-screen, `Command::output()` blocks the TUI until the child exits, and the first line of stderr lands on the status bar. Fine for quick print-only tools (`claude --print`, `gh pr view --web`); pick `fullscreen = true` for anything long-running so the TUI is properly suspended and restored. The `{diff}` placeholder is **lazy** — gwm only shells out to `git diff {base}..{head}` (into a tempfile) when the template references it.

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

# --- file copies main → worktree (run in order) -----------------------------
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

# --- regex guards on copied files -------------------------------------------
[[bootstrap.guard]]
name           = "no-aws-rds"
deny_patterns  = ["amazonaws\\.com", "\\.rds\\."]
on_match       = "seed-from-example"   # or "abort"
example_file   = ".env.example"

# --- inline fallback when a required source is missing ----------------------
# Key is the destination filename normalised: ".env.testing" → "env_testing".
[bootstrap.fallback.env_testing]
target  = ".env.testing"
content = """
APP_ENV=testing
DB_CONNECTION=sqlite
DB_DATABASE=:memory:
"""

# --- symlinks to refuse (inherited from main) ------------------------------
[[bootstrap.no_symlink]]
path = "vendor"

[[bootstrap.no_symlink]]
path = "node_modules"

# --- shell commands after copies -------------------------------------------
[[bootstrap.command]]
name = "composer install"
run  = "composer install --no-interaction --prefer-dist"
when = "file_exists:composer.json"
env  = { COMPOSER_IGNORE_PLATFORM_REQ = "ext-imagick" }

# --- composable when predicates --------------------------------------------
# Atoms : file_exists:<path> | cmd_exists:<bin> | env_set:<VAR>
#         env_eq:<VAR>=<value> | glob_exists:<pattern>
# Ops   : ! (NOT) | && (AND) | || (OR)
# Precedence: ! > && > ||

[[bootstrap.command]]
name = "install (bun if available)"
run  = "bun install"
when = "file_exists:package.json && cmd_exists:bun"

[[bootstrap.command]]
name = "install (npm fallback)"
run  = "npm ci"
when = "file_exists:package.json && !cmd_exists:bun"

[[bootstrap.command]]
name = "full local build"
run  = "./scripts/full-build.sh"
when = "glob_exists:src/**/*.rs && !env_set:CI"

# --- doctor knobs -----------------------------------------------------------
# Trunks the orphan-branch check treats as merge destinations.
# Default: ["dev", "main"]. `trunks = []` disables the filter entirely.
[doctor]
trunks = ["master", "release-3.x", "release-4.x"]

# --- TUI knobs --------------------------------------------------------------
# Safety countdown (seconds) applied to the delete-confirm overlay when
# `delete branch on remove` (`p` in the TUI) is armed. Range 0..=5;
# above 5 is clamped on read; 0 disables the countdown. Default: 3.
[tui]
confirm_countdown_secs = 3

# --- `o: open` dispatch (issue #73) ------------------------------------------
# Three modes: "shell" (default — $SHELL in the worktree), "editor"
# ($EDITOR <path>), "finder" (pre-#73 OS file manager). `shell_cmd` /
# `editor_cmd` override the env var when non-empty.
[tui.open]
mode       = "shell"
shell_cmd  = ""
editor_cmd = "hx"

# --- TUI `l: git_tui` launcher (issue #75) -----------------------------------
# Default: `lazygit -p {path}` fullscreen=true (matches pre-#75 behaviour).
# Placeholders: {path}.
[git_tui]
command    = "lazygit -p {path}"
fullscreen = true

# --- TUI `R: review` launcher (issue #75) ------------------------------------
# Either a free-form `command` (placeholders: {base} {head} {path} {diff})
# or a `tool = "<preset>"` sugar (lumen / claude / codex / aider / gh).
# `command` always wins when both are set. `{diff}` is lazy — only
# materialised when the template references it. Base resolution chain:
# upstream → branch.<n>.gwm-base → [review].default_base → "dev" → "main".
[review]
tool                  = "lumen"
skip_when_no_changes  = true     # default true — `git rev-list --count {base}..{head} == 0` ⇒ skip
# default_base        = "dev"    # optional pin overriding the auto-discovery chain
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
6. Wire `gcd` into your shell: `echo 'eval "$(gwm shell-init zsh)"' >> ~/.zshrc`.
7. Add `gwm doctor` to CI / pre-commit to catch broken setups before push.

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

### Node project (bun preferred, npm fallback)

```toml
[[bootstrap.copy]]
from = ".env"
to   = ".env"
required = false

[[bootstrap.no_symlink]]
path = "node_modules"

[[bootstrap.command]]
name = "install (bun)"
run  = "bun install"
when = "file_exists:package.json && cmd_exists:bun"

[[bootstrap.command]]
name = "install (npm fallback)"
run  = "npm ci"
when = "file_exists:package.json && !cmd_exists:bun"
```

### Quick create + cd

```bash
gwm create feat 42 cool-thing
cd "$(gwm path cool-thing)"
# …or with the shell wrapper installed:
gcd cool-thing
```

### Picker → cd in one keystroke

```bash
# After `eval "$(gwm shell-init zsh)"`
gcd            # opens the picker; cd into the chosen worktree on Enter
```

### Hand-off into tmux / zellij

```bash
# Inside tmux:
gwm tmux api-rewrite              # new window in current session
gwm tmux api-rewrite --split      # …or split the current pane

# Inside zellij (>= 0.40):
gwm zellij api-rewrite            # new tab
gwm zellij api-rewrite --split    # …or new pane in current tab
```

### Link a worktree to a GitHub issue / PR

```bash
# Auto-detected from feat/#123-foo branches → no link needed.
gwm status                                  # shows the local link + (with gh) live state

# Explicit linking:
gwm link issue 456                          # current worktree → issue #456
gwm link pr   789  --worktree api-rewrite   # named worktree → PR #789
gwm open      pr                            # open the PR URL in $BROWSER
gwm unlink    issue                         # drop the explicit issue link

# Scripting:
gwm status --json
```

### Pre-push sanity check

```bash
gwm doctor && git push           # blocks the push on any warning/failure
```

### Opt-in pre-commit hook (contributors)

The repo ships an opt-in hook at `.githooks/pre-commit` that combines two gates:

1. **Env-dependent test pre-validation** — if any staged `tests/*.rs` file references ambient state (`assert_cmd`, `std::env::var`, `which::which`, `dirs::`, `Command::cargo_bin`), the suite is re-run under a stripped `PATH="$(dirname cargo):/usr/bin:/bin"` to catch tests that pass in a rich dev shell but fail on minimal CI.
2. **Local `gwm doctor`** — if staged files touch `.gwm.toml`, the bootstrap / doctor modules, the example config, or their tests, `gwm doctor` runs. Exit `0` is silent, `1` is advisory (commit proceeds), `2` blocks the commit. Unknown exits fail open.

Enable per-clone:

```bash
git config core.hooksPath .githooks
git commit --no-verify        # bypass for one commit, sparingly
```

## Architecture (for skill agents asked to extend gwm)

```
src/
├── lib.rs               # public re-exports — tests import these
├── main.rs              # bin entry, alias expansion, dispatches to cli::run
├── error.rs             # GwmError (thiserror) + Result alias
├── config.rs            # serde TOML → Config (worktree, bootstrap, hooks, doctor, tui, tui.open,
│                        #   tui.keys[.modal], tui.macro1/2, git_tui, review, theme, gitmoji, aliases…)
├── config_cli.rs        # `gwm config get/set/…` (comment-preserving toml_edit)
├── naming.rs            # BranchSpec, kebab(), parse_branch()
├── worktree.rs          # discover_repo, list, add, remove, prune, find_fuzzy, sync helpers (libgit2 + shell-out)
├── bootstrap.rs         # run(BootstrapCtx) → BootstrapReport (copies / guards / commands / when DSL)
├── lifecycle.rs         # [hooks.*] phases (pre/post create·bootstrap·remove)
├── sync.rs              # fetch + rebase/merge onto upstream
├── review.rs            # `gwm review <PR#>` — materialise a PR into a worktree (#308)
├── workspace.rs         # multi-repo `--workspace` discovery (#36)
├── presets.rs           # `gwm init --preset` stack templates (#37)
├── history.rs / trust.rs   # destructive-op journal (undo) · TOFU trust ledger for .gwm.toml
├── github.rs            # issue / PR link storage in git config + `gh` shell-out, BranchLink
├── issue_templates.rs / pr_templates.rs / templating.rs   # gwm new / gwm pr
├── labels.rs / milestones.rs / gitmoji.rs / aliases.rs    # declarative GitHub sets, commit-prefix, CLI aliases
├── launcher.rs / multiplexer.rs   # [git_tui]/[review] launcher pipeline · tmux/zellij hand-off
├── daemon.rs / json_api.rs        # JSON-RPC 2.0 daemon + `--format=json` DTOs/schemas (#38)
├── statusline.rs        # `gwm statusline` prompt one-liner off the daemon (#309)
├── command_log.rs       # process-global command log (Command Logs modal, #290)
├── doctor.rs            # `gwm doctor` checks (config, env, orphan branches, prunable wts, launcher PATH probes)
├── cli.rs               # clap subcommands + handlers (incl. exec / clean, #313)
└── tui/
    ├── mod.rs           # crossterm event loop (filter, overlays, launchers, clipboard, PTY)
    ├── app.rs           # App state, transitions, Action dispatcher, GitHubFetchState<T>, workspace
    ├── ui.rs            # ratatui drawing — panes, bordered sidebar, modals, CI indicator (#299)
    ├── keymap.rs        # [tui.keys] remappable list-view bindings + chords (#290)
    ├── modal_keymap.rs  # [tui.keys.modal.<context>] overlay bindings (#219)
    ├── palette.rs       # `:` command palette
    ├── theme.rs         # role-based colours + presets
    ├── wt_tree.rs       # Working Tree file-explorer tree (#300)
    ├── commit_graph.rs  # Recent Commits lazygit-style topology renderer
    └── state/           # create_form, filter, confirm, link_prompt, sidebar, github_fetch, spinner,
                         #   async_task, command_logs, config_panel, pty_overlay — one slice per overlay
```

Tests under `tests/` mirror this layout — ~73 integration test files, one (or more) per module: e.g. `bootstrap_tests.rs`, `cli_binary.rs`, `config_tests.rs`, `doctor_tests.rs`, `daemon_tests.rs`/`daemon_integration.rs`, `json_api_tests.rs`, `review_tests.rs`/`review_integration.rs`, `statusline_tests.rs`, `workspace_tests.rs`, `presets_tests.rs`, `exec_tests.rs`, `clean_tests.rs`, `worktree_integration.rs`, plus the `tui_*` state-machine suites and `tests/common/` helpers. **TDD bar: any new behaviour ships with a matching test file or new assertions in an existing one** (project rule, enforced in `CLAUDE.md`).

## Differences vs. the bash + gwq stack

| Capability                  | bash + gwq            | gwm                                                      |
|:----------------------------|:----------------------|:---------------------------------------------------------|
| worktree engine             | `gwq` CLI external    | `libgit2` vendored                                       |
| bootstrap                   | hardcoded shell       | declarative TOML + composable `when` predicates          |
| portability across repos    | per-project script    | one binary + per-repo config                             |
| TUI                         | linear bash menu      | full ratatui screen + filter + GitHub panel              |
| picker                      | none                  | `gwm switch` / `gcd` shell wrapper                       |
| multiplexer hand-off        | none                  | `gwm tmux` / `gwm zellij` (window/tab/split)             |
| GitHub linking              | none                  | issue / PR per-branch git config + `gh` live status      |
| diagnostics                 | none                  | `gwm doctor` (exit 0/1/2, CI-ready)                      |
| anti-RDS guard              | hardcoded `grep`      | configurable regex deny-list                             |
| tests                       | 0                     | large suite across ~73 test files (config / bootstrap / when DSL / doctor / github / daemon + JSON API / review / statusline / workspace / presets / exec / clean / multiplexer / TUI state machines + commit graph / launcher / CLI / homebrew / flake / pre-commit hook) |
| install                     | `chmod +x` per repo   | `cargo install --path .` (or Homebrew / Nix / prebuilts) |

## Troubleshooting

**`error: not inside a git repository`** — run `gwm` from inside a repo or pass a path explicitly.

**`gwm create` fails with "branch ... already exists"** — the branch was created in a previous run that didn't finish. `git branch -D <branch>` or pick another issue number, then retry.

**`gwm remove` reports "pattern '...' is ambiguous"** — multiple worktrees match the substring. Pass a more specific pattern or the exact dir name from `gwm list`.

**Bootstrap step shows ✗ on a `.env` copy with guard match + no example_file** — either set `example_file` in the guard, or change `on_match` to `"abort"` and rely on `.env.example`. Either way, the source `.env` is never copied past a guard match.

**TUI shows `(prunable)` next to a worktree** — its working dir was deleted out-of-band. Run `gwm prune` (or hit `r` in the TUI after manual cleanup).

**`gwm doctor` exits `2` complaining about an orphan branch** — the branch matches `<type>/#<N>-<slug>` but isn't reachable from any trunk in `[doctor] trunks`. Either delete the branch, merge it, or add its merge target to `trunks`.

**`gwm tmux` says `not inside a tmux session`** — `$TMUX` is unset. Start tmux first; gwm refuses to spawn a stray server.

**`gwm zellij` errors on `--cwd`** — your zellij is older than 0.40. Upgrade, or fall back to opening the path manually.

**`gwm status` shows only the local link, no live data** — `gh` isn't on `$PATH` (or you're outside a GitHub repo). Install GitHub CLI and `gh auth login`.

**`cargo install --path .` fails to build libgit2** — install a C toolchain (`xcode-select --install` on macOS, `build-essential` on Debian/Ubuntu). The `git2` crate uses `vendored-libgit2` so it builds from source.

**`.env` was copied even though it points to prod** — the guard's regex didn't match. Test it with `echo $YOUR_HOST | grep -E '<pattern>'`. Regex syntax is Rust `regex` crate (PCRE-like, no lookaround).

**`gcd` says command not found** — the shell-init wrapper isn't sourced. Re-run `eval "$(gwm shell-init <shell>)"` in your current shell and add it to your shell's rc file.

**Pressing `r` / `R` in the TUI does nothing / shows a status hint** — `[review]` is opt-in. Either no `[review]` section exists in `.gwm.toml`, the resolved binary isn't on `$PATH` (`gwm doctor` flags it as Warning), or `skip_when_no_changes = true` (default) found 0 commits between `{base}..{head}`. Add a `[review] tool = "lumen"` (or another preset) to enable it. (`r` = PTY overlay, `R` = fullscreen.)

**`R: review` resolves the wrong `{base}`** — the chain is upstream → `branch.<n>.gwm-base` → `[review].default_base` → `"dev"` → `"main"`. Pin it explicitly with `[review] default_base = "<branch>"` or set the per-branch override with `git config branch.<name>.gwm-base <ref>`.

**`l` launches lazygit when the repo wants gitui (or vice versa)** — `[git_tui]` defaults to `lazygit -p {path}`. Override:
```toml
[git_tui]
command = "gitui -d {path}"
fullscreen = true
```

**Pressing `o` opens a shell when you want the file manager (or vice versa)** — `[tui.open] mode = "shell"` is the new default since issue #73. Set `mode = "finder"` for the pre-#73 OS file manager hand-off, or `mode = "editor"` to spawn `$EDITOR <path>`.

**Pressing `y` does nothing / status bar says "no clipboard tool found"** — install a per-OS clipboard helper: `pbcopy` (macOS, built-in), `wl-copy` (Wayland), `xclip` / `xsel` (X11), `clip` (Windows, built-in). The probe list is platform-fixed; first hit on `$PATH` wins.

## Quick reference card

```
gwm                          # TUI  (gwm --workspace <dir> = multi-repo)
gwm init [--preset <stack>]  # scaffold .gwm.toml (optionally from a preset)
gwm create <t> <#> <desc>    # create + bootstrap
gwm list [--format json]     # list worktrees (json = stable schema)
gwm path|cd <pat>            # print path
gwm switch | gwm s | gcd     # interactive picker (cd via shell wrapper)
gwm bootstrap [pat]          # re-run bootstrap
gwm sync [pat] [--merge]     # fetch + rebase (or merge) onto upstream
gwm remove <pat> [-b]        # remove (-b drops branch)
gwm prune                    # clean stale refs
gwm exec [slug...] -- <cmd>  # run <cmd> in each worktree (✓/✗ rollup)
gwm clean [slug...] [--yes]  # report / reclaim build artifacts
gwm review <PR#>             # materialise a PR into a worktree
gwm new <t> <desc>           # create issue from template, then its worktree
gwm pr [--draft] [--render]  # render [pr_template], then gh pr create
gwm daemon                   # JSON-RPC 2.0 over a unix socket
gwm statusline [--watch]     # one-line prompt summary off the daemon
gwm types                    # show branch types
gwm doctor [--format json]   # diagnose setup (exit 0/1/2)
gwm config get|set|list      # edit .gwm.toml (comment-preserving)
gwm history | gwm undo       # destructive-op journal / undo
gwm completions <shell>      # emit shell completion script (bash/elvish/fish/powershell/zsh)
gwm shell-init  <shell>      # emit gcd wrapper to eval (bash/fish/powershell/zsh)
gwm tmux   <pat> [-p]        # tmux window / split hand-off
gwm zellij <pat> [-p]        # zellij tab / pane hand-off
gwm link   issue|pr <N>      # bind to GitHub issue / PR
gwm unlink issue|pr          # drop the link
gwm open   [issue|pr]        # open URL in $BROWSER
gwm status [--json]          # local link + live gh state
```

## Related

- Repo: https://github.com/kbrdn1/gwm-cli
- Bash predecessor: `tools/worktree-manager.sh` (skill: `worktree-wrapper`) — `gwm` is the multi-repo replacement.
- Naming convention: `CONTRIBUTING.md` (per repo) — matches `gwm` defaults.
- Project rules for contributors / AI agents: `CLAUDE.md` (TDD mandatory, `gwm doctor` before PRs touching `.gwm.toml` / bootstrap schema / doctor).
