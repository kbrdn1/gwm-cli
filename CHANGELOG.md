# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Removed

- The `winget-publish` job. `WINGET_TOKEN` was never provisioned, so the
  guard step painted a red "publish kbrdn1.gwm to winget" job on every
  stable release run, and the channel is blocked upstream anyway: the
  initial manifest PR (microsoft/winget-pkgs#403295) sits on Needs-CLA and
  `komac update` can only update a package that already exists. winget
  joins the AUR, Nixpkgs and aqua as a channel fed by hand; the manual
  `komac update` recipe lives in CONTRIBUTING, and the job's absence is
  pinned by a test. (#448)

## Past releases

In reverse chronological order:

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
