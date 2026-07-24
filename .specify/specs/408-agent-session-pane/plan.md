# Implementation Plan: Agent session pane

**Branch**: `feat/#408-agent-session-pane` | **Date**: 2026-07-21 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `.specify/specs/408-agent-session-pane/spec.md`

## Summary

Detect AI-agent coding sessions (Claude Code, Codex, opencode, Mistral Vibe)
per managed worktree by reading each agent's on-disk session artefacts —
`std::fs` only, cross-platform including Windows — and surface them as: an
agent column in the TUI worktree table, a Status-pane line, a generic detail
overlay on `a`, an additive `agents` field on the JSON/daemon list contract,
and a compact statusline indicator. Detection runs off-thread through the
existing `TaskRunner`; the render path stays pure. All storage layouts were
pinned against real installations in [research.md](research.md).

## Technical Context

**Language/Version**: Rust, MSRV 1.86 (crate-wide floor; this feature is std::fs + serde only, no new floor)
**Primary Dependencies**: none new — `serde`/`serde_json` (in tree) for Codex/opencode/Vibe JSON, `dirs::home_dir()` (in tree) at the single production call site
**Storage**: read-only consumption of third-party artefact stores under the user home (see research.md D2–D5); gwm persists nothing new
**Testing**: `cargo test` — `tests/agent_sessions_tests.rs` (new, TempDir-seeded fixtures), `tests/tui_app_tests.rs` (state transitions), `tests/contract_tests.rs` (additive baseline), `tests/statusline_tests.rs` (indicator)
**Target Platform**: Linux, macOS, Windows — all three are required CI checks; Windows is a hard requirement (FR-010)
**Project Type**: single Rust crate (bin + internal lib seam)
**Performance Goals**: TUI frame time unaffected (detection is off-thread, snapshot-read only); bounded artefact scans (30-day window for Codex, newest-first for Vibe) so cost is independent of years of history
**Constraints**: no process scanning, no OS-specific APIs, no new dependencies, no blocking I/O on the render path, additive-only contract change (SCHEMA_VERSION stays 1)
**Scale/Scope**: 4 backends behind 1 trait; ~1 new src module + 1 new TUI state machine + additive edits to json_api/contract/statusline/tui table; 1 new keybinding

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [x] **I. TDD** — yes. Every story lands red-first: detection/backends in
      `tests/agent_sessions_tests.rs` against seeded `TempDir` roots (base-dir
      parameter, never `$HOME`); TUI transitions ratatui-free in
      `tests/tui_app_tests.rs`; contract additivity in `tests/contract_tests.rs`;
      statusline in `tests/statusline_tests.rs`. No test reads ambient state —
      the base-dir seam removes the `$HOME` dependency by construction; the
      stripped-PATH pre-validation run happens before push regardless.
- [x] **II. Machine contracts** — touched, additively: new optional `agents`
      field on the `list` row (JSON + daemon byte-identical) and a statusline
      addition. `src/contract.rs` + `tests/contract_tests.rs` updated in the
      same changeset; the call is **additive, minor, SCHEMA_VERSION stays 1**,
      field enters at the experimental tier (research.md D9). No CLI
      subcommand, flag, or exit code changes.
- [x] **III. Errors are values** — detection is deliberately *total*: every
      backend degrades to an empty result on missing/malformed input (FR-009),
      so the module's public surface returns values, not `Result`s that could
      panic; no `unwrap()`/`expect()` outside tests. The one clock-skew edge
      clamps (research.md D10).
- [x] **IV. Native git** — no git operations at all in this feature; no new
      external process; no new dependency; std APIs used are within the 1.86
      floor (`std::fs`, `SystemTime` — nothing newer).
- [x] **V. Trust boundaries** — nothing executed, nothing copied, nothing
      deleted, no new IPC endpoint (the daemon field rides the existing
      owner-only socket). Read-only scans of the user's own home; artefact
      *contents* are treated as untrusted input (parse-or-skip, bounded reads —
      first line only for Codex).
- [x] **VI. Pure render path** — detection is `TaskKind::AgentSessions` on the
      TaskRunner (debounced like `Sidebar`); `draw` reads the last snapshot from
      app state only. The overlay is a ratatui-free state machine in
      `src/tui/state/detail_overlay.rs`, tested from `tests/tui_app_tests.rs`.
- [x] **VII. Docs parity** — `docs/2.tui` (column, Status pane, overlay, key),
      `docs/5.integrations` (statusline), `docs/schema/` (experimental `agents`
      field), CHANGELOG `[Unreleased]`, each with its `docs/fr/` mirror in the
      same PR. No `.gwm.toml` change → `examples/gwm.toml.example` untouched
      (keybinding rebind rides the existing `[tui.keys]` docs).

**Post-design re-check (after Phase 1)**: all seven gates still hold; no
Complexity Tracking entries needed.

## Project Structure

### Documentation (this feature)

```text
.specify/specs/408-agent-session-pane/
├── spec.md
├── plan.md              # this file
├── research.md          # Phase 0 — layouts pinned on real installations
├── data-model.md        # Phase 1 — entities & states
├── quickstart.md        # Phase 1 — how to exercise the feature
├── contracts/
│   └── agents-field.md  # Phase 1 — additive JSON contract delta
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 (/speckit.tasks — not created here)
```

### Source Code (repository root)

```text
src/
├── agent_sessions.rs        # NEW — AgentKind, AgentSession, Freshness,
│                            #       SessionSource trait + 4 backends,
│                            #       summarize() aggregation (pure, std::fs)
├── lib.rs                   # + pub mod agent_sessions
├── json_api.rs              # + optional `agents` on JsonWorktree (additive)
├── contract.rs              # + agents field registered in the frozen surface
├── statusline.rs            # + compact agent indicator in render()
└── tui/
    ├── app.rs               # + agent snapshot state, `a` action wiring
    ├── ui.rs                # + AGENT column, Status pane line, overlay render
    ├── keymap.rs            # + Action::AgentSessions (default `a`, rebindable)
    └── state/
        ├── async_task.rs    # + TaskKind::AgentSessions
        └── detail_overlay.rs # NEW — generic row-list overlay state machine

tests/
├── agent_sessions_tests.rs  # NEW — backends, slug, matching, freshness,
│                            #       bounds, malformed-input degradation
├── tui_app_tests.rs         # + overlay open/dismiss/empty, column snapshot,
│                            #       help-overlay coverage (auto-forced)
├── contract_tests.rs        # + additive baseline for `agents`
└── statusline_tests.rs      # + indicator present/absent

docs/
├── 2.tui/…                  # column, Status pane, overlay, `a` key (EN)
├── 5.integrations/…         # statusline indicator (EN)
├── schema/…                 # experimental `agents` field tier entry
└── fr/…                     # FR mirror of each touched page
```

**Structure Decision**: single crate, one new pure module + one new TUI state
machine; everything else is additive edits to existing seams (`TaskKind`,
`JsonWorktree`, `contract.rs`, `statusline::render`, keymap `Action`). This
mirrors how #299 (CI indicator) and #300 (wt file tree) landed.

## Complexity Tracking

No constitution violations — table intentionally empty.
