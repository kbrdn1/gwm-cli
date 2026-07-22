# Feature Specification: Agent session pane

**Feature Branch**: `feat/#408-agent-session-pane`
**Created**: 2026-07-21
**Status**: Draft
**Input**: GitHub issue #408 — detect AI-agent coding sessions (Claude Code, Codex,
opencode, Mistral Vibe) per worktree from their on-disk session artefacts and
surface them across the TUI, daemon and statusline.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See which agent is working where (Priority: P1)

A developer runs several AI coding agents in parallel, one isolated worktree per
agent. Opening the `gwm` TUI, they see at a glance, for every worktree, whether
an agent session is attached to it, which agent it is, and whether that session
was recently active — without switching terminals or hunting through windows.

**Why this priority**: this is the launch hook of the feature and the single gap
the reference tool covers that `gwm` does not. Detection plus the at-a-glance
indicator is on its own a viable, demoable MVP; every other story builds on it.

**Independent Test**: seed fake session artefacts for each supported agent under
a temporary home directory, point two of them at a known worktree path, open the
worktree list, and verify the worktree row shows the expected agent indicator
and freshness while unrelated worktrees show none.

**Acceptance Scenarios**:

1. **Given** a worktree with a recent Claude Code session whose recorded working
   directory matches the worktree path, **When** the user opens the TUI,
   **Then** the worktree row shows a Claude Code indicator marked as recently
   active.
2. **Given** a worktree that no agent has ever touched, **When** the user opens
   the TUI, **Then** its agent cell is empty and the row is otherwise unchanged.
3. **Given** a session whose last activity is older than the idle threshold,
   **When** the list renders, **Then** the indicator is shown in its idle state,
   visually distinct from an active one.
4. **Given** sessions from two different agents attached to the same worktree,
   **When** the list renders, **Then** the row reflects the most recently active
   agent, and the full set remains reachable from the detail views (stories 2–3).
5. **Given** none of the four agent tools is installed (no artefact directories
   exist), **When** the TUI opens, **Then** everything behaves as today — no
   error, no placeholder noise, no startup delay.

---

### User Story 2 - Inspect the sessions behind a worktree (Priority: P2)

From the worktree list, the developer selects a worktree and opens a detail
overlay listing every agent session associated with it: agent name, freshness,
last-activity time, and the session's identity, so they can tell a live pair
apart from last week's leftovers.

**Why this priority**: the column answers "is something here?"; the overlay
answers "what exactly?". It is also the designated foundation for a future rich
detail view, so its structure must be generic, but it is not needed for the MVP
demo.

**Independent Test**: with seeded artefacts for multiple sessions on one
worktree, trigger the overlay keybinding and verify every session appears with
agent, freshness and last-activity fields; dismiss it and verify the TUI returns
to its previous state.

**Acceptance Scenarios**:

1. **Given** a worktree with two sessions from different agents, **When** the
   user presses the overlay key on that row, **Then** an overlay lists both
   sessions with agent name, activity state and a human-readable last-activity
   time, most recent first.
2. **Given** the overlay is open, **When** the user presses the dismiss key,
   **Then** the overlay closes and the list is exactly as before.
3. **Given** a worktree with no sessions, **When** the user presses the overlay
   key, **Then** the overlay states that no agent session was found rather than
   opening empty.
4. **Given** the selected worktree, **When** the user reads the Status pane,
   **Then** it includes the same agent summary the row indicator abbreviates.

---

### User Story 3 - Session info on machine surfaces (Priority: P3)

Automation built on `gwm` — shell prompts via the statusline, scripts and
integrations via the daemon — can read the same per-worktree session facts the
TUI shows, so an agent-aware prompt or dashboard needs no additional scanner.

**Why this priority**: extends the feature to the ecosystem `gwm` already
promises (frozen machine contracts), but has no standalone demo value before the
human surface exists.

**Independent Test**: with seeded artefacts, query the machine listing surface
and verify each worktree entry carries its session summary; verify the
statusline output includes the active-agent hint for the current worktree; and
verify both surfaces stay valid and silent about sessions when detection finds
nothing.

**Acceptance Scenarios**:

1. **Given** a worktree with an active session, **When** a consumer requests the
   machine-readable worktree listing, **Then** the entry includes the agent
   session summary as an additive field, and existing consumers that ignore it
   keep working unchanged.
2. **Given** the current worktree has an active session, **When** the statusline
   renders, **Then** it includes a compact agent indicator.
3. **Given** no sessions exist anywhere, **When** either surface is queried,
   **Then** the output is well-formed and simply omits or empties the session
   fields.

---


### User Story 4 - Drive it from the CLI, pin when detection is not enough (Priority: P2, added by convergence 2026-07-22)

Scripting or working outside the TUI, the developer lists the detected
sessions per worktree with a dedicated command, sees the same agent indicator
in the plain `gwm list` table, and — when a session's recorded directory does
not match the worktree it really serves (agent launched from a subdirectory,
a moved worktree) — manually pins that session to the right worktree.
Auto-detection stays the default; a pin is a per-worktree override.

**Why this priority**: closes the human-CLI gap (the machine surface shipped
with US3) and covers the real mismatch cases passive detection cannot.

**Independent Test**: with seeded artefacts, `gwm agents` lists sessions per
worktree (human and `--format=json`); `gwm agents attach <wt> <session-id>`
makes an unmatched session appear on that worktree across every surface;
`gwm agents detach <wt>` restores pure detection; `gwm list` shows an AGENT
column.

**Acceptance Scenarios**:

1. **Given** seeded sessions, **When** the user runs `gwm agents`, **Then**
   each worktree lists its sessions (agent, freshness, last activity, id),
   and `--format=json` returns the same data machine-readably.
2. **Given** a session whose recorded directory matches no worktree,
   **When** the user attaches it to a worktree by id, **Then** it appears on
   that worktree in `gwm agents`, `gwm list --format=json`, the TUI and the
   statusline, marked as pinned where the surface shows detail.
3. **Given** a pinned session, **When** the user detaches it, **Then** the
   worktree returns to pure auto-detection.
4. **Given** an unknown session id, **When** the user attaches it, **Then**
   the command fails with a clear message and a hint to run `gwm agents`.
5. **Given** the plain `gwm list` table, **Then** it carries an AGENT column
   with the same compact indicator as the TUI.

---

### Edge Cases

- Two worktrees whose paths differ only by a trailing separator, or reach the
  same directory through a symlink: session-to-worktree matching must compare
  canonicalised paths, not raw strings.
- Path comparison on case-insensitive filesystems (macOS, Windows): a session
  recorded with different casing must still match its worktree.
- A session artefact references a worktree that has since been removed: the
  session is simply unmatched — never an error, never a phantom row.
- Malformed or truncated artefacts (empty file, invalid line, unreadable
  directory): skipped silently; one broken record must not hide the others.
- An agent's artefact store is huge (years of sessions): detection must bound
  its reading (recency-first) so the TUI's responsiveness never depends on
  artefact history size.
- Very fresh artefacts with skewed clocks (mtime in the future): treated as
  active, never as an error.
- Windows: home-relative artefact locations resolve correctly; no surface may
  silently return nothing on Windows by design (explicit anti-goal from the
  reference implementation).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST detect coding-agent sessions for Claude Code,
  Codex, opencode and Mistral Vibe by reading each tool's persisted session
  records from the user's home area — never by inspecting running processes.
- **FR-002**: The system MUST associate each detected session with the managed
  worktree whose path matches the session's recorded working directory, using
  canonicalised, platform-appropriate path comparison.
- **FR-003**: The system MUST classify each session's freshness (at minimum:
  active, idle) from the recency of its artefacts, using documented thresholds.
- **FR-004**: The worktree list MUST show a per-row agent indicator carrying the
  detected agent and its freshness; rows without sessions stay visually
  unchanged from today.
- **FR-005**: The Status pane MUST include the selected worktree's session
  summary.
- **FR-006**: A keybinding on the worktree list (default `a`, rebindable like
  existing keys) MUST open a detail overlay listing all sessions of the selected
  worktree; its layout MUST be generic enough to host other detail content
  later.
- **FR-007**: The machine-readable worktree listing (JSON output and daemon)
  MUST expose the session summary as an **additive** field under the existing
  schema-versioning rules; no existing field changes shape or meaning.
- **FR-008**: The statusline output MUST include a compact indicator for the
  current worktree's active session, and remain unchanged when there is none.
- **FR-009**: Detection MUST degrade to "no sessions" — silently and without
  user-visible errors — when an agent tool is not installed, its artefact area
  is missing, or individual records are unreadable or malformed.
- **FR-010**: All detection and all surfaces MUST work on Linux, macOS and
  Windows alike; Windows support is a hard requirement, not best-effort.
- **FR-011**: Session detection MUST never block interactive rendering; the
  interface stays responsive regardless of artefact volume, showing the last
  known result until a refresh completes.
- **FR-012** *(convergence 2026-07-22)*: A dedicated CLI command MUST list
  detected sessions per worktree, human-readably and as JSON.
- **FR-013** *(convergence)*: The user MUST be able to pin a detected session
  to a worktree by id, and to remove that pin; a pin overlays auto-detection
  (which remains the default) and is honoured by every surface.
- **FR-014** *(convergence)*: The plain `gwm list` table MUST show the same
  compact agent indicator as the TUI table.

### Key Entities

- **Agent session**: one persisted coding-agent working session; carries the
  agent kind, the recorded working directory, a last-activity timestamp, and a
  stable identifier derived from its artefacts.
- **Agent kind**: the supported tools — Claude Code, Codex, opencode, Mistral
  Vibe — open to extension without reworking the surfaces.
- **Freshness**: the session's activity classification (active / idle) derived
  from artefact recency against documented thresholds.
- **Session summary (per worktree)**: the aggregation surfaces consume — the
  set of sessions matched to one worktree, plus the most-recently-active one
  for compact display.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For every supported agent, a session whose recorded working
  directory is a managed worktree is visible in the list within one refresh
  cycle of opening the TUI — with zero configuration.
- **SC-002**: A user can tell which of ten worktrees has an active agent in
  under 5 seconds, without leaving the list view.
- **SC-003**: With no agent tooling installed, observable behaviour (output,
  timing, exit codes) is indistinguishable from today's release across all
  three platforms.
- **SC-004**: Existing machine-surface consumers (JSON, daemon, statusline)
  continue to work unmodified after the feature ships; the session field is
  purely additive.
- **SC-005**: The full detection-to-display path is exercised by automated
  tests seeded from temporary directories on all three CI platforms — no test
  depends on a real agent installation.

## Assumptions

- Freshness thresholds default to: **active** = artefact activity within the
  last 5 minutes, **idle** = anything older. Thresholds are internal defaults
  for now; making them configurable is not in scope.
- When several sessions match one worktree, the compact surfaces (row
  indicator, statusline) show the most recently active one; the overlay and
  machine surfaces expose the full set.
- "Session ended" cannot be observed from artefacts alone (no process
  scanning), so an ended-but-recent session may briefly read as active; this
  is accepted. Process-level liveness is explicitly deferred (issue #408).
- The four agents' storage layouts are taken from inspection of real
  installations as recorded in issue #408; a layout change in a future agent
  release degrades to "no sessions" for that agent (FR-009), never to an error.

## Out of Scope

- Process-level liveness on macOS/Linux (deferred post-demo, tracked in #408).
- Container execution (tracked separately).
- The rich PR/Issue detail view (separate priority; only the overlay's generic
  structure is shared).
- Configurable freshness thresholds and user-defined additional agents.
