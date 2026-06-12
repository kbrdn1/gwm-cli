//! Contextual modal / overlay keymap (issue #219).
//!
//! The global keymap in [`crate::tui::keymap`] resolves the `View::List`
//! verbs (`j`, `g g`, `o`, `R`, …) and, deliberately, *only* those: its
//! module note explains why `Esc` / `Enter` were kept hard-coded — their
//! meaning depends on the active view, and a single global table cannot
//! express "Enter submits in the create modal but activates the focused
//! button in the confirm modal".
//!
//! This module lifts that limitation for the modals/overlays. Every modal
//! is a [`KeyContext`]; each context owns a small set of typed verbs
//! ([`ModalAction`]); the same physical key can map to different verbs in
//! different contexts without any global conflict, because resolution is
//! always scoped to the **active** context.
//!
//! ## Single-stroke only
//!
//! Unlike the global keymap, modal bindings are **single keystrokes** —
//! no chords, no prefixes, no pending buffer. Modals are short-lived and
//! the project refuses a Vim-style runtime timeout (see the global
//! keymap's chord/prefix note). A user override that supplies a
//! multi-stroke chord (`"g g"`) is rejected at load time with a precise
//! message rather than silently never firing.
//!
//! ## What stays hard-coded
//!
//! - `Ctrl+C` — the emergency quit in `run_app`, ahead of every lookup.
//! - `View::List`'s `Esc` / `Enter` — still contextual on filter / picker
//!   / sticky-filter state, which the config language cannot express.
//! - The PTY overlay's `Esc` — it is an *emergency* detach from a child
//!   that otherwise receives every keystroke; making it rebindable would
//!   silently steal a key from lazygit / the shell. Documented in the
//!   `View::Pty` branch.
//!
//! ## Config surface
//!
//! Bindings live under `[tui.keys.<context-path>]` in `.gwm.toml`, nested
//! below the existing global `[tui.keys]` table:
//!
//! ```toml
//! [tui.keys]            # global verbs — unchanged
//! quit = ["q"]
//!
//! [tui.keys.confirm]    # contextual verbs — new
//! confirm = ["y"]
//! cancel  = ["n", "Esc"]
//!
//! [tui.keys.link.choose_target]
//! issue = ["i"]
//! pr    = ["p"]
//! ```
//!
//! The walker that turns that TOML into a [`ModalKeymap`] lives in
//! [`crate::config`]; this module owns the typed model, the defaults, the
//! per-context conflict validation, and the single-stroke resolver.

use crate::error::{GwmError, Result};
use crate::tui::keymap::{KeyStroke, Source};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// KeyContext
// ---------------------------------------------------------------------------

/// One modal / overlay surface whose keys are independently rebindable.
///
/// `config_path` is the dotted key under `[tui.keys]` that addresses the
/// context's sub-table (`confirm`, `link.choose_target`, `config.edit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyContext {
  /// Create-worktree modal (also reused by the rename / `View::Edit` modal).
  Create,
  /// Delete-confirmation modal.
  Confirm,
  /// Keybindings / help overlay (scroll-only).
  Help,
  /// Command-logs overlay (issue #226, scroll-only + copy).
  CommandLogs,
  /// Settings panel navigation (issue #232).
  Config,
  /// Settings panel while a numeric field is being edited (sub-mode of
  /// [`KeyContext::Config`]; a separate context because `Enter` means
  /// *commit* here but *activate* in nav).
  ConfigEdit,
  /// Bootstrap-report overlay (scroll-only / close).
  Report,
  /// Browse-links menu (issue #224 / #290).
  OpenMenu,
  /// Command palette overlay (issue #32).
  CommandPalette,
  /// Link prompt, stage 1 — choose issue vs PR.
  LinkChooseTarget,
  /// Link prompt, stage 2 — type the issue / PR number.
  LinkInputNumber,
}

impl KeyContext {
  /// Dotted key under `[tui.keys]` addressing this context's sub-table.
  pub fn config_path(self) -> &'static str {
    match self {
      KeyContext::Create => "create",
      KeyContext::Confirm => "confirm",
      KeyContext::Help => "help",
      KeyContext::CommandLogs => "command_logs",
      KeyContext::Config => "config",
      KeyContext::ConfigEdit => "config.edit",
      KeyContext::Report => "report",
      KeyContext::OpenMenu => "open_menu",
      KeyContext::CommandPalette => "palette",
      KeyContext::LinkChooseTarget => "link.choose_target",
      KeyContext::LinkInputNumber => "link.input_number",
    }
  }

  /// Inverse of [`Self::config_path`] — used by the config walker to map a
  /// `[tui.keys.<path>]` sub-table back to a typed context.
  pub fn from_config_path(path: &str) -> Option<Self> {
    Self::all().iter().copied().find(|c| c.config_path() == path)
  }

  /// Every context, in declaration order (the order `gwm tui keys` lists).
  pub fn all() -> &'static [KeyContext] {
    use KeyContext::*;
    &[
      Create,
      Confirm,
      Help,
      CommandLogs,
      Config,
      ConfigEdit,
      Report,
      OpenMenu,
      CommandPalette,
      LinkChooseTarget,
      LinkInputNumber,
    ]
  }
}

// ---------------------------------------------------------------------------
// ModalAction + defaults table
// ---------------------------------------------------------------------------

/// Declarative definition of every modal verb, grouped by context, with its
/// local verb slug and built-in default keystrokes. One ordered list keeps
/// the enum, the `context`/`verb`/`default` accessors, and `all()` in sync.
macro_rules! define_modal_actions {
  ( $( $ctx:ident { $( $variant:ident => $verb:literal [ $( $chord:literal ),* $(,)? ] ),* $(,)? } )* ) => {
    /// A context-qualified modal verb. Variant names are
    /// `<Context><Verb>` so the flat enum stays unambiguous; the
    /// `(context, verb)` pair is what the config surface addresses.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ModalAction {
      $( $( $variant, )* )*
    }

    impl ModalAction {
      /// The context this verb belongs to.
      pub fn context(self) -> KeyContext {
        match self { $( $( ModalAction::$variant => KeyContext::$ctx, )* )* }
      }

      /// The context-local verb slug used under `[tui.keys.<context>]`.
      pub fn verb(self) -> &'static str {
        match self { $( $( ModalAction::$variant => $verb, )* )* }
      }

      /// Built-in default keystroke literals (each a single stroke).
      fn default_chord_strs(self) -> &'static [&'static str] {
        match self { $( $( ModalAction::$variant => &[ $( $chord, )* ], )* )* }
      }

      /// Every verb, in declaration order.
      pub fn all() -> impl Iterator<Item = Self> {
        [ $( $( ModalAction::$variant, )* )* ].into_iter()
      }
    }
  };
}

define_modal_actions! {
  Create {
    CreateCancel    => "cancel"     [ "Esc" ],
    CreateNextField => "next_field" [ "Tab" ],
    CreatePrevField => "prev_field" [ "BackTab" ],
    CreateSubmit    => "submit"     [ "Enter" ],
    CreatePrevType  => "prev_type"  [ "Up", "Left", "h" ],
    CreateNextType  => "next_type"  [ "Down", "Right", "l" ],
  }
  Confirm {
    ConfirmConfirm      => "confirm"       [ "y" ],
    ConfirmActivate     => "activate"      [ "Enter" ],
    ConfirmCancel       => "cancel"        [ "n", "Esc" ],
    ConfirmFocusConfirm => "focus_confirm" [ "Left", "h" ],
    ConfirmFocusCancel  => "focus_cancel"  [ "Right", "l" ],
    ConfirmToggleFocus  => "toggle_focus"  [ "Tab" ],
  }
  Help {
    HelpClose        => "close"         [ "Esc", "q", "?" ],
    HelpScrollDown   => "scroll_down"   [ "Down", "j" ],
    HelpScrollUp     => "scroll_up"     [ "Up", "k" ],
    HelpScrollRight  => "scroll_right"  [ "Right", "l" ],
    HelpScrollLeft   => "scroll_left"   [ "Left", "h" ],
    HelpScrollTop    => "scroll_top"    [ "Home", "g" ],
    HelpScrollBottom => "scroll_bottom" [ "End", "G" ],
  }
  CommandLogs {
    CommandLogsClose        => "close"         [ "Esc", "q" ],
    CommandLogsCopy         => "copy"          [ "y" ],
    CommandLogsScrollDown   => "scroll_down"   [ "Down", "j" ],
    CommandLogsScrollUp     => "scroll_up"     [ "Up", "k" ],
    CommandLogsScrollRight  => "scroll_right"  [ "Right", "l" ],
    CommandLogsScrollLeft   => "scroll_left"   [ "Left", "h" ],
    CommandLogsScrollTop    => "scroll_top"    [ "Home", "g" ],
    CommandLogsScrollBottom => "scroll_bottom" [ "End", "G" ],
  }
  Config {
    ConfigClose        => "close"         [ "Esc", "q" ],
    ConfigNextTab      => "next_tab"      [ "Tab" ],
    ConfigPrevTab      => "prev_tab"      [ "BackTab" ],
    ConfigToggleLayer  => "toggle_layer"  [ "L" ],
    ConfigActivate     => "activate"      [ "Space", "Enter" ],
    ConfigSelectNext   => "select_next"   [ "Down", "j" ],
    ConfigSelectPrev   => "select_prev"   [ "Up", "k" ],
    ConfigScrollRight  => "scroll_right"  [ "Right", "l" ],
    ConfigScrollLeft   => "scroll_left"   [ "Left", "h" ],
    ConfigScrollTop    => "scroll_top"    [ "Home", "g" ],
    ConfigScrollBottom => "scroll_bottom" [ "End", "G" ],
  }
  ConfigEdit {
    ConfigEditSubmit => "submit" [ "Enter" ],
    ConfigEditCancel => "cancel" [ "Esc" ],
  }
  Report {
    ReportClose => "close" [ "Esc", "q", "Enter" ],
  }
  OpenMenu {
    OpenMenuClose  => "close"  [ "Esc", "q" ],
    OpenMenuToggle => "toggle" [ "j", "k", "Down", "Up" ],
    OpenMenuAccept => "accept" [ "Enter" ],
    OpenMenuIssue  => "issue"  [ "i" ],
    OpenMenuPr     => "pr"     [ "p" ],
  }
  CommandPalette {
    CommandPaletteClose  => "close"  [ "Esc" ],
    CommandPaletteAccept => "accept" [ "Enter" ],
    CommandPalettePrev   => "prev"   [ "Up" ],
    CommandPaletteNext   => "next"   [ "Down", "Tab" ],
  }
  LinkChooseTarget {
    LinkChooseNext   => "next"   [ "j", "Down" ],
    LinkChoosePrev   => "prev"   [ "k", "Up" ],
    LinkChooseIssue  => "issue"  [ "i" ],
    LinkChoosePr     => "pr"     [ "p" ],
    LinkChooseAccept => "accept" [ "Enter" ],
    LinkChooseCancel => "cancel" [ "Esc" ],
  }
  LinkInputNumber {
    LinkInputSubmit => "submit" [ "Enter" ],
    LinkInputCancel => "cancel" [ "Esc" ],
  }
}

impl ModalAction {
  /// Resolve a `(context, verb-slug)` pair to a typed verb. Used by the
  /// config walker to translate `[tui.keys.<context>].<verb>` keys.
  pub fn from_context_verb(ctx: KeyContext, verb: &str) -> Option<Self> {
    Self::all().find(|a| a.context() == ctx && a.verb() == verb)
  }

  /// Default keystrokes for this verb, parsed. Panics on a malformed
  /// literal — that is a programmer error in the table above, never user
  /// input (same contract as the global keymap's `def`).
  fn default_keys(self) -> Vec<KeyStroke> {
    self
      .default_chord_strs()
      .iter()
      .map(|s| {
        parse_single(s)
          .unwrap_or_else(|e| panic!("default modal binding {:?} for {:?} failed to parse: {}", s, self, e))
      })
      .collect()
  }
}

/// Parse a binding string that must resolve to exactly one keystroke.
/// Modal bindings have no chord machinery, so a multi-stroke string is a
/// hard error (returned to the user verbatim by the config walker).
pub fn parse_single(s: &str) -> Result<KeyStroke> {
  let strokes = KeyStroke::parse_chord(s)?;
  match strokes.into_iter().collect::<Vec<_>>().as_slice() {
    [one] => Ok(one.clone()),
    _ => Err(GwmError::Config(format!(
      "modal bindings must be a single keystroke, got chord {:?} (modals have no chord timeout)",
      s
    ))),
  }
}

// ---------------------------------------------------------------------------
// ModalKeymap
// ---------------------------------------------------------------------------

/// One resolved binding: a verb, the single-strokes that fire it (any one
/// match suffices), and the layer it came from.
#[derive(Debug, Clone)]
pub struct ModalBinding {
  pub action: ModalAction,
  pub keys: Vec<KeyStroke>,
  pub source: Source,
}

/// The resolved contextual keymap: every modal verb with its keys. Built
/// from [`ModalKeymap::defaults`] then layered with user overrides via
/// [`ModalKeymap::apply_override`].
#[derive(Debug, Clone)]
pub struct ModalKeymap {
  entries: Vec<ModalBinding>,
}

impl ModalKeymap {
  /// Built-in defaults — mirror the historical hard-coded modal routing in
  /// `src/tui/mod.rs` before issue #219.
  pub fn defaults() -> Self {
    let entries = ModalAction::all()
      .map(|action| ModalBinding {
        action,
        keys: action.default_keys(),
        source: Source::Default,
      })
      .collect();
    Self { entries }
  }

  /// Replace the keys bound to `action` with `keys` and re-validate the
  /// action's **context**. An empty `Vec` unbinds the verb.
  ///
  /// Validation rejects the same stroke wired to two different verbs *in
  /// the same context* (cross-context reuse is fine — that is the whole
  /// point). A default binding in the same context silently vacates any
  /// stroke the override claims, mirroring the global keymap: explicit
  /// user intent wins over a shipped default.
  pub fn apply_override(&mut self, action: ModalAction, keys: Vec<KeyStroke>) -> Result<()> {
    let ctx = action.context();
    let claimed: Vec<&KeyStroke> = keys.iter().collect();

    // Build the post-override key→action map for this context (excluding the
    // action being replaced), vacating claimed strokes from defaults.
    let mut map: HashMap<KeyStroke, ModalAction> = HashMap::new();
    for b in &self.entries {
      if b.action.context() != ctx || b.action == action {
        continue;
      }
      for k in &b.keys {
        if b.source == Source::Default && claimed.contains(&k) {
          continue;
        }
        map.insert(k.clone(), b.action);
      }
    }
    for k in &keys {
      if let Some(prev) = map.get(k) {
        return Err(GwmError::Config(format!(
          "tui.keys.{}: key {} bound to both {:?} and {:?} — conflict",
          ctx.config_path(),
          k,
          prev.verb(),
          action.verb()
        )));
      }
    }

    // Commit: vacate claimed strokes from same-context defaults, then
    // replace the target verb's binding.
    let claimed_owned: Vec<KeyStroke> = keys.clone();
    for entry in self.entries.iter_mut() {
      if entry.action.context() == ctx && entry.action != action && entry.source == Source::Default {
        entry.keys.retain(|k| !claimed_owned.contains(k));
      }
    }
    if let Some(entry) = self.entries.iter_mut().find(|b| b.action == action) {
      entry.keys = keys;
      entry.source = Source::UserConfig;
    } else {
      self.entries.push(ModalBinding {
        action,
        keys,
        source: Source::UserConfig,
      });
    }
    Ok(())
  }

  /// Resolve a single keystroke against the bindings of `ctx`. Returns the
  /// matched verb, or `None` when nothing in this context binds the stroke
  /// (the caller then applies the context's text-input / default fallback).
  pub fn resolve(&self, ctx: KeyContext, stroke: &KeyStroke) -> Option<ModalAction> {
    self
      .entries
      .iter()
      .filter(|b| b.action.context() == ctx)
      .find(|b| b.keys.iter().any(|k| k == stroke))
      .map(|b| b.action)
  }

  /// Every binding whose verb lives in `ctx`, in declaration order.
  pub fn bindings_for(&self, ctx: KeyContext) -> Vec<&ModalBinding> {
    self.entries.iter().filter(|b| b.action.context() == ctx).collect()
  }

  /// Snapshot of every binding, declaration order, for `gwm tui keys` /
  /// the help overlay / `gwm doctor`.
  pub fn list(&self) -> &[ModalBinding] {
    &self.entries
  }

  /// The first key bound to `action`, rendered for an inline hint (the
  /// statusbar chip / help-overlay footer). `None` when the verb is
  /// unbound — the caller drops it rather than advertise a phantom key.
  /// Mirrors [`crate::tui::keymap::Keymap::primary_chord`].
  pub fn primary_key(&self, action: ModalAction) -> Option<String> {
    self
      .entries
      .iter()
      .find(|b| b.action == action)
      .and_then(|b| b.keys.first())
      .map(|k| k.to_string())
  }

  /// Every key bound to `action`, comma-joined (`"n, Esc"`) or empty when
  /// unbound — the help-overlay row form, matching the global keymap's
  /// `keys_for` rendering.
  pub fn keys_display(&self, action: ModalAction) -> String {
    self
      .entries
      .iter()
      .find(|b| b.action == action)
      .map(|b| b.keys.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(", "))
      .unwrap_or_default()
  }
}

impl Default for ModalKeymap {
  fn default() -> Self {
    Self::defaults()
  }
}
