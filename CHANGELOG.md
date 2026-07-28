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
  remove/bootstrap hook placeholders resolve empty. PR/MR detection is
  unaffected — it queries the forge with the whole branch name. `doctor`
  treats the branch as user-managed and never flags it.

  Names are validated against libgit2's own branch-name rules rather than a
  hand-written list, plus three rules of our own: no `.` / `..` path component
  (a worktree directory named `..` would escape the base), no leading `-`
  (git accepts it; `gwm remove` and `git branch -d` would not), and a 255-byte
  cap — a branch name is a *path* of components while the worktree directory
  is a single one, so `a×130/b×130` is a legal ref and an illegal directory
  name, and without the cap the branch is created before the directory fails,
  leaving it orphaned.
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
  `.gwm.toml` must not get a terminal escape channel out of a health check. This states the limitation, it
  does not remove it — deriving the parser from the pattern is tracked by #417.
  ([#415](https://github.com/kbrdn1/gwm-cli/issues/415))

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
