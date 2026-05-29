# gwm-cli — house rules for AI assistants

This file is the project-level AGENTS.md. Anything stated here OVERRIDES
defaults and applies to every contribution made via an AI assistant in
this repository.

## 🔴 Primordial rule — Test-Driven Development is mandatory

**No production code lands without a failing test that pinned the
behaviour down first.** This is not a guideline, it is a hard merge
requirement. PRs that add or change behaviour without tests are sent
back, full stop.

### The TDD loop (red → green → refactor)

1. **Red** — write a failing test that captures the new behaviour or
   the bug you are fixing. Run it. It MUST fail for the right reason
   (assertion mismatch, not a compile error in unrelated code). Commit
   the test alone if it helps reviewers see the contract.
2. **Green** — write the minimum production code to make the test pass.
   No extra branches, no speculative abstractions.
3. **Refactor** — clean up while the tests are green. Re-run the full
   suite after every refactor step.

### What counts as "behaviour"

Anything observable from outside the function under test:

- A new CLI subcommand, flag, or output format → end-to-end test in
  `tests/cli_binary.rs` via `assert_cmd`.
- A new public function in `src/<module>.rs` → unit test in
  `tests/<module>_tests.rs`.
- A new bootstrap step (file copy, guard, no-symlink, command hook) →
  integration test in `tests/bootstrap_tests.rs` exercising it against
  a `tempfile::TempDir`.
- A libgit2 worktree operation → integration test in
  `tests/worktree_integration.rs` using `tests/common::init_repo()`.
- A TUI state transition → state-machine test in
  `tests/tui_app_tests.rs` (ratatui-free).

### Exceptions (narrow, must be argued in the PR description)

The bar to skip a test is "the change is observably untestable from
the public surface". Concretely:

- **Pure formatting / typo fixes** in user-facing strings → no test
  required if the string is incidental (a log line, a help blurb). If
  the string is asserted somewhere, update the assertion.
- **Dependency bumps** without behaviour change → CI green is the test.
- **Comments-only changes** → no test required.

Everything else needs a test. "I tested it manually" is not an
exception; codify the manual test as an integration test.

### Enforcement

- PR template ships with a `cargo test` checkbox under **Tests**. Do
  not tick it unless the suite actually ran green locally.
- Reviewers will run `git log --stat <branch>..HEAD -- tests/` and
  block the PR if the touched module has no companion test diff.
- `tests/cli_binary.rs::help_prints_subcommands` should be updated
  every time a new subcommand is added — treat this as the canary.

## Other house rules

- **Reconcile open PRs before any tag.** Before cutting an RC or a
  stable, run `gh pr list --state open` and account for every open
  PR: either it's in the changeset, intentionally deferred, or
  closed as stale. The v0.3.0 stable shipped without three queued
  feature PRs (#51, #52, #53) because this check was skipped —
  forced an immediate v0.4.0 promotion 38 minutes later. Two
  minutes upfront beats a rushed follow-up release.
- **Release notes are per-version, never the index.** The release
  workflows (`release.yml` / `pre-release.yml`) source their
  `body_path` from `changelogs/<version>.md` (stable) or
  `changelogs/pre-releases/<version>.md` (rc/alpha/beta), NOT from
  the top-level `CHANGELOG.md` (which is the in-progress index —
  entries get moved into the per-version file when the release is
  cut, so the index is empty at tag time). Before tagging, verify
  the per-version file exists and contains the release contents;
  the workflow now hard-fails if the file is missing rather than
  silently publishing the empty index (witnessed on v0.6.0 /
  v0.6.0-rc.1 — both releases had to be re-edited post-hoc via
  `gh release edit --notes-file`).
- **Do not stack deep PR chains.** After the v0.7.0 hardening run,
  the cost of rebasing stacked PRs was higher than the cost of waiting
  for review. For decompositions touching related surfaces, keep at
  most 2-3 PRs open, merge each PR as soon as Copilot + CI are green,
  wait for `dev` to settle, then branch the next one.
- **Batch low-risk encapsulation nits after a stack merges.** If a
  Copilot review asks for private fields, accessors, or re-export
  cleanup while several dependent PRs are queued, prefer filing or
  applying a small polish PR after the stack lands. Do not force a
  cascade of mechanical rebases for non-behavioural cleanup.
- **Parallel agents only when file ownership is disjoint.** Sub-agents
  work well for independent surfaces. If multiple tasks all touch
  shared files such as `src/tui/app.rs`, `src/tui/state/*`, or config
  plumbing, dispatch them sequentially instead of creating avoidable
  merge conflicts.
- **Follow-up issues beat scope creep.** If review uncovers a design
  bug whose fix changes the shape of the implementation, file a
  focused follow-up issue rather than hiding it in the current PR.
  Keep the original PR atomic unless the bug invalidates its contract.
- **Verify MSRV against the whole codebase, not the feature you just
  added.** Before declaring or changing MSRV, run `cargo clippy
  --all-targets -- -W clippy::incompatible_msrv` locally; prefer
  `cargo msrv verify` when available. The v0.7.0 cycle caught an
  existing `std::iter::repeat_n` usage after a separate `LazyLock`
  MSRV discussion, so grep for newer APIs before assuming the latest
  edit is the limiting factor.
- **Keep root `CHANGELOG.md` as in-progress only.** PRs may add entries
  under `[Unreleased]`, but must not reintroduce bullets already moved
  into the latest `changelogs/pre-releases/<previous-rc>.md`. The guard
  from #147 now ships as `.github/scripts/check-rc-changelog-dupes.sh`
  and runs in CI on every pre-release tag (`pre-release.yml`). Run it
  locally before cutting an RC — `./.github/scripts/check-rc-changelog-dupes.sh <tag>`
  (e.g. `v0.8.0-rc.4`) — so a duplicated bullet is caught before the tag,
  not by a red CI job after it.
- **Release workflow edits must prove publishing credentials.** The
  v0.7.0 stable tag built all five release artifacts, then the GitHub
  Release publish step failed with `Bad credentials` and required
  manual recovery. Any PR touching `release.yml`, token permissions, or
  release actions should explain how the publish path was validated and
  should keep #146 in view.
- **Pre-validate environment-dependent tests.** Any test that reads
  `$PATH`, the user's home directory, or other ambient state must be
  pre-validated locally against a stripped environment before the test
  gets pushed — CI runners don't have `lazygit`, a pre-created
  `~/cc-worktree/`, or your installed dev tooling. The one-liner that
  reproduces a CI-like minimal PATH:

  ```bash
  PATH="$(dirname "$(command -v cargo)"):/usr/bin:/bin" cargo test
  ```

  Run it before push. The cost is one minute; the cost of skipping
  it is at least two CI round-trips (witnessed on PR #43 — three
  fix commits before the suite went green). If a test can't be made
  env-independent, assert intent (sigils, names) instead of exit
  codes; cover the deterministic 0/1/2 contract in a separate hand-
  built unit test.
- **Run `gwm doctor` locally** before opening a PR that touches
  `.gwm.toml`, the bootstrap schema, or the doctor module itself. The
  same check runs in CI as an advisory job — green there means you'll
  not surprise a reviewer.
- **Indentation**: 2 spaces. `cargo fmt` is run on every commit; CI
  enforces `cargo fmt --check`.
- **Linter**: `cargo clippy --all-targets -- -D warnings` must pass.
  Do not `#[allow(...)]` warnings without a comment explaining why.
- **No `unwrap()` on user-facing paths**: return a `GwmError` variant
  instead. `unwrap()` is acceptable inside tests and in genuinely
  infallible spots (e.g. `.lock()` on a never-poisoned mutex), but it
  must be a deliberate choice, not a shortcut.
- **No `println!` in TUI render code**: the status bar is the only
  channel for runtime feedback inside the TUI.
- **Branch convention**: `<type>/#<issue>-<description>`. Use
  `gwm create <type> <issue> <description>` — it bootstraps the
  worktree and creates the branch in one go.
- **Commit format**: Gitmoji + Conventional Commits. See
  [CONTRIBUTING.md](CONTRIBUTING.md#commits).
- **Merge strategy**: regular merge commit, never squash, never delete
  the source branch. The atomic commit history is the artefact.

## Where to look for the rest

- Branch / commit / PR conventions → [CONTRIBUTING.md](CONTRIBUTING.md)
- Community standards → [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Public roadmap → [ROADMAP.md](ROADMAP.md)
- Release process → [CONTRIBUTING.md §Releases](CONTRIBUTING.md#releases)
