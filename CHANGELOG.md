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

- **The Working Tree listing reads as columns**
  ([#622](https://github.com/kbrdn1/gwm-cli/issues/622)). The `M` / `A` / `D`
  / `?` status letter used to lead the file name, so on a 27-file change set
  it sat at a different offset on every row and the eye had to re-find it. It
  is now a right-aligned column of its own, in both the sidebar pane and the
  full-size overlay, and it keeps the per-category colour it always had.

  The column is pinned to the right edge of its surface at every width, and
  the letter is priced far below what the `+N -M` column costs: a pane or an
  overlay too narrow to seat the counts still seats the letter. Before this
  the letter was an inline badge two cells wide that no width ever dropped,
  so charging it the counts' floor would have lost a capability rather than
  yielded a column.

  The `+N -M` line counts from #592 now ride the sidebar pane too, inside
  the letters. That pane had deliberately stopped at the rows to save the
  `git diff --numstat` they need, since it re-reads on every selection
  change; it pays for that read now, on the sidebar worker rather than the
  render path.

  In both surfaces the letter shares the right end of the row with those counts,
  and the two yield in a fixed order: the letter is carved out first, so a
  narrowing terminal drops the counts and never the letter. `+N -M` says how
  much a file changed, the letter says what happened to it, and a row that no
  longer says what it is has lost its subject rather than a detail.

  The overlay title carries the changed-file count, `Working Tree (27)`. It
  comes from the per-category counts rather than the row count, so it agrees
  with the footer: the rows also hold directories, the `… N more` overflow
  notice and the `✓ clean` sentinel, none of which is a changed file. It is
  withheld while the listing is still being read, since `(0)` there would be a
  claim rather than a count.

  Directory rows lead with a `▾` disclosure caret instead of the folder glyph
  they carried since #300. A folder glyph and a file glyph are two icons of
  the same weight, which left indentation alone to separate the two levels;
  a caret is a different shape. File rows keep their per-extension glyph.

  The collapsed leading path the issue also asks for was already there:
  `build_tree` has folded single-child directory chains into one `a/b/c` row
  since #300.

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
  tag cannot be cut from a set that documents another version. The tape reports
  the path as well as the version, and the run refuses to continue unless it is
  the file cargo just built: a stale binary carrying the same version number
  answers a version check perfectly, which is the shape the v1.10.0 near miss
  had. `tests/capture_pipeline_tests.rs` drives all of that against a throwaway
  repo with stubbed tools, and a companion guard pins the phase order rather
  than leaving it to the prose.

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
