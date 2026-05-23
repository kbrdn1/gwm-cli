//! Inline fuzzy-filter state for the worktree list (issue #21, extracted
//! from `tui::app::App` per #124 / #102, closes #104).
//!
//! Two concerns live here:
//!
//! 1. **Buffer + active flag** — the live `/` prompt: the user opens it
//!    with `/`, types into `query`, then commits (Enter, sticky filter)
//!    or cancels (Esc, clear). The `App` orchestrator wraps the
//!    transitions so it can update the status-bar copy and the sidebar
//!    cache; the pure state lives here.
//!
//! 2. **Memoised matched-indices cache** — the load-bearing reason for
//!    the extraction (#104). The prior `App::filtered_indices` was
//!    recomputed 3–5× per render frame (every `tui/ui.rs` call site:
//!    list height, visible rows, title hint, footer counter, selection
//!    resolver). On a repo with a non-trivial worktree list and a
//!    typed query, that's the same `nucleo_matcher::Pattern::parse +
//!    Matcher + score` pass repeating per frame for no observable
//!    reason. The cache here stores the result alongside the
//!    `worktrees.len()` it was computed against; any mutation that
//!    changes the query OR the worktrees length invalidates it, so the
//!    closure runs once per query/list change instead of once per
//!    frame.

use crate::worktree::WorktreeInfo;
use nucleo_matcher::{
  pattern::{CaseMatching, Normalization, Pattern},
  Config as NucleoConfig, Matcher, Utf32Str,
};

/// Pure fuzzy-match function over a slice of `WorktreeInfo`. Extracted
/// from `App::filtered_indices` so it can be unit-tested without an
/// `App`, and so the `FilterState::filtered_indices` memo path stays
/// agnostic of the matching algorithm. Empty query is the identity over
/// the input slice. Otherwise, returns the indices of every worktree
/// whose `name` scores against the `nucleo_matcher` pattern, ranked by
/// descending score with stable tie-breaking on original index.
///
/// The matching contract is identical to the pre-extraction behaviour
/// (see #21): exact substring > prefix > subsequence, smart case,
/// smart Unicode normalisation.
pub fn fuzzy_match_indices(query: &str, worktrees: &[WorktreeInfo]) -> Vec<usize> {
  if query.is_empty() {
    return (0..worktrees.len()).collect();
  }
  let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
  let mut matcher = Matcher::new(NucleoConfig::DEFAULT);
  let mut buf: Vec<char> = Vec::new();
  let mut scored: Vec<(u32, usize)> = Vec::with_capacity(worktrees.len());
  for (i, w) in worktrees.iter().enumerate() {
    let hay = Utf32Str::new(&w.name, &mut buf);
    if let Some(score) = pattern.score(hay, &mut matcher) {
      scored.push((score, i));
    }
  }
  scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
  scored.into_iter().map(|(_, i)| i).collect()
}

/// Inline fuzzy-filter state machine + memoised matched-indices cache.
/// `Default` opens the filter in the closed / empty / cold-cache state.
#[derive(Debug, Default)]
pub struct FilterState {
  /// `true` while the user is typing in the `/` bar. Toggles by
  /// [`Self::open`] / [`Self::close_keep`] / [`Self::close_cancel`]
  /// (the close methods describe the sticky-vs-clear contract).
  pub active: bool,
  /// Live query buffer. Empty = no filter active; the visible list is
  /// the identity over `App.worktrees`.
  pub query: String,
  /// Cached matched-indices vec from the last call to
  /// [`Self::filtered_indices`]. `None` = cold cache (must recompute).
  /// Any buffer mutation, explicit [`Self::invalidate`], or worktrees-
  /// length change clears it.
  cached_indices: Option<Vec<usize>>,
  /// Worktrees vec length the cache was computed against. If it
  /// changes between calls, the cache auto-invalidates — defence in
  /// depth so a caller that mutates `App.worktrees` without
  /// remembering to call `invalidate()` can never read indices that
  /// point past the new vec.
  cache_worktrees_len: usize,
}

impl FilterState {
  pub fn new() -> Self {
    Self::default()
  }

  /// Append a character to the query buffer and invalidate the cache.
  /// Called by the event loop on every keypress while `active`.
  pub fn push_char(&mut self, c: char) {
    self.query.push(c);
    self.cached_indices = None;
  }

  /// Pop the last character off the query buffer. Backspace handler.
  /// No-op on an empty buffer (the user already cleared the filter;
  /// the second backspace must not toggle `active` off — Esc does
  /// that). Invalidates the cache iff a character actually came off.
  pub fn pop_char(&mut self) {
    if self.query.pop().is_some() {
      self.cached_indices = None;
    }
  }

  /// Overwrite the query buffer wholesale. Invalidates the cache.
  /// Used by tests and by any future caller that wants to set the
  /// filter programmatically (e.g. a "restore session" path).
  pub fn set_query(&mut self, q: String) {
    self.query = q;
    self.cached_indices = None;
  }

  /// Clear the buffer, close the bar, and invalidate the cache.
  /// Used by [`Self::close_cancel`] and as a standalone reset.
  pub fn clear(&mut self) {
    self.query.clear();
    self.active = false;
    self.cached_indices = None;
  }

  /// Open the filter bar. Preserves the existing query so the user can
  /// refine an already-sticky filter; `close_cancel` is how they start
  /// fresh. Does NOT invalidate the cache: opening the bar doesn't
  /// change what's filtered, only that the next keypress targets the
  /// buffer.
  pub fn open(&mut self) {
    self.active = true;
  }

  /// Close the filter bar but keep the query — the sticky-filter path
  /// (Enter). Cache survives: the filter set didn't change, only the
  /// input target. Subsequent reads stay cached.
  pub fn close_keep(&mut self) {
    self.active = false;
  }

  /// Close the filter bar AND clear the query — the cancel path (Esc).
  /// Delegates to `clear` so the cache invalidation contract stays in
  /// one place.
  pub fn close_cancel(&mut self) {
    self.clear();
  }

  /// Explicit cache flush. Called by `App::refresh` after it mutates
  /// `App.worktrees`, so the next render recomputes against the fresh
  /// list. (The `cache_worktrees_len` auto-invalidation catches len
  /// changes too, but `refresh` may produce a vec of the same length
  /// with different contents — clearing here is the safe play.)
  pub fn invalidate(&mut self) {
    self.cached_indices = None;
  }

  /// Memoised lookup of the matched-indices vec. On cold cache, runs
  /// `compute(&self.query, worktrees)` and stores the result; on hot
  /// cache (no buffer mutation since the previous call AND same
  /// worktrees length), returns the cached slice directly.
  ///
  /// Returns a borrowed slice so the call sites in `tui/ui.rs` don't
  /// pay for a clone on the hot path. The `compute` closure shape
  /// matches [`fuzzy_match_indices`] — the App passes that fn in,
  /// keeping `FilterState` agnostic of the matcher (and trivial to
  /// unit-test with a counting closure).
  pub fn filtered_indices<F>(&mut self, worktrees: &[WorktreeInfo], compute: F) -> &[usize]
  where
    F: FnOnce(&str, &[WorktreeInfo]) -> Vec<usize>,
  {
    let len_changed = self.cache_worktrees_len != worktrees.len();
    let stale = self.cached_indices.is_none() || len_changed;
    if stale {
      let fresh = compute(&self.query, worktrees);
      self.cached_indices = Some(fresh);
      self.cache_worktrees_len = worktrees.len();
    }
    // Safe: `stale` branch above guarantees `Some` before we reach here.
    self
      .cached_indices
      .as_deref()
      .expect("cached_indices populated above on cold/stale cache")
  }

  /// `&self`-friendly read for callers that already know the cache is
  /// warm (or are willing to recompute via `compute` without storing).
  /// Used by `App::selected()` which is `&self` for ergonomics — most
  /// callers of `selected` hold a shared borrow and can't take the
  /// `&mut` needed by [`Self::filtered_indices`]. Returns an owned
  /// `Vec<usize>` because the caller may hit the recompute branch and
  /// we can't surface a temporary as `&[usize]`. Cheap when the cache
  /// is hot (the per-frame render already populated it); equivalent to
  /// the pre-extraction cost when cold.
  pub fn snapshot_indices<F>(&self, worktrees: &[WorktreeInfo], compute: F) -> Vec<usize>
  where
    F: FnOnce(&str, &[WorktreeInfo]) -> Vec<usize>,
  {
    let len_changed = self.cache_worktrees_len != worktrees.len();
    match self.cached_indices.as_deref() {
      Some(cached) if !len_changed => cached.to_vec(),
      _ => compute(&self.query, worktrees),
    }
  }
}
