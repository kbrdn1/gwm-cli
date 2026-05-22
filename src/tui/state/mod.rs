//! Sub-structs of `tui::app::App`, extracted per #102 to decompose the
//! 1300-line god struct into coherent slices of state. Each module here
//! owns one concern (modal countdown, filter buffer, sidebar focus, …)
//! and exposes a pure-state API that the `App` orchestrator composes.
//!
//! Decomposition order (one PR per slice, all tracking #102):
//!
//! - `confirm` — safety countdown for the destructive-action modal (#125)
//! - `create_form` — issue/type/slug input form (#123)
//! - `filter` — fuzzy filter buffer + memoised indices (#124)
//! - `link_prompt` — two-stage issue/PR linking prompt (#126)
//! - `sidebar` — scroll offsets + commit-line cache (#127, this PR)
//! - `github_fetch` — TTL cache + inflight dedupe for `gh` shell-outs (#128)

pub mod confirm;
pub mod create_form;
pub mod filter;
pub mod link_prompt;
pub mod sidebar;
