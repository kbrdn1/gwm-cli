//! The frozen, versioned **machine contracts** gwm pledges to keep stable
//! across a 1.0 line (issue #317).
//!
//! Three surfaces are machine-readable and consumed by tooling outside
//! this repo — editor plugins, status bars, CI scripts:
//!
//! 1. **JSON output schemas** — the `--format=json` payloads of `list`,
//!    `doctor` and `path` (built from the [`crate::json_api`] DTOs) plus the
//!    hand-built `gwm status --json` payload, all documented in
//!    `docs/schema/*.json`.
//! 2. **Daemon JSON-RPC 2.0 protocol** — the `list` / `doctor` / `path` /
//!    `subscribe` methods and the `worktrees.changed` notification
//!    ([`crate::daemon`]).
//! 3. **`.gwm.toml` config schema** — the [`crate::config::Config`]
//!    section set.
//!
//! Surfaces 1 and 2 share the same DTOs (a daemon `list` result is
//! byte-identical to `gwm list --format=json`), so they share a single
//! [`SCHEMA_VERSION`]. The config schema evolves on its own cadence and
//! is governed by `serde(deny_unknown_fields)` + the freeze tests rather
//! than a runtime version integer.
//!
//! ## Versioning policy
//!
//! [`SCHEMA_VERSION`] is bumped **only** on a backward-incompatible change
//! to a *stable* field (rename, removal, or a type/semantics change). Adding
//! an optional field is backward-compatible and does NOT bump it — consumers
//! must ignore unknown fields. The integer is surfaced at runtime in the
//! daemon's `worktrees.changed` notification (`params.schema_version`) so a
//! long-lived `subscribe` client can detect a drift it was not built for;
//! one-shot CLI consumers key off `gwm --version`.
//!
//! ## Tiers
//!
//! Every field/section is **stable** or **experimental** for 1.0:
//!
//! - **Stable** — frozen by a contract test; a rename/removal fails CI and
//!   is a conscious breaking decision (a [`SCHEMA_VERSION`] bump). All
//!   `docs/schema/*.json` fields, the four daemon methods + the notification,
//!   and the documented `.gwm.toml` sections are stable.
//! - **Experimental** — may change without a version bump; called out in
//!   `docs/schema/README.md`. The workspace-only `repo` field on a
//!   `worktree-list` row is experimental (it rides the `--workspace`
//!   feature, itself young).
//!
//! The authoritative per-field tier table lives in `docs/schema/README.md`;
//! the freeze tests in `tests/contract_tests.rs` enforce it.

/// Version of the shared JSON contract — the `--format=json` payloads and
/// the daemon RPC results/notifications (they are byte-identical). Bumped
/// only on a backward-incompatible change to a stable field. See the
/// module docs for the policy.
pub const SCHEMA_VERSION: u32 = 1;

/// The daemon JSON-RPC request methods frozen for 1.0. A new method is an
/// additive (backward-compatible) change; renaming or removing one is a
/// breaking change. Pinned by `tests/contract_tests.rs`.
pub const DAEMON_METHODS: &[&str] = &["list", "doctor", "path", "subscribe"];

/// The daemon JSON-RPC notification method(s) frozen for 1.0.
pub const DAEMON_NOTIFICATIONS: &[&str] = &["worktrees.changed"];

/// The top-level `.gwm.toml` sections frozen as stable for 1.0. Mirrors
/// the fields of [`crate::config::Config`]; the freeze test cross-checks
/// the two so a renamed section can't silently drift the contract.
pub const CONFIG_SECTIONS: &[&str] = &[
  // Scalar, not a section: `forge = "github" | "gitlab"` (issue #419).
  // Additive and optional, so pre-#419 configs keep parsing unchanged.
  "forge",
  // `[forge_hosts]` — host → backend, read from the user-level config
  // only (issue #419). Also additive; an absent table authorises nothing,
  // which is the safe default rather than a permissive one.
  "forge_hosts",
  "worktree",
  "bootstrap",
  "hooks",
  "doctor",
  "tui",
  "theme",
  "git_tui",
  "review",
  "labels",
  "milestones",
  "branch_types",
  "aliases",
  "gitmoji",
  "issue_template",
  "pr_template",
  "exec",
  "clean",
];
