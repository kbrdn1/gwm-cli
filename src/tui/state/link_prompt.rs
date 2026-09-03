//! Two-stage issue/PR link prompt state (issue #67, extracted from
//! `tui::app::App` per #126 / #102).
//!
//! Pure state — no `App` / `Repository` / `Config` dependency. The
//! prompt owns three things:
//!
//! 1. **`stage`** — `ChooseTarget` (user is picking Issue / Pr) or
//!    `InputNumber` (user is typing the issue / PR number).
//! 2. **`target`** — `Some(Issue|Pr)` once a target has been committed,
//!    `None` while still in `ChooseTarget`.
//! 3. **`number`** — the digits the user has typed so far during
//!    `InputNumber`. Digits-only; the push/pop primitives enforce that
//!    so the orchestrator's `parse::<u64>` on submit never has to defend
//!    against garbage.
//!
//! The `App` orchestrator owns the side-effecting `link_prompt_submit`
//! (which composes `github::link_issue` / `github::link_pr` against
//! `self.repo`, refreshes the cached link, and assigns the status-bar
//! copy). Enter / Esc / view transitions are also driven by the
//! orchestrator — this module only knows the buffer and the stage
//! machine.
//!
//! `LinkTarget` is imported from `tui::app` on this branch (which still
//! holds the canonical TUI-side definition). PR #132 also dedupes
//! `LinkTarget` against `cli::LinkTarget`, but that's a parallel branch
//! off `dev` — once #132 merges, the import here will resolve through
//! the re-export the dedupe pass installs in `tui::app`, so no further
//! change is needed at the call site.

use crate::tui::app::LinkTarget;

/// Stage of the two-step link prompt. `ChooseTarget` is the entry state
/// (the user rotates / picks); `InputNumber` is the typing state (the
/// user enters digits, then Enter to submit or Esc to cancel).
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum LinkPromptStage {
  #[default]
  ChooseTarget,
  InputNumber,
}

/// Pure state for the two-stage issue/PR link prompt. `Default` opens
/// the prompt in the initial state (stage = ChooseTarget, no committed
/// target, `Issue` highlighted, empty number buffer).
#[derive(Debug)]
pub struct LinkPrompt {
  /// Which stage of the prompt we're in. Drives the keypress dispatch
  /// (j/k/Enter/i/p during ChooseTarget, digits/Enter/Backspace during
  /// InputNumber) at the event-loop layer.
  pub stage: LinkPromptStage,
  /// Committed target. `None` while in `ChooseTarget`, `Some(Issue|Pr)`
  /// after [`Self::commit_target`]. Distinct from [`Self::selected`]:
  /// the highlight can move freely while nothing is committed yet.
  pub target: Option<LinkTarget>,
  /// The highlighted row in the `ChooseTarget` vertical picker (#217).
  /// `j`/`k` move it via [`Self::toggle_selection`]; `Enter` commits it.
  /// Opens on `Issue` — most branches link an issue first; a PR comes
  /// later when the branch is pushed. Frozen once a target is committed
  /// so a stray keystroke during `InputNumber` can't flip it.
  pub selected: LinkTarget,
  /// Digits typed by the user during `InputNumber`. Always digits-only:
  /// [`Self::push_char`] drops non-digits, [`Self::pop_char`] is the
  /// backspace handler. The orchestrator's submit handler calls
  /// `.parse::<u64>()` on this directly.
  pub number: String,
}

impl Default for LinkPrompt {
  fn default() -> Self {
    Self {
      stage: LinkPromptStage::default(),
      target: None,
      // `LinkTarget` is a shared `cli::ValueEnum` with no `Default`, so the
      // open-highlight default lives here rather than on that CLI type.
      selected: LinkTarget::Issue,
      number: String::new(),
    }
  }
}

impl LinkPrompt {
  pub fn new() -> Self {
    Self::default()
  }

  /// Return to the freshly-opened state (stage = ChooseTarget, no
  /// committed target, `Issue` highlighted, empty buffer). Called by the
  /// orchestrator when the prompt opens, cancels, or completes a
  /// successful submit.
  pub fn reset(&mut self) {
    self.stage = LinkPromptStage::ChooseTarget;
    self.target = None;
    self.selected = LinkTarget::Issue;
    self.number.clear();
  }

  /// Move the highlight in the `ChooseTarget` picker. With exactly two
  /// targets, `j` (down) and `k` (up) land on the same other row, so a
  /// single flip covers both. No-op during `InputNumber` so a stray
  /// keystroke after commit can't flip the user's choice mid-typing.
  pub fn toggle_selection(&mut self) {
    if self.stage != LinkPromptStage::ChooseTarget {
      return;
    }
    self.selected = match self.selected {
      LinkTarget::Issue => LinkTarget::Pr,
      LinkTarget::Pr => LinkTarget::Issue,
    };
  }

  /// Point the highlight straight at `target` (issue #624 — a click lands on
  /// a row, not one step away). Same stage guard as
  /// [`Self::toggle_selection`]: a click landing on the picker's old rows
  /// after it advanced to `InputNumber` must not flip a committed choice.
  pub fn select_target(&mut self, target: LinkTarget) {
    if self.stage != LinkPromptStage::ChooseTarget {
      return;
    }
    self.selected = target;
  }

  /// Commit a target and advance to `InputNumber`. Clears the buffer so
  /// the user starts fresh on the digit input. Called from the
  /// orchestrator's `link_prompt_choose` wrapper, which also updates
  /// the status bar with the per-target hint.
  pub fn commit_target(&mut self, target: LinkTarget) {
    self.target = Some(target);
    self.stage = LinkPromptStage::InputNumber;
    self.number.clear();
  }

  /// Append a digit to the number buffer. No-op during `ChooseTarget`,
  /// and drops any non-digit char during `InputNumber` so the
  /// orchestrator's `parse::<u64>()` on submit never has to defend
  /// against garbage.
  pub fn push_char(&mut self, c: char) {
    if self.stage == LinkPromptStage::InputNumber && c.is_ascii_digit() {
      self.number.push(c);
    }
  }

  /// Pop the last digit off the number buffer. Backspace handler.
  /// No-op during `ChooseTarget` so a stray backspace can't surprise-
  /// empty a buffer the user already typed into during a previous
  /// open cycle (the orchestrator's `enter_link_prompt` does call
  /// `reset()`, but defence in depth).
  pub fn pop_char(&mut self) {
    if self.stage == LinkPromptStage::InputNumber {
      self.number.pop();
    }
  }
}
