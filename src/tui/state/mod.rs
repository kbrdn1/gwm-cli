//! Sub-structs of `tui::app::App`, extracted per #102 to decompose the
//! 1300-line god struct into coherent slices of state. Each module here
//! owns one concern (modal countdown, filter buffer, sidebar focus, …)
//! and exposes a pure-state API that the `App` orchestrator composes.
//!
//! Decomposition order (one PR per slice, all tracking #102):
//!
//! - `command_logs` — scroll + snapshot for the Command Logs overlay (#226)
//! - `confirm` — safety countdown for the destructive-action modal (#125)
//! - `create_form` — issue/type/slug input form (#123)
//! - `filter` — fuzzy filter buffer + memoised indices (#124)
//! - `link_prompt` — two-stage issue/PR linking prompt (#126)
//! - `sidebar` — scroll offsets + commit-line cache (#127)
//! - `github_fetch` — TTL cache + inflight dedupe for `gh` shell-outs (#128, this PR)
//! - `async_task` — generic off-thread spine (coalescing + late-drop) for slow ops (#231)
//! - `config_panel` — scroll + resolved-row snapshot for the Configuration overlay (#232)
//!
//! `commits` and `exec_picker` left this list in #635: their state now sits
//! in `tui::views::<view>`, next to that view's rendering and its key
//! handling, rather than one slice here and two more in `app.rs` / `ui.rs`.

pub mod async_task;
pub mod clean_overlay;
pub mod command_logs;
pub mod config_panel;
pub mod confirm;
pub mod create_form;
pub mod detail_overlay;
pub mod filter;
pub mod github_fetch;
pub mod link_prompt;
pub mod markdown;
pub mod note_editor;
pub mod pty_overlay;
pub mod rich_view;
pub mod sidebar;
pub mod spinner;
pub mod working_tree;
