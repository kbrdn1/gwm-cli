# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Inline review comments in the rich PR view**
  ([#528](https://github.com/kbrdn1/gwm-cli/issues/528)). The comments
  anchored to a diff hunk now render in the `I` view, under the reviews: one
  section per thread, showing its anchor (`src/tui/app.rs:7-11`, marked
  `resolved` / `outdated` when it applies), the diff hunk it hangs from, and
  the reply chain in order. These are reachable through GraphQL alone
  (`gh pr view --json comments` returns the conversation), so they travel on
  a **second** request, fired when the view opens rather than alongside every
  PR fetch: `gwm status` calls the same fetch on every invocation and has no
  use for them. Hunk lines are truncated instead of wrapped, since a wrapped
  `+` line's continuation would carry no sigil and read as context, and a
  long hunk drops its head rather than its tail because the anchored line is
  the last one. Every cap states its count from the forge's own totals, and
  each comment header keeps its permalink so `Enter` opens the full thread.
  On a GitLab remote the section is present and says the backend cannot
  reach these, which is a different fact from "there are none": GitLab
  carries them as discussion notes with `position` data, under a different
  request and a different shape, deliberately out of scope here.

- **Rich PR / issue view in the TUI**
  ([#420](https://github.com/kbrdn1/gwm-cli/issues/420)). `I` opens the linked
  PR (or the linked issue when there is no PR) on its description, author,
  branch pair, diff size, CI rollup, submitted reviews and conversation,
  without leaving the TUI. `gwm` already asked `gh` for the rollup and threw
  the per-check detail away; it now asks for `body`, `author`, `comments`,
  `reviews`, `additions` / `deletions` and the branch refs in the same single
  request, so the view costs no extra round trip. Bodies are wrapped against
  the modal width and re-wrapped on resize, and every cap says how much it
  dropped (`… 312 more lines`) rather than stopping silently. Remote text is
  neutralised on the way in, so a bidi override in a comment cannot reorder
  what the terminal paints. `Enter` opens the selected row's URL, `f`
  re-fetches, and the whole verb set is rebindable under
  `[tui.keys.modal.rich_view]`. On GitLab the view renders the summary tier
  plus the description, author and branch pair; approvals, notes and the diff
  size need separate API calls and are not fetched.

- **Multi-row selection in the TUI, and a batch delete on top of it**
  ([#484](https://github.com/kbrdn1/gwm-cli/issues/484)). `Space` marks the
  highlighted worktree, `d` then deletes every marked row in one batch; with
  nothing marked it stays the single-row delete it has always been. Only `d`
  reads the mark set, so the worktrees footer carries the count
  (` 3 of 12 · 2 marked `) rather than letting a live selection go invisible
  under `b` / `s` / `p`. Marks are keyed by on-disk path, which is what makes
  them survive the fuzzy reranking and stay unambiguous in workspace mode,
  where two repos can hold the same worktree id. Opening the filter and the
  manual `f` refresh clear them; the background auto-refresh only prunes rows
  that no longer exist, otherwise a 60s timer would eat a selection still
  being built. The confirm overlay snapshots its targets when it opens, so a
  refresh landing during the safety countdown cannot retarget the deletion,
  and for a batch it reports the size and how many targets carry a branch
  instead of listing rows, with `D` arming the branch deletion batch-wide. A
  batch never stops at the first error: every target is attempted through its
  own repo handle and only after re-checking that its id still resolves to the
  path the overlay named (a worktree removed and recreated from another shell
  during the countdown gets the same id back, and removing by id alone would
  have deleted it), the confirm stays open narrowed to what failed (narrowed,
  never recomputed: `worktree::remove` prunes the admin entry before deleting
  the directory, so a removal that fails on the filesystem drops its own row,
  and recomputing would have fallen back to the cursor row), and the status
  line names the failures.
- **`gwm remove` takes several patterns**
  ([#484](https://github.com/kbrdn1/gwm-cli/issues/484)). `gwm remove a b c`
  removes the batch in one command and `--dry-run` prints one plan per
  pattern. Every pattern is resolved before anything is touched, so an unknown
  or ambiguous one fails the whole command with nothing removed, which is what
  `gwm list --format json | ... | xargs -n1 gwm remove` could not do: it
  removed the first half of the batch and then reported the typo. Patterns
  naming the same worktree collapse to a single removal.

- **`symfony` config preset**
  ([#392](https://github.com/kbrdn1/gwm-cli/issues/392)). A seventh
  `gwm init --preset`, next to `laravel` on the composer side but built on
  Symfony's own dotenv convention, which is the mirror image of Laravel's:
  `.env` is committed and holds the neutral defaults, `.env.local` is
  gitignored and holds the secrets. So the preset copies `.env.local` and
  `.env.test.local` rather than `.env`, and the `no-aws-rds` guard seeds from
  the committed `.env` instead of an `.env.example`. `var/` joins `vendor/` in
  the no-symlink invariants, because it holds the compiled service container
  and the cached routes: sharing it between two worktrees running different
  code is worse than a slow first request. `composer install` and
  `direnv allow .` run on the same `when` predicates as the Laravel preset.

### Changed

- **The TUI delete runs the remove hooks and records the undo journal**
  ([#521](https://github.com/kbrdn1/gwm-cli/issues/521)). `d` called
  `worktree::remove` directly, so `[hooks.pre_remove]` / `[hooks.post_remove]`
  never ran and nothing reached `gwm undo`: an interactive delete was
  unrecoverable, and a hook written to guard a removal only held for whoever
  used the CLI. Both paths now go through one sequence, so `gwm undo` puts
  back what `d` removed, exactly as it does for `gwm remove`. `undo` stays
  per-worktree: a batch of ten appends ten entries and pops one at a time.
  Two consequences worth knowing before you upgrade. A `pre_remove` that
  refuses now refuses in the TUI too, for that one target, with the batch
  carrying on and the confirm overlay staying open on what failed. And
  because running a hook means executing code out of `.gwm.toml`, a delete in
  a repo whose config has remove hooks is gated on the TOFU trust ledger
  (#95): unapproved, it refuses rather than silently skipping the guard hook.
  The gate asks about the two remove phases only, so a config whose hooks are
  all `post_create` runs nothing on a delete and is never asked. Hook output
  lands on the Command Logs transcript (`3`) as it does everywhere else.
- **A refused removal is no longer recorded as undoable**
  ([#521](https://github.com/kbrdn1/gwm-cli/issues/521)). `gwm remove` wrote
  its journal entry before the destructive call, so a target that then got
  refused (its path moved since it was resolved, git declining the removal)
  still showed up in `gwm history` as something `gwm undo` would replay. The
  branch OID is still captured beforehand, which is what the entry needs; only
  the write moved after the removal succeeds.
- **`cycle_sidebar_layout` moved from `Space` to `z`**
  ([#484](https://github.com/kbrdn1/gwm-cli/issues/484)), to make room for the
  row mark. Space-to-mark is the convention in lazygit, k9s and fzf, so the
  default was picked on merit rather than on which verb was there first. Both
  pre-#484 defaults are one `[tui.keys]` line away
  (`cycle_sidebar_layout = ["Space"]`, `toggle_select = ["z"]`), and
  `gwm tui keys` prints the resolved set with a per-row source. One upgrade
  note: `z` is now a shipped default, so a `.gwm.toml` that binds a chord
  *starting* with `z` (say `top = ["z z"]`) is a prefix conflict and is
  refused at load time, the same way any chord/prefix pair has always been.
  Rebind that chord, or move `cycle_sidebar_layout` elsewhere.

### Fixed

- **`gwm doctor` now reads `[hooks.*]`, not just `[[bootstrap.command]]`.**
  Two checks walked the bootstrap commands alone, so a config whose commands
  all live in lifecycle hooks got a report about a file the doctor had barely
  read: a typo in a hook's `when` predicate was announced as "no `when:`
  predicates configured", and a hook invoking a binary that is not installed
  produced a clean bill of health right up to the moment `gwm create` ran it
  and failed. Both now walk the six phases as well, and the `when` failure
  names the phase and step it came from (`bogus:1 (on hook post_create
  \`install\`)`). Surfaced by the `symfony` preset, whose commands are all
  hooks, but it applied to every hooks-based config since the phases landed.
  `LifecycleHooksConfig::all_steps()` enumerates the phases through an
  exhaustive destructuring, so adding a seventh phase without teaching the
  consumers about it is now a compile error rather than a silent blind spot.
  The PATH probe also honours each step's `when` predicate now, on bootstrap
  commands as well, which it never did: the `node` preset ships `bun install`
  under `cmd_exists:bun` and `npm ci` under `!cmd_exists:bun`, so probing both
  regardless warned about whichever one the predicate had switched off, and a
  Warning takes `gwm doctor` to exit code 1. The predicate is evaluated
  against the main checkout, the same approximation the `.envrc` probe already
  made; an unrecognised keyword still evaluates to `true`, matching the step
  running anyway at bootstrap time. Only two predicate shapes are
  evaluated, because a `.gwm.toml` never went through the trust gate:
  `cmd_exists:` on a bare binary name, which is a `$PATH` lookup on the same
  set the probe reports, and `file_exists:` on a single repo-root component
  that is not itself a symlink, which is a `stat` on something the config's
  own author committed. Everything else is a channel out of the repo for a
  file nobody vetted, and one declined atom leaves the whole expression
  unevaluated: `glob_exists:` picks its own root and walks it, a
  multi-component `file_exists:` escapes through a committed symlink
  (`outside/etc/passwd` with `outside -> /`) in a way no spelling check
  catches, `env_set:` / `env_eq:` read the process environment and report the
  answer through which binaries got probed, and a `cmd_exists:` argument with
  a path separator is `file_exists:` under another name. Declining costs
  nothing, the step simply stays probed, which is what the check did before it
  evaluated anything.
  A step whose binary cannot be resolved statically is left alone for the
  same reason, from the other side: `lifecycle::run_step` expands `{path}` /
  `{repo}` in `run` before spawning, so a hook reading `{path}/scripts/setup`
  was probed as that literal string and always came back missing, and a step
  that sets its own `PATH` in `env` resolves against that rather than against
  the doctor's ambient one. Same for a script that opens on a shell word: a
  `run` is handed whole to `sh -c`, so `cd sub && ./setup.sh` or
  `if [ -f composer.json ]; then …` used to be probed as `cd` and `if`. That
  one bit hooks harder than bootstrap commands, since a hook is a script far
  more often.


### Docs

- **Retired the em dash across the whole `docs/` tree**
  ([#516](https://github.com/kbrdn1/gwm-cli/issues/516)). 1586 occurrences in
  78 of the 79 pages, English and French, replaced by whatever connector the
  dash was standing in for: a colon where it introduced a list or an
  explanation, a full stop where it joined two independent clauses, a comma or
  parentheses around an aside. Fenced code blocks are untouched, since they
  reproduce shell comments and program output. Schema and reference tables
  used a bare dash as a cell value for two different things, "no default" and
  "this preset adds nothing here"; those now read `_(required)_` and
  `_(none)_`. 45 headings change shape, so their generated anchors change with
  them; none of them was the target of an internal link, and the 194 internal
  anchors in the tree resolve exactly as they did before.

## Past releases

In reverse chronological order:

- [`1.6.1`](changelogs/1.6.1.md), 2026-08-04
- [`1.6.0`](changelogs/1.6.0.md), 2026-08-03
- [`1.5.0`](changelogs/1.5.0.md), 2026-07-26
- [`1.4.0`](changelogs/1.4.0.md), 2026-07-25
- [`1.3.0`](changelogs/1.3.0.md), 2026-07-24
- [`1.2.0`](changelogs/1.2.0.md), 2026-07-21
- [`1.1.1`](changelogs/1.1.1.md), 2026-07-16
- [`1.1.0`](changelogs/1.1.0.md), 2026-07-15
- [`1.0.3`](changelogs/1.0.3.md), 2026-07-09
- [`1.0.2`](changelogs/1.0.2.md), 2026-07-06
- [`1.0.1`](changelogs/1.0.1.md), 2026-07-01
- [`1.0.0`](changelogs/1.0.0.md), 2026-06-26
- [`0.9.0`](changelogs/0.9.0.md), 2026-06-07
- [`0.8.0`](changelogs/0.8.0.md), 2026-06-01
- [`0.7.0`](changelogs/0.7.0.md), 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md), 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md), 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md), 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md), 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md), 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md), 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.10.0-rc.4`](changelogs/pre-releases/0.10.0-rc.4.md), 2026-06-17
- [`0.10.0-rc.3`](changelogs/pre-releases/0.10.0-rc.3.md), 2026-06-17
- [`0.10.0-rc.2`](changelogs/pre-releases/0.10.0-rc.2.md), 2026-06-16
- [`0.10.0-rc.1`](changelogs/pre-releases/0.10.0-rc.1.md), 2026-06-10
- [`0.9.0-rc.3`](changelogs/pre-releases/0.9.0-rc.3.md), 2026-06-07
- [`0.9.0-rc.2`](changelogs/pre-releases/0.9.0-rc.2.md), 2026-06-06
- [`0.9.0-rc.1`](changelogs/pre-releases/0.9.0-rc.1.md), 2026-06-02
- [`0.8.0-rc.5`](changelogs/pre-releases/0.8.0-rc.5.md), 2026-06-01
- [`0.8.0-rc.4`](changelogs/pre-releases/0.8.0-rc.4.md), 2026-05-29
- [`0.8.0-rc.3`](changelogs/pre-releases/0.8.0-rc.3.md), 2026-05-29
- [`0.8.0-rc.2`](changelogs/pre-releases/0.8.0-rc.2.md), 2026-05-23
- [`0.8.0-rc.1`](changelogs/pre-releases/0.8.0-rc.1.md), 2026-05-23
- [`0.7.0-rc.3`](changelogs/pre-releases/0.7.0-rc.3.md), 2026-05-23
- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md), 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md), 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md), 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md), 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md), 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md), 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md), 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md), 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md), 2026-05-18
