# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- ⬆️ Bumped `libz-sys` `1.1.31` → `1.1.33` to clear
  [RUSTSEC-2026-0544](https://rustsec.org/advisories/RUSTSEC-2026-0544)
  (heap buffer overflow), which was failing the `cargo audit --deny warnings`
  CI gate. `libz-sys` is a runtime dependency, direct and through `git2`
  ([#736](https://github.com/kbrdn1/gwm-cli/pull/736),
  [#741](https://github.com/kbrdn1/gwm-cli/issues/741)).
- 🔒 Accepted [RUSTSEC-2026-0543](https://rustsec.org/advisories/RUSTSEC-2026-0543)
  (`chrono` unmaintained, no fixed version) in `.cargo/audit.toml`, with its
  reason: a warning rather than a vulnerability, and leaving `chrono` is a
  migration of its own. To be revisited at the next minor release
  ([#741](https://github.com/kbrdn1/gwm-cli/issues/741)).

### Fixed

- 🐛 **Bare repos with linked worktrees work**: the TUI, `gwm doctor`,
  `gwm config` and the other commands that read `.gwm.toml` no longer fail
  with `not inside a git repository` in a bare repo (such as `project/.bare`
  with a `project/.git` file pointing to it), neither from the project
  directory nor from inside a worktree. The worktree that has the default
  branch checked out takes the place of the main checkout: its `.gwm.toml` is
  read and `{repo_path}` points to it, while `{repo}` stays the project's
  name. `discover_repo` now finds the main repo of a worktree through git's
  common dir instead of counting parent directories, which also fixes
  worktrees of a plain `project.git`. `gwm hooks install` puts the hook in
  the bare repo's own `hooks/`, where git loads it for every worktree
  ([#744](https://github.com/kbrdn1/gwm-cli/issues/744)).

### Docs

- The doc captures are regenerated at 1.11.1, so the TUI captures paint the
  version the manifest declares and the `docs_assets_tests` guard on
  `captured-version.txt` passes again. `github-linking.png` is the exception:
  `generate.sh` skips it while the trunk checkout is dirty, so it still shows
  1.11.0 until the next release run
  ([#741](https://github.com/kbrdn1/gwm-cli/issues/741)).
- **The logo sits on a dithered tile**: a rounded square whose ground is a
  ramp dithered in 4 tones, `#505050` to `#262626` in the dark logo and
  `#f5f5f5` to `#c4c4c4` in the light one, the mark scaled to 0.75 on it.
  The README heading and the banner pick it up from the same files; the
  banner is redrawn with it and moves nowhere else
  ([#739](https://github.com/kbrdn1/gwm-cli/issues/739)).


## Past releases

In reverse chronological order:

- [`1.11.1`](changelogs/1.11.1.md), 2026-11-12
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
