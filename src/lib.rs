//! gwm — git worktree manager (lib root).
//!
//! **Internal test seam — NOT a public API.** This library target exists for
//! one reason: the binary (`src/main.rs`) and the integration tests under
//! `tests/` share a single module tree, and Rust integration tests can only
//! reach it through a `pub` lib. The whole crate is therefore `#![doc(hidden)]`
//! and every `pub` item below carries **no SemVer guarantee** — signatures,
//! modules, and types here are free to change in any release, major or not.
//!
//! Do not `cargo add gwm-cli` to build on `gwm::*`. The published crate ships
//! this lib only as a byproduct of shipping the `gwm` binary; it is not a
//! supported dependency. The contracts that *are* stable — the CLI, the
//! `--format=json` / daemon payloads, and the `.gwm.toml` schema — are
//! documented in `docs/6.development/3.stability.md` and frozen by
//! `tests/contract_tests.rs`.
#![doc(hidden)]

pub mod agent_sessions;
pub mod aliases;
pub mod bootstrap;
pub mod clean;
pub mod cli;
pub mod clipboard;
pub mod command_log;
pub mod config;
pub mod config_cli;
pub mod contract;
pub mod daemon;
pub mod doctor;
pub mod error;
pub mod exec;
pub mod forge;
pub mod github;
pub mod gitlab;
pub mod gitmoji;
pub mod history;
pub mod hooks;
// `issue_templates` is reached only through `crate::issue_templates` from
// `cli.rs`; no integration test imports it, so it need not be `pub` (#342).
pub(crate) mod issue_templates;
pub mod json_api;
pub mod labels;
pub mod launcher;
pub mod lifecycle;
pub mod milestones;
pub mod multiplexer;
pub mod naming;
pub mod notes;
pub mod pr_templates;
pub mod presets;
pub mod review;
pub mod statusline;
pub mod sync;
pub mod templating;
pub mod trust;
pub mod tui;
pub mod workspace;
pub mod worktree;
