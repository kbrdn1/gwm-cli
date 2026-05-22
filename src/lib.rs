//! gwm — git worktree manager (lib root).
//!
//! Used both by the `gwm` binary (`src/main.rs`) and by integration tests
//! under `tests/`. Module surface is intentionally `pub` for testability.

pub mod bootstrap;
pub mod cli;
pub mod config;
pub mod doctor;
pub mod error;
pub mod github;
pub mod labels;
pub mod launcher;
pub mod multiplexer;
pub mod naming;
pub mod tui;
pub mod worktree;
