<!--
  SYNC IMPACT REPORT
  ==================
  Version change: (unratified template) -> 1.0.0
  Bump rationale: initial ratification. The file previously shipped as the
  stock SpecKit template with unfilled bracket tokens and was never
  adopted, so this is a first adoption, not an amendment.

  Modified principles: none (no prior ratified principles existed).

  Added sections:
    - I. Test-Driven Development is non-negotiable
    - II. Machine contracts are frozen, human output is not
    - III. Errors are values, never panics on user paths
    - IV. Native libgit2 first, one self-contained binary
    - V. Safe by default at every trust boundary
    - VI. The render path stays pure
    - VII. Every user-facing surface is documented, in both languages
    - Quality Standards (formatting / testing / release & distribution)
    - Governance (single-maintainer amendment process, compliance gates)

  Removed sections: none.

  Templates:
    - .specify/templates/plan-template.md ......... UPDATED (Constitution Check
      gate filled with seven checkable yes/no gates, replacing the
      "[Gates determined based on constitution file]" placeholder)
    - .specify/templates/tasks-template.md ........ UPDATED (removed the
      "Tests are OPTIONAL" stance, which directly contradicted Principle I;
      test tasks are now mandatory and test-first; Path Conventions now name
      this repo's real Rust layout)
    - .specify/templates/spec-template.md ......... REVIEWED, no change needed
      (no test-optionality or constraint language conflicts with any principle)
    - .specify/templates/commands/*.md ............ N/A (this scaffold ships no
      per-repo command templates; the speckit commands are global skills and
      are out of scope for a repo-level constitution)

  Deferred TODOs: none. No placeholder tokens remain.
-->

# gwm-cli Constitution

**Version**: 1.0.0 | **Ratified**: 2026-07-21 | **Last Amended**: 2026-07-21

## Document Hierarchy

This constitution defines **architectural principles**. For operational guidance:

- **Constitution** (`.specify/memory/constitution.md`) — architectural principles & quality gates
- **CONTRIBUTING.md** — git workflow, branches, commits, pull requests, releases
- **CLAUDE.md** — AI assistant operational guidance
- **docs/** — technical documentation (`docs/fr/` mirrors it in French)

Where the two overlap, this file owns the *invariant* and CONTRIBUTING owns the
*procedure*. A rule about how a branch is named belongs there; a rule about what
may never ship belongs here.

## Core Principles

### I. Test-Driven Development is non-negotiable

No production code lands without a failing test that pinned the behaviour down
first. This is the primordial rule of the repository, not a preference.

- **Red first**: a test MUST be written and MUST be observed failing — for the
  right reason (assertion mismatch, not a compile error elsewhere) — before the
  production code that satisfies it exists.
- **Green minimally**: only the code needed to pass. No speculative branches, no
  abstractions with a single caller.
- **Tests live in `tests/`**: no inline `#[cfg(test)] mod tests` blocks inside
  `src/`. Each module has a companion `tests/<module>_tests.rs` or
  `tests/<module>_integration.rs`.
- **The test goes where the behaviour is observable**: CLI surface →
  `tests/cli_binary.rs` (`assert_cmd`); libgit2 worktree ops →
  `tests/worktree_integration.rs` (`tests/common::init_repo()`); bootstrap steps
  → `tests/bootstrap_tests.rs` (`tempfile::TempDir`); TUI state transitions →
  `tests/tui_app_tests.rs` (ratatui-free); pure logic → the matching
  `tests/*_tests.rs`.
- **Tests MUST NOT depend on the author's shell**: any test reading `$PATH`, the
  home directory, or installed tooling MUST be pre-validated against a stripped
  environment (`PATH="$(dirname "$(command -v cargo)"):/usr/bin:/bin" cargo test`)
  before it is pushed. CI runners have none of your dev tooling.
- **Exception** (narrow, MUST be argued in the PR description): the change is
  observably untestable from the public surface — incidental string/typo fixes
  not asserted anywhere, dependency bumps with no behaviour change, comment-only
  changes. "I tested it manually" is not an exception; codify it.

**Rationale**: `gwm` performs destructive operations on other people's
repositories — it removes worktrees, deletes build artifacts, copies files
carrying secrets, and executes commands from repo-supplied config. A regression
here costs a user their work, not a page render. Tests written after the fact
document what the code does; tests written first document what it must do, and
only the second kind catches the bug you did not think of.

### II. Machine contracts are frozen, human output is not

Every surface a script can key off is part of a SemVer promise. Every surface a
human merely reads is free to change.

- **Covered, breaking a MAJOR bump**: CLI subcommands, flags and argument shapes;
  the deterministic `0`/`1`/`2` exit-code meanings; the `--format=json` payloads
  of `list`, `doctor`, `path`, `status`; the daemon JSON-RPC methods,
  `worktrees.changed` notification, and error codes; the `.gwm.toml` top-level
  section set.
- **Not covered, may change any release**: TUI layout, colours and themes; all
  human-readable prose (log lines, status-bar text, help blurbs, non-JSON
  output); the published `[lib]` API, which is an internal test seam
  (`#![doc(hidden)]`), not a consumable crate.
- **One source of truth**: the frozen names live in `src/contract.rs` and are
  pinned by `tests/contract_tests.rs`. Touching a covered surface MUST update
  both, in the same commit.
- **Additive is minor**: a new subcommand, optional flag, or JSON field is a
  minor. Renaming or removing one is a major, deliberately taken.
- Surfaces covered by the written promise but *not* mechanically freeze-tested
  (CLI names, exit-code meanings) are exactly as binding. The absence of a guard
  test is not a licence to break them quietly.

**Rationale**: the JSON API, the daemon and `gwm statusline` exist so other
programs can build on `gwm`. A contract nobody can rely on is not a contract.
Splitting the surface explicitly is what lets the TUI be reworked freely without
anyone's prompt breaking.

### III. Errors are values, never panics on user paths

Any code reachable from a user invocation MUST return a `Result` carrying a
`GwmError` variant rather than panicking.

- **No `unwrap()` / `expect()` / `panic!` on a user-facing path.** Failure modes
  get a named `GwmError` variant with a message that says what to do next.
- **Permitted**: inside `tests/`, and at genuinely infallible points (e.g.
  `.lock()` on a never-poisoned mutex). Those MUST be a deliberate, commented
  choice, not a shortcut past an inconvenient `Result`.
- **`#[allow(...)]` requires a comment** explaining why the lint does not apply.

**Rationale**: a panic in a worktree-removal path aborts mid-operation and leaves
the user's repository in a state they did not ask for and cannot easily read. An
error value carries context to the surface and lets the operation unwind
cleanly.

### IV. Native libgit2 first, one self-contained binary

Worktree operations go through vendored `libgit2` (`git2`), not by shelling out
to `git`.

- **Git object/worktree manipulation MUST use `git2`.** Shelling out is
  permitted only where no libgit2 equivalent exists (porcelain such as
  `git status --short`, rebase/merge driving) or where the tool is inherently
  external (`gh`, `lazygit`, `tmux`).
- **Every external process is an explicit, documented dependency**, surfaced by
  `gwm doctor` when missing, and MUST degrade with a clear message rather than a
  cryptic failure when absent.
- **The shipped artifact is one binary with no runtime dependency** beyond `git`
  itself and whatever the user opted into.
- **MSRV is the floor for the whole crate**, not for the newest edit. It is
  declared in `Cargo.toml` and verified against the entire codebase
  (`cargo msrv verify`, or `cargo clippy --all-targets -- -W clippy::incompatible_msrv`)
  before it is declared or changed.

**Rationale**: the tool this replaces was a stack of bash wrappers around
`git worktree`. Parsing porcelain output is how those broke. Native calls give
typed errors and no locale/format surprises, and a single binary is why `gwm`
installs from Cargo, Homebrew, Scoop, Nix, aqua, the AUR, `.deb` and `.rpm`
without shipping a runtime.

### V. Safe by default at every trust boundary

`gwm` executes commands from repository config, copies files that may hold
credentials, and exposes an IPC socket. Each is a boundary and each MUST be
closed by default.

- **Repo-supplied code execution requires trust.** Bootstrap and lifecycle hooks
  run arbitrary commands from `.gwm.toml`; they MUST pass the TOFU trust ledger
  (`src/trust.rs`) before running. Any new path that resurrects or re-runs
  bootstrap MUST route through the same gate, fail-fast, before any side effect.
- **Untrusted sources are opt-in, never default.** Code fetched from a fork
  (`gwm review <PR#>`) MUST NOT bootstrap unless the user explicitly asked
  (`--bootstrap`).
- **Secrets do not get copied by accident.** File-copy steps MUST honour the
  deny-list regexes; that mechanism exists because credentials once rode a
  copied `.env` into a worktree.
- **IPC is owner-only.** The daemon socket MUST be `0600` inside an owner-only
  directory, with explicit resource bounds (max line length, read timeout, max
  connections). Permissions are set explicitly, not left to `umask`.
- **Destructive operations are reversible or gated.** Removal and artifact
  reclaim MUST either journal for `gwm undo` or verify their preconditions
  (git-ignored, no tracked file, not a symlink) before deleting.

**Rationale**: a worktree manager is trusted with the developer's whole
checkout, and `.gwm.toml` arrives with a cloned repository. Convenient-by-default
here means "clone a repo, run arbitrary code". Each of these rules is the
residue of a real incident or a real review finding.

### VI. The render path stays pure

Drawing a frame MUST NOT perform I/O.

- **No `println!` in TUI render code.** The status bar is the only channel for
  runtime feedback inside the TUI.
- **No shelling out, no blocking git call, no filesystem scan from `draw`.**
  Work that can block MUST go through the async task layer (`TaskRunner`), with
  the render path reading the last known state and showing a loading placeholder
  on a cache miss.
- **TUI state transitions MUST be testable without a terminal.** The state
  machines live in `src/tui/state/` and are exercised ratatui-free from
  `tests/tui_app_tests.rs`.

**Rationale**: a frame that shells out to git renders at the speed of the
slowest repository on disk. Keeping I/O out of `draw` is what makes the TUI
usable on a large workspace, and keeping state ratatui-free is what makes it
testable at all.

### VII. Every user-facing surface is documented, in both languages

A behaviour a user can reach that is not written down does not exist.

- **New or changed CLI surface** MUST update the matching `docs/3.cli` section.
- **New or changed config schema** MUST update `examples/gwm.toml.example` and
  the `docs/4.configuration` section.
- **New or changed machine contract** MUST update `docs/schema/`, including the
  stable-vs-experimental tier of each field.
- **`docs/fr/` mirrors `docs/`.** A documentation change ships its French
  counterpart in the same PR, not as a follow-up. Nothing in CI checks this —
  it is enforced in review, and is binding for that reason alone.

**Rationale**: the README is a landing page that delegates to `docs/`, and the
schema docs are the only thing standing between an integrator and reverse-
engineering the JSON. Deferred translation never happens, so parity is enforced
at the same moment the English lands or it drifts permanently.

## Quality Standards

### Code Formatting

- **Formatter**: `cargo fmt` (rustfmt defaults, except indentation). CI enforces
  `cargo fmt --all -- --check`.
- **Indentation**: 2 spaces.
- **Linter**: `cargo clippy --all-targets --all-features -- -D warnings` MUST
  pass. An `#[allow(...)]` MUST carry a comment justifying it.

### Testing Standards

- **Test isolation**: each test MUST be independent and MUST NOT depend on
  execution order, ambient tooling, or a pre-existing home directory. Disk work
  goes in a `tempfile::TempDir`; git work starts from `tests/common::init_repo()`.
- **Naming**: `tests/<module>_tests.rs` for unit-level coverage,
  `tests/<module>_integration.rs` for integration, `tests/cli_binary.rs` for
  end-to-end. `tests/cli_binary.rs::help_prints_subcommands` is the canary and
  MUST be updated whenever a subcommand is added.
- **Coverage**: there is **no line-coverage threshold and no coverage gate**.
  The gate is Principle I plus the companion-test rule enforced in review — a
  reviewer runs `git log --stat <branch>..HEAD -- tests/` and blocks a PR whose
  touched module has no test diff. Percentage targets are deliberately not used;
  they measure lines executed, not behaviour pinned.
- **Cross-platform**: the suite MUST pass on Linux, macOS and Windows. All three
  are required status checks.

### Release & Distribution Standards

- **Versioning is SemVer**, with `-rc.N` / `-alpha.N` / `-beta.N` pre-releases
  cut from `dev`. What counts as breaking is defined by Principle II and detailed
  in `docs/6.development/3.stability.md`.
- **`main` is protected with `enforce_admins` on.** There is no direct push, for
  anyone, including the maintainer. Everything reaches `main` through a pull
  request with green checks — hotfixes and release promotions included.
- **Release notes are per-version files** (`changelogs/<version>.md`, or
  `changelogs/pre-releases/<version>.md`), never the in-progress root
  `CHANGELOG.md` index.
- **Crate identity is finalised before the tag.** Any change to the package name
  or version MUST land in the commit the tag points at, so publishing from the
  tag is reproducible.
- **Security advisories block.** `cargo audit --deny warnings` is a required
  check; a plain `cargo audit` exits 0 on warnings and is not sufficient.

## Governance

### Amendment Process

This is a single-maintainer repository. The amendment process is scaled to that
reality rather than pretending at a committee:

1. **Proposal** — open a PR that edits this file and states the rationale for
   the change in its description.
2. **Impact** — enumerate the dependent artifacts the change touches (templates
   under `.specify/templates/`, `CONTRIBUTING.md`, `CLAUDE.md`, `docs/`) and
   update them in the same PR.
3. **Version** — increment the version per the semantics below and update
   **Last Amended**.
4. **Merge** — the required status checks are the gate. `main` requires **0
   approvals** by design: GitHub forbids approving your own PR, so requiring one
   would be a permanent lockout on a solo repo. The PR is the rail that
   guarantees the checks run; the checks are the approval.

### Version Semantics

- **MAJOR**: a principle is removed, or redefined in a way that retroactively
  invalidates code that previously complied.
- **MINOR**: a principle or section is added, or materially expanded.
- **PATCH**: clarification, rewording, typo, non-semantic refinement.

### Compliance Enforcement

- **SpecKit gates**: `/speckit.plan` runs the Constitution Check before Phase 0
  and re-checks after Phase 1 design; `/speckit.analyze` reports drift across
  spec, plan and tasks.
- **CI**: `rustfmt`, `clippy`, `test` on three platforms, the pre-commit hook
  smoke test, and `cargo audit` are required checks on `main`. `gwm doctor` runs
  advisory.
- **Pull request review**: adherence to MUST principles is verified per PR, with
  the companion-test check (Principle I) applied explicitly.
- **Technical debt**: a violation ships only with an explicit, written
  justification using the template below.

### Complexity Justification

When a principle must be violated:

```markdown
## Constitution Violation Justification

**Principle Violated**: [Principle name]
**Why Needed**: [Specific technical requirement]
**Alternatives Considered**: [Simpler approaches rejected]
**Mitigation**: [How impact is minimized]
**Type**: [Technical debt | Architectural decision]
```
