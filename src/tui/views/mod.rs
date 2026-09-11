//! One module per TUI view: its state, its `draw_*` and its key handling
//! together (issue #635).
//!
//! `state/` was already split one module per view; the behaviour lived in
//! `app.rs` and the rendering in `ui.rs`, so adding or changing a view
//! meant editing three files, two of them enormous — which is what made
//! `app.rs` and `ui.rs` absorb 14 % of the repo's commits each and forced
//! the "parallel agents only when file ownership is disjoint" house rule.
//! The layering was horizontal while the variability is vertical.
//!
//! This is a pilot, not a plan: two views live here, and whether the rest
//! follow is a decision to take on measured churn rather than up front.

pub mod commits;
