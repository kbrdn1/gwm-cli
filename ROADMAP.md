# gwm roadmap

This document tracks where `gwm` is heading. It complements [CHANGELOG.md](CHANGELOG.md) (what already shipped) and the [open issues](https://github.com/kbrdn1/gwm-cli/issues) (the source of truth for scope details).

Each item below links to its GitHub issue. The scope, alternatives considered, and acceptance criteria live there. This file is the map, not the spec.

## Current state: v1.6.1 stable

The current **stable** line is **v1.6.1** (2026-08-04), a follow-up closing the
gaps left by the v1.6.0 **security fix affecting every earlier version** (see
the highlights table). The machine-readable
contracts frozen at 1.0.0 still hold: the CLI subcommands / flags / exit codes,
the `--format=json` schemas, the daemon JSON-RPC protocol, and the `.gwm.toml`
section set will not break without a major bump (see
[Stability & compatibility](docs/6.development/3.stability.md)).

Since the 1.0.0 milestone (2026-06-26): the **1.0.x patches** hardened the line
(security-only 1.0.3 among them); **1.1.0** shipped the first outside-report
features ([#363](https://github.com/kbrdn1/gwm-cli/issues/363): persisted
sidebar layout, OSC 52 yank that works over SSH) and **1.1.1** fixed global
config resolution on macOS; **1.2.0** was the distribution push
([#383](https://github.com/kbrdn1/gwm-cli/issues/383): Scoop, `.deb` / `.rpm`,
AUR, aqua, with winget wired and pending upstream); **1.3.0** shipped the
**agent session pane** ([#408](https://github.com/kbrdn1/gwm-cli/issues/408))
and its follow-ups, including the Windows named pipe transport for the daemon
([#439](https://github.com/kbrdn1/gwm-cli/issues/439)); **1.4.0** completed the
`?` help overlay ([#453](https://github.com/kbrdn1/gwm-cli/issues/453)) and
closed the TUI-polish trio ([#436](https://github.com/kbrdn1/gwm-cli/issues/436)
/ [#437](https://github.com/kbrdn1/gwm-cli/issues/437) /
[#438](https://github.com/kbrdn1/gwm-cli/issues/438)); **1.5.0** made gwm
multi-forge ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)), adding a
`Forge` trait and a GitLab (`glab`) backend behind it; **1.6.0** fixes
[GHSA-fffq-vg6f-gxqm](https://github.com/kbrdn1/gwm-cli/security/advisories/GHSA-fffq-vg6f-gxqm)
and lands the naming-flexibility line. See
[Shipped highlights](#shipped-highlights).

1.0.0 promotes the entire **0.10.0 train**: the **rc.1** Settings-editability +
TUI enrichment cycle; the **rc.2** train (embedded PTY overlays, the TUI keymap
redesign, rebindable contextual modal keys + the Settings Keys tab, multi-repo
workspace mode, config presets for `gwm init`, the JSON API + daemon, the
current-PR CI indicator, and the file-explorer Working Tree pane); the **rc.3**
train that **closes the GitHub loop**: `gwm review <PR#>` and `gwm statusline`;
and the **rc.4** fleet-chore pair `gwm exec` / `gwm clean`. The post-rc.4 stable
delta completes the 1.0 commitment: the frozen, versioned machine contracts
([#317](https://github.com/kbrdn1/gwm-cli/issues/317)), the published stability
policy ([#318](https://github.com/kbrdn1/gwm-cli/issues/318)), the frozen
`exec` / `clean` surface decision ([#319](https://github.com/kbrdn1/gwm-cli/issues/319)),
and the additive features it anticipated: named `[exec]` / `[clean]` profiles,
bounded `--jobs`, and `--workspace` fan-out ([#324](https://github.com/kbrdn1/gwm-cli/issues/324),
[#326](https://github.com/kbrdn1/gwm-cli/issues/326)), plus TUI exec / clean
overlays ([#325](https://github.com/kbrdn1/gwm-cli/issues/325)). See the
[Shipped highlights](#shipped-highlights) table for the per-issue breakdown and
[`changelogs/1.0.0.md`](changelogs/1.0.0.md) for the consolidated notes. The
MSRV is **1.95** (raised by `rusqlite`'s bundled `libsqlite3-sys`, which
declares no floor of its own; MSRV bumps ride a minor per the stability
policy). It is held by a CI job that installs the declared floor and both
resolves and compiles the committed lockfile against it on all three runners
([#491](https://github.com/kbrdn1/gwm-cli/issues/491) ✅), because the previous
setup let `Cargo.toml` claim 1.86 for a whole release line while the graph
needed more, silently.

The 0.9.x stable line ships:

- **Native worktree ops via libgit2 (vendored)**: single binary, no `gwq` / `git` CLI dependency.
- **CLI + ratatui TUI**: `gwm <subcommand>` for scripts, `gwm` alone opens the interactive interface.
- **Per-repo `.gwm.toml` + user-level global config**: branch / path conventions, configurable branch types, declarative GitHub labels / milestones, file copies, regex guards (`abort` or `seed-from-example`), no-symlink invariants. A user-level `~/.config/gwm/config.toml` deep-merges **underneath** each repo's `.gwm.toml` (repo wins on conflicts; `GWM_NO_GLOBAL_CONFIG=1` forces repo-only). `{home}` / `{repo}` / `{repo_path}` / `{repo_parent}` placeholders for repo-relative bases.
- **Config CLI**: `gwm config get / set / unset / list / validate / path / edit`, git-config-style over dotted keys, comment-preserving via `toml_edit`.
- **Lifecycle hooks `[hooks.*]`**: declarative `pre_create` / `post_create` / `pre_bootstrap` / `post_bootstrap` / `pre_remove` / `post_remove` phases, per-step `on_fail = abort|warn|ignore`, `--skip-hooks` escape hatch, gated by the same `when:` predicates (`file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, with `!`, `&&`, `||` composition). Legacy `[[bootstrap.command]]` is auto-aliased to `[[hooks.post_create]]`.
- **CLI aliases + Gitmoji convention**: `[aliases]` in `.gwm.toml` (or `~/.config/gwm/aliases.toml`) expand `gwm <alias>` to argv before clap parses, with `gwm aliases list`; `[gitmoji]` mapping powers `gwm commit-prefix`, `gwm types --gitmoji`, and an opt-in `gwm hooks install commit-msg` hook.
- **GitHub issue / PR templates**: `[issue_template]` + `gwm new <type> <desc>` (create issue from a form template, then spin up the linked worktree); `[pr_template]` + `gwm pr [--draft] [--base] [--render]` with `{commits}` / `{files_changed}` placeholders.
- **Safety daily**: `--dry-run` on `gwm remove` / `gwm prune` (preview before destroying); `gwm undo` + `gwm history` backed by an operation journal at `$XDG_DATA_HOME/gwm/history.toml` (100-entry rotation, per-repo filtering) to recover a misfired removal without `git reflog`.
- **`gwm sync [<pattern>] [--merge]`**: fetch a worktree's upstream and rebase (or merge) its branch onto it; refuses a dirty tree or missing upstream, aborts a conflicting rebase/merge to keep the worktree usable.
- **Bootstrap hardening for hostile clones**: TOFU trust ledger on `.gwm.toml`, `--allow-bootstrap` / `--deny-bootstrap`, path-traversal rejection, symlink-safe copy/write primitives, load-time regex validation for deny patterns.
- **Async-task spine (new in 0.9.0)**: a generic off-thread worker (coalescing + late-result drop) keeps the event loop responsive: the worktree list refresh (`f` / `r`), the GitHub issue/PR fetch (`F`, per-key generation guard fixes a stale-data race), `gwm sync` (`S`), and bootstrap (`b`) all run on it, animating the statusbar spinner instead of blocking. The TOFU gate stays synchronous before any bootstrap spawn.
- **In-TUI pane-key family `1` / `2` / `3` / `4` (new in 0.9.0)**: `1` / `2` focus the worktrees / status panes; `3` opens a lazygit-style Command Logs overlay (scrollable transcript of the external commands gwm ran); `4` opens a Configuration panel showing the **resolved** `.gwm.toml` with a per-row source column (repo / user / default). `.` opens the docs in the browser.
- **Full theme-role coverage (new in 0.9.0)**: the resolved `[theme]` is threaded through every `draw_*` site, with dedicated `name` / `path` chrome roles and `staged` / `modified` / `untracked` working-tree roles; all defaults preserved, pinned by `tests/tui_theme_audit_tests.rs`.
- **Lazygit-style details sidebar**: four bordered subsections (Worktree / Issue · PR / Working Tree / Recent Commits), status-coloured branch names, header status dot tracking linked PR / issue state, 300-commit Recent Commits buffer with the full topology renderer (`○ ◎ │ ╮ ╭ ╯ ╰ ┴ ┬ ─`).
- **Measured TUI sidebar perf pass**: branch age is cached on `WorktreeInfo`, `filtered_indices` is memoised on `FilterState`, Recent Commits uses a cached libgit2 revwalk keyed by `(worktree path, head OID, limit)`, and commit-graph pipes store `git2::Oid` instead of heap-allocated hash strings.
- **Configurable launchers**: `[git_tui]` drives `l` (default `lazygit -p {path}`), `[review]` drives `R` (presets: `lumen` / `claude` / `codex` / `aider` / `gh`, plus free-form `command =`). Placeholders `{base} {head} {path} {diff}` with lazy diff materialisation.
- **GitHub issue / PR linking + auto-detection**: branches matching `<type>/#<N>-<slug>` auto-link to their issue; CLI `link / unlink / open / status` for explicit overrides; live state surfaces in the TUI sidebar via `gh`. A branch's PR is also resolved ephemerally (`gh pr list --head`) and surfaced as `detected`, never written to git config, so an explicit link always wins (`gwm status`, the sidebar `F` refresh, opt-in `gwm list --detect-pr`).
- **TUI personalisation**: `[tui.keys]` remappable keymap with multi-key chord support (`g g`) + `gwm tui keys`; `:` command palette overlay sharing the keystroke `Action` dispatcher; `[theme]` role-based colours with `catppuccin` / `gruvbox` / `tokyo-night` / `claude-dark` presets + `gwm theme list / show`; sidebar stashes mode toggled by `s`.
- **`[tui.open]` dispatch**: `o` key now spawns `$SHELL` in the worktree by default; opt back to OS file manager via `mode = "finder"`.
- **`y: yank`**: copy the selected worktree's path to the clipboard (pbcopy / wl-copy / xclip / xsel / clip).
- **Vim motions**: `gg` / `G` jump to first / last; `Tab` swaps focus between the list and the sidebar; `j` / `k` / `↑` / `↓` move selection or scroll the focused panel.
- **Fuzzy filter (`/`)**: sticky `nucleo-matcher` filter on the worktree list; smart-case, AND on spaces, contiguous beats spread-out; same engine powers `gwm switch` (picker mode), `gwm path / cd / remove / bootstrap` (fuzzy CLI lookup).
- **One-line `cd`**: `gwm shell-init <shell>` wires up a `gcd <pattern>` (resolve + cd) and bare `gcd` (picker + cd) for zsh / bash / fish / PowerShell.
- **Shell completions**: `gwm completions <shell>` for zsh / bash / fish / PowerShell / elvish (static script generated from the live clap argument tree).
- **Multiplexer integration**: `gwm tmux <pattern> [-p]` and `gwm zellij <pattern> [-p]` open the worktree in a new window / pane / tab; refuse to spawn outside an active session.
- **Responsive + polished TUI chrome**: the details sidebar stacks under the table on a narrow terminal (`< 120` cols) instead of disappearing; `V` cycles `auto → side-by-side → stacked`, `H` / `[tui] sidebar_position` flips it left/right. Borderless styled header, single-line statusline with reverse-video badge chips, content-sized themed modals (confirm buttons, animated spinner), git-style working-tree colourisation.
- **`gwm doctor`**: 8 checks (parses / guard refs / `when` predicates / external binaries / prunable / orphan branches / base writable / unbound `quit` keymap), exit codes `0/1/2` for CI.
- **Confirm-overlay countdown**: safety countdown on the delete-confirm overlay when `p` (delete-branch-on-remove) is armed; duration tunable via `[tui].confirm_countdown_secs` (0..=5, clamped).
- **State-sliced TUI internals**: `tui::app::App` is decomposed into `tui/state/{create_form,filter,confirm,link_prompt,sidebar,github_fetch}.rs`, with dedicated tests for each state slice.
- **Release pipeline**: `release.yml` on `vX.Y.Z` tags, `pre-release.yml` on `-rc.N` / `-alpha.N` / `-beta.N` tags, 5-target build matrix (Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64), GitHub Release assets published through the `gh` CLI with the workflow token ([#146](https://github.com/kbrdn1/gwm-cli/issues/146) resolved), per-version changelog body sourced from `changelogs/<version>.md` (hard-fails if missing), pre-release `[Unreleased]` dupe guard, Homebrew tap update job on stable releases, `cargo binstall` support, Nix flake at the repo root. CI test matrix runs on `ubuntu-latest` / `macos-latest` / `windows-latest`.
- **1000+ tests**: integration and state-machine tests covering config (repo + global layering), aliases, gitmoji, hooks, config CLI, naming, bootstrap, doctor, GitHub linking + PR auto-detection, launcher, multiplexer, homebrew formula, binstall metadata, pre-commit hook, TUI state slices (keymap / palette / theme / sidebar), undo/history journal, worktree libgit2 integration, release workflow guards, and CLI end-to-end.

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
| [#67](https://github.com/kbrdn1/gwm-cli/issues/67) ([PR #68](https://github.com/kbrdn1/gwm-cli/pull/68)) | v0.6.0-rc.1 | Issue / PR linking: CLI + TUI controls, `gh`-backed live status     |
| [#69](https://github.com/kbrdn1/gwm-cli/issues/69) ([PR #70](https://github.com/kbrdn1/gwm-cli/pull/70)) | v0.6.0 | TUI Details sidebar redesign (four bordered subsections)            |
| [#71](https://github.com/kbrdn1/gwm-cli/issues/71) ([PR #72](https://github.com/kbrdn1/gwm-cli/pull/72)) | v0.6.0 | TUI Recent Commits panel: lazygit-style layout + full topology renderer |
| [#73](https://github.com/kbrdn1/gwm-cli/issues/73) ([PR #74](https://github.com/kbrdn1/gwm-cli/pull/74)) | v0.6.0 | Lazygit-style sidebar facelift (`Created` row, status colours, `[tui.open]`, `y: yank`) |
| [#75](https://github.com/kbrdn1/gwm-cli/issues/75) ([PR #76](https://github.com/kbrdn1/gwm-cli/pull/76)) | v0.6.0 | Configurable launchers (`[git_tui]` + `[review]`): keymap reshuffle `r/R → f/F`, new `R` |
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
| [#35](https://github.com/kbrdn1/gwm-cli/issues/35) ([PR #289](https://github.com/kbrdn1/gwm-cli/pull/289)) / [#290](https://github.com/kbrdn1/gwm-cli/issues/290) ([PR #292](https://github.com/kbrdn1/gwm-cli/pull/292)) / [#219](https://github.com/kbrdn1/gwm-cli/issues/219) ([PR #293](https://github.com/kbrdn1/gwm-cli/pull/293)) / [#294](https://github.com/kbrdn1/gwm-cli/issues/294) ([PR #297](https://github.com/kbrdn1/gwm-cli/pull/297)) / [#300](https://github.com/kbrdn1/gwm-cli/issues/300) ([PR #301](https://github.com/kbrdn1/gwm-cli/pull/301)) / [#299](https://github.com/kbrdn1/gwm-cli/issues/299) ([PR #302](https://github.com/kbrdn1/gwm-cli/pull/302)) / [#36](https://github.com/kbrdn1/gwm-cli/issues/36) ([PR #303](https://github.com/kbrdn1/gwm-cli/pull/303)) / [#37](https://github.com/kbrdn1/gwm-cli/issues/37) ([PR #305](https://github.com/kbrdn1/gwm-cli/pull/305)) / [#38](https://github.com/kbrdn1/gwm-cli/issues/38) ([PR #306](https://github.com/kbrdn1/gwm-cli/pull/306)) | v0.10.0-rc.2 | PTY + power-user + integration train: embedded PTY overlays for lazygit (`l` / `L`) and a native `$SHELL` (`o` / `O`) via `portable-pty` + `tui-term` (MSRV → 1.86); TUI keymap redesign (unified list-view bindings: `p`/`P` pull/push, `c` edit-worktree, `e` exit-to-worktree, `y`/`w` yank, `t` mux pane, `h`/`H` macros, plus`[tui.macro1]` / `[tui.macro2]`); rebindable contextual modal keys under `[tui.keys.modal.<context>]`; edit every keymap live from the Settings Keys tab (keystroke capture + validated write-back); Working Tree pane as a nerd-font file-explorer tree (git-coloured rows, bounded scan); current-PR CI indicator in the Status pane; multi-repo workspace mode (`--workspace`, REPO column, `gwm create --repo`); config presets for `gwm init` (`--preset` / `--list-presets` / `--show`); JSON API (`--format=json` on `list` / `doctor` / `path`) + `gwm daemon` (JSON-RPC 2.0 over a unix socket with `subscribe`) |
| [#308](https://github.com/kbrdn1/gwm-cli/issues/308) ([PR #310](https://github.com/kbrdn1/gwm-cli/pull/310)) | v0.10.0-rc.3 | `gwm review <PR#>`: materialise an existing GitHub PR into an isolated worktree (resolves head via `gh`, fetches `refs/pull/<N>/head` (cross-fork aware, any PR state) into `review/pr-<N>-<author>-<slug>`, links the PR + points the diff base at `origin/<base>`); safe-by-default (no bootstrap / lifecycle hooks unless `--bootstrap`), `--name` overrides the branch |
| [#309](https://github.com/kbrdn1/gwm-cli/issues/309) ([PR #311](https://github.com/kbrdn1/gwm-cli/pull/311)) | v0.10.0-rc.3 | `gwm statusline`: the first real consumer of `gwm daemon` (#38), a dependency-free client rendering a compact one-line worktree summary (active branch, count, dirty / ahead / behind, linked issue / PR) for tmux / starship / zsh; `--watch` rides the `subscribe` stream, degrades to a blank line + exit `0` when no daemon is reachable; `docs/5.integrations/4.daemon-consumers.md` (EN + FR) |
| [#313](https://github.com/kbrdn1/gwm-cli/issues/313) ([PR #314](https://github.com/kbrdn1/gwm-cli/pull/314)) | v0.10.0-rc.4 | Fleet chores across worktrees: `gwm exec [<slug>...] -- <cmd>` (run a command in each worktree sequentially, every non-main worktree by default, or listed slugs; everything after `--` forwarded verbatim; `✓ / ✗` rollup, non-zero exit on any failure) and `gwm clean [<slug>...] [--yes]` (report, or with `--yes` reclaim, heavy build artifacts `target/` / `node_modules/` / `dist/` / `build/`; deletes only git-ignored dirs, never follows symlinks, not journaled into `gwm history`) |
| [#317](https://github.com/kbrdn1/gwm-cli/issues/317) / [#318](https://github.com/kbrdn1/gwm-cli/issues/318) / [#319](https://github.com/kbrdn1/gwm-cli/issues/319) / [#324](https://github.com/kbrdn1/gwm-cli/issues/324) / [#325](https://github.com/kbrdn1/gwm-cli/issues/325) / [#326](https://github.com/kbrdn1/gwm-cli/issues/326) / [#334](https://github.com/kbrdn1/gwm-cli/issues/334) | v1.0.0 | The 1.0 commitment: frozen, versioned machine contracts pinned by `tests/contract_tests.rs` (`SCHEMA_VERSION = 1`, daemon `schema_version`, `docs/schema/README.md`) (#317); published stability & compatibility policy (`docs/6.development/3.stability.md`, EN + FR) (#318); frozen `exec` / `clean` surface decision (#319); named `[exec]` / `[clean]` config profiles + bounded `--jobs` parallelism (#324); `--workspace` fan-out for `gwm exec` / `gwm clean` (#326); TUI exec (`x`) / clean (`X`) overlays (#325); help-overlay + overlay polish (#334) |
| [#363](https://github.com/kbrdn1/gwm-cli/issues/363) | v1.1.0 | First outside-report release: persisted sidebar layout (`V` / `H` write back to config) and OSC 52 clipboard yank that works over SSH |
| [#383](https://github.com/kbrdn1/gwm-cli/issues/383) | v1.2.0 | Distribution push: Scoop bucket, native `.deb` / `.rpm` packages, AUR (`gwm-cli-bin`), aqua standard registry; winget wired, pending upstream merge |
| [#408](https://github.com/kbrdn1/gwm-cli/issues/408) ([PR #435](https://github.com/kbrdn1/gwm-cli/pull/435)) + follow-ups [#439](https://github.com/kbrdn1/gwm-cli/issues/439) ([PR #444](https://github.com/kbrdn1/gwm-cli/pull/444)) / [#440](https://github.com/kbrdn1/gwm-cli/issues/440) ([PR #442](https://github.com/kbrdn1/gwm-cli/pull/442)) / [#441](https://github.com/kbrdn1/gwm-cli/issues/441) ([PR #443](https://github.com/kbrdn1/gwm-cli/pull/443)) / [#445](https://github.com/kbrdn1/gwm-cli/issues/445) ([PR #446](https://github.com/kbrdn1/gwm-cli/pull/446)) | v1.3.0 | **Agent session pane**: detect AI-agent sessions (Claude Code, Codex, opencode, Mistral Vibe) per worktree from on-disk artefacts, surfaced as an AGENT column (table + TUI), an `a` detail overlay with pinning, `gwm agents` CLI, additive JSON `agents` field, statusline segment. Follow-ups: **Windows named pipe transport** for `gwm daemon` / `gwm statusline` (owner-only pipe, server identity verified by owner SID) (#439); `gwm clean` ENOTEMPTY race tolerance (#440); process-level liveness: a dead recorded PID demotes the session immediately on unix (#441); fixed-height attach prompt with a scrollbar (#445) |
| [#437](https://github.com/kbrdn1/gwm-cli/issues/437) ([PR #452](https://github.com/kbrdn1/gwm-cli/pull/452)) / [#438](https://github.com/kbrdn1/gwm-cli/issues/438) ([PR #454](https://github.com/kbrdn1/gwm-cli/pull/454)) / [#436](https://github.com/kbrdn1/gwm-cli/issues/436) ([PR #455](https://github.com/kbrdn1/gwm-cli/pull/455)) / [#453](https://github.com/kbrdn1/gwm-cli/issues/453) ([PR #456](https://github.com/kbrdn1/gwm-cli/pull/456)) | v1.4.0 | **TUI polish + complete help overlay**: Working Tree scroll from the Status context (`J` / `K`, rebindable, viewport-clamped) (#437); responsive sidebar heights via a pure layout solver (guaranteed floors, proportional split, Agents pane never clipped, Working Tree scrollbar) (#438); CI checks overlay: one row per `statusCheckRollup` entry with workflow + duration, `Enter` opens details, `/` filter, `f` refresh (#436); `?` help overlay documents every modal context with a per-section completeness guard, which-key re-audit (`exec` / `agents`), and a reserved-typing contract across every input sub-mode (#453) |
| [#419](https://github.com/kbrdn1/gwm-cli/issues/419) ([PR #458](https://github.com/kbrdn1/gwm-cli/pull/458)) + [#463](https://github.com/kbrdn1/gwm-cli/issues/463) ([PR #464](https://github.com/kbrdn1/gwm-cli/pull/464)) | v1.5.0 | **Multi-forge**: a `Forge` trait with two backends, the existing GitHub one (`gh`) and a new GitLab one (`glab`). Worktrees, bootstrap, branch naming and the `branch.<name>.gwm-*` link storage stay forge-neutral; only the network layer knows which forge is in play. Ships the `forge` key in `.gwm.toml`, a `[forge_hosts]` table read from the user's own global config, `gwm trust add`, `$GWM_GLAB`, and a refusal to assume an unrecognised host is GitHub (which would have sent an authenticated call, and a token, to whatever host a cloned repo named). GitLab specifics absorbed at the parse boundary: `iid`, nested subgroup paths, the `/-/` URL infix, date-only milestones, project-vs-group labels, and a pipeline-to-CI-state map where an unknown status never aggregates to green. Fix: the trust ledger keys on the repo again, not on its host (#463) |

| [GHSA-fffq-vg6f-gxqm](https://github.com/kbrdn1/gwm-cli/security/advisories/GHSA-fffq-vg6f-gxqm) + [#415](https://github.com/kbrdn1/gwm-cli/issues/415) / [#416](https://github.com/kbrdn1/gwm-cli/issues/416) / [#417](https://github.com/kbrdn1/gwm-cli/issues/417) / [#418](https://github.com/kbrdn1/gwm-cli/issues/418) / [#479](https://github.com/kbrdn1/gwm-cli/issues/479) + [#491](https://github.com/kbrdn1/gwm-cli/issues/491) | v1.6.0 | **Security fix + naming flexibility.** A branch name could inject a command into a lifecycle hook: placeholders were expanded into `sh -c` unescaped, and git permits `;`, `|`, `&`, `$`, backticks and redirections in a ref name, so a branch someone else pushed ran arbitrary commands as anyone who had trusted their own repo's hooks, with no trust prompt in the path (the gate covers the repo's hooks, never the branch name entering them). Affects every version up to and including 1.5.0, no backport. Values are shell-escaped on expansion, `env` values stay raw because they never see a shell, and hooks additionally get `GWM_*` environment variables that need no quoting. Alongside it: `gwm create --name` drops the `<type> <issue> <desc>` requirement (#416), the TUI create and rename forms present the fields the repo's patterns actually ask for in pattern order, in both directions (#418), and the declared MSRV becomes an honest 1.95 held by a CI job that resolves and compiles the locked graph at the floor (#491). Verifying the sequence produced the Fixed section: terminal-escape neutralisation of echoed config (#473), single-pass placeholder expansion (#494), branch rollback on a failed `gwm create` (#487), Windows path rules on free-form names (#475), and worktree-aware branch reads (#477) |

| [#502](https://github.com/kbrdn1/gwm-cli/issues/502) / [#506](https://github.com/kbrdn1/gwm-cli/issues/506) / [#507](https://github.com/kbrdn1/gwm-cli/issues/507) + [#423](https://github.com/kbrdn1/gwm-cli/issues/423) / [#511](https://github.com/kbrdn1/gwm-cli/issues/511) | v1.6.1 | **Bidi follow-up to the security release, and the documentation pipeline.** The 1.6.0 neutralisation rests on `char::is_control`, which covers C0, DEL and C1: it does not cover the twelve characters carrying the `Bidi_Control` property, which are `Cf`, not `Cc`, and reorder how a terminal renders the text around them without ever being a control byte. The pre-trust bootstrap summary inherited the gap, the one output whose job is to let someone authorise a shell command out of an unvetted repo. The TUI worktrees table was never on the path the CLI sinks protect at all: measured on ratatui 0.30, every render path drops the zero-width bytes but `List` and `Table` keep the `Bidi_Control` ones, so a fetched ref could read in an order it is not stored in. Both closed, the neutralisation landing in the width-clipping funnel so a column added later inherits it, plus an alias expansion refused rather than neutralised because it becomes argv before clap is reached. Alongside: the published documentation now resyncs and redeploys itself when `main` moves ([#423](https://github.com/kbrdn1/gwm-cli/issues/423), <https://gwm.kbrdn.dev>), and `herdr-plugin-gwm` gets an integration page in English and French ([#511](https://github.com/kbrdn1/gwm-cli/issues/511)) |

If an issue still shows `open` on GitHub even though its work shipped, it's a tracking issue waiting for a follow-up audit: check the CHANGELOG and the linked PR before reopening scope work on it.

## Next up

**1.0.0 is cut** (2026-06-26): the 0.10.0 train plus the 1.0 commitment work
(frozen contracts #317, stability policy #318, the `exec` / `clean` surface
freeze #319 and its additive follow-ons #324 / #325 / #326) all shipped to the
stable line. See the [Shipped highlights](#shipped-highlights) table above and
[`changelogs/1.0.0.md`](changelogs/1.0.0.md) for the consolidated notes.

With the machine surface frozen under SemVer, every minor since is the home for
additive work: new subcommands / flags, new opt-in `.gwm.toml` sections,
additive JSON fields under the same `SCHEMA_VERSION`. Anything that would break
a frozen surface waits for a future major.

The next feature line is queued from a comparative read of the field
(`chmouel/lazyworktree` and `d-kuro/gwq`) against the actual gwm codebase. Four
capability gaps came out of it. Two have shipped: the agent session pane
([#408](https://github.com/kbrdn1/gwm-cli/issues/408)) in v1.3.0 with its
follow-ups, and multi-forge support
([#419](https://github.com/kbrdn1/gwm-cli/issues/419)) in v1.5.0, which landed
deliberately ahead of the rich PR/Issue view so that view is born multi-forge
instead of being rewritten later. See the table above for both.

**The lot below is queued for a future v1.7.0 cut.** It is not ordered by size.
Each step sits where it is cheapest to land, which usually means ahead of the
thing that would otherwise have to be reopened to accommodate it.

### 1. Naming flexibility ([#415](https://github.com/kbrdn1/gwm-cli/issues/415) ✅ → [#416](https://github.com/kbrdn1/gwm-cli/issues/416) ✅ → [#417](https://github.com/kbrdn1/gwm-cli/issues/417) ✅ → [#479](https://github.com/kbrdn1/gwm-cli/issues/479) ✅ → [#418](https://github.com/kbrdn1/gwm-cli/issues/418) ✅), plus [#480](https://github.com/kbrdn1/gwm-cli/issues/480) ✅ / [#481](https://github.com/kbrdn1/gwm-cli/issues/481) ✅ / [#482](https://github.com/kbrdn1/gwm-cli/issues/482) ✅ / [#475](https://github.com/kbrdn1/gwm-cli/issues/475) ✅

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
- [x] [#479](https://github.com/kbrdn1/gwm-cli/issues/479) : rename a free-form worktree, either freely again or into the pattern
- [x] [#418](https://github.com/kbrdn1/gwm-cli/issues/418) : token-driven create form with a live branch/path preview

Everything checked above is on `dev` and unreleased, so none of it has shipped to a
stable line yet. **The original sequence is complete**; everything else listed below is
follow-up work that came out of building the first three, not a widening of the plan.

Two of #418's three asks turned out to have shipped ahead of it, which is worth
recording because it is what a long sequence does to its own tail: the live
branch/path preview arrived with #217's follow-up and #416, and `Ctrl-T` with #416.
What #418 actually contributed is the field set and the focus order, derived from the
patterns rather than fixed at the canonical triple. Its scope note had argued for a
generic token-driven form so exotic tokens would be supported rather than dropped,
but the token vocabulary is closed at three editable names, so that design and the
narrow one coincide except on ordering. A drift guard holds the intent instead.

A checkbox here tracks what is merged into `dev`, which is not the same thing as what
GitHub shows as closed. `Closes #N` in a pull request body only fires when the pull
request targets the default branch, so a merge into `dev` closes nothing on its own:
an issue is either closed by hand once its work lands there, or left open until `dev`
reaches `main`. Both states appear above and neither means the work is missing.

[#478](https://github.com/kbrdn1/gwm-cli/issues/478) is already closed inside #417's
PR: `branch_pattern` and `path_pattern` need not carry the same segments, so a
worktree created as `gwm create fix 42 login` under `feat/#{issue}-{desc}` keeps its
`fix` only in the directory name, and rebuilding the triple from the branch alone
silently renamed that component on every edit.

[#479](https://github.com/kbrdn1/gwm-cli/issues/479) was the other half of free-form
naming: the rename form refused a free-form branch outright, so a worktree named for a
spike could neither be renamed again nor promoted into the convention once it grew an
issue number. It shipped ahead of #418 rather than beside it, because both rewrite the
same create form and running them concurrently buys only merge conflicts. That
ordering is what unblocked #418.

Three more came out of reviewing #417 and were split out rather than folded into an
already large PR. They touched different files, so they ran in parallel and are all on
`dev`:

- [x] [#480](https://github.com/kbrdn1/gwm-cli/issues/480) : `{repo}` expands to the workspace display name when a branch is written and to the directory basename when one is read, so in a workspace holding two repos that share a basename a `{repo}` pattern writes what it cannot read back
- [x] [#481](https://github.com/kbrdn1/gwm-cli/issues/481) : renaming a worktree closes its open pull request without warning, since the remote rename is a delete plus a create and GitHub closes a PR whose head branch is renamed, including through its own rename API
- [x] [#482](https://github.com/kbrdn1/gwm-cli/issues/482) : a segment frozen as a literal is lost when the pattern reorders its placeholders, because the recovery bounds each literal by the canonical `type, issue, desc` rank against markers whose real order differs

One follow-up of #416 was taken ahead of #418 rather than after it, because it had to
land before the feature shipped rather than after:

- [x] [#475](https://github.com/kbrdn1/gwm-cli/issues/475) : a free-form name was not validated against Windows path rules, so `< > " |` and the reserved device names (`CON`, `NUL`, `COM1`…) passed every check and failed only when the directory was created, after `pre_create` hooks had run and with the branch already left behind. `--name` merged into `dev` two days after `v1.5.0` was tagged, so it had never reached a stable line: tightening the validator was additive then and would have been a breaking change once it shipped.

Four bugs found while verifying this sequence end to end are fixed alongside it. None
is naming work; they are recorded here because they were found here:

- [x] [#477](https://github.com/kbrdn1/gwm-cli/issues/477) : `gwm pr`, `gwm commit-prefix` and `gwm bootstrap` read the main checkout's branch (or, for `bootstrap`, no branch at all) when run from a worktree, because `discover_repo` deliberately walks back to the main working directory and they asked that handle where they were
- [x] [#494](https://github.com/kbrdn1/gwm-cli/issues/494) : `expand_placeholders` substituted `{home}` / `{repo}` first and then substituted what those expansions produced, so a repo whose own name contains `{type}` made `{repo}/{desc}` write a type the pattern never mentions. Now single pass, which is also an *enlargement*: `BranchParser::compile` had been refusing that whole class rather than mirror a formatter that rewrote its own output, and those patterns round-trip. Fixing the root meant reverting part of #418, which had derived the form's fields after the resolution as a workaround
- [x] [#473](https://github.com/kbrdn1/gwm-cli/issues/473) : a `.gwm.toml` is data from a repo nobody has vetted, and the commands that read it back skip the trust gate on purpose, so echoing a value verbatim handed that file a terminal escape channel out of a read-only command. Neutralised at each **sink** rather than each producer, which is what covers checks and rows nobody has written yet. The worst site was not in `gwm config` at all but in the TOFU prompt, whose bootstrap summary could be made to erase the row naming the shell it asks permission to run; and `toml`'s parse error quotes the offending source line, so `gwm list` inside the repo was enough
- [x] [#487](https://github.com/kbrdn1/gwm-cli/issues/487) : `worktree::add` creates the branch before the directory, because `WorktreeAddOptions::reference` takes a reference that already exists, so *any* late failure left it orphaned. #474 and #475 each closed one input set; a full disk is not an input set, so the ordering is what actually bounds the class. Three conditions now gate the rollback, and each came from a different way of getting it wrong: this call created the branch, it still points where the call put it, and no checkout has it as HEAD. The last one is the review's: deleting the *reference* rather than the branch spares a `branch.<name>` config section the command never wrote, but it also drops `git_branch_delete`'s refusal on a branch a linked worktree stands on, and that residue is a worktree bound to nothing, which no check reports

### 2. Bulk selection ([#484](https://github.com/kbrdn1/gwm-cli/issues/484) ✅)

`Space` marks the active row and `d` acts on the marked set rather than on a
single row. The request came in for bulk cleanup, which is the obvious first
consumer, but demand is not why it went first.

It changes what "the current row" means, and the two features behind it both
open on the current row: opening on it directly is the advantage the rich
PR/Issue view claims over `snacks.gh`, and a note is attached to whichever
worktree is selected. Landing a multi-row selection model after them would have
meant reopening both. Landing it before means they are written against the final
model once.

- [x] [#484](https://github.com/kbrdn1/gwm-cli/issues/484) ([PR #520](https://github.com/kbrdn1/gwm-cli/pull/520)) : `Space` marks a row, `d` deletes the marked set behind one confirm that reports a count instead of listing rows, with `D` arming the branch deletion for the whole batch. `gwm remove a b c` is the non-interactive half, and it resolves every pattern before touching anything, so a typo removes nothing at all, which is what the `gwm list --format json | ... | xargs -n1 gwm remove` workaround could not offer

Scope was negotiated with the reporter rather than assumed, and the answer was
narrower than the issue title: only `d` reads the mark set. Every other verb
keeps acting on the cursor row, which would be a footgun if it were silent, so
the pane footer carries the mark count for as long as the set is non-empty.
Marks are keyed by path, since the fuzzy filter reranks indices on every
keystroke and a worktree id is unique only inside one repo, which a workspace
merges several of. The filter and the manual `f` clear them; the background
auto-refresh only prunes rows that no longer exist, because a sixty second timer
eating a selection still being built would make the feature unusable.
`cycle_sidebar_layout` moved off `Space` onto `z`, which is a default worth
naming twice: a `.gwm.toml` binding a chord that starts with `z` is now a prefix
conflict and is refused at load time.

Two defects came out of reviewing it, both in the part that was new rather than
in the part that was moved:

- the confirm overlay recomputed its batch after a partial failure. `worktree::remove` prunes the admin entry before it deletes the directory (#98, so a partial failure cannot leave a phantom worktree), which means a removal that fails on the filesystem still drops its own row, the refresh then prunes its mark, and the recomputation fell back to the cursor row. A second confirm deleted a worktree that had never been marked. A batch can now only ever narrow
- a worktree id is the `.git/worktrees/<id>` entry name, and git hands it back to whoever recreates a worktree with that basename. The overlay snapshots its targets and fires after a countdown, and the CLI resolves the whole batch before running any hook, so both had a window in which the id pointed elsewhere. Both go through a checked removal now, and the check sits on the handle the prune acts on rather than in a wrapper that resolves the name a second time

[#521](https://github.com/kbrdn1/gwm-cli/issues/521) came out of the same review
and was deliberately left out of the PR: the TUI delete writes no undo journal
entry and runs no `pre_remove` / `post_remove` hooks, unlike the CLI. That is
older than this feature, but a batch makes it bite harder, since ten worktrees
can now leave in one keystroke with nothing for `gwm undo` to pop.

### 3. Symfony preset ([#392](https://github.com/kbrdn1/gwm-cli/issues/392) ✅)

A seventh `gwm init --preset`. Modelled on the Laravel one, but not a copy of
it: Symfony's dotenv convention is the reverse of Laravel's, `.env` is committed
and carries the neutral defaults while `.env.local` is gitignored and carries
the secrets. So the preset copies `.env.local` and `.env.test.local` rather than
`.env`, and the `no-aws-rds` guard seeds from the committed `.env` instead of an
`.env.example` a Symfony project does not have. `var/` joins `vendor/` in the
no-symlink invariants, because it holds the compiled service container and the
cached routes.

Placed here because it looked like the only item in the lot that is not
sequenced against the others: it touches `src/presets.rs` and
`examples/presets/`, disjoint from the TUI files the rest of the line lives in,
so it can run in parallel with any of them without a merge conflict.

That held for the merge order and not for the blast radius. Every stack preset
puts its commands in `[hooks.*]` rather than `[[bootstrap.command]]`, and two
`gwm doctor` checks walked the bootstrap commands alone, so this one arrived
with a clean report about a file the doctor had barely read: a typo in a hook's
`when` predicate came back as "no `when:` predicates configured", and a hook
invoking a binary that is not installed passed as fine right up to the moment
`gwm create` ran it. Fixed in the same PR rather than deferred, since it applied
to every hooks-based config and the preset was only the thing that made it
visible. `LifecycleHooksConfig::all_steps()` now enumerates the six phases
through an exhaustive destructuring, so the next consumer cannot quietly read
half the config.

### 4. Rich PR/Issue view ([#420](https://github.com/kbrdn1/gwm-cli/issues/420))

Read a pull request or issue in full without leaving the TUI: description,
individual checks, reviews, conversation, and inline review comments. Modelled
on `snacks.nvim`'s `snacks.gh`, whose key lesson is that inline diff comments are
reachable only through GraphQL, not through `gh --json`.

The first of the two big TUI features, and it inherits both the detail overlay
the agent pane (#408) already paid for and the `Forge` trait (#419), so it is
born reading GitLab too. Where gwm can beat the reference: `snacks.gh` has no
notion of a worktree, so the user picks from a flat list; gwm already knows the
worktree → branch → PR → issue chain and can open on the current row directly.
That last point is why bulk selection (#484) goes ahead of it: "the current row"
has to mean its final thing before two features are written against it.

Shipped in two of the three steps the issue laid out: `gwm` now asks `gh` for
the description, the author, the reviews and the conversation in the request it
already made for the rollup, and `I` renders all of it. The third step, the
comments anchored to a diff hunk, is [#528](https://github.com/kbrdn1/gwm-cli/issues/528):
those are reachable through GraphQL alone, so they are a second transport rather
than a wider field list, and they were always the last of the three.

### 5. Per-worktree notes ([#515](https://github.com/kbrdn1/gwm-cli/issues/515))

gwm holds everything about a worktree except the part only its author can write:
where they were. It knows the branch, the linked issue, the diff against base and
the agent session, but not what had just been figured out, what is blocking, or
what to check before opening the PR. With eight worktrees open that context lives
outside gwm, so returning to one after two days means rebuilding it from
`git log` and memory.

A note attached to the worktree, editable from the TUI and flagged in the table.
Storage is settled: a plain markdown file at `.git/gwm/notes/<branch>.md` in the
main checkout, which survives `gwm remove`, stays readable from the main checkout
and is never committed. No sweeper is needed, because "the branch is gone" is a
question git answers in one call, and `gwm doctor` reports what is left. Plain
text, because a note is something one may want to `grep` or open without gwm
running, which is also what rules git config out despite gwm already keeping
three per-branch keys there.

Comes after the PR/Issue view, which pays for the overlay machinery it would
reuse.

### 6. Container execution ([#421](https://github.com/kbrdn1/gwm-cli/issues/421) ✅)

A `container:` block on the existing `exec` / aliases surface, wrapping the
command in `docker run`. Anything exposing a Docker-compatible socket works for
free, so there is no runtime to integrate. The cost is not the wrapper, it is the
mount: a linked worktree's `.git` is a file holding an absolute host path, so
mounting the worktree alone produces a container in which git does not answer.
The reference implementation has exactly that bug, in the middle of its own
agent-per-worktree use case. gwm mirrors the host paths and mounts the main
checkout's gitdir alongside, which turns this from a column to match into one to
win.

It was deferred for want of observed demand. It moves into the line because the
comparison page now sits behind it rather than ahead: container execution is a
column in that table, and writing the table first would mean publishing a column
gwm loses by default.

Built narrower than the reference's nine fields: `image`, `runtime`,
`extra_args`. The dropped ones are not missing, they are covered: `-w`, `-e`
and `-v` are `docker run` flags, and `extra_args` lands after gwm's own so a
repeat overrides them. Two decisions the issue asked to be resolved rather than
discovered: there is no `interactive` knob, because `gwm exec` is a fan-out over
N worktrees and a TTY per container means nothing there, while the TUI exec
overlay owns a real pty and therefore runs its container with `-i -t`; and the
command is the container's CMD, so an image's ENTRYPOINT receives it as
arguments. Refused on Windows, where mirroring a host path into a Linux
container cannot work and the `.git` file would name a drive letter anyway. The block rides a
profile only: the inline `gwm exec -- <cmd>` is the frozen surface (#319) and a
config file must not change what an unchanged command line does. `[aliases]`
needed nothing, contrary to what the issue title suggests. A gwm alias is argv
substitution towards a subcommand, not a custom command, so `t = "exec --profile
ci"` inherits the container through the profile it expands to.

### 7. Comparison page ([#422](https://github.com/kbrdn1/gwm-cli/issues/422))

gwm against gwq and lazyworktree. Content rather than product work, but it is
what decides whether any of the above is seen. gwq is the incumbent by star
count and has been dormant since May.

Last of the product steps, and the position is the point: a comparison is worth
writing once the features it compares are the ones shipping. That now includes container
execution (#421), which is why it moved behind that rather than behind the
notes.

**The issue needs rewriting before it is picked up.** It was filed against an
older read of the field: there was no container column in it, and the
lazyworktree half has not been re-read since. Both have to be redone against
what the two projects are now, otherwise the page ships a comparison of what
they were.

### 8. Documentation in German, Spanish and Japanese ([#522](https://github.com/kbrdn1/gwm-cli/issues/522))

Five locales instead of two, immediately before the cut and after the comparison
page. The audience data does not ask for this: of the 122 stargazers, 83 declare
a location and no non-English bloc reaches 10%. It is a forward bet, and each
language carries a different one. German is the largest non-English bloc already
present. Spanish is not visible in the current stargazers at all, which is the
point, since it buys reach that does not exist yet. Japanese is the only one with
a competitive argument: gwq, the incumbent this project is positioned against, is
Japanese-authored, and that community sustains a dense native technical-content
culture that generates real inbound.

The site side is tracked separately in `kbrdn1/kbrdn-docs#54` and is deliberately
decoupled. The sync pipeline carries the locale as a boolean (`isFr`), and
turning that into a locale key can land before a single translation exists,
because a declared locale with no content falls back to English. The SEO wiring
needs nothing: `hreflang` alternates plus an `x-default` on the unprefixed root
are already emitted per page.

⚠️ The content is 51,612 English words across 41 pages, so roughly 155,000 at
three languages, and no native reader is lined up for any of the three. Worth
naming before it is scheduled against a date.

### Deferred

- [#414](https://github.com/kbrdn1/gwm-cli/issues/414): **process-level agent liveness on macOS / Linux**: detection reads transcript mtime, which is *activity*, not liveness. It lags a session that is thinking or waiting on a long tool call, and a crashed one looks recent for a while. The targeted Unix refinement shipped in v1.3.0 ([#441](https://github.com/kbrdn1/gwm-cli/issues/441)): a Claude Code session whose recorded PID is gone drops to idle immediately. What remains is the general case, deferred by the issue's own title.

### Maintenance

- [#500](https://github.com/kbrdn1/gwm-cli/issues/500) ✅: `exec_tests` flaked on Linux with `ETXTBSY`: the test writes an executable and runs it, which races against the fork-to-exec window of any other test in the harness that spawns a process. Fixed in v1.6.1 by retrying on that one errno, with the reason written next to it so it does not read as flake-hiding later
- The transitive graph needs a manual `cargo update` from time to time. `.github/dependabot.yml` sets no `allow: dependency-type: all`, so dependabot only ever PRs the direct dependencies. Last full refresh: [#499](https://github.com/kbrdn1/gwm-cli/pull/499), which also took the vendored libgit2 from 1.9.3 to 1.9.6. Do it **after** an MSRV change lands, never before: a lockfile refresh is exactly how a floor moves without anyone deciding to, so it has to be checked against a floor that is already settled

### Visibility

Not feature work, but tracked here because it gates whether any of the above is
seen. The launch itself happens at the v1.7.0 delivery, so these are its
prerequisites rather than a parallel track.

Measured on 2026-08-04, and it reframes the section: the repository went from 30
to 122 stars in fourteen days, and over that window the referrers put X first by
a factor of three, from Terminal Trove and from the ratatui maintainer. Neither
was solicited, and nothing in this repository was feeding them. What those relays
republish is visual, which is why the first two items below are images.

- [#523](https://github.com/kbrdn1/gwm-cli/issues/523): **re-record the demo GIF**. Frame 0 is an empty terminal, and on GitHub, on X and in a Terminal Trove listing the first frame is the thumbnail. It also renders `gwm 1.0.1` in the version badge, six minor versions behind, and it predates the agent session pane, which is now the line the repository description leads on
- [#524](https://github.com/kbrdn1/gwm-cli/issues/524): **visual coverage of the docs**: 26 of the 41 English pages carry no image, including the whole CLI section and `configuration/gwm-toml.md`, the longest page in the tree and the one that answers the long-tail queries the site exists to capture. The agent session pane has neither a page nor a capture
- [#525](https://github.com/kbrdn1/gwm-cli/issues/525): **feed the relays**, before the launch rather than after, since they are what amplified the project last time: `awesome-ratatui` first because that is the circle that produced the traffic, then Terminal Trove and the ratatui showcase, then the generic awesome lists. Bing Webmaster Tools is the remaining indexing gap, and it also feeds DuckDuckGo; Google Search Console is done
- The comparison page ([#422](https://github.com/kbrdn1/gwm-cli/issues/422)) sits in the numbered line above as its last product step rather than here, and the translations ([#522](https://github.com/kbrdn1/gwm-cli/issues/522)) follow it
- [#516](https://github.com/kbrdn1/gwm-cli/issues/516) ✅: the em dash is retired across `docs/`: 1586 occurrences in 78 of the 79 pages, English and French, replaced by whatever connector the dash was standing in for (a colon where it introduced a list or an explanation, a full stop where it joined two independent clauses, a comma or parentheses around an aside). Fenced code blocks are untouched, so 60 survive there. 45 headings changed shape and none was the target of an internal link, checked by resolving the tree's 194 internal anchors against the heading slugs of both the before and after trees. Merged in [#518](https://github.com/kbrdn1/gwm-cli/pull/518)
- [#423](https://github.com/kbrdn1/gwm-cli/issues/423) ✅: the documentation is published at **<https://gwm.kbrdn.dev>** and keeps itself there. A merge into `main` touching `docs/`, `changelogs/` or `Cargo.toml` posts a `repository_dispatch` to `kbrdn1/kbrdn-docs`, which reruns the conversion, commits whatever drifted and redeploys. The in-repo tree stays the source of truth: the site is generated from it, never edited on the other side

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
3. `gwm create <type> <issue> <slug>` to spin up an isolated worktree (the issue auto-links itself, see [`docs/integrations/github-linking.md`](docs/5.integrations/1.github-linking.md)).
4. Open a PR targeting `dev` following the conventions in [CONTRIBUTING.md](CONTRIBUTING.md) (Gitmoji + Conventional Commits, tests required, never squash; full docs version at [`docs/development/contributing.md`](docs/6.development/2.contributing.md)).
5. The issue is the source of truth: this roadmap is updated to reflect what ships in each release.

Items marked `good first issue` (when applicable) are intentionally scoped so a newcomer can land them without a deep dive into the codebase.

## Out of scope (for now)

A few directions the project deliberately steers clear of:

- **Replacing lazygit / gitui in scope**: `gwm` is a worktree manager. Git history surgery stays with the dedicated tools that already do it well; `gwm` integrates with them rather than competing.
- **GUI front-end**: the terminal is the target. A GUI app would split focus and dilute the design.
- **Worktree synchronisation across machines**: too much surface (state, conflict, networking) for a tool whose value is local responsiveness.

That can change if a concrete use case shows up. Open a feature-request issue with the rationale.
