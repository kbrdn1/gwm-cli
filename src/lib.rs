//! gwm — git worktree manager (lib root).
//!
//! Used both by the `gwm` binary (`src/main.rs`) and by integration tests
//! under `tests/`. Module surface is intentionally `pub` for testability.

pub mod aliases;
pub mod bootstrap;
pub mod clean;
pub mod cli;
pub mod command_log;
pub mod config;
pub mod config_cli;
pub mod daemon;
pub mod doctor;
pub mod error;
pub mod exec;
pub mod github;
pub mod gitmoji;
pub mod history;
pub mod hooks;
pub mod issue_templates;
pub mod json_api;
pub mod labels;
pub mod launcher;
pub mod lifecycle;
pub mod milestones;
pub mod multiplexer;
pub mod naming;
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
