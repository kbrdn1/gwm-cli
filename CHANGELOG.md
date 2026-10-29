# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **The capture preflight passed a vhs that writes nothing**
  ([#700](https://github.com/kbrdn1/gwm-cli/issues/700)). vhs 0.12.0 runs a
  tape to the end, prints `Creating ….gif`, exits 0 and never starts the
  encoder ([charmbracelet/vhs#787](https://github.com/charmbracelet/vhs/issues/787)).
  It was installed on the release machine on 2026-09-10 and nobody saw it until
  the v1.11.0 cut. The per-tape freshness check in `generate.sh` did fire, and
  the run did exit 1, but only after each tape it reached had failed twice,
  every failure reported as the write race the retry exists for, until the
  theme gallery's own check stopped it. The preflight, the half
  `tests/capture_pipeline_tests.rs` drives, checked vhs through
  `version-stamp.tape`, whose stamp is written by a shell redirection inside
  the tape and so lands whether vhs can write or not. The preflight now also
  requires the tape's own gif, the one file only vhs writes, and stops before
  any tape when it is missing. Measured against the real vhs both ways: 0.12.1
  wrote the gif 10 runs out of 10, and a no-op `ffmpeg` first on `PATH`, which
  reproduces 0.12.0's signature, stops the preflight. The floor, vhs 0.12.1 or
  later, is in CONTRIBUTING.md § Releases and `docs/_capture/README.md`.

### Security

- ⬆️ Bumped `libz-sys` `1.1.29` → `1.1.31` to clear [RUSTSEC-2026-0418]
  (its build script leaks environment variables into the build directory),
  published on 2026-10-28 and failing the `cargo audit --deny warnings` CI
  gate from then on. It is both a direct dependency (declared for its static
  zlib) and pulled by `git2 → libgit2-sys`; the `version = "1"` requirement
  already allows 1.1.31, so only `Cargo.lock` changes.

[RUSTSEC-2026-0418]: https://rustsec.org/advisories/RUSTSEC-2026-0418

### Docs

- `skills/SKILL.md` presents 1.11.0 as the current line and gains the 1.10.0
  line it never had; its key table follows the 1.10.0 moves (`e` / `E` for
  rename and exit, `c` / `C` for commits and CI checks) and lists the
  list-view keys 1.10.0 and 1.11.0 added, plus `o` on the agents overlay; its
  CLI references gain `gwm herdr`, `gwm create --issue` and
  `gwm doctor --fix`. CONTRIBUTING.md links the docs site at `gwm.kbrdn.dev`,
  the address it is served at, instead of a `gwm-docs` subdomain that does
  not resolve ([#700](https://github.com/kbrdn1/gwm-cli/issues/700)).
- **The README banner takes the kbrdn.dev design**: a pixel field, a rounded
  panel, rounded chips and install box, a dashed rule over the footer. Same
  content as the v1.11.0 banner. Its generator config now lives beside it as
  `docs/_assets/promo.config.json`
  ([#704](https://github.com/kbrdn1/gwm-cli/issues/704)).
- **The README banner draws gwm instead of photographing it**: the card's
  thumbnail is an isometric illustration, the repo's `.git` at the base and
  three worktrees stacked on it, the newest one fresh out of
  `gwm create feat 42 user-auth` with its `.env` copied and `post_create` run.
  The TUI capture stays three lines below in the README. The test chip reads
  3585, the nextest summary of CI on `be7e3a80`.


## Past releases

In reverse chronological order:

- [`1.11.0`](changelogs/1.11.0.md), 2026-10-26
- [`1.10.0`](changelogs/1.10.0.md), 2026-09-01
- [`1.9.0`](changelogs/1.9.0.md), 2026-08-16
- [`1.8.0`](changelogs/1.8.0.md), 2026-08-13
- [`1.7.1`](changelogs/1.7.1.md), 2026-08-12
- [`1.7.0`](changelogs/1.7.0.md), 2026-08-12
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
