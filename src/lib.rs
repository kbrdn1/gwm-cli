//! gwm — git worktree manager (lib root).
//!
//! Used both by the `gwm` binary (`src/main.rs`) and by integration tests
//! under `tests/`. Module surface is intentionally `pub` for testability.

pub mod bootstrap;
pub mod cli;
pub mod config;
pub mod error;
pub mod naming;
pub mod tui;
pub mod worktree;
