# gwm — roadmap

This document tracks where `gwm` is heading. It complements [CHANGELOG.md](CHANGELOG.md) (what already shipped) and the [open issues](https://github.com/kbrdn1/gwm-cli/issues) (the source of truth for scope details).

Each item below links to its GitHub issue. The scope, alternatives considered, and acceptance criteria live there — this file is the map, not the spec.

## Current state — v1.5.0 stable

The current **stable** line is **v1.5.0** (2026-07-26). The machine-readable
contracts frozen at 1.0.0 still hold: the CLI subcommands / flags / exit codes,
the `--format=json` schemas, the daemon JSON-RPC protocol, and the `.gwm.toml`
section set will not break without a major bump (see
[Stability & compatibility](docs/6.development/3.stability.md)).

Since the 1.0.0 milestone (2026-06-26): the **1.0.x patches** hardened the line
(security-only 1.0.3 among them); **1.1.0** shipped the first outside-report
features ([#363](https://github.com/kbrdn1/gwm-cli/issues/363) — persisted
sidebar layout, OSC 52 yank that works over SSH) and **1.1.1** fixed global
config resolution on macOS; **1.2.0** was the distribution push
([#383](https://github.com/kbrdn1/gwm-cli/issues/383) — Scoop, `.deb` / `.rpm`,
AUR, aqua, with winget wired and pending upstream); **1.3.0** shipped the
**agent session pane** ([#408](https://github.com/kbrdn1/gwm-cli/issues/408))
and its follow-ups, including the Windows named pipe transport for the daemon
([#439](https://github.com/kbrdn1/gwm-cli/issues/439)); **1.4.0** completed the
`?` help overlay ([#453](https://github.com/kbrdn1/gwm-cli/issues/453)) and
closed the TUI-polish trio ([#436](https://github.com/kbrdn1/gwm-cli/issues/436)
/ [#437](https://github.com/kbrdn1/gwm-cli/issues/437) /
[#438](https://github.com/kbrdn1/gwm-cli/issues/438)); **1.5.0** made gwm
multi-forge ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)), adding a
`Forge` trait and a GitLab (`glab`) backend behind it. See
[Shipped highlights](#shipped-highlights).

1.0.0 promotes the entire **0.10.0 train**: the **rc.1** Settings-editability +
TUI enrichment cycle; the **rc.2** train (embedded PTY overlays, the TUI keymap
redesign, rebindable contextual modal keys + the Settings Keys tab, multi-repo
workspace mode, config presets for `gwm init`, the JSON API + daemon, the
current-PR CI indicator, and the file-explorer Working Tree pane); the **rc.3**
train that **closes the GitHub loop** — `gwm review <PR#>` and `gwm statusline`;
and the **rc.4** fleet-chore pair `gwm exec` / `gwm clean`. The post-rc.4 stable
delta completes the 1.0 commitment: the frozen, versioned machine contracts
([#317](https://github.com/kbrdn1/gwm-cli/issues/317)), the published stability
policy ([#318](https://github.com/kbrdn1/gwm-cli/issues/318)), the frozen
`exec` / `clean` surface decision ([#319](https://github.com/kbrdn1/gwm-cli/issues/319)),
and the additive features it anticipated — named `[exec]` / `[clean]` profiles,
bounded `--jobs`, and `--workspace` fan-out ([#324](https://github.com/kbrdn1/gwm-cli/issues/324),
[#326](https://github.com/kbrdn1/gwm-cli/issues/326)) — plus TUI exec / clean
overlays ([#325](https://github.com/kbrdn1/gwm-cli/issues/325)). See the
[Shipped highlights](#shipped-highlights) table for the per-issue breakdown and
[`changelogs/1.0.0.md`](changelogs/1.0.0.md) for the consolidated notes. The
MSRV is **1.86** (raised by the PTY overlay's `portable-pty` / `tui-term`
dependencies; MSRV bumps ride a minor per the stability policy).

The 0.9.x stable line ships:

- **Native worktree ops via libgit2 (vendored)** — single binary, no `gwq` / `git` CLI dependency.
- **CLI + ratatui TUI** — `gwm <subcommand>` for scripts, `gwm` alone opens the interactive interface.
- **Per-repo `.gwm.toml` + user-level global config** — branch / path conventions, configurable branch types, declarative GitHub labels / milestones, file copies, regex guards (`abort` or `seed-from-example`), no-symlink invariants. A user-level `~/.config/gwm/config.toml` deep-merges **underneath** each repo's `.gwm.toml` (repo wins on conflicts; `GWM_NO_GLOBAL_CONFIG=1` forces repo-only). `{home}` / `{repo}` / `{repo_path}` / `{repo_parent}` placeholders for repo-relative bases.
- **Config CLI** — `gwm config get / set / unset / list / validate / path / edit`, git-config-style over dotted keys, comment-preserving via `toml_edit`.
- **Lifecycle hooks `[hooks.*]`** — declarative `pre_create` / `post_create` / `pre_bootstrap` / `post_bootstrap` / `pre_remove` / `post_remove` phases, per-step `on_fail = abort|warn|ignore`, `--skip-hooks` escape hatch, gated by the same `when:` predicates (`file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!`, `&&`, `||` composition). Legacy `[[bootstrap.command]]` is auto-aliased to `[[hooks.post_create]]`.
- **CLI aliases + Gitmoji convention** — `[aliases]` in `.gwm.toml` (or `~/.config/gwm/aliases.toml`) expand `gwm <alias>` to argv before clap parses, with `gwm aliases list`; `[gitmoji]` mapping powers `gwm commit-prefix`, `gwm types --gitmoji`, and an opt-in `gwm hooks install commit-msg` hook.
- **GitHub issue / PR templates** — `[issue_template]` + `gwm new <type> <desc>` (create issue from a form template, then spin up the linked worktree); `[pr_template]` + `gwm pr [--draft] [--base] [--render]` with `{commits}` / `{files_changed}` placeholders.
- **Safety daily** — `--dry-run` on `gwm remove` / `gwm prune` (preview before destroying); `gwm undo` + `gwm history` backed by an operation journal at `$XDG_DATA_HOME/gwm/history.toml` (100-entry rotation, per-repo filtering) to recover a misfired removal without `git reflog`.
- **`gwm sync [<pattern>] [--merge]`** — fetch a worktree's upstream and rebase (or merge) its branch onto it; refuses a dirty tree or missing upstream, aborts a conflicting rebase/merge to keep the worktree usable.
- **Bootstrap hardening for hostile clones** — TOFU trust ledger on `.gwm.toml`, `--allow-bootstrap` / `--deny-bootstrap`, path-traversal rejection, symlink-safe copy/write primitives, load-time regex validation for deny patterns.
- **Async-task spine (new in 0.9.0)** — a generic off-thread worker (coalescing + late-result drop) keeps the event loop responsive: the worktree list refresh (`f` / `r`), the GitHub issue/PR fetch (`F`, per-key generation guard fixes a stale-data race), `gwm sync` (`S`), and bootstrap (`b`) all run on it, animating the statusbar spinner instead of blocking. The TOFU gate stays synchronous before any bootstrap spawn.
- **In-TUI pane-key family `1` / `2` / `3` / `4` (new in 0.9.0)** — `1` / `2` focus the worktrees / status panes; `3` opens a lazygit-style Command Logs overlay (scrollable transcript of the external commands gwm ran); `4` opens a Configuration panel showing the **resolved** `.gwm.toml` with a per-row source column (repo / user / default). `.` opens the docs in the browser.
- **Full theme-role coverage (new in 0.9.0)** — the resolved `[theme]` is threaded through every `draw_*` site, with dedicated `name` / `path` chrome roles and `staged` / `modified` / `untracked` working-tree roles; all defaults preserved, pinned by `tests/tui_theme_audit_tests.rs`.
- **Lazygit-style details sidebar** — four bordered subsections (Worktree / Issue · PR / Working Tree / Recent Commits), status-coloured branch names, header status dot tracking linked PR / issue state, 300-commit Recent Commits buffer with the full topology renderer (`○ ◎ │ ╮ ╭ ╯ ╰ ┴ ┬ ─`).
- **Measured TUI sidebar perf pass** — branch age is cached on `WorktreeInfo`, `filtered_indices` is memoised on `FilterState`, Recent Commits uses a cached libgit2 revwalk keyed by `(worktree path, head OID, limit)`, and commit-graph pipes store `git2::Oid` instead of heap-allocated hash strings.
- **Configurable launchers** — `[git_tui]` drives `l` (default `lazygit -p {path}`), `[review]` drives `R` (presets: `lumen` / `claude` / `codex` / `aider` / `gh`, plus free-form `command =`). Placeholders `{base} {head} {path} {diff}` with lazy diff materialisation.
- **GitHub issue / PR linking + auto-detection** — branches matching `<type>/#<N>-<slug>` auto-link to their issue; CLI `link / unlink / open / status` for explicit overrides; live state surfaces in the TUI sidebar via `gh`. A branch's PR is also resolved ephemerally (`gh pr list --head`) and surfaced as `detected` — never written to git config, so an explicit link always wins (`gwm status`, the sidebar `F` refresh, opt-in `gwm list --detect-pr`).
- **TUI personalisation** — `[tui.keys]` remappable keymap with multi-key chord support (`g g`) + `gwm tui keys`; `:` command palette overlay sharing the keystroke `Action` dispatcher; `[theme]` role-based colours with `catppuccin` / `gruvbox` / `tokyo-night` / `claude-dark` presets + `gwm theme list / show`; sidebar stashes mode toggled by `s`.
- **`[tui.open]` dispatch** — `o` key now spawns `$SHELL` in the worktree by default; opt back to OS file manager via `mode = "finder"`.
- **`y: yank`** — copy the selected worktree's path to the clipboard (pbcopy / wl-copy / xclip / xsel / clip).
- **Vim motions** — `gg` / `G` jump to first / last; `Tab` swaps focus between the list and the sidebar; `j` / `k` / `↑` / `↓` move selection or scroll the focused panel.
- **Fuzzy filter (`/`)** — sticky `nucleo-matcher` filter on the worktree list; smart-case, AND on spaces, contiguous beats spread-out; same engine powers `gwm switch` (picker mode), `gwm path / cd / remove / bootstrap` (fuzzy CLI lookup).
- **One-line `cd`** — `gwm shell-init <shell>` wires up a `gcd <pattern>` (resolve + cd) and bare `gcd` (picker + cd) for zsh / bash / fish / PowerShell.
- **Shell completions** — `gwm completions <shell>` for zsh / bash / fish / PowerShell / elvish (static script generated from the live clap argument tree).
- **Multiplexer integration** — `gwm tmux <pattern> [-p]` and `gwm zellij <pattern> [-p]` open the worktree in a new window / pane / tab; refuse to spawn outside an active session.
- **Responsive + polished TUI chrome** — the details sidebar stacks under the table on a narrow terminal (`< 120` cols) instead of disappearing; `V` cycles `auto → side-by-side → stacked`, `H` / `[tui] sidebar_position` flips it left/right. Borderless styled header, single-line statusline with reverse-video badge chips, content-sized themed modals (confirm buttons, animated spinner), git-style working-tree colourisation.
- **`gwm doctor`** — 8 checks (parses / guard refs / `when` predicates / external binaries / prunable / orphan branches / base writable / unbound `quit` keymap), exit codes `0/1/2` for CI.
- **Confirm-overlay countdown** — safety countdown on the delete-confirm overlay when `p` (delete-branch-on-remove) is armed; duration tunable via `[tui].confirm_countdown_secs` (0..=5, clamped).
- **State-sliced TUI internals** — `tui::app::App` is decomposed into `tui/state/{create_form,filter,confirm,link_prompt,sidebar,github_fetch}.rs`, with dedicated tests for each state slice.
- **Release pipeline** — `release.yml` on `vX.Y.Z` tags, `pre-release.yml` on `-rc.N` / `-alpha.N` / `-beta.N` tags, 5-target build matrix (Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64), GitHub Release assets published through the `gh` CLI with the workflow token ([#146](https://github.com/kbrdn1/gwm-cli/issues/146) resolved), per-version changelog body sourced from `changelogs/<version>.md` (hard-fails if missing), pre-release `[Unreleased]` dupe guard, Homebrew tap update job on stable releases, `cargo binstall` support, Nix flake at the repo root. CI test matrix runs on `ubuntu-latest` / `macos-latest` / `windows-latest`.
- **1000+ tests** — integration and state-machine tests covering config (repo + global layering), aliases, gitmoji, hooks, config CLI, naming, bootstrap, doctor, GitHub linking + PR auto-detection, launcher, multiplexer, homebrew formula, binstall metadata, pre-commit hook, TUI state slices (keymap / palette / theme / sidebar), undo/history journal, worktree libgit2 integration, release workflow guards, and CLI end-to-end.

See [`changelogs/0.9.0.md`](changelogs/0.9.0.md) for the full v0.9.0 release notes, and [`changelogs/`](changelogs/) for the per-version archive.

## Shipped highlights

For reference (each linked to its closing PR):

| Issue | Shipped in | Feature                                                                         |
|:------|:-----------|:--------------------------------------------------------------------------------|
| [#18](https://github.com/kbrdn1/gwm-cli/issues/18) | v0.3.0 | Shell completions (zsh / bash / fish / powershell / elvish)                     |
| [#19](https://github.com/kbrdn1/gwm-cli/issues/19) | v0.3.0 | `gwm cd <pattern>` + `gwm shell-init <shell>` (the `gcd` wrapper)               |
| [#20](https://github.com/kbrdn1/gwm-cli/issues/20) | v0.3.0 | `gwm doctor` (initial check set)                                                |
| [#21](https://github.com/kbrdn1/gwm-cli/issues/21) | v0.3.0 | TUI fuzzy filter (`/`)                                                          |
| [#22](https://github.com/kbrdn1/gwm-cli/issues/22) | v0.4.0 | `gwm switch` (picker UI printing the chosen path on stdout)                     |
| [#23](https://github.com/kbrdn1/gwm-cli/issues/23) | v0.4.0 | Tmux / Zellij integration (`gwm tmux` / `gwm zellij`)                           |
| [#25](https://github.com/kbrdn1/gwm-cli/issues/25) | v0.4.0 | Extended `when:` predicates (`cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!` / `&&` / `\|\|`) |
| [#26](https://github.com/kbrdn1/gwm-cli/issues/26) | v0.5.0 | Homebrew tap (`brew tap kbrdn1/tap && brew install gwm`)                        |
| [#28](https://github.com/kbrdn1/gwm-cli/issues/28) | v0.5.0 | Nix flake (`nix profile install github:kbrdn1/gwm-cli`)                         |
| [#30](https://github.com/kbrdn1/gwm-cli/issues/30) | v0.5.0 | TUI confirm-overlay countdown                                                   |
| [#47](https://github.com/kbrdn1/gwm-cli/issues/47) | v0.5.0 | `gwm doctor`: skip merged gwm-style branches in the orphan check                |
| [#59](https://github.com/kbrdn1/gwm-cli/issues/59) | v0.5.0 | `[doctor].trunks` config knob                                                   |
| [#67](https://github.com/kbrdn1/gwm-cli/issues/67) ([PR #68](https://github.com/kbrdn1/gwm-cli/pull/68)) | v0.6.0-rc.1 | Issue / PR linking — CLI + TUI controls, `gh`-backed live status     |
| [#69](https://github.com/kbrdn1/gwm-cli/issues/69) ([PR #70](https://github.com/kbrdn1/gwm-cli/pull/70)) | v0.6.0 | TUI Details sidebar redesign (four bordered subsections)            |
| [#71](https://github.com/kbrdn1/gwm-cli/issues/71) ([PR #72](https://github.com/kbrdn1/gwm-cli/pull/72)) | v0.6.0 | TUI Recent Commits panel — lazygit-style layout + full topology renderer |
| [#73](https://github.com/kbrdn1/gwm-cli/issues/73) ([PR #74](https://github.com/kbrdn1/gwm-cli/pull/74)) | v0.6.0 | Lazygit-style sidebar facelift (`Created` row, status colours, `[tui.open]`, `y: yank`) |
| [#75](https://github.com/kbrdn1/gwm-cli/issues/75) ([PR #76](https://github.com/kbrdn1/gwm-cli/pull/76)) | v0.6.0 | Configurable launchers (`[git_tui]` + `[review]`) — keymap reshuffle `r/R → f/F`, new `R` |
| [#77](https://github.com/kbrdn1/gwm-cli/issues/77) | v0.6.0 | Docs restructure into `docs/` tree (Nuxt Content conventions) + README shrunk to landing |
| [#80](https://github.com/kbrdn1/gwm-cli/issues/80) / [#81](https://github.com/kbrdn1/gwm-cli/issues/81) / [#82](https://github.com/kbrdn1/gwm-cli/issues/82) | v0.7.0-rc.1 | Configurable branch types, declarative GitHub labels, declarative GitHub milestones |
| [#93](https://github.com/kbrdn1/gwm-cli/issues/93) / [#94](https://github.com/kbrdn1/gwm-cli/issues/94) / [#95](https://github.com/kbrdn1/gwm-cli/issues/95) / [#96](https://github.com/kbrdn1/gwm-cli/issues/96) | v0.7.0-rc.1 | Bootstrap hardening: symlink-safe copies, path traversal rejection, TOFU trust ledger, guard regex load validation |
| [#97](https://github.com/kbrdn1/gwm-cli/issues/97) / [#98](https://github.com/kbrdn1/gwm-cli/issues/98) / [#99](https://github.com/kbrdn1/gwm-cli/issues/99) / [#100](https://github.com/kbrdn1/gwm-cli/issues/100) / [#101](https://github.com/kbrdn1/gwm-cli/issues/101) | v0.7.0-rc.2 | Static regex lifting, worktree removal ordering fix, stale-branch refusal, argv-injection guards, E2E create/remove/init tests |
| [#102](https://github.com/kbrdn1/gwm-cli/issues/102) / [#123](https://github.com/kbrdn1/gwm-cli/issues/123) / [#124](https://github.com/kbrdn1/gwm-cli/issues/124) / [#125](https://github.com/kbrdn1/gwm-cli/issues/125) / [#126](https://github.com/kbrdn1/gwm-cli/issues/126) / [#127](https://github.com/kbrdn1/gwm-cli/issues/127) / [#128](https://github.com/kbrdn1/gwm-cli/issues/128) | v0.7.0-rc.2 | `tui::app::App` decomposed into focused `tui/state/` sub-structs |
| [#103](https://github.com/kbrdn1/gwm-cli/issues/103) / [#104](https://github.com/kbrdn1/gwm-cli/issues/104) | v0.7.0-rc.2 | TUI render-loop perf: cached branch age and memoised `filtered_indices` |
| [#105](https://github.com/kbrdn1/gwm-cli/issues/105) / [#106](https://github.com/kbrdn1/gwm-cli/issues/106) | v0.7.0-rc.2 | Typed error variants and shared constructors/render helpers |
| [#138](https://github.com/kbrdn1/gwm-cli/issues/138) | v0.7.0-rc.3 | `GitHubFetch` cache keyed by issue/PR number; late results dropped after `invalidate()` |
| [#131](https://github.com/kbrdn1/gwm-cli/pull/131) / [#134](https://github.com/kbrdn1/gwm-cli/pull/134) | v0.7.0-rc.3 | TUI state encapsulation polish for `ConfirmModal` and `FilterState` |
| [#107](https://github.com/kbrdn1/gwm-cli/issues/107) / [#108](https://github.com/kbrdn1/gwm-cli/issues/108) | v0.7.0 | Measured P3 TUI sidebar perf: cached libgit2 Recent Commits and `Oid` commit graph pipes |
| [#146](https://github.com/kbrdn1/gwm-cli/issues/146) / [#147](https://github.com/kbrdn1/gwm-cli/issues/147) / [#112](https://github.com/kbrdn1/gwm-cli/issues/112) | v0.8.0-rc.1 | Release hardening: `gh`-CLI publish + workflow token, pre-release `[Unreleased]` dupe guard, Windows in the test matrix |
| [#86](https://github.com/kbrdn1/gwm-cli/issues/86) / [#85](https://github.com/kbrdn1/gwm-cli/issues/85) | v0.8.0-rc.1 | CLI aliases (`[aliases]` in `.gwm.toml` + user fallback, pre-clap expansion), gitmoji mapping + `gwm commit-prefix` + opt-in `commit-msg` hook |
| [#31](https://github.com/kbrdn1/gwm-cli/issues/31) / [#29](https://github.com/kbrdn1/gwm-cli/issues/29) | v0.8.0-rc.2 | Safety daily: `--dry-run` on `gwm remove` / `gwm prune`, `gwm undo` + `gwm history` operation journal at `$XDG_DATA_HOME/gwm/history.toml` |
| [#89](https://github.com/kbrdn1/gwm-cli/issues/89) / [#88](https://github.com/kbrdn1/gwm-cli/issues/88) | v0.8.0-rc.3 | Config CLI (`gwm config get/set/unset/list/validate/path/edit`, comment-preserving `toml_edit`) + `[hooks.*]` lifecycle hooks (six phases, `on_fail`, `[[bootstrap.command]]` compat) |
| [#83](https://github.com/kbrdn1/gwm-cli/issues/83) / [#84](https://github.com/kbrdn1/gwm-cli/issues/84) | v0.8.0-rc.3 | GitHub templates: `[issue_template]` + `gwm new`, `[pr_template]` + `gwm pr` with `{commits}` / `{files_changed}` placeholders |
| [#87](https://github.com/kbrdn1/gwm-cli/issues/87) / [#32](https://github.com/kbrdn1/gwm-cli/issues/32) / [#33](https://github.com/kbrdn1/gwm-cli/issues/33) / [#34](https://github.com/kbrdn1/gwm-cli/issues/34) | v0.8.0-rc.3 | TUI personalisation: `[tui.keys]` remappable keymap with chords + `gwm tui keys`, command palette (`:`), `[theme]` role-based presets, sidebar stashes mode (`s`) |
| [#24](https://github.com/kbrdn1/gwm-cli/issues/24) / [#27](https://github.com/kbrdn1/gwm-cli/issues/27) | v0.8.0-rc.4 | Quick wins: `gwm sync [<pattern>] [--merge]` (fetch + rebase/merge onto upstream, conflict-safe) and `cargo-binstall` support via `[package.metadata.binstall]` |
| [#190](https://github.com/kbrdn1/gwm-cli/issues/190) / [#188](https://github.com/kbrdn1/gwm-cli/issues/188) / [#185](https://github.com/kbrdn1/gwm-cli/issues/185) / [#187](https://github.com/kbrdn1/gwm-cli/issues/187) / [#180](https://github.com/kbrdn1/gwm-cli/issues/180) / [#179](https://github.com/kbrdn1/gwm-cli/issues/179) / [#181](https://github.com/kbrdn1/gwm-cli/issues/181) / [#175](https://github.com/kbrdn1/gwm-cli/issues/175) | v0.8.0-rc.5 | Personalisation + polish: user-level global config (`~/.config/gwm/config.toml`, deep-merged under `.gwm.toml`), responsive TUI sidebar (`V` orientation / `H` left-right + `[tui] sidebar_position`), `claude-dark` preset + reworked borderless header, modal polish pass (themed content-sized frames, confirm buttons, spinner), single-line statusline, working-tree colourisation, ephemeral PR auto-detection, `{repo_path}` / `{repo_parent}` placeholders |
| [#170](https://github.com/kbrdn1/gwm-cli/issues/170) / [#210](https://github.com/kbrdn1/gwm-cli/issues/210) / [#211](https://github.com/kbrdn1/gwm-cli/issues/211) / [#214](https://github.com/kbrdn1/gwm-cli/issues/214) / [#169](https://github.com/kbrdn1/gwm-cli/issues/169) | v0.9.0-rc.1 | Theme-role coverage finish: `[theme]` threaded through every `draw_*` site, dedicated `name` / `path` roles for the chrome and `staged` / `modified` / `untracked` roles for the working-tree panel (all defaults preserved, pinned by `tests/tui_theme_audit_tests.rs`); `git2` `0.20` → `0.21` source migration |
| [#217](https://github.com/kbrdn1/gwm-cli/issues/217) / [#220](https://github.com/kbrdn1/gwm-cli/issues/220) / [#222](https://github.com/kbrdn1/gwm-cli/issues/222) / [#224](https://github.com/kbrdn1/gwm-cli/issues/224) | v0.9.0-rc.2 | TUI statusbar / layout / modal polish: contextual statusbar (context chip + animated GitHub-fetch spinner + action log), direct pane-focus keys (`1` / `2`, rebindable `focus_worktrees` / `focus_status`), off-thread GitHub `F` fetch, reworked create-worktree / link / confirm-delete / Issue-PR modals, stacked-by-default sidebar with per-axis split ratios |
| [#231](https://github.com/kbrdn1/gwm-cli/issues/231) / [#226](https://github.com/kbrdn1/gwm-cli/issues/226) / [#232](https://github.com/kbrdn1/gwm-cli/issues/232) / [#255](https://github.com/kbrdn1/gwm-cli/issues/255) / [#258](https://github.com/kbrdn1/gwm-cli/issues/258) / [#262](https://github.com/kbrdn1/gwm-cli/issues/262) | v0.9.0-rc.3 | Async-task spine: generic off-thread worker (coalescing + late-result drop), off-thread worktree refresh (`f` / `r`) and GitHub `F` fetch (per-key generation fixes a stale-data race) on the shared spine; in-TUI Command Logs overlay (`3`), Configuration panel (`4`, per-row source attribution), and `sync` action (`S`, off-thread rebase) completing the `1`/`2`/`3`/`4` pane-key family; fuzzy-filter-in-pane-title + command-palette input polish; internal refactor / perf sweep (#235–#244, per-frame sidebar clone dropped) |
| [#256](https://github.com/kbrdn1/gwm-cli/issues/256) / [#233](https://github.com/kbrdn1/gwm-cli/issues/233) / [#248](https://github.com/kbrdn1/gwm-cli/issues/248) | v0.9.0 | Stable delta: off-thread bootstrap (`b`) on the spine (trust gate stays synchronous; Report on completion), open the docs in the browser (`.`, rebindable `open_docs`), and a CI-flaky GitHub-detect test fixed at the root (distinct write-once fake-`gh` scripts) |
| [#279](https://github.com/kbrdn1/gwm-cli/issues/279) / [#257](https://github.com/kbrdn1/gwm-cli/issues/257) / [#276](https://github.com/kbrdn1/gwm-cli/issues/276) / [#267](https://github.com/kbrdn1/gwm-cli/issues/267) / [#283](https://github.com/kbrdn1/gwm-cli/issues/283) / [#285](https://github.com/kbrdn1/gwm-cli/issues/285) / [#281](https://github.com/kbrdn1/gwm-cli/issues/281) / [#287](https://github.com/kbrdn1/gwm-cli/issues/287) | v0.10.0-rc.1 | Settings-editability + TUI enrichment: editable Settings panel (`4`) with category tabs (Theme / Worktree / TUI / All), per-layer selector (`L`), live-persist TOML; reusable `LoaderWidget` (delete modal consumer); async create-worktree and quit-wait on the spine; Working Tree colour-coded nerdfont counts + row recolouring; Status pane `Diff +ins -del` vs base (three-dot); Issue/PR `●/●` pastilles + nerdfont state-chip badges; cached GitHub state (titles + states survive restarts), initial startup refresh, periodic `[tui].auto_refresh_secs` auto-refresh; herdr-style scrollbars on scrollable modals; flat which-key hints |
| [#35](https://github.com/kbrdn1/gwm-cli/issues/35) ([PR #289](https://github.com/kbrdn1/gwm-cli/pull/289)) / [#290](https://github.com/kbrdn1/gwm-cli/issues/290) ([PR #292](https://github.com/kbrdn1/gwm-cli/pull/292)) / [#219](https://github.com/kbrdn1/gwm-cli/issues/219) ([PR #293](https://github.com/kbrdn1/gwm-cli/pull/293)) / [#294](https://github.com/kbrdn1/gwm-cli/issues/294) ([PR #297](https://github.com/kbrdn1/gwm-cli/pull/297)) / [#300](https://github.com/kbrdn1/gwm-cli/issues/300) ([PR #301](https://github.com/kbrdn1/gwm-cli/pull/301)) / [#299](https://github.com/kbrdn1/gwm-cli/issues/299) ([PR #302](https://github.com/kbrdn1/gwm-cli/pull/302)) / [#36](https://github.com/kbrdn1/gwm-cli/issues/36) ([PR #303](https://github.com/kbrdn1/gwm-cli/pull/303)) / [#37](https://github.com/kbrdn1/gwm-cli/issues/37) ([PR #305](https://github.com/kbrdn1/gwm-cli/pull/305)) / [#38](https://github.com/kbrdn1/gwm-cli/issues/38) ([PR #306](https://github.com/kbrdn1/gwm-cli/pull/306)) | v0.10.0-rc.2 | PTY + power-user + integration train: embedded PTY overlays for lazygit (`l` / `L`) and a native `$SHELL` (`o` / `O`) via `portable-pty` + `tui-term` (MSRV → 1.86); TUI keymap redesign (unified list-view bindings — `p`/`P` pull/push, `c` edit-worktree, `e` exit-to-worktree, `y`/`w` yank, `t` mux pane, `h`/`H` macros — plus `[tui.macro1]` / `[tui.macro2]`); rebindable contextual modal keys under `[tui.keys.modal.<context>]`; edit every keymap live from the Settings Keys tab (keystroke capture + validated write-back); Working Tree pane as a nerd-font file-explorer tree (git-coloured rows, bounded scan); current-PR CI indicator in the Status pane; multi-repo workspace mode (`--workspace`, REPO column, `gwm create --repo`); config presets for `gwm init` (`--preset` / `--list-presets` / `--show`); JSON API (`--format=json` on `list` / `doctor` / `path`) + `gwm daemon` (JSON-RPC 2.0 over a unix socket with `subscribe`) |
| [#308](https://github.com/kbrdn1/gwm-cli/issues/308) ([PR #310](https://github.com/kbrdn1/gwm-cli/pull/310)) | v0.10.0-rc.3 | `gwm review <PR#>` — materialise an existing GitHub PR into an isolated worktree (resolves head via `gh`, fetches `refs/pull/<N>/head` — cross-fork aware, any PR state — into `review/pr-<N>-<author>-<slug>`, links the PR + points the diff base at `origin/<base>`); safe-by-default (no bootstrap / lifecycle hooks unless `--bootstrap`), `--name` overrides the branch |
| [#309](https://github.com/kbrdn1/gwm-cli/issues/309) ([PR #311](https://github.com/kbrdn1/gwm-cli/pull/311)) | v0.10.0-rc.3 | `gwm statusline` — first real consumer of `gwm daemon` (#38): a dependency-free client rendering a compact one-line worktree summary (active branch, count, dirty / ahead / behind, linked issue / PR) for tmux / starship / zsh; `--watch` rides the `subscribe` stream, degrades to a blank line + exit `0` when no daemon is reachable; `docs/5.integrations/4.daemon-consumers.md` (EN + FR) |
| [#313](https://github.com/kbrdn1/gwm-cli/issues/313) ([PR #314](https://github.com/kbrdn1/gwm-cli/pull/314)) | v0.10.0-rc.4 | Fleet chores across worktrees: `gwm exec [<slug>...] -- <cmd>` (run a command in each worktree sequentially — every non-main worktree by default, or listed slugs; everything after `--` forwarded verbatim; `✓ / ✗` rollup, non-zero exit on any failure) and `gwm clean [<slug>...] [--yes]` (report — or with `--yes` reclaim — heavy build artifacts `target/` / `node_modules/` / `dist/` / `build/`; deletes only git-ignored dirs, never follows symlinks, not journaled into `gwm history`) |
| [#317](https://github.com/kbrdn1/gwm-cli/issues/317) / [#318](https://github.com/kbrdn1/gwm-cli/issues/318) / [#319](https://github.com/kbrdn1/gwm-cli/issues/319) / [#324](https://github.com/kbrdn1/gwm-cli/issues/324) / [#325](https://github.com/kbrdn1/gwm-cli/issues/325) / [#326](https://github.com/kbrdn1/gwm-cli/issues/326) / [#334](https://github.com/kbrdn1/gwm-cli/issues/334) | v1.0.0 | The 1.0 commitment: frozen, versioned machine contracts pinned by `tests/contract_tests.rs` (`SCHEMA_VERSION = 1`, daemon `schema_version`, `docs/schema/README.md`) (#317); published stability & compatibility policy (`docs/6.development/3.stability.md`, EN + FR) (#318); frozen `exec` / `clean` surface decision (#319); named `[exec]` / `[clean]` config profiles + bounded `--jobs` parallelism (#324); `--workspace` fan-out for `gwm exec` / `gwm clean` (#326); TUI exec (`x`) / clean (`X`) overlays (#325); help-overlay + overlay polish (#334) |
| [#363](https://github.com/kbrdn1/gwm-cli/issues/363) | v1.1.0 | First outside-report release: persisted sidebar layout (`V` / `H` write back to config) and OSC 52 clipboard yank that works over SSH |
| [#383](https://github.com/kbrdn1/gwm-cli/issues/383) | v1.2.0 | Distribution push: Scoop bucket, native `.deb` / `.rpm` packages, AUR (`gwm-cli-bin`), aqua standard registry; winget wired, pending upstream merge |
| [#408](https://github.com/kbrdn1/gwm-cli/issues/408) ([PR #435](https://github.com/kbrdn1/gwm-cli/pull/435)) + follow-ups [#439](https://github.com/kbrdn1/gwm-cli/issues/439) ([PR #444](https://github.com/kbrdn1/gwm-cli/pull/444)) / [#440](https://github.com/kbrdn1/gwm-cli/issues/440) ([PR #442](https://github.com/kbrdn1/gwm-cli/pull/442)) / [#441](https://github.com/kbrdn1/gwm-cli/issues/441) ([PR #443](https://github.com/kbrdn1/gwm-cli/pull/443)) / [#445](https://github.com/kbrdn1/gwm-cli/issues/445) ([PR #446](https://github.com/kbrdn1/gwm-cli/pull/446)) | v1.3.0 | **Agent session pane**: detect AI-agent sessions (Claude Code, Codex, opencode, Mistral Vibe) per worktree from on-disk artefacts — AGENT column (table + TUI), `a` detail overlay with pinning, `gwm agents` CLI, additive JSON `agents` field, statusline segment. Follow-ups: **Windows named pipe transport** for `gwm daemon` / `gwm statusline` (owner-only pipe, server identity verified by owner SID) (#439); `gwm clean` ENOTEMPTY race tolerance (#440); process-level liveness — a dead recorded PID demotes the session immediately on unix (#441); fixed-height attach prompt with a scrollbar (#445) |
| [#437](https://github.com/kbrdn1/gwm-cli/issues/437) ([PR #452](https://github.com/kbrdn1/gwm-cli/pull/452)) / [#438](https://github.com/kbrdn1/gwm-cli/issues/438) ([PR #454](https://github.com/kbrdn1/gwm-cli/pull/454)) / [#436](https://github.com/kbrdn1/gwm-cli/issues/436) ([PR #455](https://github.com/kbrdn1/gwm-cli/pull/455)) / [#453](https://github.com/kbrdn1/gwm-cli/issues/453) ([PR #456](https://github.com/kbrdn1/gwm-cli/pull/456)) | v1.4.0 | **TUI polish + complete help overlay**: Working Tree scroll from the Status context (`J` / `K`, rebindable, viewport-clamped) (#437); responsive sidebar heights via a pure layout solver (guaranteed floors, proportional split, Agents pane never clipped, Working Tree scrollbar) (#438); CI checks overlay — one row per `statusCheckRollup` entry with workflow + duration, `Enter` opens details, `/` filter, `f` refresh (#436); `?` help overlay documents every modal context with a per-section completeness guard, which-key re-audit (`exec` / `agents`), and a reserved-typing contract across every input sub-mode (#453) |
| [#419](https://github.com/kbrdn1/gwm-cli/issues/419) ([PR #458](https://github.com/kbrdn1/gwm-cli/pull/458)) + [#463](https://github.com/kbrdn1/gwm-cli/issues/463) ([PR #464](https://github.com/kbrdn1/gwm-cli/pull/464)) | v1.5.0 | **Multi-forge**: a `Forge` trait with two backends, the existing GitHub one (`gh`) and a new GitLab one (`glab`). Worktrees, bootstrap, branch naming and the `branch.<name>.gwm-*` link storage stay forge-neutral; only the network layer knows which forge is in play. Ships the `forge` key in `.gwm.toml`, a `[forge_hosts]` table read from the user's own global config, `gwm trust add`, `$GWM_GLAB`, and a refusal to assume an unrecognised host is GitHub (which would have sent an authenticated call, and a token, to whatever host a cloned repo named). GitLab specifics absorbed at the parse boundary: `iid`, nested subgroup paths, the `/-/` URL infix, date-only milestones, project-vs-group labels, and a pipeline-to-CI-state map where an unknown status never aggregates to green. Fix: the trust ledger keys on the repo again, not on its host (#463) |

If an issue still shows `open` on GitHub even though its work shipped, it's a tracking issue waiting for a follow-up audit — check the CHANGELOG and the linked PR before reopening scope work on it.

## Next up

**1.0.0 is cut** (2026-06-26) — the 0.10.0 train plus the 1.0 commitment work
(frozen contracts #317, stability policy #318, the `exec` / `clean` surface
freeze #319 and its additive follow-ons #324 / #325 / #326) all shipped to the
stable line. See the [Shipped highlights](#shipped-highlights) table above and
[`changelogs/1.0.0.md`](changelogs/1.0.0.md) for the consolidated notes.

With the machine surface frozen under SemVer, every minor since is the home for
additive work — new subcommands / flags, new opt-in `.gwm.toml` sections,
additive JSON fields under the same `SCHEMA_VERSION`. Anything that would break
a frozen surface waits for a future major.

The next feature line is queued from a comparative read of the field
(`chmouel/lazyworktree` and `d-kuro/gwq`) against the actual gwm codebase. Four
capability gaps came out of it. Two have shipped: the agent session pane
([#408](https://github.com/kbrdn1/gwm-cli/issues/408)) in v1.3.0 with its
follow-ups, and multi-forge support
([#419](https://github.com/kbrdn1/gwm-cli/issues/419)) in v1.5.0, which landed
deliberately ahead of the rich PR/Issue view so that view is born multi-forge
instead of being rewritten later. See the table above for both. The remaining
two are ordered by what they unlock rather than by size.

### 1. Naming flexibility ([#415](https://github.com/kbrdn1/gwm-cli/issues/415) ✅ → [#416](https://github.com/kbrdn1/gwm-cli/issues/416) ✅ → [#417](https://github.com/kbrdn1/gwm-cli/issues/417) ✅ → [#418](https://github.com/kbrdn1/gwm-cli/issues/418) → [#479](https://github.com/kbrdn1/gwm-cli/issues/479)), plus [#480](https://github.com/kbrdn1/gwm-cli/issues/480) / [#481](https://github.com/kbrdn1/gwm-cli/issues/481) / [#482](https://github.com/kbrdn1/gwm-cli/issues/482)

`gwm create` requires the full `<type> <issue> <desc>` triple, and there is no
way to name a worktree freely. Working through this surfaced a real defect
rather than just a missing feature:

`worktree.branch_pattern` is configurable and editable live from the Settings
panel, and `BranchSpec::branch_name` honours it. But `parse_branch` matches a
hardcoded regex instead. A user who customises the pattern silently loses
issue/PR auto-linking, gitmoji, and the branch-convention check, with no warning
connecting cause to effect.

Sequenced cheapest-first, each step shippable on its own:

- [x] [#415](https://github.com/kbrdn1/gwm-cli/issues/415) : warn in `doctor` / `config validate` when the pattern is customised (turns a silent failure into a stated one)
- [x] [#416](https://github.com/kbrdn1/gwm-cli/issues/416) : free-form naming via `gwm create --name`, additive and contract-safe
- [x] [#417](https://github.com/kbrdn1/gwm-cli/issues/417) : derive the parser from the pattern and drop `BRANCH_RE`, which removes the defect at the root
- [ ] [#418](https://github.com/kbrdn1/gwm-cli/issues/418) : token-driven create form with a live branch/path preview
- [ ] [#479](https://github.com/kbrdn1/gwm-cli/issues/479) : rename a free-form worktree, either freely again or into the pattern

The first three are on `dev` and unreleased. **#418 is the only step of the original
sequence still to build**; everything else listed below is follow-up work that came
out of building the first three, not a widening of the plan.

[#478](https://github.com/kbrdn1/gwm-cli/issues/478) is already closed inside #417's
PR: `branch_pattern` and `path_pattern` need not carry the same segments, so a
worktree created as `gwm create fix 42 login` under `feat/#{issue}-{desc}` keeps its
`fix` only in the directory name, and rebuilding the triple from the branch alone
silently renamed that component on every edit.

[#479](https://github.com/kbrdn1/gwm-cli/issues/479) is the other half of free-form
naming, still open: the rename form refuses a free-form branch outright, so a worktree
named for a spike can neither be renamed again nor promoted into the convention once
it grows an issue number. It sits next to #418 because both rewrite the same create
form, and running them concurrently buys only merge conflicts.

Three more came out of reviewing #417 and were split out rather than folded into an
already large PR. They are independent of #418 and #479 and touch different files, so
they run in parallel with them:

- [ ] [#480](https://github.com/kbrdn1/gwm-cli/issues/480) : `{repo}` expands to the workspace display name when a branch is written and to the directory basename when one is read, so in a workspace holding two repos that share a basename a `{repo}` pattern writes what it cannot read back
- [ ] [#481](https://github.com/kbrdn1/gwm-cli/issues/481) : renaming a worktree closes its open pull request without warning, since the remote rename is a delete plus a create and GitHub closes a PR whose head branch is renamed, including through its own rename API
- [ ] [#482](https://github.com/kbrdn1/gwm-cli/issues/482) : a segment frozen as a literal is lost when the pattern reorders its placeholders, because the recovery bounds each literal by the canonical `type, issue, desc` rank against markers whose real order differs

### 2. Rich PR/Issue view ([#420](https://github.com/kbrdn1/gwm-cli/issues/420))

Read a pull request or issue in full without leaving the TUI: description,
individual checks, reviews, conversation, and inline review comments. Modelled
on `snacks.nvim`'s `snacks.gh`, whose key lesson is that inline diff comments are
reachable only through GraphQL, not through `gh --json`.

Comes last, and inherits both the detail overlay the agent pane (#408) already
paid for and the `Forge` trait (#419), so it is born reading GitLab too. Where gwm can beat the reference: `snacks.gh` has no notion of a worktree,
so the user picks from a flat list; gwm already knows the
worktree → branch → PR → issue chain and can open on the current row directly.

### Deferred

- [#421](https://github.com/kbrdn1/gwm-cli/issues/421) — **container execution**: a `container:` block on the existing `exec` / aliases surface. Low cost (it wraps `docker run`, and anything exposing a Docker-compatible socket works for free), but no observed demand yet, so it waits until after the discovery push.

### Visibility

Not feature work, but tracked here because it gates whether any of the above is
seen:

- [#422](https://github.com/kbrdn1/gwm-cli/issues/422) — comparison page covering gwm, gwq and lazyworktree (gwq is the incumbent by star count and has been dormant since May)
- [#423](https://github.com/kbrdn1/gwm-cli/issues/423) — sync `docs/` to the Astro docs site, with the in-repo tree as the source of truth

## Ambitious

Larger investments with strategic payoff. Gated by user demand or a concrete
first consumer. The previous "Ambitious" items all shipped in the v0.10.0 rc
trains above: config presets (#37) and the JSON-RPC API + daemon (#38) in rc.2,
the daemon's "concrete first consumer" as `gwm statusline` (#309) in rc.3, and
the fan-out / disk-hygiene candidates from the post-rc.2 gap review as
`gwm exec` and `gwm clean` (#313) in the 1.0 line.

Both ambitious bets from the field review have now shipped. The **agent
session pane** ([#408](https://github.com/kbrdn1/gwm-cli/issues/408)), a new
detection subsystem with four backends consumed by the TUI, `gwm agents`, the
daemon and `gwm statusline`, shipped in v1.3.0. The **`Forge` trait**
([#419](https://github.com/kbrdn1/gwm-cli/issues/419)), a structural change to
the GitHub layer, shipped in v1.5.0 with the GitLab backend it exists to enable
as its first consumer.

No other large bets are queued. They land once an issue with a concrete first
consumer is filed.

## How to contribute

1. Pick an item that interests you and read its issue for scope details.
2. Comment on the issue if you intend to work on it (avoids parallel duplication).
3. `gwm create <type> <issue> <slug>` to spin up an isolated worktree (the issue auto-links itself — see [`docs/integrations/github-linking.md`](docs/5.integrations/1.github-linking.md)).
4. Open a PR targeting `dev` following the conventions in [CONTRIBUTING.md](CONTRIBUTING.md) (Gitmoji + Conventional Commits, tests required, never squash; full docs version at [`docs/development/contributing.md`](docs/6.development/2.contributing.md)).
5. The issue is the source of truth — this roadmap is updated to reflect what ships in each release.

Items marked `good first issue` (when applicable) are intentionally scoped so a newcomer can land them without a deep dive into the codebase.

## Out of scope (for now)

A few directions the project deliberately steers clear of:

- **Replacing lazygit / gitui in scope** — `gwm` is a worktree manager. Git history surgery stays with the dedicated tools that already do it well; `gwm` integrates with them rather than competing.
- **GUI front-end** — the terminal is the target. A GUI app would split focus and dilute the design.
- **Worktree synchronisation across machines** — too much surface (state, conflict, networking) for a tool whose value is local responsiveness.

That can change if a concrete use case shows up. Open a feature-request issue with the rationale.
