# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- TUI confirm-overlay safety countdown ([#30](https://github.com/kbrdn1/gwm-cli/issues/30)). When `delete branch on remove` (`p`) is armed, the `d → y` flow now requires a configurable countdown (default 3s, visualised by a 10-cell progress bar) before the destructive action fires. `Esc` / `n` cancels at any time; pressing `y` again during the countdown disarms it. The countdown duration is exposed via a new `[tui].confirm_countdown_secs` knob in `.gwm.toml` (range `0..=5`, default `3`); `0` falls back to the classic single-keystroke confirm even when `p` is armed; values above `5` are clamped on read so a typo like `confirm_countdown_secs = 30` can never strand the destructive path behind a 30-second wait. The classic flow (`p` OFF) is unchanged — countdown only adds friction to the high-risk path the user has explicitly armed. State machine pinned by `tests/tui_app_tests.rs::countdown_*` / `tick_*` / `confirm_press_y_*` / `confirm_dismiss_*` (12 tests, `Instant` injected so the suite never sleeps); config parsing / clamping by `tests/config_tests.rs::tui_*` (5 tests).
- `gwm tmux <pattern>` + `gwm zellij <pattern>` ([#23](https://github.com/kbrdn1/gwm-cli/issues/23)) — open the matched worktree in a new tmux window (`tmux new-window -n <name> -c <path>`) or a new zellij tab (`zellij action new-tab --name <name> --cwd <path>`). Both verbs accept `-p` / `--split` to spawn into a split of the current pane (`tmux split-window` / `zellij action new-pane`) instead. Detection is gated on `$TMUX` / `$ZELLIJ` — outside a multiplexer session, the command refuses with a clear error rather than spawning a stray server. Zellij `--cwd` on `new-tab` needs ≥ 0.40. New `src/multiplexer.rs` keeps the argv builders pure so `tests/multiplexer_tests.rs` can pin the incantations without requiring tmux or zellij on the test runner.
- `.githooks/pre-commit` — opt-in POSIX shell hook that gates commits on two checks contributors used to enforce by hand (#56). Gate 1 re-runs the test suite under a stripped PATH when staged `tests/*.rs` hunks touch ambient state (`assert_cmd`, `std::env::var`, `which::which`, `dirs::`, `Command::cargo_bin`) — catches CI-only failures locally. Gate 2 runs `gwm doctor` when staged paths touch `.gwm.toml`, `src/{bootstrap,doctor}.rs`, `examples/gwm.toml.example`, or `tests/{bootstrap,doctor}*`, blocking on doctor exit 2 and advising on exit 1. Install with `git config core.hooksPath .githooks`; bypass with `git commit --no-verify`.
- `[doctor].trunks` in `.gwm.toml` ([#59](https://github.com/kbrdn1/gwm-cli/issues/59)) — `gwm doctor`'s orphan-branch check now reads its trunk list from a new `[doctor]` table, default `["dev", "main"]`. Repos with non-standard trunk conventions (`master`, release trains like `release-3.x` / `release-4.x`) can opt in via config instead of seeing every merged gwm-style branch flagged as "unmerged orphan" — the silent no-op that previously affected every repo not following gwm-cli's own convention. An explicit empty list (`trunks = []`) disables the merge filter entirely. Zero-diff for repos without a `[doctor]` section, since `#[serde(default)]` on `Config::doctor` resolves to the same `["dev", "main"]` that lived in the removed `TRUNK_BRANCHES` const.
- Homebrew tap install path ([#26](https://github.com/kbrdn1/gwm-cli/issues/26)) — `brew tap kbrdn1/tap && brew install gwm` becomes a first-class install channel on macOS. Canonical formula template lives at `packaging/homebrew/gwm.rb.template` (placeholders `__TAG__` / `__VERSION__` / `__SHA256_ARM64__` / `__SHA256_X86_64__`), rendered into `Formula/gwm.rb` by `.github/scripts/render-tap-formula.sh` which validates every sha256 is exactly 64 hex chars before emission. The formula targets macOS arm64 + intel, installs the `gwm` binary, generates shell completions via `gwm completions`, and ships a `--version` smoke test.

### CI

- New `hook-smoke` job in `.github/workflows/ci.yml` (#56). Cheap parallel job (~30s, no Rust toolchain) that `shellcheck`s the hook, asserts it is executable, verifies the empty-index short-circuit, and exercises gate 2's "gwm not in PATH" skip path. Guards against hook bit-rot without doubling CI time.
- New `homebrew-tap-update` job in `.github/workflows/release.yml` ([#26](https://github.com/kbrdn1/gwm-cli/issues/26)). Runs after `release` on stable tags only (rc/alpha/beta are filtered out), fetches the macOS sha256 sidecars from the freshly-published release, renders `Formula/gwm.rb` via the script, and pushes to [`kbrdn1/homebrew-tap`](https://github.com/kbrdn1/homebrew-tap) with a bot identity. Requires `HOMEBREW_TAP_TOKEN` (fine-grained PAT scoped to the tap repo, `contents: write`) — see CONTRIBUTING.md > Releases > Homebrew tap for bootstrap. `continue-on-error: true` during the PAT-provisioning window so the GitHub release still ships; flip to `false` after the first successful tap sync.

### Tests

- Sentinel / regression-only audit ([#57](https://github.com/kbrdn1/gwm-cli/issues/57)) — classified all 193 tests at v0.4.0 into three buckets (drives production logic, regression sentinel, dead weight), deleted 3 bucket-#3 tests (`bootstrap_when_tests::evaluator_is_pure_for_a_given_cwd`, `doctor_tests::prunable_check_detail_uses_singular_plural_correctly`, `cli_binary::shell_init_posix_does_not_use_paren_function_syntax`) that asserted constants against themselves or duplicated positive coverage, and annotated the 11 retained sentinels with a `// regression: <one-line>` tag so the incident target is discoverable without `git blame`. Total: **193 → 190**. Full audit table in [`claudedocs/test-audit-0.4.0.md`](claudedocs/test-audit-0.4.0.md).

### Docs

- `README.md` — new `### via Homebrew (macOS)` install section under `## install`, pointing at the tap and the canonical formula template ([#26](https://github.com/kbrdn1/gwm-cli/issues/26)).
- `CONTRIBUTING.md` — new "Homebrew tap (`brew install kbrdn1/tap/gwm`)" subsection under Releases covering the PAT bootstrap recipe (fine-grained, `contents: write`, tap-scoped only), the one-time `continue-on-error` flip, and `gh workflow run` retry instructions for the case where a release ships but the tap sync failed ([#26](https://github.com/kbrdn1/gwm-cli/issues/26)).
- `CONTRIBUTING.md` — new "Local hooks (recommended, opt-in)" subsection under Development describing the install one-liner, both gates, the doctor exit-code contract, the `--no-verify` bypass, and the CI smoke-test safety net (#56).
- `CLAUDE.md` + `CONTRIBUTING.md` §Releases — new "Step 0: reconcile open PRs" rule. Before any RC or stable cut, run `gh pr list --state open` and reconcile every open PR (in the changeset, intentionally deferred, or closed as stale). Codifies the lesson from the v0.3.0 cut, which shipped without three queued feature PRs (#51, #52, #53) and required an immediate v0.4.0 promotion 38 minutes later. Step 0 sits explicitly at the top of both the pre-release and stable-release procedures.
- `README.md` + `gwm shell-init` output + `gwm switch --help` — document the `gcd` two-paths routing ([#58](https://github.com/kbrdn1/gwm-cli/issues/58)). The bare `gcd` (no argument) call routes to `gwm switch` (interactive picker), while `gcd <pattern>` routes to `gwm cd <pattern>` (fuzzy resolve); both branches wait on a successful exit code before performing the `cd`. The bridge was wired up in PR [#53](https://github.com/kbrdn1/gwm-cli/pull/53) but lived only in the code — fresh users reading the README, the eval'd wrapper, or `gwm switch --help` now see the no-arg route surfaced consistently across all three surfaces. New `tests/cli_binary.rs::shell_init_*_header_documents_no_arg_route` + `switch_help_mentions_gcd_wrapper` pin the docstring so future drift breaks the suite instead of going unnoticed.
- `examples/gwm.toml.example` — documents the new `[doctor].trunks` knob with a commented-out block matching the bootstrap-step style already in the file.
- README — bumped the suite-size advert to 190 tests, added the missing `bootstrap_when_tests.rs`, `doctor_tests.rs`, and `flake_tests.rs` entries to the file tree, and pointed at the audit doc for sentinel hygiene.

## Past releases

In reverse chronological order:

- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
