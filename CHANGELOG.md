# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **CLI aliases (`[aliases]` in `.gwm.toml` + user-level fallback)**
  ([#86](https://github.com/kbrdn1/gwm-cli/issues/86)). `git config`
  ships with `[alias]`; `gwm` now mirrors the shape. Declare an
  alias under `[aliases]` in `.gwm.toml` (repo-level, follows the
  repo across machines) or in `~/.config/gwm/aliases.toml`
  (user-level fallback). `gwm <alias>` is expanded to argv tokens
  BEFORE clap parses, so `wip = "create feat 0 wip"` makes `gwm
  wip` behave as `gwm create feat 0 wip`. Resolution order:
  built-in subcommands always win (`gwm list` can never be
  shadowed), then repo, then user. Shell pipelines (`&&`, `|`,
  `;`, backticks) in alias values are refused at load time — use a
  shell alias for shell semantics. New `gwm aliases list`
  subcommand prints the resolved chain grouped by source, with
  shadowed user entries flagged inline.
- **Gitmoji mapping + `gwm commit-prefix` + opt-in `commit-msg` hook**
  (issue #85). The repo's Gitmoji + Conventional Commits convention is
  now first-class. Three new surfaces:
  - `gwm commit-prefix [--branch <name>] [--unicode]` prints the
    canonical commit prefix (e.g. `:sparkles: feat(#41):` or
    `✨ feat(#41):`) — handy for shell prompts, AI assistants, and
    scripted commit composition.
  - `gwm types --gitmoji` extends the existing branch-type list with
    two more columns (unicode emoji + `:shortcode:`).
  - `gwm hooks install commit-msg [--force]` installs an opt-in
    `.git/hooks/commit-msg` that auto-prepends the resolved prefix
    when the user's commit message doesn't already start with one.
    Non-destructive by default (refuses to overwrite a pre-existing
    hook without `--force`).
- New `[gitmoji]` block in `.gwm.toml` lets teams override individual
  shortcodes without redeclaring the whole table (the ten built-in
  defaults are baked into the binary). Custom branch types are
  supported — `migration = ":truck:"` round-trips through `gwm types
  --gitmoji`.
- Under `--unicode`, `gwm commit-prefix` and the unicode column of
  `gwm types --gitmoji` now normalise known `:shortcode:` overrides
  (e.g. `feat = ":rocket:"` → `🚀 feat(#1):` instead of
  `:rocket: feat(#1):`). The known-shortcode set extends to the most
  commonly-swapped Gitmoji entries (`:rocket:`, `:fire:`, `:lock:`,
  `:art:`, `:lipstick:`, `:hammer:`, `:bookmark:`, …). Unknown
  shortcodes fall through verbatim — no panic, no substitution.
  (#85)

### Fixed

- Pre-release publishing now fails before upload when root `[Unreleased]` repeats bullets or issue references already shipped in the immediately previous RC notes. (#147)
- Stable GitHub Releases now publish through the GitHub CLI with the workflow token and clobberable asset uploads, avoiding the `softprops/action-gh-release` bad-credentials failure seen on v0.7.0. (#146)
- CI now runs the main cargo build/test matrix on `windows-latest`, exercising Windows-only path validation coverage. (#112)

## Past releases

In reverse chronological order:

- [`0.7.0`](changelogs/0.7.0.md) — 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

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
