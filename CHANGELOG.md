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

  A pattern whose split can move is refused rather than compiled into a parser
  that reads back the wrong thing. The test is never "is there a separator" but
  "can the boundary between two placeholders land in more than one place":
  `{issue}{desc}` reads `42123-x` as `4212` + `3-x`, `{desc}{issue}` is
  ambiguous outright since `a12` is what both `a` + `12` and `a1` + `2`
  produce, and a non-empty separator guarantees nothing either:
  `{type}-{issue}9{desc}` writes `feat-42919x` from issue `42` and desc `19x`,
  and the greedy `\d+` slides right across the `9` to read issue `4291`.

  Both halves of the rule are narrower than they look, so patterns that read
  back perfectly well are not refused along the way. Adjacency is fine when the
  alphabets are disjoint (`{type}{issue}` writes `feat42`, and `[a-z]+` stops
  at the first digit while `\d+` stops at the first letter). A separator inside
  the left placeholder's charset is fine when the right one cannot supply it
  back, which is why `{desc}-{issue}` stays legal. And a multi-character
  separator only counts when the left side can eat a *repeating* prefix of it,
  which is why `{type}-{issue}9-{desc}` works where `{type}-{issue}9{desc}`
  does not. The same placeholder twice is refused too. `gwm doctor` and
  `gwm config validate` report every refusal with the fix in it.

  The parser is checked against the formatter rather than argued to mirror it:
  `compile` writes one probe branch with `expand_placeholders` and refuses the
  pattern when it cannot read that back. `{repo}` / `{home}` expansions are
  substituted again by the formatter, so a repo directory named `{type}` (or
  named `type` under a `{{repo}}` pattern) made every read-back feature go
  quiet with nothing saying why; the check closes that class rather than its
  two known instances. A `~` prefix stays out of it, because `shellexpand::tilde` is a
  divergence no parser can undo, and `gwm doctor` already names every feature
  it takes down.

  The rule is pinned by enumeration rather than by examples: a test generates
  every pattern over the three placeholders and a set of separators, decides
  independently whether each one round-trips, and requires the compiler to
  accept exactly those, so neither a silent mis-split nor an over-strict
  refusal can survive.

  A pattern that **freezes** a segment as a literal instead of writing it from
  a placeholder keeps working exactly as before. `feat/#{issue}-{desc}` and
  `{type}/#1-{desc}` were readable in 1.5.0 only because the hardcoded regex
  happened to have a group where the literal sits; the derived parser recovers
  the literal on purpose, so gitmoji, `gwm commit-prefix` and auto-linking
  still work on those repos. The recovery is an exact match rather than a
  guess. It is positional first, in that a literal is only read as a segment if it
  sits where that segment goes, before `{issue}` for a type and after it for a
  description, and then an exact match, a branch type being looked up in the
  repo's configured list (so `feature/#{issue}-{desc}` recovers nothing, since
  `feature` names a namespace), an issue number being all digits, and a
  description being whatever `DESC_RE` accepts. Position has to come first:
  `feat/#{issue}-fix` freezes both, and `feat` and `fix` are each a configured
  branch type, so an oracle asked to pick a globally unique candidate found two
  and dropped the pair. What is left over after position is decided per
  segment: a segment is recovered when every reading of the pattern names it
  with the same value, so `feat/feat/#{issue}-{desc}` freezes the type its two
  readings agree on, and `feat/#{issue}-fix/done` freezes the type while
  leaving the description its readings disagree about alone.
  The whole obligation is enumerated rather than sampled:
  1.5.0 read a branch iff it matched one hardcoded regex, so a test runs that
  regex over every pattern in the family it accepts and requires the same triple
  back. One divergence in that family is deliberate: 1.5.0's description group
  was `[a-z0-9-]+`, looser than `DESC_RE`, so `{type}/#{issue}---fix` handed
  back `--fix`, a description its own `BranchSpec::validate` rejects, which the
  rename form could not submit and `gwm create` could never have produced. The
  leading dashes are dropped, and a leading `-` is in any case what #416 banned
  from a name, since `gwm remove` and `git branch -d` read it as a flag. `{repo}` is deliberately not a source, so a repo called `docs` does
  not type its own branches. What such a pattern costs is reported separately
  and unchanged from #415: `gwm create fix 42 x` under `feat/#{issue}-{desc}`
  writes a `feat/` branch, so the type you asked for is not the one anyone
  reads back. The TUI rename form shows a frozen segment, and whether it can
  be changed depends on where the new value could go. `path_pattern` is asked
  too: when it writes the segment, editing it renames the worktree directory
  and leaves the branch alone, which is a real rename and is allowed, and the
  preview says so by showing the branch unchanged. Only when neither pattern
  writes the segment is the edit refused, since the submit would rebuild the
  same branch and the same directory. Segments `branch_pattern` writes are
  always editable, so the rename that worked on `feat/#{issue}-{desc}` before
  #417 still works.

  Both live previews expand this repo's own patterns too. They hardcoded
  `<type>/#<issue>-<desc>` and `<type>-<issue>-<desc>`, so under a custom
  pattern they promised names the repo would never create: with
  `feat/#{issue}-{desc}`, picking `docs` in the rename type selector previewed
  `docs/#42-x` while submitting wrote `feat/#42-x`. A preview that disagrees
  with what submitting does is worse than no preview at all.

  The value that form shows comes from the worktree's **directory** when
  `path_pattern` carries the segment and `branch_pattern` does not. The two
  patterns need not carry the same segments, and when they do not, neither
  name holds the whole triple: under `feat/#{issue}-{desc}` with the default
  `path_pattern`, `gwm create fix 42 x` writes the branch `feat/#42-x` and the
  directory `fix-42-x`, and `fix` exists nowhere else. Rebuilding from the
  branch alone read the type as `feat`, so renaming the description also moved
  the directory to `feat-42-…` and dropped what the worktree was created with.
  The branch still wins for every segment it writes itself, being the
  worktree's identity, and a directory renamed by hand must not rewrite it.
  ([#478](https://github.com/kbrdn1/gwm-cli/issues/478))

  `{type}` matches `[a-z]+`, not an alternation of the configured branch types,
  which is what the issue proposed. The alternation would have stopped
  recognising a branch created before a type was retired from `.gwm.toml`,
  taking `doctor`'s orphan check and `gwm commit-prefix` away from a name the
  previous release read fine. Nothing needs it either: once adjacent
  placeholders are refused, `[a-z]+` splits every pattern in the documented
  table, and the TUI rename, which does require a configured type, checks the
  resolved list itself and says so precisely.
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
