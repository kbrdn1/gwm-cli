# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- A config value no longer reaches the terminal with its Unicode bidi
  overrides intact. The neutralisation added in 1.6.0 replaces control
  characters, and `char::is_control` covers C0, DEL and C1: it does not cover
  `U+202A` through `U+202E` or `U+2066` through `U+2069`. Those are `Cf`, not
  `Cc`. They reorder how a terminal renders the text around them without ever
  being a control byte, so a value carrying one can display something other
  than what it is.

  The site that matters is the pre-trust bootstrap summary, whose whole job is
  to let someone decide whether to authorise a shell command out of a repo
  they have not vetted. A summary that can be made to misrepresent the command
  it asks about is worse than no summary. `gwm config get`, `types`,
  `trust list` and the rendered diagnostics carry the same exposure at lower
  stakes, and all of them inherit the fix: it lands in the two sinks every
  echo site already goes through, the way they inherited the control-character
  rule.

  The implicit marks (`U+200E`, `U+200F`, `U+061C`) are deliberately left
  alone. They carry no override of their own and occur in legitimate
  multilingual text, so replacing them would corrupt values rather than
  protect them. This is a gap in the 1.6.0 mitigation, not a regression:
  1.5.0 and earlier neutralised nothing at all.
  ([#502](https://github.com/kbrdn1/gwm-cli/issues/502))

- A race in the test harness, not reachable from the product.
  `exec_in_dir_runs_a_relative_script_from_the_worktree` writes an executable
  and immediately runs it, and `execve` refuses a file that is open for
  writing by any process: a child forked by another test thread carries a copy
  of that write handle until it execs, so the spawn intermittently returned
  `ETXTBSY` on Linux. It is retried on that errno alone, with the reason
  written at the retry, and every other spawn error still fails on the first
  attempt.
  ([#500](https://github.com/kbrdn1/gwm-cli/issues/500))

- The same shape in `config_tests`, where the guard around `$HOME` documented
  a boundary narrower than the real one, and five tests skipped it on the
  strength of that doc.
  `expand_placeholders` resolves the home directory before it looks at a
  single token, so every call is a concurrent reader whatever the template
  says, while one test rewrites the variable with `set_var`. The guard is now
  taken by every test in that binary that can observe `$HOME`, and its doc
  states the hazard rather than a proxy for it.
  ([#503](https://github.com/kbrdn1/gwm-cli/issues/503))

## Past releases

In reverse chronological order:

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
