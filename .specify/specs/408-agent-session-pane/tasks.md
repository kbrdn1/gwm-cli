# Tasks: Agent session pane

**Input**: Design documents from `.specify/specs/408-agent-session-pane/`
**Prerequisites**: plan.md, spec.md, research.md (D1–D10), data-model.md, contracts/agents-field.md

**Tests**: MANDATORY (constitution Principle I). Every implementation task is
preceded by a red-first test task in the same phase. Run the new test, watch it
fail for the right reason, then implement.

**Organization**: by user story, so each story is an independently testable
increment. Note on parallelism: nearly all Phase 2 tasks touch the same two
files (`src/agent_sessions.rs`, `tests/agent_sessions_tests.rs`), so they are
sequential by design — no dishonest `[P]` markers.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [X] T001 Declare `pub mod agent_sessions;` in src/lib.rs, create src/agent_sessions.rs (module doc only) and tests/agent_sessions_tests.rs (imports + `mod common` if needed); `cargo test` still green

## Phase 2: Foundational — detection core (blocks all user stories)

Pure module, base-dir injection everywhere (never `$HOME`), per research.md D2–D7, D10.

- [X] T002 RED: slug tests in tests/agent_sessions_tests.rs — `/Users/x/Projects/gwm-cli` → `-Users-x-Projects-gwm-cli`, `.` → `-` (`/Users/x/.claude` → `-Users-x--claude`), case preserved, existing `-` kept (research.md D2 evidence table)
- [X] T003 GREEN: implement `claude_slug(path) -> String` in src/agent_sessions.rs (`[^A-Za-z0-9]` → `-`)
- [X] T004 RED: freshness tests — activity ≤ 300 s → `Active`, older → `Idle`, mtime in the future clamps to `Active`, `ended == true` forces `Idle` (D10)
- [X] T005 GREEN: implement `Freshness::classify(last_activity, ended, now)` + `ACTIVE_WINDOW` const
- [X] T006 RED: `ClaudeCodeSource::scan` tests against a seeded TempDir — matched slug dir yields sessions from `*.jsonl` only (`memory/` dir ignored), activity = max mtime, id = filename uuid; missing base dir → empty; note: Claude scan takes the worktree-path set (D2 forward matching)
- [X] T007 GREEN: implement `ClaudeCodeSource`
- [X] T008 RED: `CodexSource::scan` tests — first line `session_meta.payload.cwd` recovered; legacy `.json` (non-jsonl) skipped; malformed first line skipped silently; date dirs walked newest-first with 30-day bound (fixture with an old date dir proves the cutoff) (D3)
- [X] T009 GREEN: implement `CodexSource`
- [X] T010 RED: `OpencodeSource::scan` tests — `worktree` field recovered; `global.json` (id `global`, worktree `/`) skipped; activity from `time.updated` epoch-ms with fallback `time.created` then file mtime (D4)
- [X] T011 GREEN: implement `OpencodeSource`
- [X] T012 RED: `VibeSource::scan` tests — `environment.working_directory` recovered; non-null `end_time` → `ended = true`; null/absent → `false`; session dirs taken newest-first by `<ts>` prefix; timestamps in meta.json never string-parsed (D5)
- [X] T013 GREEN: implement `VibeSource` + the `SessionSource` trait tying the four backends together
- [X] T014 RED: `summarize` tests — session→worktree matching canonicalised (trailing separator, symlinked path), case-insensitive on macOS/Windows and sensitive on Linux (`cfg` split), unmatched sessions dropped without error, per-worktree list most-recent-first, `top` = most recent (D7, data-model)
- [X] T015 GREEN: implement `summarize(&[AgentSession], &[(id, PathBuf)]) -> BTreeMap<…>` + `WorktreeAgents`

**Checkpoint**: `cargo test --test agent_sessions_tests` green; module complete and hermetic.

## Phase 3: User Story 1 — See which agent is working where (Priority: P1) MVP

**Goal**: agent indicator with freshness in the worktree table, detection off-thread.

**Independent Test**: seeded artefacts for two agents on one worktree → row shows the most recent agent + freshness; unrelated rows unchanged; no agent tooling → indistinguishable from today (spec US1 scenarios 1–5).

- [X] T016 [US1] RED: state tests in tests/tui_app_tests.rs — new `TaskKind::AgentSessions` completes → snapshot replaced atomically in app state; refresh is debounced/coalesced like `TaskKind::Sidebar`; no snapshot yet → table renders without agent cells (no placeholder noise)
- [X] T017 [US1] GREEN: add `TaskKind::AgentSessions` in src/tui/state/async_task.rs (+ label/`is_git` classification), snapshot state + spawn wiring in src/tui/app.rs; single production call site resolves the four base dirs from `dirs::home_dir()` and runs the scans off-thread (constitution VI)
- [X] T018 [US1] RED: row-content tests — with a seeded snapshot the table row for the matched worktree carries the agent glyph + freshness style role, session-less rows carry an empty cell (assert at the row-build seam used by existing table tests in tests/tui_app_tests.rs)
- [X] T019 [US1] GREEN: add the AGENT column in src/tui/ui.rs (+ theme role for active/idle), reading only the snapshot

**Checkpoint**: MVP demoable — `cargo run` in this worktree shows the live claude session (quickstart.md).

## Phase 4: User Story 2 — Inspect the sessions behind a worktree (Priority: P2)

**Goal**: generic detail overlay on `a` + Status pane summary line.

**Independent Test**: two seeded sessions on one worktree → overlay lists both (agent, freshness, last-activity), dismiss restores the list, empty worktree → "no agent session found" (spec US2 scenarios 1–4).

- [X] T020 [US2] RED: overlay state-machine tests in tests/tui_app_tests.rs — `Closed → Open{title, rows}` on trigger, dismiss → `Closed` with list state untouched, open on session-less worktree → single "no agent session found" row, rows are generic (label, value, style-role) triples (data-model)
- [X] T021 [US2] GREEN: implement src/tui/state/detail_overlay.rs (ratatui-free, content-agnostic) + session→rows mapping (most recent first, human-readable last-activity)
- [X] T022 [US2] RED: keymap tests — new `Action` variant reachable from the worktree list with default `a` (prove no collision with existing list bindings), rebindable via `[tui.keys]`; `help_overlay_documents_every_action` forces the help entry (goes red by itself once the variant exists)
- [X] T023 [US2] GREEN: add the `Action` variant + default `a` in src/tui/keymap.rs, help row, overlay rendering in src/tui/ui.rs, wiring in src/tui/app.rs
- [X] T024 [US2] RED: Status pane tests — sidebar payload for the selected worktree includes the agent summary line (same info the row abbreviates); absent when no session (assert at the `build_sidebar_payload` seam)
- [X] T025 [US2] GREEN: add the summary line to the Status pane section in src/tui/ui.rs

**Checkpoint**: US1 + US2 fully functional; overlay reusable for the future rich view.

## Phase 5: User Story 3 — Session info on machine surfaces (Priority: P3)

**Goal**: additive `agents` field (JSON + daemon) + statusline indicator, per contracts/agents-field.md.

**Independent Test**: seeded artefacts → `gwm list --format=json` rows carry `agents`; statusline shows the compact hint for the current worktree; nothing seeded → both surfaces byte-identical to today (spec US3 scenarios 1–3).

- [X] T026 [US3] RED: contract tests in tests/contract_tests.rs — `agents` registered in the frozen-surface baseline as experimental-tier, additive-compat case (deserializing a payload with `agents` into a pre-feature consumer struct succeeds), field omitted (not null) when empty; note daemon parity is already pinned byte-identical by existing tests — reference it, do not duplicate
- [X] T027 [US3] GREEN: add `JsonWorktreeAgents`/`JsonAgentSession` to src/json_api.rs (`skip_serializing_if = "Option::is_none"`), populate in list building, register in src/contract.rs — SCHEMA_VERSION stays 1
- [X] T028 [US3] RED: statusline tests in tests/statusline_tests.rs — active session on current worktree → compact indicator in `render`; no session → output byte-identical to today
- [X] T029 [US3] GREEN: implement the indicator in src/statusline.rs reading the `agents` field

**Checkpoint**: all three stories complete; machine consumers keep working unmodified (SC-004).

## Phase 6: Polish & Cross-Cutting Concerns

- [X] T030 Docs EN — docs/2.tui (AGENT column, Status pane line, overlay + `a` key, rebinding pointer), docs/5.integrations (statusline indicator), docs/schema (experimental `agents` field + tiers table entry)
- [X] T031 Docs FR — mirror every page touched by T030 under docs/fr/ (constitution VII: same PR, not a follow-up)
- [X] T032 CHANGELOG.md — entry under `[Unreleased]` (feature + additive contract note)
- [X] T033 Full gates — `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, then the stripped-env run: `PATH="$(dirname "$(command -v cargo)"):/usr/bin:/bin" cargo test` (constitution I; must be green before push)

## Dependencies & Execution Order

- **Phase 1 → Phase 2**: T001 first (module files must exist).
- **Phase 2 blocks all stories**: T002–T015 are strictly sequential RED→GREEN pairs (same two files).
- **Phase 3 (US1) → Phase 4 (US2)**: US2's overlay feeds off the snapshot wiring from T017 and the row seam from T018 — sequential in practice despite story independence in the spec.
- **Phase 5 (US3)**: depends only on Phase 2 (`summarize` output) + T017's snapshot for the list path; can start after Phase 3.
- **Phase 6**: after all stories.

## Implementation Strategy

MVP = Phase 1 + 2 + 3 (US1): demoable agent column. Then US2, then US3, then
polish. Stop at any checkpoint to validate independently (quickstart.md). Commit
after each RED→GREEN pair or logical group, Gitmoji + Conventional, `refs #408`.

## Phase 7: User Story 4 — CLI surface + manual pinning (convergence 2026-07-22)

**Goal**: `gwm agents` (list / attach / detach), AGENT column in plain `gwm list`, pins in git branch config overlaying auto-detection.

- [X] T034 [US4] RED: pin overlay tests in tests/agent_sessions_tests.rs — a pinned session id is assigned to the pinned worktree even when its cwd matches nothing; unknown pin id is ignored silently; a pin on an already-cwd-matched session does not duplicate it
- [X] T035 [US4] GREEN: pins parameter threaded through detect_all (+ pure apply step), agents_home() env seam (GWM_AGENTS_HOME → dirs::home_dir) at the single production resolution point
- [X] T036 [US4] RED: cli_binary tests — `gwm agents` lists seeded sessions per worktree (human + --format=json), `help_prints_subcommands` canary gains `agents`
- [X] T037 [US4] GREEN: `gwm agents [pattern]` subcommand (list default)
- [X] T038 [US4] RED: cli_binary tests — attach by id makes an unmatched session appear (agents + list --format=json), detach restores detection, unknown id exits 1 with hint
- [X] T039 [US4] GREEN: `gwm agents attach <pattern> <session-id>` / `gwm agents detach <pattern>` + branch-config persistence (gwm-agent-pin, one pin per worktree)
- [X] T040 [US4] RED: cli_binary test — plain `gwm list` table carries the AGENT column with the compact indicator
- [X] T041 [US4] GREEN: AGENT column in the human list table
- [X] T042 [US4] Docs EN/FR (CLI reference + keybindings/sidebar cross-refs + schema README pinned note) + CHANGELOG update
- [X] T043 [US4] Full gates: fmt, clippy, cargo test, stripped-PATH run
