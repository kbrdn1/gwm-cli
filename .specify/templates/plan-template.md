# Implementation Plan: [FEATURE]

**Branch**: `[###-feature-name]` | **Date**: [DATE] | **Spec**: [link]
**Input**: Feature specification from `/specs/[###-feature-name]/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

[Extract from feature spec: primary requirement + technical approach from research]

## Technical Context

<!--
  ACTION REQUIRED: Replace the content in this section with the technical details
  for the project. The structure here is presented in advisory capacity to guide
  the iteration process.
-->

**Language/Version**: [e.g., Python 3.11, Swift 5.9, Rust 1.75 or NEEDS CLARIFICATION]
**Primary Dependencies**: [e.g., FastAPI, UIKit, LLVM or NEEDS CLARIFICATION]
**Storage**: [if applicable, e.g., PostgreSQL, CoreData, files or N/A]
**Testing**: [e.g., pytest, XCTest, cargo test or NEEDS CLARIFICATION]
**Target Platform**: [e.g., Linux server, iOS 15+, WASM or NEEDS CLARIFICATION]
**Project Type**: [single/web/mobile - determines source structure]
**Performance Goals**: [domain-specific, e.g., 1000 req/s, 10k lines/sec, 60 fps or NEEDS CLARIFICATION]
**Constraints**: [domain-specific, e.g., <200ms p95, <100MB memory, offline-capable or NEEDS CLARIFICATION]
**Scale/Scope**: [domain-specific, e.g., 10k users, 1M LOC, 50 screens or NEEDS CLARIFICATION]

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Answer each gate yes/no. A `no` blocks the phase until it is either fixed or
justified in the Complexity Tracking table below.

- [ ] **I. TDD** — does every behavioural task in this plan have a failing test
      written first, in the right `tests/` file for the surface it touches
      (`cli_binary.rs` / `worktree_integration.rs` / `bootstrap_tests.rs` /
      `tui_app_tests.rs` / `<module>_tests.rs`)? Do any of them read `$PATH`,
      `$HOME`, or installed tooling — and if so, are they pre-validated under a
      stripped environment?
- [ ] **II. Machine contracts** — does this touch a frozen surface (CLI
      subcommand/flag, exit code, `--format=json` payload, daemon method, or a
      `.gwm.toml` section)? If yes, are `src/contract.rs` and
      `tests/contract_tests.rs` updated in the same changeset, and is the
      additive-vs-breaking call stated explicitly?
- [ ] **III. Errors are values** — is every new user-reachable path returning a
      named `GwmError` variant, with no `unwrap()` / `expect()` / `panic!`
      outside tests and commented infallible spots?
- [ ] **IV. Native git** — do new git operations go through `git2` rather than
      shelling out? Is every new external process a documented dependency that
      degrades with a clear message when absent? Does the change stay within the
      declared MSRV for the whole crate?
- [ ] **V. Trust boundaries** — does this execute repo-supplied commands, copy
      files that could hold secrets, expose IPC, or delete anything? If yes: does
      it pass the TOFU trust gate fail-fast, honour the copy deny-list, keep the
      socket owner-only, and journal for `gwm undo` or verify preconditions
      before deleting?
- [ ] **VI. Pure render path** — does this add any `println!`, shell-out,
      blocking git call, or filesystem scan reachable from TUI `draw`? Is new TUI
      state exercised ratatui-free from `tests/tui_app_tests.rs`?
- [ ] **VII. Docs parity** — are the affected `docs/3.cli` / `docs/4.configuration`
      / `docs/schema/` sections updated, plus `examples/gwm.toml.example` if the
      schema moved, plus the `docs/fr/` mirror in the same changeset?

## Project Structure

### Documentation (this feature)

```text
specs/[###-feature]/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)
<!--
  ACTION REQUIRED: Replace the placeholder tree below with the concrete layout
  for this feature. Delete unused options and expand the chosen structure with
  real paths (e.g., apps/admin, packages/something). The delivered plan must
  not include Option labels.
-->

```text
# [REMOVE IF UNUSED] Option 1: Single project (DEFAULT)
src/
├── models/
├── services/
├── cli/
└── lib/

tests/
├── contract/
├── integration/
└── unit/

# [REMOVE IF UNUSED] Option 2: Web application (when "frontend" + "backend" detected)
backend/
├── src/
│   ├── models/
│   ├── services/
│   └── api/
└── tests/

frontend/
├── src/
│   ├── components/
│   ├── pages/
│   └── services/
└── tests/

# [REMOVE IF UNUSED] Option 3: Mobile + API (when "iOS/Android" detected)
api/
└── [same as backend above]

ios/ or android/
└── [platform-specific structure: feature modules, UI flows, platform tests]
```

**Structure Decision**: [Document the selected structure and reference the real
directories captured above]

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| [e.g., 4th project] | [current need] | [why 3 projects insufficient] |
| [e.g., Repository pattern] | [specific problem] | [why direct DB access insufficient] |
