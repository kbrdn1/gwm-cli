//! Command palette (issue #32).
//!
//! Pressing `:` (the default binding of [`Action::CommandPalette`])
//! opens a single-line input at the bottom of the TUI. The user types
//! a verb (`create`, `delete`, `bootstrap`, …); a fuzzy-matched menu
//! above the input surfaces the candidates with their one-line
//! descriptions. `Enter` fires the highlighted action, `Esc` cancels,
//! `Tab` / arrow keys cycle the highlight.
//!
//! The palette and the help overlay share a single registry
//! ([`palette_entries`]) so neither surface can quietly drift from
//! the other: an `Action` variant that exists in `keymap::ACTIONS`
//! but not here would be reachable by key only, and vice versa.
//! `registry_covers_every_action_variant` in
//! `tests/palette_tests.rs` is the tripwire.

use super::keymap::Action;
use nucleo_matcher::{
  pattern::{CaseMatching, Normalization, Pattern},
  Config as NucleoConfig, Matcher, Utf32Str,
};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// One row in the palette's command registry. Stable across runs;
/// `name` is what the user types after `:`, `description` is the
/// one-line gloss shown next to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
  pub action: Action,
  pub name: &'static str,
  pub description: &'static str,
}

/// Full palette registry. Order is the suggestion order used when
/// the input buffer is empty (the user pressed `:` and hasn't typed
/// yet) — most-frequent actions first so the top entries on screen
/// are also the ones a user with no specific verb in mind is most
/// likely to want.
pub const fn palette_entries() -> &'static [PaletteEntry] {
  &[
    PaletteEntry {
      action: Action::Create,
      name: "create",
      description: "new worktree (form opens)",
    },
    PaletteEntry {
      action: Action::DeleteConfirm,
      name: "delete",
      description: "delete the selected worktree (with confirm)",
    },
    PaletteEntry {
      action: Action::Bootstrap,
      name: "bootstrap",
      description: "re-run bootstrap on the selected worktree",
    },
    PaletteEntry {
      action: Action::Refresh,
      name: "refresh",
      description: "refresh the worktree list",
    },
    PaletteEntry {
      action: Action::Open,
      name: "open",
      description: "open the selected worktree (shell / editor / finder)",
    },
    PaletteEntry {
      action: Action::OpenMenu,
      name: "open-menu",
      description: "open issue or PR URL in the browser",
    },
    PaletteEntry {
      action: Action::LinkPrompt,
      name: "link",
      description: "link the selected worktree to an issue or PR",
    },
    PaletteEntry {
      action: Action::FetchGithub,
      name: "fetch-github",
      description: "refresh GitHub issue/PR status via `gh`",
    },
    PaletteEntry {
      action: Action::GitTui,
      name: "git-tui",
      description: "launch the [git_tui] launcher (default lazygit)",
    },
    PaletteEntry {
      action: Action::Review,
      name: "review",
      description: "run the [review] launcher against the resolved base",
    },
    PaletteEntry {
      action: Action::Yank,
      name: "yank",
      description: "yank the selected worktree path to the clipboard",
    },
    PaletteEntry {
      action: Action::Filter,
      name: "filter",
      description: "open the fuzzy filter bar",
    },
    PaletteEntry {
      action: Action::ToggleSidebar,
      name: "toggle-sidebar",
      description: "toggle the git preview sidebar",
    },
    PaletteEntry {
      action: Action::ToggleSidebarMode,
      name: "toggle-sidebar-mode",
      description: "cycle the sidebar between commits and stashes",
    },
    PaletteEntry {
      action: Action::ToggleDeleteBranch,
      name: "toggle-delete-branch",
      description: "toggle whether `delete` also drops the branch",
    },
    PaletteEntry {
      action: Action::FocusSwap,
      name: "focus-swap",
      description: "swap focus between worktree list and sidebar",
    },
    PaletteEntry {
      action: Action::Top,
      name: "top",
      description: "jump to the first worktree",
    },
    PaletteEntry {
      action: Action::Bottom,
      name: "bottom",
      description: "jump to the last worktree",
    },
    PaletteEntry {
      action: Action::Down,
      name: "down",
      description: "select the next worktree",
    },
    PaletteEntry {
      action: Action::Up,
      name: "up",
      description: "select the previous worktree",
    },
    PaletteEntry {
      action: Action::Help,
      name: "help",
      description: "show the help overlay",
    },
    PaletteEntry {
      action: Action::Quit,
      name: "quit",
      description: "quit the TUI",
    },
    // Reflective entry — opens the palette itself. Useful if the
    // user remaps `:` and forgets the new chord; typing it through
    // the palette still works (assuming they can reach the palette
    // another way, e.g. via a separate binding pointing at the same
    // Action).
    PaletteEntry {
      action: Action::CommandPalette,
      name: "command-palette",
      description: "this command palette",
    },
  ]
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Pure state for the palette overlay. Owns the input buffer, the
/// fuzzy-filtered match list (cached per buffer change), and the
/// highlight index. No terminal or ratatui dependency so the event
/// loop in `src/tui/mod.rs` can drive it through method calls and
/// `tests/palette_tests.rs` can pin behaviour without spawning a TUI.
#[derive(Debug)]
pub struct PaletteState {
  pub open: bool,
  buffer: String,
  /// Cached indices into `palette_entries()` matching `buffer`.
  /// Rebuilt by [`Self::recompute_matches`] whenever the buffer
  /// changes. Empty when `buffer` is non-empty and no entry fuzzy-
  /// matches; equal to `0..palette_entries().len()` when the buffer
  /// is empty.
  matches: Vec<usize>,
  highlight: usize,
}

impl PaletteState {
  pub fn new() -> Self {
    let mut s = Self {
      open: false,
      buffer: String::new(),
      matches: Vec::new(),
      highlight: 0,
    };
    s.recompute_matches();
    s
  }

  pub fn buffer(&self) -> &str {
    &self.buffer
  }

  pub fn highlight(&self) -> usize {
    self.highlight
  }

  /// Currently-visible entries in match-rank order. Whatever the
  /// renderer paints is exactly this list — sharing the slice keeps
  /// the highlight index meaningful in both contexts.
  pub fn matches(&self) -> Vec<&'static PaletteEntry> {
    self.matches.iter().map(|&i| &palette_entries()[i]).collect()
  }

  /// Open the palette with an empty buffer and the full registry
  /// visible. Called from the event loop when `:` (the default
  /// binding of `Action::CommandPalette`) fires.
  pub fn open(&mut self) {
    self.open = true;
    self.buffer.clear();
    self.highlight = 0;
    self.recompute_matches();
  }

  /// Close the palette without firing anything. Called on `Esc` and
  /// after a successful [`Self::accept`].
  pub fn close(&mut self) {
    self.open = false;
    self.buffer.clear();
    self.highlight = 0;
    self.recompute_matches();
  }

  pub fn push_char(&mut self, c: char) {
    self.buffer.push(c);
    self.recompute_matches();
    self.highlight = 0;
  }

  pub fn pop_char(&mut self) {
    self.buffer.pop();
    self.recompute_matches();
    self.highlight = 0;
  }

  /// Move the highlight one row down, wrapping to the top when it
  /// runs off the end of the visible matches. No-op when the match
  /// list is empty.
  pub fn cycle_highlight_down(&mut self) {
    if self.matches.is_empty() {
      return;
    }
    self.highlight = (self.highlight + 1) % self.matches.len();
  }

  /// Move the highlight one row up, wrapping to the bottom on
  /// underflow. No-op when the match list is empty.
  pub fn cycle_highlight_up(&mut self) {
    if self.matches.is_empty() {
      return;
    }
    self.highlight = if self.highlight == 0 {
      self.matches.len() - 1
    } else {
      self.highlight - 1
    };
  }

  /// Fire the highlighted entry. Returns its `Action` and closes the
  /// palette on success; returns `None` and leaves the palette open
  /// when there is no match (the user typed something that filters
  /// every entry out — better to let them backspace than to silently
  /// drop the keystroke).
  pub fn accept(&mut self) -> Option<Action> {
    if self.matches.is_empty() {
      return None;
    }
    let idx = *self.matches.get(self.highlight)?;
    let action = palette_entries()[idx].action;
    self.close();
    Some(action)
  }

  fn recompute_matches(&mut self) {
    let registry = palette_entries();
    if self.buffer.is_empty() {
      self.matches = (0..registry.len()).collect();
      return;
    }
    // Reuse the same nucleo matcher configuration the worktree
    // filter uses (smart case, smart normalisation) so the palette
    // and the `/` filter rank identical queries identically.
    let pattern = Pattern::parse(&self.buffer, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(NucleoConfig::DEFAULT);
    let mut buf: Vec<char> = Vec::new();
    let mut scored: Vec<(u32, usize)> = Vec::with_capacity(registry.len());
    for (i, entry) in registry.iter().enumerate() {
      let hay = Utf32Str::new(entry.name, &mut buf);
      if let Some(score) = pattern.score(hay, &mut matcher) {
        scored.push((score, i));
      }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    self.matches = scored.into_iter().map(|(_, i)| i).collect();
  }
}

impl Default for PaletteState {
  fn default() -> Self {
    Self::new()
  }
}
