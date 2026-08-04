# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
