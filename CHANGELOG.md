# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **`cargo test` wrote the contributor's real undo journal**
  ([#724](https://github.com/kbrdn1/gwm-cli/issues/724)). About fifteen
  `gwm remove` runs in gwm's own suite spawned the binary without
  `GWM_HISTORY_FILE`, so every local run appended entries for temporary repos to
  the developer's `history.toml` and, at the cap, evicted real ones. The tests
  that created a worktree without pinning `[worktree] base` left it under
  `~/cc-worktree`, or under the base set by the developer's global config, which
  the spawned binary also read. Every test now spawns `gwm` through one builder,
  `tests/common::gwm()`, which points the journal, the trust ledger, the global
  config, `aliases.toml`, the agent-session scan and `$HOME` at a scratch
  directory per test, and a guard fails if another test file resolves the built
  binary, or runs `gwm` off `PATH`, itself. CI was never affected. On Windows
  `$HOME` does not move `{home}`, so a test that creates worktrees there still
  pins its base. Leftovers of earlier runs are not removed:
  `~/cc-worktree/.tmp*` and `$TMPDIR/.tmp*-worktrees` can be deleted by hand.

- **The undo journal evicted real entries before dead ones**
  ([#713](https://github.com/kbrdn1/gwm-cli/issues/713)). `history.toml` is
  capped at 100 entries across all repos and used to drop the oldest one on
  overflow, whatever its repo. A tool that ran real `gwm remove` in temporary
  repos from its test suite left entries that can never be undone, and each
  one pushed out a genuine, older entry. On overflow the journal now drops the
  oldest entry whose repo no longer exists, otherwise the oldest entry of the
  repo holding the most entries, so one repo can no longer flush another's
  history (at worst the repos end up with an even share). Nothing is purged on write: a repo on an unmounted disk keeps its
  entries until the room is needed. A single-repo journal rotates exactly as
  before. The variables that keep a consumer's test suite off the user's
  journal, trust ledger and config are now listed in
  [Daemon consumers](https://github.com/kbrdn1/gwm-cli/blob/main/docs/5.integrations/4.daemon-consumers.md#isolating-gwm-in-a-consumers-tests).

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
  already allows 1.1.31, so `Cargo.lock` moves and `third-party/README.md`
  records the new provenance. The zlib notice 1.1.31 vendors is
  byte-identical, so `third-party/zlib-LICENSE` keeps its blob id.
- ⬆️ Bumped `rusqlite` `0.40` → `0.41.1` to clear [RUSTSEC-2026-0459]
  ([#720](https://github.com/kbrdn1/gwm-cli/issues/720)): the SQLite
  `libsqlite3-sys` 0.38.2 bundled carries CVE-2026-119375, fixed from 0.39.0,
  and 0.39 is only reachable through `rusqlite` 0.41. The bundled SQLite moves
  to 3.54.1. gwm only opens opencode's session database read-only, through
  calls 0.41 leaves unchanged.
- ⬆️ Bumped `rustix` `1.1.4` → `1.1.6` (transitive, lock only) to clear
  [RUSTSEC-2026-0486], an unsoundness in its xattr functions. Both advisories
  were published at the end of October and turned the
  `cargo audit --deny warnings` gate red on every PR; neither can go green
  alone, so they land together.

[RUSTSEC-2026-0418]: https://rustsec.org/advisories/RUSTSEC-2026-0418
[RUSTSEC-2026-0459]: https://rustsec.org/advisories/RUSTSEC-2026-0459
[RUSTSEC-2026-0486]: https://rustsec.org/advisories/RUSTSEC-2026-0486

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
