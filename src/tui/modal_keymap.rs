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
//! Bindings live under `[tui.keys.modal.<context-path>]` in `.gwm.toml`,
//! nested below a dedicated `modal` namespace inside the global `[tui.keys]`
//! table. The separate namespace keeps a modal context from colliding with a
//! same-named global action (`create` / `help` / `command_logs` / `link` are
//! both) at the `tui.keys.<name>` path — a collision the layered merge would
//! otherwise resolve by silently dropping the global override (issue #219
//! review):
//!
//! ```toml
//! [tui.keys]                  # global verbs — arrays, unchanged
//! quit   = ["q"]
//! create = ["c"]              # global action; coexists with the modal below
//!
//! [tui.keys.modal.confirm]    # contextual verbs — single strokes
//! confirm = ["y"]
//! cancel  = ["n", "Esc"]
//!
//! [tui.keys.modal.link.choose_target]
//! issue = ["i"]
//! pr    = ["p"]
//! ```
//!
//! The walker that turns that TOML into a [`ModalKeymap`] lives in
//! [`crate::config`]; this module owns the typed model, the defaults, the
//! per-context conflict validation, and the single-stroke resolver.

use crate::error::{GwmError, Result};
use crate::tui::keymap::{KeyStroke, Source};
use crossterm::event::{KeyCode, KeyModifiers};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// KeyContext
// ---------------------------------------------------------------------------

/// One modal / overlay surface whose keys are independently rebindable.
///
/// `config_path` is the dotted key under `[tui.keys.modal]` that addresses
/// the context's sub-table (`confirm`, `link.choose_target`, `config.edit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyContext {
  /// Create-worktree modal (also reused by the rename / `View::Edit` modal).
  Create,
  /// Delete-confirmation modal.
  Confirm,
  /// Keybindings / help overlay (scroll-only).
  Help,
  /// Generic detail overlay (issue #408, scroll-only / close) — agent
  /// sessions today, the rich PR/Issue view tomorrow.
  Detail,
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
  /// Exec profile picker overlay (issue #325).
  ExecPicker,
  /// Clean reclaim overlay (issue #325).
  Clean,
  /// CI checks overlay (issue #436) — the detail-overlay shell opened on
  /// the linked PR's per-check rollup list.
  CiChecks,
}

impl KeyContext {
  /// Strokes an ALWAYS-typing context reserves for its input (Codex
  /// review #456): the dispatch routes them into the query / number /
  /// value before the modal resolution, so a verb bound to one would be
  /// unreachable — and `close = ["x"]` would leave the overlay with no
  /// exit at all. [`ModalKeymap::apply_override`] refuses such bindings
  /// up front. ConfigEdit qualifies too (iteration 13): the context only
  /// exists while a value edit is live, and a text field consumes every
  /// unmodified printable (uppercase included) plus Backspace — its two
  /// verbs are the edit's only exits. Create is exempt at the context
  /// level (its type-cycling verbs live on the Type field, which takes
  /// no text input) — the per-verb exception is
  /// [`ModalAction::reserved_typing_stroke`]. Mirrors the dispatch
  /// routes (`App::palette_input_key`, the link number stage,
  /// `App::settings_edit_input_key`).
  pub fn reserved_typing_stroke(self, stroke: &KeyStroke) -> bool {
    use crossterm::event::{KeyCode as KC, KeyModifiers as KM};
    if stroke.modifiers.intersects(KM::CONTROL | KM::ALT) {
      return false;
    }
    match (self, stroke.code) {
      (KeyContext::CommandPalette | KeyContext::LinkInputNumber | KeyContext::ConfigEdit, KC::Backspace) => true,
      // A shifted letter is an uppercase (kitty-style) — not palette
      // input, so it stays bindable.
      (KeyContext::CommandPalette, KC::Char(c)) if stroke.modifiers.contains(KM::SHIFT) => c.is_ascii_digit(),
      (KeyContext::CommandPalette, KC::Char(c)) => c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-',
      (KeyContext::LinkInputNumber, KC::Char(c)) => c.is_ascii_digit(),
      // A text field takes uppercase input, so unlike the palette a
      // shifted letter IS typing here.
      (KeyContext::ConfigEdit, KC::Char(_)) => true,
      _ => false,
    }
  }

  /// Dotted key under `[tui.keys.modal]` addressing this context's sub-table.
  pub fn config_path(self) -> &'static str {
    match self {
      KeyContext::Create => "create",
      KeyContext::Confirm => "confirm",
      KeyContext::Help => "help",
      KeyContext::Detail => "detail",
      KeyContext::CommandLogs => "command_logs",
      KeyContext::Config => "config",
      KeyContext::ConfigEdit => "config.edit",
      KeyContext::Report => "report",
      KeyContext::OpenMenu => "open_menu",
      KeyContext::CommandPalette => "palette",
      KeyContext::LinkChooseTarget => "link.choose_target",
      KeyContext::LinkInputNumber => "link.input_number",
      KeyContext::ExecPicker => "exec",
      KeyContext::Clean => "clean",
      KeyContext::CiChecks => "ci_checks",
    }
  }

  /// Inverse of [`Self::config_path`] — used by the config walker to map a
  /// `[tui.keys.modal.<path>]` sub-table back to a typed context.
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
      Detail,
      CommandLogs,
      Config,
      ConfigEdit,
      Report,
      OpenMenu,
      CommandPalette,
      LinkChooseTarget,
      LinkInputNumber,
      ExecPicker,
      Clean,
      CiChecks,
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

      /// The context-local verb slug used under `[tui.keys.modal.<context>]`.
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
    // Issue #416. Ctrl-modified on purpose: the create overlay reserves
    // unmodified printable keys for the text fields, so a bare letter here
    // would be swallowed while typing a description.
    CreateToggleMode => "toggle_mode" [ "Ctrl+t" ],
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
  Detail {
    DetailClose      => "close"        [ "Esc", "q" ],
    DetailSelectNext => "select_next"  [ "Down", "j" ],
    DetailSelectPrev => "select_prev"  [ "Up", "k" ],
    DetailAttach     => "attach"       [ "a" ],
    DetailDetach     => "detach"       [ "d" ],
    DetailInput      => "attach_by_id" [ "i" ],
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
  ExecPicker {
    ExecPickerNext   => "next"   [ "j", "Down" ],
    ExecPickerPrev   => "prev"   [ "k", "Up" ],
    ExecPickerAccept => "accept" [ "Enter" ],
    ExecPickerCancel => "cancel" [ "Esc" ],
  }
  Clean {
    CleanNext    => "next"    [ "j", "Down" ],
    CleanPrev    => "prev"    [ "k", "Up" ],
    CleanConfirm => "confirm" [ "y", "Enter" ],
    CleanCancel  => "cancel"  [ "n", "Esc" ],
  }
  // #436: the defaults mirror the list view's own keys — `/` filters and
  // `f` refreshes there too (user feedback 2026-07-24).
  CiChecks {
    CiChecksClose   => "close"       [ "Esc", "q" ],
    CiChecksNext    => "select_next" [ "j", "Down" ],
    CiChecksPrev    => "select_prev" [ "k", "Up" ],
    CiChecksOpen    => "open"        [ "Enter" ],
    CiChecksFilter  => "filter"      [ "/" ],
    CiChecksRefresh => "refresh"     [ "f" ],
  }
}

impl ModalAction {
  /// Resolve a `(context, verb-slug)` pair to a typed verb. Used by the
  /// config walker to translate `[tui.keys.modal.<context>].<verb>` keys.
  pub fn from_context_verb(ctx: KeyContext, verb: &str) -> Option<Self> {
    Self::all().find(|a| a.context() == ctx && a.verb() == verb)
  }

  /// `true` when binding this verb to `stroke` would leave it unreachable
  /// or misleading because a typing route consumes the key first (Codex
  /// review #456). Context-wide reservations
  /// ([`KeyContext::reserved_typing_stroke`]) apply to every verb; the
  /// create verbs that must stay operative from the TEXT fields — submit
  /// (only ever fires from Description), cancel and the field navigation
  /// — add a per-verb case: every unmodified printable and Backspace is
  /// typing there. Only the type-cycling verbs keep bare letters: they
  /// act on the Type field, which takes no text input.
  pub fn reserved_typing_stroke(self, stroke: &KeyStroke) -> bool {
    use crossterm::event::{KeyCode as KC, KeyModifiers as KM};
    if self.context().reserved_typing_stroke(stroke) {
      return true;
    }
    matches!(
      self,
      ModalAction::CreateSubmit
        | ModalAction::CreateCancel
        | ModalAction::CreateNextField
        | ModalAction::CreatePrevField
        // #416: free-form mode has `Name` as its only field, so a bare
        // printable bound here would be swallowed as typing with no way
        // back to the structured form.
        | ModalAction::CreateToggleMode
    ) && !stroke.modifiers.intersects(KM::CONTROL | KM::ALT)
      && matches!(stroke.code, KC::Char(_) | KC::Backspace)
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
  let stroke = match strokes.into_iter().collect::<Vec<_>>().as_slice() {
    [one] => one.clone(),
    _ => {
      return Err(GwmError::Config(format!(
        "modal bindings must be a single keystroke, got chord {:?} (modals have no chord timeout)",
        s
      )))
    }
  };
  // Ctrl+C is the emergency quit handled in `run_app` ahead of every lookup,
  // so a modal binding to it would never fire — reject it rather than let
  // `gwm tui keys` / footer hints advertise an unreachable action (#219 review).
  if stroke.modifiers.contains(KeyModifiers::CONTROL)
    && matches!(stroke.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'c'))
  {
    return Err(GwmError::Config(format!(
      "modal bindings cannot use {:?}: Ctrl+C is the reserved emergency quit (handled before any modal lookup)",
      s
    )));
  }
  Ok(stroke)
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
    // Refuse a binding the reserved typing would swallow (Codex review
    // #456): with `palette.close = ["x"]` the override replaces Esc, then
    // the filter typing consumes `x` — the overlay is left with no exit
    // short of Ctrl-C. Better a clear config error up front.
    for k in &keys {
      if action.reserved_typing_stroke(k) {
        return Err(GwmError::Config(format!(
          "context {}: key {} is reserved for typing input there and cannot be bound to {} — \
           the dispatch routes it into the input before the modal resolution",
          ctx.config_path(),
          k,
          action.verb()
        )));
      }
    }
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
          "context {}: key {} bound to both {:?} and {:?} — conflict",
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
