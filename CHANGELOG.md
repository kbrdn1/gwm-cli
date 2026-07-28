# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Free-form worktree naming. `gwm create --name spike-redis` skips the
  `<type> <issue> <desc>` triple entirely — not every worktree corresponds to
  an issue. In the TUI, `Ctrl-T` toggles the create form between the
  structured triple and a single `Name` field. The flag is exclusive with the
  positionals, so a partial triple is still the typo it always was; the mode
  is chosen explicitly, never inferred from how many arguments arrived.

  The name becomes the branch verbatim, and is validated verbatim: `--name
  " spike"` is refused rather than trimmed into `spike`, which would be a
  different branch from the one asked for. `branch_pattern` / `path_pattern`
  do not apply — they are written in terms of `{type}` / `{issue}` / `{desc}`,
  and a free-form name has none of them — while `[worktree].base` still does,
  so free-form worktrees land beside the structured ones. `base` is only
  expanded with the placeholders it documents, though: the structured path
  feeds `{type}` / `{issue}` / `{desc}` through `base` too, and since an
  unfed placeholder is left *literal*, a base written with one of them is
  refused here rather than turned into a directory called `{type}`.

  What a free-form worktree gives up is stated rather than discovered: issue
  auto-linking goes inactive (`gwm link` remains), `gwm commit-prefix` errors
  because a prefix is derived from the branch type and there is none, and
  create/remove/bootstrap hook placeholders resolve empty. PR/MR detection is
  unaffected — it queries the forge with the whole branch name. `doctor`
  treats the branch as user-managed and never flags it. All of which applies
  to a name that does *not* match the branch convention: nothing records how
  a worktree was named, only what its branch is, so `--name 'feat/#42-x'` is
  read back as structured and keeps every one of those.

  The accepted-name rules are enumerated from the three things a free-form
  name has to be at once, rather than accreted one example at a time. It is a
  **git branch** — validated with libgit2's branch-level oracle, which is
  stricter than the reference-level one (`refs/heads/HEAD` is a valid
  reference name, `HEAD` is not a usable branch name). It is a **single
  filesystem path component**, which a branch name is not: no `.` / `..`
  component, and at most 255 bytes, since `a×130/b×130` is a legal ref and an
  illegal directory name and without the cap the branch is created before the
  directory fails, leaving it orphaned. And it is a **literal value during
  hook expansion**, so no `{` / `}`: placeholders are substituted in sequence,
  and `spike-{issue}` would have its own name rewritten inside the `{branch}`
  value a hook receives. Plus one rule belonging to none of them: no leading
  `-`, which git accepts but `gwm remove` and `git branch -d` read as a flag.
  Windows-specific path rules are deliberately **not** covered — they cannot
  be measured from a Unix machine, and the gap is tracked by
  [#475](https://github.com/kbrdn1/gwm-cli/issues/475) rather than guessed at
  here.
  No `SCHEMA_VERSION` bump — `JsonWorktree` carries no `type` / `desc`, so
  the wire format is unchanged.
  ([#416](https://github.com/kbrdn1/gwm-cli/issues/416))

### Fixed

- `gwm doctor` and `gwm config validate` now warn when
  `worktree.branch_pattern` does not survive a format-then-parse round-trip.
  The pattern drives how a branch name is *written* but not how one is *read
  back* — the parser is still a hardcoded regex — so a mismatched pattern
  silently broke issue/PR auto-linking, gitmoji selection, lifecycle hook
  placeholders and the branch-convention check. The warning names whichever
  segments actually break: a custom pattern is not automatically a broken one
  (`{type}/#{issue}-prefix-{desc}` still recovers `type` and `issue`, only
  `desc` comes back wrong), and claiming otherwise would defeat the point of a
  warning whose whole value is accuracy. The probe enumerates the value space
  `gwm create` admits rather than sampling it — every configured branch type,
  a single- and a multi-digit issue, a desc with and without the `-` it
  allows, and the real repo name for `{repo}` — so a pattern that breaks on
  only some values is reported as breaking on only some values. `gwm config validate` prints it on
  stderr, reads the *effective* pattern so one set only in the global
  `~/.config/gwm/config.toml` is caught too, and still exits `0`: a custom
  pattern is valid configuration, not an error (`gwm doctor` reports it as a
  `!` check, so that command exits `1` like any other Warning). The
  config-supplied pattern is neutralised for control characters before it is
  echoed — neither command goes through the trust gate, so an unvetted
  `.gwm.toml` must not get a terminal escape channel out of a health check.
  This states the limitation; the entry below removes its cause, so the set of
  patterns it has anything to say about is much smaller than it was.
  ([#415](https://github.com/kbrdn1/gwm-cli/issues/415))

- `worktree.branch_pattern` is now read back by a parser compiled from that
  same pattern, so customising it no longer disables the features that re-read
  a branch name. The pattern drove how a branch was *written* while a hardcoded
  `^([a-z]+)/#(\d+)-([a-z0-9-]+)$` decided how one was *read*, so a repo that
  set `branch_pattern = "{type}-{issue}-{desc}"` created `feat-41-foo` and then
  failed to recognise the branch it had just created: no issue auto-linking, no
  gitmoji, `gwm commit-prefix` erroring, empty hook placeholders on the
  remove / bootstrap paths, a rename modal that refused to open, and a `doctor`
  orphan check that skipped every branch as user-managed. One source of truth
  now, so the conventions people actually use keep all of it: `{type}-{issue}-{desc}`,
  `{type}_{issue}_{desc}`, `{type}/{issue}-{desc}`, `wt/{type}/#{issue}-{desc}`,
  `{desc}/#{issue}-{type}` and a literal wedged anywhere all round-trip.

  Two patterns are refused rather than compiled into a parser that reads back
  the wrong thing: two placeholders written back to back (`{issue}{desc}` reads
  `42123-x` as `4212` + `3-x`; `{desc}{issue}` is ambiguous outright, since
  `a12` is what both `a` + `12` and `a1` + `2` produce), and the same
  placeholder twice. Note that adjacency is the whole rule: a separator drawn
  from the left placeholder's own charset is fine, so `{desc}-{issue}` works,
  because an issue number cannot contain the `-` and there is therefore exactly
  one valid split. `gwm doctor` and `gwm config validate` report the refusal
  with the fix in it.

  Two narrowings ship with this, both deliberate and both reported rather than
  silent. `{type}` compiles to an alternation of the repo's configured branch
  types instead of `[a-z]+`, so a branch carrying a type the repo does not
  declare is no longer claimed as gwm's: `doctor` leaves it alone and the TUI
  rename refuses it, which is what that modal already did one step later. And a
  literal in the type's position is text rather than a type, so
  `feat/#{issue}-{desc}` no longer yields a branch type even when `feat` is the
  only configured one; inferring intent from literal text is guesswork on any
  repo where a type name is also an ordinary word. `gwm doctor` names the
  missing placeholder and how to get it back.
  ([#417](https://github.com/kbrdn1/gwm-cli/issues/417))

### Docs

- `changelogs/1.5.0.md` was corrected after the `v1.5.0` tag: its caveat said
  the GitLab backend had been verified from `glab`'s documentation, when it had
  in fact been driven end to end by the real `glab` 1.109.0 binary against a
  local fake GitLab server. The published release body was re-sourced from the
  corrected file with `gh release edit --notes-file`, so this diff on an
  archived version file is deliberate and already live on the release page. The
  `v1.5.0` tag itself is untouched.

## Past releases

In reverse chronological order:

- [`1.5.0`](changelogs/1.5.0.md) — 2026-07-26
- [`1.4.0`](changelogs/1.4.0.md) — 2026-07-25
- [`1.3.0`](changelogs/1.3.0.md) — 2026-07-24
- [`1.2.0`](changelogs/1.2.0.md) — 2026-07-21
- [`1.1.1`](changelogs/1.1.1.md) — 2026-07-16
- [`1.1.0`](changelogs/1.1.0.md) — 2026-07-15
- [`1.0.3`](changelogs/1.0.3.md) — 2026-07-09
- [`1.0.2`](changelogs/1.0.2.md) — 2026-07-06
- [`1.0.1`](changelogs/1.0.1.md) — 2026-07-01
- [`1.0.0`](changelogs/1.0.0.md) — 2026-06-26
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

- [`0.10.0-rc.4`](changelogs/pre-releases/0.10.0-rc.4.md) — 2026-06-17
- [`0.10.0-rc.3`](changelogs/pre-releases/0.10.0-rc.3.md) — 2026-06-17
- [`0.10.0-rc.2`](changelogs/pre-releases/0.10.0-rc.2.md) — 2026-06-16
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
