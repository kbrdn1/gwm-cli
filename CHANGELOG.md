# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The doc captures regenerate as one step, in the order the traps require**
  ([#631](https://github.com/kbrdn1/gwm-cli/issues/631)). Regenerating the set
  was a four-step sequence held together by a maintainer's notes: bump,
  `cargo install`, `generate.sh` for 22 of the 24 tapes, then `demo.tape` and
  `github-linking.tape` by hand. Every step had an ordering constraint that was
  invisible until it bit, and none of it was visible to CI: the files exist,
  the widths pass, `vhs` exits 0.

  `docs/_capture/generate.sh` now covers the whole set and owns the order.
  It builds the `gwm` it drives from the tree being captured and puts it first
  on `PATH`, instead of documenting whichever build a shell happens to resolve;
  v1.10.0 came within a commit of publishing captures 175 commits stale that
  way, correctly sized and green. `github-linking.tape` runs first, before
  anything else writes under `docs/`, and only when the repo's main checkout is
  clean and its branch has an open PR, so the release commit cannot leak into
  the Working Tree pane it photographs. The main checkout, not the current
  directory: the pane follows the selected row, which is row 1 wherever the
  tape ran, and the tape now takes that path from the script rather than
  hardcoding one. `demo.tape` runs last, because it is the one tape that
  changes the fixture for the others. Both were previously outside the loop and
  unreported, so a run finished on a tick over two assets it had left stale.

### Added

- **A guard on the version the captures advertise**
  ([#631](https://github.com/kbrdn1/gwm-cli/issues/631)). 17 of the 24 tapes
  open the TUI, whose header paints a `gwm X.Y.Z` chip, so a set regenerated
  before the version bump advertises the previous release for the life of this
  one. v1.8.0 shipped that way and v1.10.0 repeated it.

  Reading the chip back out of the pixels would need OCR. `version-stamp.tape`
  instead asks `gwm --version` **through vhs**, from the same shell and the
  same `PATH` every other tape resolves `gwm` through, `generate.sh` aborts the
  run when the answer is not the version in `Cargo.toml`, and commits it to
  `docs/_capture/captured-version.txt` once the run completes. The new
  `docs_assets_tests::captures_were_generated_at_the_manifest_version` compares
  the two, so the release PR goes red rather than the docs going stale, and the
  tag cannot be cut from a set that documents another version. What it proves
  is that the build above won the `PATH`, not that the binary was fresh: two
  builds of the same version are the same answer to it, and a companion guard
  pins the phase order rather than leaving it to the prose.

### Docs

- CONTRIBUTING.md § Releases carries the capture step and the three ordering
  constraints behind it, and `docs/_capture/README.md` no longer describes two
  tapes as living outside the script
  ([#631](https://github.com/kbrdn1/gwm-cli/issues/631)).


## Past releases

In reverse chronological order:

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
