# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Config presets for `gwm init`** (issue #37): `gwm init --preset <name>`
  seeds an opinionated `.gwm.toml` for a known stack instead of the generic
  template. Built-ins: `laravel` (env copies + AWS-RDS guard + `vendor/`
  no-symlink + composer install), `node` / `nuxt` (`node_modules/`
  no-symlink + bun-or-npm install), `rust` (`target/` no-symlink + cargo
  fetch), `go` (`bin/` no-symlink + `go mod download`), `python-uv`
  (`.venv/` no-symlink + `uv sync`), and `generic` (the documented default).
  `gwm init --list-presets` enumerates them with one-line descriptions, and
  `gwm init --preset <name> --show` prints the resolved TOML to stdout
  without writing (handy for diffing against an existing config). Preset
  bodies are embedded in the binary and kept in sync with
  `examples/presets/<name>.toml`. `gwm init` with no flag stays byte-for-byte
  the generic template.
- **Multi-repo workspace mode** (issue #36): operate across every git repo one
  level below a root directory instead of a single repo. Two entry points:
  - `gwm --workspace ~/Projects` opens the TUI over every direct-child repo,
    and `gwm list --workspace ~/Projects` prints the merged worktree table.
    Both accept the flag before or after the subcommand (`global = true`).
  - Bare `gwm` in a directory that is *not* itself a git repo but holds
    child repos prompts `No git repo here. Open <dir> as a workspace? [Y/n]`
    (declined silently when stdin is not a terminal, so pipes / CI keep the
    old single-repo behaviour).
  - The CLI table and the TUI list gain a leading **REPO** column naming each
    row's repo. In the TUI the active repo follows the selection, so every
    selection-driven action (lazygit, terminal, sync, delete, link, open, …)
    operates on the highlighted worktree's own repo.
  - `gwm create` in workspace mode requires `--repo <name>` to disambiguate
    which child repo gets the new worktree; an absent or unknown name lists
    the candidates.
  - `.gwm.toml` stays per-repo — each row inherits its own repo's config.
    There is no workspace-level config in this version; the keymap and theme
    resolve once from the first repo, matching the single-repo "resolved once,
    relaunch to change" contract.
- **Current-PR CI status indicator in the Status pane** (issue #299): the
  Issue / PR section (`2`) now renders an overall CI state for the linked PR
  instead of a bare ` · checks N/M`. A coloured nerd-font indicator
  (` CI passing 9/9` green, ` CI failing 7/9` red, ` CI running 8/9` yellow)
  is derived from the `statusCheckRollup` already fetched — no extra GitHub
  request. The state follows **failing > running > passing** so a red check is
  never hidden behind an in-flight one; a PR with no checks renders nothing.

- **Edit every keymap from the Settings panel** (issue #294): the Settings
  panel (`4`) gains a **Keys** tab that lists every rebindable binding — the
  global list-view actions (`[global]`) and every modal verb grouped by
  context (`[modal.<context>]`) — each with its current key(s) and a
  `default`/`user`/`repo` source badge. Select a binding, press the activate
  key, and **capture** a new key live: the key column becomes a `[ … ]` input
  that records the actual keystroke(s) pressed. A multi-stroke global chord
  commits on `Enter` (`Backspace` drops the last stroke); a single-stroke
  modal verb auto-commits on the first key; `Esc` cancels.
  - The capture writes a TOML array to the layer the panel targets
    (`[tui.keys]` for a global action, `[tui.keys.modal.<context>]` for a
    modal verb), honouring the project ↔ global layer toggle (`L`).
  - The write is validated before it touches disk — a conflict /
    prefix-collision aborts it, leaves the previous binding live, and reports
    the error on the statusbar — then the keymap reloads so the new key works
    immediately, no restart.
  - An empty capture unbinds the action (global). `Esc`, `Enter`, `Backspace`
    and `Ctrl+C` can't be assigned via capture by design — hand-edit
    `.gwm.toml` for those.
  - New `config_cli::set_array_at` array write-back backs the persistence.
- **Rebindable modal keys with contextual actions** (issue #219): the
  modal/overlay keys that used to be hard-coded (create form, delete-confirm,
  link prompt, settings panel, command logs, help, open-menu, command palette,
  bootstrap report) are now remappable under nested
  `[tui.keys.modal.<context>]` sub-tables in `.gwm.toml`. Each context owns
  typed verbs, so the same physical key can mean different things in different
  modals (`Enter` is `submit` in the create form but `activate` in the
  delete-confirm modal). The dedicated `modal` namespace keeps a context from
  colliding with a same-named global action (`create`/`help`/`command_logs`/
  `link`): a global `create` array and a modal `[tui.keys.modal.create]` table
  coexist across config layers.
  - Modal bindings are single keystrokes only (no chord timeout); a
    multi-stroke chord is rejected at load time.
  - Two-stage surfaces use a dotted path —
    `[tui.keys.modal.link.choose_target]`,
    `[tui.keys.modal.link.input_number]`, `[tui.keys.modal.config.edit]`.
  - `gwm tui keys` lists every context and verb; `gwm doctor` validates the
    contextual bindings (re-reading the on-disk config) and reports
    per-context conflicts.
  - The Keybindings help overlay and the statusbar footer hints now resolve
    modal keys from the override layer instead of fixed strings.
  - `Ctrl+C`, the list view's contextual `Esc`/`Enter`, and the PTY overlay's
    emergency `Esc` stay hard-coded by design.
- **TUI keymap redesign** (issue #290): unified, consistent key bindings across
  the worktree list. New actions and default bindings:
  - `p` → `pull` — git pull on the selected worktree's branch (async,
    progress shown in status bar).
  - `P` → `push` — git push on the selected worktree's branch (async).
  - `c` → `edit_worktree` — rename modal mirroring New Worktree (Type / Issue /
    Desc), pre-filled by parsing the current branch. Submitting renames the
    local branch (`git branch -m`), the remote branch when it exists on origin
    (`git push origin :<old> <new>:<new>` + upstream re-track), and moves the
    worktree directory on disk (`git worktree move`) so the slug stays in sync
    — all off-thread.
  - `e` → `exit_to_worktree` — quit TUI and print the selected path to stdout,
    enabling `cd "$(gwm)"` shell patterns.
  - `y` → `yank_branch_name` — copy the selected branch name to clipboard.
  - `w` → `yank_worktree_name` — copy the selected worktree name to clipboard.
  - `t` → `mux_pane` — open the selected worktree in a new tmux/zellij pane.
  - `h` / `H` → `macro_one` / `macro_two` — run user-configured commands from
    `[tui.macro1]` / `[tui.macro2]` in the project `.gwm.toml`.
  - `s` (was `S`) → `sync`; `D` (was `p`) → `toggle_delete_branch`;
    `l` → `lazygit_pty`; `r` → `review_pty`.
  - Sidebar keys: `V` → `toggle_sidebar` (show/hide), `S` → `toggle_sidebar_mode`
    (Commits ↔ Stashes), `Space` → `cycle_sidebar_layout` (auto / side-by-side /
    stacked), `v` → `toggle_sidebar_position` (left ↔ right).
  - Action slugs aligned: `lazygit_fullscreen`, `terminal_pty`,
    `terminal_fullscreen`, `review_fullscreen`, `yank_path`.
  - Pre-#290 `[tui.keys]` slugs (`git_tui`, `review`, `yank`, `open`,
    `open_menu`, …) still load via backward-compat aliases.
- **`[tui.macro1]` / `[tui.macro2]` config** (issue #290): user-defined
  commands launched from the TUI. Each entry accepts a `command` string and an
  optional `open_in` field (`"pty"` — default; `"mux_pane"` for a new
  tmux/zellij pane).

- **PTY overlay for lazygit and native terminal** (issue #35): press `l` to
  open lazygit inside a ~90 % fullscreen embedded PTY overlay; press `L` to
  open it fullscreen; `o` / `O` do the same for a native `$SHELL` session. Both
  overlays stay inside the TUI — no alternate screen swap. `Esc` closes the
  overlay; `q` inside lazygit quits lazygit and auto-closes. The keybindings
  (`lazygit_pty`, `lazygit_fullscreen`, `terminal_pty`, `terminal_fullscreen`)
  are fully rebindable in `[tui.keys]`. Powered by `portable-pty 0.9` +
  `tui-term 0.3` (`tui_term::vt100` — bundled vt100 0.16).

### Changed

- **Working Tree pane as a file-explorer tree** (issue #300): the Status
  pane's Working Tree section (`2`) now renders `git status` as a nested,
  nerd-font file tree instead of a flat `XY PATH` list. Directories sort
  before files (alphabetical within a level) and single-child directory
  chains collapse (`src/tui/` then `ui.rs`); each file row carries an
  extension-driven file-type icon plus its `M` / `A` / `D` / `?` status
  badge, all painted in the file's change-category colour (so a row matches
  the footer count it belongs to). The counts footer is unchanged. The tree
  builder (flat status → tree model) is a pure, ratatui-free function with
  unit tests.
  - **Tree connector lines** (`├─` / `└─` / `│`) draw the hierarchy like
    `tree(1)`, in the muted role.
  - **Directory rows are coloured retroactively by git**: a folder takes the
    aggregate change-category of its subtree — only-modified → yellow,
    only-new → green, only-deleted → red, mixed → neutral accent.
  - An **extra space** pads each nerd-font glyph (most render double-width in
    a single cell) so the following name isn't clipped.
  - A pathological untracked directory is bounded at **two levels**: the
    `git status` scan is streamed and **stopped after 5000 records** (git is
    killed once the cap is hit, so its directory walk can't run away), and the
    tree then renders at most **500 file leaves** with the remainder shown as
    a single `… N more` row (`… N+ more` when the scan itself was truncated,
    since the true total is then unknown) — selecting such a worktree can
    neither stall the scan nor flood the non-scrollable section. The scan
    runs under `--no-optional-locks` so killing it at the cap can't leave a
    stale `.git/index.lock`, and stderr is drained off-thread to avoid a pipe
    deadlock.
  - File and directory names are **sanitised** before rendering (control
    characters → `?`), so a verbatim `-z` filename can't corrupt the sidebar
    layout or inject terminal escape sequences.
  - `git_status_short` now reads `git status --porcelain -z
    --untracked-files=all`: `-uall` expands an untracked directory into its
    individual files (git-ignored paths stay excluded), and `--porcelain -z`
    emits paths verbatim and NUL-delimited — so filenames with spaces,
    arrows, quotes, or non-ASCII bytes (and rename source/destination) parse
    unambiguously. The footer counts share the same `-z` parser, so a rename
    counts once and the total always matches the rows rendered.
- **Worktree table: label the issue/PR badge column** (issue #294): the second
  table column (the `●` / `●` issue / PR pastilles) now carries an `I/P` header
  so the badges read self-explanatory next to the `Worktree` / `Branch` columns.

### Dependencies

- Added `portable-pty 0.9` (cross-platform PTY pair) and `tui-term 0.3`
  (ratatui widget rendering a vt100 parser buffer).

## Past releases

In reverse chronological order:

- [`0.9.0`](changelogs/0.9.0.md) — 2026-06-07
- [`0.8.0`](changelogs/0.8.0.md) — 2026-06-01
- [`0.7.0`](changelogs/0.7.0.md) — 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.10.0-rc.1`](changelogs/pre-releases/0.10.0-rc.1.md) — 2026-06-10
- [`0.9.0-rc.3`](changelogs/pre-releases/0.9.0-rc.3.md) — 2026-06-07
- [`0.9.0-rc.2`](changelogs/pre-releases/0.9.0-rc.2.md) — 2026-06-06
- [`0.9.0-rc.1`](changelogs/pre-releases/0.9.0-rc.1.md) — 2026-06-02
- [`0.8.0-rc.5`](changelogs/pre-releases/0.8.0-rc.5.md) — 2026-06-01
- [`0.8.0-rc.4`](changelogs/pre-releases/0.8.0-rc.4.md) — 2026-05-29
- [`0.8.0-rc.3`](changelogs/pre-releases/0.8.0-rc.3.md) — 2026-05-29
- [`0.8.0-rc.2`](changelogs/pre-releases/0.8.0-rc.2.md) — 2026-05-23
- [`0.8.0-rc.1`](changelogs/pre-releases/0.8.0-rc.1.md) — 2026-05-23
- [`0.7.0-rc.3`](changelogs/pre-releases/0.7.0-rc.3.md) — 2026-05-23
- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md) — 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md) — 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md) — 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
