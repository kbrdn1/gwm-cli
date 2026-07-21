# Contributing to gwm

Thanks for your interest in `gwm` — a Rust CLI / TUI for managing git worktrees across projects. This file describes the conventions used here. They mirror the ones used in [`fiches-pedagogiques-api-rest`](https://github.com/FlippadTeam/fiches-pedagogiques-api-rest/blob/dev/CONTRIBUTING.md) so the muscle memory is the same.

## Table of contents

- [About this repository](#about-this-repository)
- [Project layout](#project-layout)
- [Development](#development)
- [Testing](#testing)
- [Branches](#branches)
- [Commits](#commits)
- [Labels](#labels)
- [Pull Requests](#pull-requests)
- [Merge strategy](#merge-strategy)
- [Branch protection](#branch-protection)
- [Releases](#releases)

## About this repository

`gwm` is a single-binary Rust crate (`bin` + reusable `lib`):

- **bin** `gwm` — entry point: dispatches to subcommands (CLI) or opens the TUI.
- **lib** `gwm` — modules (`aliases`, `bootstrap`, `clean`, `cli`, `command_log`, `config`, `config_cli`, `daemon`, `doctor`, `error`, `exec`, `github`, `gitmoji`, `history`, `hooks`, `issue_templates`, `json_api`, `labels`, `launcher`, `lifecycle`, `milestones`, `multiplexer`, `naming`, `pr_templates`, `presets`, `review`, `statusline`, `sync`, `templating`, `trust`, `tui`, `workspace`, `worktree`) exposed publicly so integration tests in `tests/` can drive them directly.

It uses [`git2`](https://docs.rs/git2) (vendored libgit2) for worktree operations and [`ratatui`](https://docs.rs/ratatui) for the TUI.

## Project layout

```
gwm-cli/
├── Cargo.toml
├── CHANGELOG.md
├── CONTRIBUTING.md
├── LICENSE.md
├── README.md
├── docs/                 # full documentation tree (README delegates here)
├── examples/
│   ├── gwm.toml.example
│   └── presets/          # embedded `gwm init --preset` bodies
├── src/
│   ├── lib.rs            # public re-exports
│   ├── main.rs           # bin entry point
│   ├── error.rs
│   ├── config.rs         # .gwm.toml parsing
│   ├── config_cli.rs     # `gwm config get/set/...` plumbing
│   ├── naming.rs         # branch / path conventions
│   ├── worktree.rs       # libgit2 worktree ops
│   ├── bootstrap.rs      # copies / guards / shell hooks
│   ├── lifecycle.rs      # [hooks.*] lifecycle phases
│   ├── hooks.rs          # git hook install (commit-msg auto-prefix)
│   ├── trust.rs          # TOFU ledger gating bootstrap (issue #95)
│   ├── doctor.rs         # 8 health checks for `gwm doctor`
│   ├── github.rs         # gh shell-out + issue / PR linking
│   ├── labels.rs         # declarative GitHub label set (issue #81)
│   ├── milestones.rs     # declarative GitHub milestone set (issue #82)
│   ├── launcher.rs       # `l` / `r` TUI launcher resolution (issue #75)
│   ├── multiplexer.rs    # tmux / zellij window+split helpers
│   ├── presets.rs        # `gwm init --preset` stack registry (issue #37)
│   ├── review.rs         # `gwm review <PR#>` (issue #308)
│   ├── sync.rs           # `gwm sync` fetch + rebase / merge
│   ├── exec.rs           # `gwm exec ... -- <cmd>` fleet runner (issue #313)
│   ├── clean.rs          # `gwm clean` artifact reclaim (issue #313)
│   ├── workspace.rs      # multi-repo workspace mode (issue #36)
│   ├── daemon.rs         # JSON-RPC unix-socket daemon (issue #38)
│   ├── json_api.rs       # `--format=json` rendering
│   ├── statusline.rs     # `gwm statusline` daemon consumer (issue #309)
│   ├── history.rs        # destructive-op journal + `gwm undo`
│   ├── command_log.rs    # TUI command log buffer
│   ├── aliases.rs        # [aliases] argv expansion
│   ├── gitmoji.rs        # branch-type → :shortcode: map
│   ├── templating.rs     # placeholder substitution
│   ├── issue_templates.rs# `gwm new` issue templating
│   ├── pr_templates.rs   # `gwm pr` body rendering
│   ├── cli.rs            # clap subcommands
│   └── tui/
│       ├── mod.rs        # event loop
│       ├── app.rs        # state
│       ├── ui.rs         # rendering
│       ├── keymap.rs     # global keymap (rebindable)
│       ├── modal_keymap.rs # per-context modal keys
│       ├── palette.rs    # command palette
│       ├── theme.rs      # theme presets / roles
│       ├── wt_tree.rs    # working-tree file tree
│       ├── commit_graph.rs # recent-commits view
│       └── state/        # per-view state machines
│           ├── async_task.rs
│           ├── command_logs.rs
│           ├── config_panel.rs
│           ├── confirm.rs
│           ├── create_form.rs
│           ├── filter.rs
│           ├── github_fetch.rs
│           ├── link_prompt.rs
│           ├── pty_overlay.rs
│           ├── sidebar.rs
│           └── spinner.rs
└── tests/                # one `*_tests.rs` / `*_integration.rs` per module
    ├── common/           # shared helpers (init_repo, paths_equal)
    ├── config_tests.rs
    ├── naming_tests.rs
    ├── bootstrap_tests.rs
    ├── trust_tests.rs    # ledger load/save/lookup/record/revoke (issue #95)
    ├── worktree_integration.rs
    ├── tui_app_tests.rs
    └── cli_binary.rs     # assert_cmd end-to-end
```

All tests live under `tests/` — no inline `#[cfg(test)] mod tests` blocks inside `src/`.

## Development

### Prerequisites

- Rust toolchain (stable channel, 1.86+ — the MSRV declared in `Cargo.toml`, raised by `tui-term` / `portable-pty` when the PTY overlay landed).
- A C compiler (libgit2 is vendored and built from source on first `cargo build`).

### Build & run

```bash
git clone https://github.com/kbrdn1/gwm-cli.git
cd gwm-cli

cargo build              # builds bin + lib
cargo run -- list        # smoke test the CLI
cargo run                # opens the TUI in the current repo
cargo install --path .   # install gwm into ~/.cargo/bin
```

### Code style

- **Indentation**: 2 spaces (matches `fiches-pedagogiques` convention).
- **Formatter**: `cargo fmt` (project uses `rustfmt` defaults except indent).
- **Linter**: `cargo clippy -- -D warnings`.
- Run `cargo fmt && cargo clippy` before opening a PR.

### Local hooks (recommended, opt-in)

A POSIX `pre-commit` script lives under [`.githooks/`](.githooks/). It is **not installed automatically** — opt in with:

```bash
git config core.hooksPath .githooks
```

Once enabled, two gates run on every `git commit`:

1. **Env-dependent test pre-validation.** If staged `tests/*.rs` hunks reference ambient state (`assert_cmd`, `std::env::var`, `which::which`, `dirs::`, `Command::cargo_bin`), the hook re-runs the suite under a stripped PATH:

   ```bash
   PATH="$(dirname "$(command -v cargo)"):/usr/bin:/bin" cargo test
   ```

   This catches tests that pass in your rich dev shell but fail on a minimal CI runner — the lesson from PR #43 (three CI round-trips before the suite went green).

2. **Local `gwm doctor`.** If staged paths touch `.gwm.toml`, `src/bootstrap.rs`, `src/doctor.rs`, `examples/gwm.toml.example`, or `tests/{bootstrap,doctor}*`, the hook runs `gwm doctor`. Exit codes follow the doctor contract:

   | Exit | Meaning  | Commit behaviour          |
   |:-----|:---------|:--------------------------|
   | `0`  | Clean    | proceeds silently         |
   | `1`  | Warnings | proceeds with advisory    |
   | `2`  | Errors   | **blocked** until resolved |

   If `gwm` is not on `PATH`, the gate prints a skip notice and the commit proceeds — the CI `doctor` job is the safety net.

Both gates short-circuit in O(1) when no staged paths match — contributors who never touch tests or config pay nothing per commit.

**Bypass** for a single commit you know is safe:

```bash
git commit --no-verify
```

CI runs `shellcheck` against the hook and a smoke test on every PR — see the `hook-smoke` job in [`ci.yml`](.github/workflows/ci.yml) — so a broken hook is caught before it reaches you.

## Testing

```bash
cargo test                              # run everything
cargo test --test config_tests          # one file
cargo test --test worktree_integration  # libgit2 integration
cargo test -- --nocapture               # see println from tests
```

### 🔴 TDD is mandatory — non-negotiable

**Test-Driven Development is the primary contribution rule of this repo.** No production code lands without a failing test that pinned the behaviour down first. This is not a guideline, it is a hard merge requirement. PRs that add or change behaviour without tests are sent back, full stop.

The loop is **red → green → refactor**:

1. **Red** — write a failing test capturing the new behaviour (or the bug you are fixing). Run it. It MUST fail for the right reason (assertion mismatch, not a compile error in unrelated code).
2. **Green** — write the minimum production code that turns the test green. No speculative abstractions.
3. **Refactor** — clean up under green tests. Re-run the suite after each refactor step.

Where the test lives:

- **unit logic** (config parsing, naming, kebab, guard regex) → tests in the matching `tests/*_tests.rs` file.
- **disk side effects** (file copy, symlink removal, command exec) → use `tempfile::TempDir`.
- **git operations** → use `tests/common::init_repo()` which gives you a fresh repo on `main` with one commit.
- **public CLI surface** → end-to-end test in `tests/cli_binary.rs` via `assert_cmd`.
- **bootstrap stages** (copy, guard, no-symlink, command) → `tests/bootstrap_tests.rs`.
- **TUI state transitions** → ratatui-free state-machine tests in `tests/tui_app_tests.rs`.

#### Exceptions (must be argued in the PR description)

The bar to skip a test is "observably untestable from the public surface":

- Pure formatting / typo fixes in incidental strings (not asserted anywhere).
- Dependency bumps with no behaviour change (CI green is the test).
- Comment-only changes.

Everything else needs a test. "I tested it manually" is not an exception — codify it as an integration test.

#### Enforcement

- Reviewers run `git log --stat <branch>..HEAD -- tests/`. If the touched module has no companion test diff and the change isn't one of the exceptions above, the PR is blocked.
- The `## Tests` checklist in the PR template is binding. Do not tick `cargo test` unless it actually ran green locally.
- `tests/cli_binary.rs::help_prints_subcommands` is the canary — update it whenever a new CLI subcommand is added.

## Branches

Main branches:

- `main` — what ships. Direct commits allowed for trivial maintenance (typos, docs, dep bumps). Anything user-visible goes through a PR.
- Feature branches: `<type>/#<issue-number>-<short-description>`.

Examples: `feat/#12-tui-search`, `fix/#45-locked-worktree-detection`, `docs/#3-update-readme`.

`gwm` itself uses this exact convention via `gwm create feat 12 tui-search`.

## Commits

Format: `<emoji> <type>(<scope>)<!>: <subject>` (Gitmoji + Conventional Commits).

### Types

| Type       | When                                                |
|:-----------|:----------------------------------------------------|
| `feat`     | new feature                                         |
| `fix`      | bug fix                                             |
| `hotfix`   | critical production bug fix                         |
| `refactor` | code restructuring, no behaviour change             |
| `docs`     | documentation only                                  |
| `test`     | adding / fixing tests                               |
| `perf`     | performance improvement                             |
| `chore`    | repo maintenance (deps, config, scripts)            |
| `ci`       | CI / GitHub Actions changes                         |
| `build`    | build system, Cargo manifest                        |

### Emojis (Gitmoji)

| Emoji | Type       |
|:------|:-----------|
| ✨    | feat       |
| 🐛    | fix        |
| 🚑️   | hotfix     |
| 📝    | docs       |
| ♻️    | refactor   |
| ⚡    | perf       |
| ✅    | test       |
| 🔧    | chore      |
| 🏗️    | build      |
| 👷    | ci         |
| 🔥    | chore (remove) |
| ⬆️    | chore (bump deps) |
| 🔒    | security   |

### Scopes (optional, used in this repo)

`config`, `naming`, `worktree`, `bootstrap`, `cli`, `tui`, `tests`, `docs`, `ci`, `structure`.

### Examples

- `✨ feat(tui): add fuzzy search on worktree list`
- `🐛 fix(worktree): handle is_prunable error gracefully`
- `🔧 chore(deps): bump ratatui to 0.29`
- `♻️ refactor(bootstrap): extract guard-matching into pure fn`
- `✅ test(naming): cover unicode descriptions`

### Breaking changes

Suffix the type with `!` and add a `BREAKING CHANGE:` footer:

```
✨ feat(config)!: replace `[[bootstrap.copy]]` with `[[steps]]`

BREAKING CHANGE: configs using the old keys must migrate to the new schema.
```

### Signing (preferred)

Commits on a PR should show up as **`Verified`** on GitHub. GPG is preferred;
SSH signing is equally accepted (GitHub verifies both the same way).

This is a preference, not a gate: nothing in CI or branch protection enforces
it, and a PR will not be rejected for unsigned commits. It is asked for because
a signed history is worth having, not because tooling demands it.

Signing a commit and getting it **verified** are two different things. GitHub
shows `Verified` only when *both* hold:

- the **public** key is registered on your GitHub account
  (Settings → SSH and GPG keys)
- the **committer email** matches a uid on the key **and** a verified email on
  your account

The second one is what usually bites. A commit signed with a perfectly good key
whose uid does not match the committer email stays `Unverified` forever. If you
use different `user.email` values across repos, check before you push:

```bash
git config user.email                  # the committer email git will stamp
gpg --list-secret-keys --keyid-format=long   # the uid(s) on your key
```

To turn signing on for this repo only:

```bash
git config user.signingkey <KEY_ID>
git config commit.gpgsign true
# SSH instead of GPG:
git config gpg.format ssh
git config user.signingkey ~/.ssh/id_ed25519.pub
```

Verify what GitHub actually thinks, which is the only opinion that counts here
(local `git log --show-signature` can disagree with it, e.g. on a keyring it
cannot read):

```bash
gh api /repos/<owner>/<repo>/commits/<sha> \
  --jq '.commit.verification | "\(.verified) \(.reason)"'   # want: true valid
```

## Labels

See [`.github/LABELS.md`](.github/LABELS.md) for the full matrix. Quick reference:

- **type**: `feature`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`
- **status**: `duplicate`, `invalid`, `wontfix`
- **domain**: `cli`, `tui`, `config`, `worktree`, `bootstrap`, `security`, `dependencies`

## Pull Requests

Before opening a PR:

- [ ] `cargo fmt`
- [ ] `cargo clippy -- -D warnings`
- [ ] `cargo test` (all green)
- [ ] Commits show as `Verified` on GitHub (preferred, see [Signing](#signing-preferred))
- [ ] CHANGELOG.md updated under `## [Unreleased]`
- [ ] If the public CLI changed: the `docs/3.cli` section updated (the README is a landing page that delegates to `docs/`)
- [ ] If the config schema changed: `examples/gwm.toml.example` and the `docs/4.configuration` section updated

Use the PR template (`.github/PULL_REQUEST_TEMPLATE.md`).

## Merge strategy

- **Never squash**. Use a regular merge commit so the atomic commit history (with its `feat` / `fix` / `refactor` labels) is preserved on `main`.
- **Never delete the source branch** after merge. Keeps traceability and lets us cherry-pick / revert.

```bash
gh pr merge <num> --merge   # NOT --squash, NOT --delete-branch
```

## Branch protection

`main` is protected. Nothing reaches it except through a pull request with green
checks, and **that includes the maintainer**: `enforce_admins` is on, so
`git push origin main` is rejected outright and there is no admin override. The
only way to lift it is to disable the protection by hand, which should be a
deliberate, visible act rather than a reflex.

Active rules (read them with `gh api repos/kbrdn1/gwm-cli/branches/main/protection`):

| Rule | Value |
|------|-------|
| Require a pull request | yes, **0 approvals** |
| Required status checks | `rustfmt`, `clippy`, `test (ubuntu-latest)`, `test (macos-latest)`, `test (windows-latest)`, `pre-commit hook smoke`, `cargo audit` |
| Require branches up to date (`strict`) | no |
| Enforce for admins | **yes** |
| Require linear history | no |
| Force pushes / deletions | blocked |

Three of those are counter-intuitive and are set that way on purpose:

- **0 required approvals**, not 1. This is a single-maintainer repo and GitHub
  forbids approving your own pull request, so requiring one approval would be a
  permanent lockout. The status checks are the real gate; the PR is the rail
  that makes sure they run.
- **Linear history off.** Turning it on would force squash or rebase merges and
  break [Merge strategy](#merge-strategy). The atomic commit history is the
  artefact, so merge commits have to stay legal.
- **`gwm doctor (advisory)`, CodeRabbit and GitGuardian are not required.** The
  first is advisory by design; the other two are third-party and can stop
  reporting. A required check that never reports blocks the branch forever, so
  only checks we own and that always run are in the list.

`strict` is off because `main` gains a merge commit that `dev` does not have on
every release; requiring "up to date" would force a back-merge into `dev` before
each cut, for no added safety since the checks re-run on the PR anyway.

This does not affect releases mechanically: `release.yml` and `pre-release.yml`
are triggered by **tags**, and protection guards branch refs, not tags. It does
change how `dev` reaches `main` (see below), and it means a **hotfix cannot go
straight to `main` either** (see [Step 0](#step-0--reconcile-open-prs-applies-to-every-tag)):
branch off `main`, open a PR back into it, let the checks run.

## Releases

Versioning is SemVer (`MAJOR.MINOR.PATCH`), with `-rc.N` / `-alpha.N` / `-beta.N` suffixes for pre-releases cut from `dev`.

- `MAJOR` → breaking change
- `MINOR` → new feature
- `PATCH` → bug fix
- `-rc.N` / `-alpha.N` / `-beta.N` → release candidate / alpha / beta cut from `dev` before promotion to `main`

What a "breaking change" actually covers — the published 1.0 compatibility
contract (which surfaces are covered by this SemVer promise, which are free to
change in a minor/patch, the MSRV policy, and the deprecation process) — lives
in [Stability & compatibility](docs/6.development/3.stability.md).

### Step 0 — Reconcile open PRs (applies to every tag)

Before any RC or stable cut, run:

```bash
gh pr list --state open
```

Every open PR must be in exactly one of these buckets:

- **In the changeset** — merged into the source branch (`dev` for RCs / stables, `main` for hotfixes) before tagging.
- **Intentionally deferred** — won't make this release, will land in a later one. Note why in the release notes if it was a known candidate.
- **Closed as stale** — superseded, obsolete, or duplicate. Close with a one-line comment pointing at the supersession.

Skipping this step caused the v0.3.0 cut to ship without three queued feature PRs (#51, #52, #53). Recovery required an immediate v0.4.0 promotion 38 minutes later. **Two minutes upfront beats a follow-up release.**

### Pre-release (from `dev`)

When `dev` is ready to be exercised by early adopters before promotion:

1. **Step 0 first** — see above.
2. Stay on `dev` (do not merge to `main` yet).
3. Write per-RC notes in a new file `changelogs/pre-releases/<version>-rc.N.md` — heading `# [<version>-rc.N] - YYYY-MM-DD`, body describing only the **delta** against the previous RC (or against the previous stable, for `rc.1`). One file per RC, not a running log. (See [`changelogs/pre-releases/0.3.0-rc.2.md`](changelogs/pre-releases/0.3.0-rc.2.md) for the expected layout.)
4. Add the entry to `CHANGELOG.md`'s `## Past releases > ### Pre-releases` index.
5. Tag: `git tag -a v0.x.y-rc.N -m "v0.x.y-rc.N" && git push --tags`.
6. GitHub Actions (`pre-release.yml`) builds binaries and publishes a **prerelease** (5 targets — Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64). The release body is populated from the per-RC file via `--notes-file changelogs/pre-releases/<version>-rc.N.md` (run `gh release edit <tag> --notes-file <path>` after the workflow if you need to refresh it).
7. Iterate: subsequent candidates are `v0.x.y-rc.2`, `v0.x.y-rc.3`, …

### Stable release (from `main`)

Once the rc is validated and promoted to `main`:

1. **Step 0 first** — see above.
2. Update `Cargo.toml` `version`.
3. Move the `## [Unreleased]` section out of `CHANGELOG.md` into a new file `changelogs/<version>.md` (e.g. `changelogs/0.3.0.md`), rename its heading to `# [<version>] - YYYY-MM-DD`, and add a one-line entry at the bottom of `CHANGELOG.md`'s `## Past releases` index pointing to the new file. `CHANGELOG.md` at the root then only carries the next `## [Unreleased]` section. (See [`changelogs/0.2.0.md`](changelogs/0.2.0.md) for the expected layout.)
4. Open a PR from `dev` to `main`, wait for the required checks, then merge it with a **merge commit** (never squash; see [Merge strategy](#merge-strategy)). `main` is [protected](#branch-protection): a local `git push origin main` is rejected, including for the maintainer, so there is no direct-merge path.

   ```bash
   gh pr create --base main --head dev --title "Release v0.x.y" --body "…"
   gh pr merge <num> --merge   # once the 7 checks are green
   ```

5. Tag the merge commit on `main`: `git checkout main && git pull && git tag -a v0.x.y -m "v0.x.y" && git push --tags`. Tags are not covered by the branch protection, so this push goes through as-is.
6. GitHub Actions (`release.yml`) builds binaries and publishes the stable release. The release body is populated from `changelogs/<version>.md` via `--notes-file` (run `gh release edit v0.x.y --notes-file changelogs/<version>.md` after the workflow if needed).

> ⚠️ **Finalise the crate identity _before_ the tag.** Any change to the
> crates.io package identity — the `[package] name`, or a `version` bump — must
> land in the **same commit the tag points at**, so `cargo publish` from that
> tag is reproducible. The `v1.0.0` tag carried `name = "gwm"`; the rename to
> `gwm-cli` (the name `gwm` was already taken on crates.io) landed **two commits
> later**, so the published `gwm-cli@1.0.0` is _not_ reachable by checking out
> `v1.0.0`. If a rename or identity change is ever needed again, do it in step 2
> (alongside the `version` bump), before the merge + tag — not after.

Triggering matrix:

| Tag pattern              | Workflow         | `prerelease` flag |
|:-------------------------|:-----------------|:------------------|
| `v0.x.y`                 | `release.yml`    | `false`           |
| `v0.x.y-rc.N`            | `pre-release.yml`| `true`            |
| `v0.x.y-alpha.N`         | `pre-release.yml`| `true`            |
| `v0.x.y-beta.N`          | `pre-release.yml`| `true`            |

### Homebrew tap (`brew install kbrdn1/tap/gwm`)

Stable releases automatically refresh [`kbrdn1/homebrew-tap`](https://github.com/kbrdn1/homebrew-tap) (`Formula/gwm.rb`) via the `homebrew-tap-update` job in [`release.yml`](.github/workflows/release.yml). Pre-releases (`-rc.N` / `-alpha.N` / `-beta.N`) are filtered out so `brew install gwm` always tracks the latest stable.

The canonical formula source lives at [`packaging/homebrew/gwm.rb.template`](packaging/homebrew/gwm.rb.template). Edits to the template (new shell completion call, license bump, extra `test do` block) flow to the tap on the next stable release — no manual sync needed.

#### One-time bootstrap (maintainer)

The job needs a fine-grained personal access token (PAT) with `contents: write` scoped to the tap repo. Create it once:

1. Generate a PAT at <https://github.com/settings/personal-access-tokens/new>:
   - **Resource owner**: your user (or the org owning `homebrew-tap`).
   - **Repository access**: select `kbrdn1/homebrew-tap` only.
   - **Permissions**: Contents → **Read and write**. Nothing else.
   - **Expiration**: ≥ 1 year (set a calendar reminder to rotate).
2. Add it as a secret on the `gwm-cli` repo:
   - <https://github.com/kbrdn1/gwm-cli/settings/secrets/actions/new>
   - Name: `HOMEBREW_TAP_TOKEN`. Value: the PAT.
3. Flip `continue-on-error: true` to `false` on the `homebrew-tap-update` job in [`release.yml`](.github/workflows/release.yml) after the first successful sync — failures should then block the workflow loudly.

#### Re-running after a failed sync

If the job failed (typically: PAT missing or expired) after the GitHub release already shipped, re-drive the tap refresh without re-tagging:

```bash
gh workflow run release.yml --ref <tag>   # e.g. v0.5.0
```

The `workflow_dispatch` path is gated to the same stable-only condition; rc/alpha/beta will skip the tap step automatically.

### Scoop bucket (`scoop install gwm`)

Stable releases automatically refresh [`kbrdn1/scoop-gwm`](https://github.com/kbrdn1/scoop-gwm) (`bucket/gwm.json`) via the `scoop-bucket-update` job in [`release.yml`](.github/workflows/release.yml), mirroring the Homebrew tap. Pre-releases are filtered out so `scoop install gwm` always tracks the latest stable. End users add the bucket once:

```powershell
scoop bucket add gwm https://github.com/kbrdn1/scoop-gwm
scoop install gwm
```

The canonical manifest source lives at [`packaging/scoop/gwm.json.template`](packaging/scoop/gwm.json.template); the render + Scoop-autoupdate contract is pinned by [`tests/scoop_manifest_tests.rs`](tests/scoop_manifest_tests.rs). Only the `__FOO__` placeholders are substituted at release time — the Scoop `$version` / `$url` autoupdate variables are left verbatim so Scoop's maintainer-side `checkver`/excavator tooling can regenerate the manifest. End users get new versions from `scoop update gwm` once the `scoop-bucket-update` job pushes the refreshed `bucket/gwm.json`, so keep the job green (that is what the client actually pulls).

#### One-time bootstrap (maintainer)

Same shape as the Homebrew tap:

1. Create the `kbrdn1/scoop-gwm` repo (a `bucket/gwm.json` + README).
2. Generate a fine-grained PAT scoped to `kbrdn1/scoop-gwm` only, **Contents → Read and write**.
3. Add it as the `SCOOP_BUCKET_TOKEN` secret on `gwm-cli`: <https://github.com/kbrdn1/gwm-cli/settings/secrets/actions/new>.
4. Flip `continue-on-error: true` to `false` on the `scoop-bucket-update` job after the first successful sync.

Re-drive a failed sync the same way: `gh workflow run release.yml --ref <tag>`.

### AUR (`yay -S gwm-cli-bin`)

**This channel is manual, and the package is not ours.** [`gwm-cli-bin`](https://aur.archlinux.org/packages/gwm-cli-bin) was submitted to the AUR on 2026-07-16 by a third-party packager, so we have no push rights on it. There is no `aur-publish` job in [`release.yml`](.github/workflows/release.yml): one existed briefly, but a job that cannot push is a job that fails silently on every tag, which is worse than no job at all (#430). AUR joins Nixpkgs and aqua as a channel we feed by hand.

End users install with any AUR helper:

```bash
yay -S gwm-cli-bin   # or: paru -S gwm-cli-bin
```

`gwm-cli-bin` is a prebuilt-binary package (downloads the linux-gnu tarball, verifies its `sha256`, installs the binary + license + bash/zsh/fish completions).

#### Refreshing the package after a stable release

Render the PKGBUILD from the release's checksums and hand it over:

```bash
TAG=v1.2.0   # the stable tag you just pushed

mkdir -p sha aur
gh release download "$TAG" --pattern 'gwm-*-unknown-linux-gnu.tar.gz.sha256' --dir sha
sh .github/scripts/render-aur-pkgbuild.sh \
  "${TAG#v}" \
  "$(awk '{print $1}' "sha/gwm-${TAG}-x86_64-unknown-linux-gnu.tar.gz.sha256")" \
  "$(awk '{print $1}' "sha/gwm-${TAG}-aarch64-unknown-linux-gnu.tar.gz.sha256")" \
  packaging/aur/PKGBUILD.template \
  > aur/PKGBUILD
```

The script writes to stdout, hence the redirect; `aur/` is scratch space, not tracked. The render contract is pinned by [`tests/aur_pkgbuild_tests.rs`](tests/aur_pkgbuild_tests.rs), so the output is trustworthy even though nothing in CI consumes it any more. Lint `aur/PKGBUILD` locally before sending (`makepkg` + `namcap` in an `archlinux` container). The `x86_64→$CARCH` `namcap` warning on the arch-suffixed `source_*` arrays is a known false positive (`$CARCH` is illegal in an array *name*).

If co-maintenance of `gwm-cli-bin` is ever granted, the job can come back: the template, the render script and its tests all survived the removal intact. That conversation is tracked in #430.

### winget (`winget install kbrdn1.gwm`)

Stable releases automatically open a manifest PR to [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs) via the `winget-publish` job in [`release.yml`](.github/workflows/release.yml). It runs [`komac`](https://github.com/russellbanks/Komac) directly — from a **pinned, digest-anchored** release binary — to build the manifest for the new version from the release's Windows `.zip` (`InstallerType: zip`, `NestedInstallerType: portable`) and push a PR from your `winget-pkgs` fork. The tag's `v` prefix is stripped to match the winget `PackageVersion`. Pre-releases are filtered out. The release-wiring contract is pinned by [`tests/winget_release_tests.rs`](tests/winget_release_tests.rs).

> **Why not the `winget-releaser` action?** It pulls `cargo-bins/cargo-binstall@main` and installs the latest `komac` at runtime — both mutable refs that would run with `WINGET_TOKEN` in scope, so a SHA pin on the action alone wouldn't protect the token. Pinning the one tool that touches the secret is the rule for every third-party binary the release workflow trusts with a credential.
>
> The expected komac digest (`KOMAC_SHA256`) is stored **in this repo**, not fetched from the same upstream release as the binary — a release whose artifacts *and* its `SHA256SUMS` were both swapped would otherwise still verify. To upgrade komac, bump `KOMAC_VERSION` and `KOMAC_SHA256` together and re-derive the digest yourself (`shasum -a 256 komac-<ver>-x86_64-unknown-linux-gnu.tar.gz`).

**komac only updates an existing package** — the job runs `komac update`, not `komac new`. The **first** `kbrdn1.gwm` manifest is submitted manually (`komac new` / `komac submit`); the job takes over from the next version onward. Every submission then goes through Microsoft's moderated validation (schema + a Windows sandbox install), which is external to this repo.

#### One-time bootstrap (maintainer)

1. Fork [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs) under your account (komac pushes its branch there): `gh repo fork microsoft/winget-pkgs --clone=false`.
2. Submit the **initial** `kbrdn1.gwm` manifest manually and get it merged.
3. Create a **classic** PAT with the `public_repo` scope — komac's fork + cross-repo-PR flow needs a classic token; new fine-grained PATs don't work here. Add it as the `WINGET_TOKEN` secret on `gwm-cli`: <https://github.com/kbrdn1/gwm-cli/settings/secrets/actions/new>.
4. Flip `continue-on-error: true` to `false` on the `winget-publish` job after the first successful automated submission.

Re-drive a failed submission the same way: `gh workflow run release.yml --ref <tag>`.

---

By contributing, you agree your changes are licensed under the MIT License (see `LICENSE.md`).
