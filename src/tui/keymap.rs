//! Configurable TUI keymap (issue #87).
//!
//! Three layers, in order of authority:
//!
//! 1. **Built-in defaults** ([`Keymap::defaults`]) — the bindings the
//!    binary ships with. Captures the historical hard-coded set
//!    (`j/k`, `g g`, `Tab`, `o`, `l`, `R`, `y`, `p`, `f/F`, `/`, `?`,
//!    `q`, …).
//! 2. **User overrides** ([`Keymap::apply_override`]) — fed from
//!    `[tui.keys]` in `.gwm.toml`. An override **replaces** the
//!    default for the targeted action; it does not merge. Passing an
//!    empty `Vec` unbinds the action entirely.
//! 3. **Hard-coded escape hatches** — `Ctrl+C` (emergency quit) and
//!    `Esc` / `Enter` keep their contextual handling in
//!    `src/tui/mod.rs`. They are deliberately outside the keymap
//!    because their semantics depend on the active view (filter bar,
//!    picker mode, sticky filter, etc.) — folding them into the
//!    keymap would require modal state the configuration language
//!    has no way to express.
//!
//! ## Chord / prefix policy
//!
//! Per the design decision recorded on PR #87, binding a chord that
//! is a strict prefix of another chord (e.g. `g` alone while `g g` is
//! also bound) is a **hard error at load time**. Resolving the
//! ambiguity at runtime would require a Vim-style 500 ms timeout in
//! the event loop, which conflicts with the project's preference for
//! a pure state-machine TUI. Easier to refuse the config and force
//! the user to pick.
//!
//! ## Quit policy
//!
//! `quit` is the only action with a guaranteed escape hatch: the
//! hard-coded `Ctrl+C` branch in `run_app` runs *before* any keymap
//! lookup, so even an empty / hostile user keymap can be exited. The
//! doctor check defined in `crate::doctor` emits a warning when no
//! non-`Ctrl+C` binding for `quit` survives the override layer.

use crate::error::{GwmError, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fmt;

// ---------------------------------------------------------------------------
// Action enum + ACTIONS table
// ---------------------------------------------------------------------------

/// Declarative macro that defines `Action`, its slug roundtrip, the
/// `ACTIONS` table, and `Action::all()` from a **single** ordered list
/// of `(Variant => "slug")` pairs. Adding a new action means editing
/// one place; the rest stays in sync mechanically.
macro_rules! define_actions {
  ($( $variant:ident => $slug:literal ),* $(,)?) => {
    /// Every user-rebindable verb the TUI exposes.
    ///
    /// Variants whose semantics depend on the active view (`Enter`,
    /// `Esc`, `Ctrl+C`) are deliberately **not** listed — see the
    /// module-level "Hard-coded escape hatches" note for why.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Action {
      $( $variant, )*
    }

    impl Action {
      /// Stable string slug used in `[tui.keys]`, `gwm tui keys`, and
      /// any future docs generator. Lowercase + underscores; never
      /// renamed in a backwards-incompatible way without a deprecation
      /// alias.
      pub fn slug(self) -> &'static str {
        match self {
          $( Action::$variant => $slug, )*
        }
      }

      /// Inverse of [`Action::slug`]. Used by the config loader to
      /// translate `.gwm.toml` keys into typed actions.
      pub fn from_slug(s: &str) -> Option<Self> {
        match s {
          $( $slug => Some(Action::$variant), )*
          _ => None,
        }
      }

      /// Iterator over every variant. Order matches the declarative
      /// macro invocation below, which is also the order surfaced by
      /// `gwm tui keys`.
      pub fn all() -> impl Iterator<Item = Self> {
        [ $( Action::$variant, )* ].into_iter()
      }
    }

    /// Static (Action, slug) table — convenience for callers that
    /// want both at once (the help-overlay renderer, `gwm tui keys`
    /// printer, `gwm doctor` reporter).
    pub const ACTIONS: &[(Action, &str)] = &[
      $( (Action::$variant, $slug), )*
    ];
  };
}

define_actions! {
  // Navigation
  Down              => "down",
  Up                => "up",
  Top               => "top",
  Bottom            => "bottom",
  ToggleSidebar     => "toggle_sidebar",
  ToggleSidebarMode => "toggle_sidebar_mode",
  FocusSwap         => "focus_swap",
  // Filter
  Filter            => "filter",
  // Lifecycle / mutating
  Refresh           => "refresh",
  Create            => "create",
  DeleteConfirm     => "delete",
  Bootstrap         => "bootstrap",
  ToggleDeleteBranch => "delete_branch",
  // Hand-offs
  GitTui            => "git_tui",
  Review            => "review",
  Yank              => "yank",
  Open              => "open",
  OpenMenu          => "open_menu",
  LinkPrompt        => "link",
  FetchGithub       => "fetch_github",
  // Overlays
  Help              => "help",
  Quit              => "quit",
  // Future surface — bound to ':' by default, picked up by #32.
  CommandPalette    => "command_palette",
}

// ---------------------------------------------------------------------------
// Key-string parser
// ---------------------------------------------------------------------------

/// One keystroke in a chord. Wraps a crossterm [`KeyCode`] +
/// [`KeyModifiers`] pair, but only retains the three modifier bits
/// that are meaningful at the keymap layer (Ctrl / Alt / Shift) —
/// other crossterm bits (keypad, repeat, …) are filtered in
/// [`KeyStroke::from_event`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyStroke {
  pub code: KeyCode,
  pub modifiers: KeyModifiers,
}

impl KeyStroke {
  pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
    Self {
      code,
      modifiers: Self::sanitize(modifiers),
    }
  }

  /// Build a stroke from a raw crossterm event, dropping modifier
  /// bits we never bind against (KEYPAD, REPEAT, SUPER, HYPER, META).
  pub fn from_event(ev: &KeyEvent) -> Self {
    Self::new(ev.code, ev.modifiers)
  }

  fn sanitize(m: KeyModifiers) -> KeyModifiers {
    m & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT)
  }

  /// Parse a chord string (`"j"`, `"g g"`, `"Ctrl+x Ctrl+s"`) into
  /// its sequence of keystrokes. Whitespace separates keystrokes;
  /// `+` separates modifiers from the key. Use `"Space"` for the
  /// literal space character.
  pub fn parse_chord(s: &str) -> Result<Vec<KeyStroke>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
      return Err(GwmError::Config(format!("keymap: empty key string {:?}", s)));
    }
    trimmed.split_whitespace().map(Self::parse_single).collect()
  }

  fn parse_single(token: &str) -> Result<KeyStroke> {
    if token.is_empty() {
      return Err(GwmError::Config("keymap: empty keystroke token".into()));
    }
    let parts: Vec<&str> = token.split('+').collect();
    if parts.iter().any(|p| p.is_empty()) {
      return Err(GwmError::Config(format!("keymap: dangling '+' in {:?}", token)));
    }
    let (key_str, mod_strs) = parts.split_last().expect("token is non-empty, split_last cannot fail");

    let mut modifiers = KeyModifiers::empty();
    for m in mod_strs {
      let bit = match *m {
        "Ctrl" => KeyModifiers::CONTROL,
        "Alt" => KeyModifiers::ALT,
        "Shift" => KeyModifiers::SHIFT,
        other => {
          return Err(GwmError::Config(format!(
            "keymap: unknown modifier {:?} in {:?}",
            other, token
          )))
        }
      };
      if modifiers.contains(bit) {
        return Err(GwmError::Config(format!(
          "keymap: duplicate modifier {:?} in {:?}",
          m, token
        )));
      }
      modifiers |= bit;
    }

    let code = parse_keycode(key_str, token)?;
    Ok(KeyStroke { code, modifiers })
  }
}

fn parse_keycode(s: &str, full_token: &str) -> Result<KeyCode> {
  let code = match s {
    "Tab" => KeyCode::Tab,
    "Enter" => KeyCode::Enter,
    "Esc" => KeyCode::Esc,
    "Up" => KeyCode::Up,
    "Down" => KeyCode::Down,
    "Left" => KeyCode::Left,
    "Right" => KeyCode::Right,
    "Backspace" => KeyCode::Backspace,
    "BackTab" => KeyCode::BackTab,
    "Home" => KeyCode::Home,
    "End" => KeyCode::End,
    "PageUp" => KeyCode::PageUp,
    "PageDown" => KeyCode::PageDown,
    "Insert" => KeyCode::Insert,
    "Delete" => KeyCode::Delete,
    "Space" => KeyCode::Char(' '),
    other if other.starts_with('F') && other.len() > 1 => {
      let n: u8 = other[1..]
        .parse()
        .map_err(|_| GwmError::Config(format!("keymap: invalid function key {:?}", other)))?;
      if !(1..=12).contains(&n) {
        return Err(GwmError::Config(format!(
          "keymap: function key out of range {:?} (expected F1..=F12)",
          other
        )));
      }
      KeyCode::F(n)
    }
    other => {
      let mut chars = other.chars();
      let (first, second) = (chars.next(), chars.next());
      match (first, second) {
        (Some(c), None) => KeyCode::Char(c),
        _ => {
          return Err(GwmError::Config(format!(
            "keymap: unknown key {:?} in {:?}",
            other, full_token
          )))
        }
      }
    }
  };
  Ok(code)
}

impl fmt::Display for KeyStroke {
  /// Canonical rendering used by `gwm tui keys` and the help overlay.
  /// Modifier order is always `Ctrl+Alt+Shift+<key>` so two bindings
  /// that compare equal also render identically.
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if self.modifiers.contains(KeyModifiers::CONTROL) {
      write!(f, "Ctrl+")?;
    }
    if self.modifiers.contains(KeyModifiers::ALT) {
      write!(f, "Alt+")?;
    }
    if self.modifiers.contains(KeyModifiers::SHIFT) {
      write!(f, "Shift+")?;
    }
    match self.code {
      KeyCode::Char(' ') => write!(f, "Space"),
      KeyCode::Char(c) => write!(f, "{c}"),
      KeyCode::Tab => write!(f, "Tab"),
      KeyCode::Enter => write!(f, "Enter"),
      KeyCode::Esc => write!(f, "Esc"),
      KeyCode::Up => write!(f, "Up"),
      KeyCode::Down => write!(f, "Down"),
      KeyCode::Left => write!(f, "Left"),
      KeyCode::Right => write!(f, "Right"),
      KeyCode::Backspace => write!(f, "Backspace"),
      KeyCode::BackTab => write!(f, "BackTab"),
      KeyCode::Home => write!(f, "Home"),
      KeyCode::End => write!(f, "End"),
      KeyCode::PageUp => write!(f, "PageUp"),
      KeyCode::PageDown => write!(f, "PageDown"),
      KeyCode::Insert => write!(f, "Insert"),
      KeyCode::Delete => write!(f, "Delete"),
      KeyCode::F(n) => write!(f, "F{n}"),
      other => write!(f, "{other:?}"),
    }
  }
}

fn format_chord(strokes: &[KeyStroke]) -> String {
  strokes.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Keymap
// ---------------------------------------------------------------------------

/// Where a given binding came from. Surfaces in `gwm tui keys` as the
/// third column so the user can audit what's hers vs. what shipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
  Default,
  UserConfig,
}

/// One entry in the resolved keymap: an action, the list of chords
/// that fire it (any one match suffices), and the source layer the
/// binding came from.
#[derive(Debug, Clone)]
pub struct Binding {
  pub action: Action,
  pub chords: Vec<Vec<KeyStroke>>,
  pub source: Source,
}

/// Outcome of [`Keymap::lookup`] for a given pending-keys buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum ChordResolution {
  /// Buffer exactly matches a bound chord. Caller fires the action
  /// and clears the buffer.
  Matched(Action),
  /// Buffer is the strict prefix of at least one bound chord.
  /// Caller keeps the buffer armed and waits for the next stroke.
  PendingPrefix,
  /// Buffer matches no binding and is not the prefix of any.
  /// Caller clears the buffer (and may retry the last stroke alone
  /// — that policy lives in the event loop, not here).
  NoMatch,
}

#[derive(Debug, Clone)]
pub struct Keymap {
  entries: Vec<Binding>,
}

impl Keymap {
  /// Built-in defaults. Mirrors the historical hard-coded set in
  /// `src/tui/mod.rs` before issue #87. Adding a default binding
  /// here automatically surfaces it in `gwm tui keys` and the help
  /// overlay.
  pub fn defaults() -> Self {
    let entries = vec![
      def(Action::Down, &["j", "Down"]),
      def(Action::Up, &["k", "Up"]),
      def(Action::Top, &["g g"]),
      def(Action::Bottom, &["G", "End"]),
      def(Action::ToggleSidebar, &["v"]),
      def(Action::ToggleSidebarMode, &["s"]),
      def(Action::FocusSwap, &["Tab"]),
      def(Action::Filter, &["/"]),
      def(Action::Refresh, &["f", "r"]),
      def(Action::Create, &["n"]),
      def(Action::DeleteConfirm, &["d"]),
      def(Action::Bootstrap, &["b"]),
      def(Action::ToggleDeleteBranch, &["p"]),
      def(Action::GitTui, &["l"]),
      def(Action::Review, &["R"]),
      def(Action::Yank, &["y"]),
      def(Action::Open, &["o"]),
      def(Action::OpenMenu, &["O"]),
      def(Action::LinkPrompt, &["L"]),
      def(Action::FetchGithub, &["F"]),
      def(Action::Help, &["?"]),
      def(Action::Quit, &["q"]),
      def(Action::CommandPalette, &[":"]),
    ];
    Self { entries }
  }

  /// Replace the chords bound to `action` with `chords` and re-validate
  /// the full keymap. An empty `Vec` unbinds the action. The validation
  /// pass rejects:
  ///
  /// - the same chord wired to two different actions (conflict);
  /// - a chord that is a strict prefix of another (`g` while `g g`
  ///   is bound) — see the module-level chord/prefix policy note.
  ///
  /// Returns `Err(GwmError::Config(_))` on failure; the keymap is
  /// left untouched on error so callers can surface the message and
  /// move on.
  pub fn apply_override(&mut self, action: Action, chords: Vec<Vec<KeyStroke>>) -> Result<()> {
    let mut candidate: Vec<(Action, Vec<Vec<KeyStroke>>)> = self
      .entries
      .iter()
      .map(|b| {
        if b.action == action {
          (b.action, chords.clone())
        } else {
          (b.action, b.chords.clone())
        }
      })
      .collect();
    if !candidate.iter().any(|(a, _)| *a == action) {
      candidate.push((action, chords.clone()));
    }
    Self::validate(&candidate)?;

    let mut replaced = false;
    for entry in self.entries.iter_mut() {
      if entry.action == action {
        entry.chords = chords.clone();
        entry.source = Source::UserConfig;
        replaced = true;
        break;
      }
    }
    if !replaced {
      self.entries.push(Binding {
        action,
        chords,
        source: Source::UserConfig,
      });
    }
    Ok(())
  }

  fn validate(entries: &[(Action, Vec<Vec<KeyStroke>>)]) -> Result<()> {
    let mut all: Vec<(&[KeyStroke], Action)> = Vec::new();
    for (action, chords) in entries {
      for chord in chords {
        if chord.is_empty() {
          return Err(GwmError::Config(format!(
            "keymap: empty chord bound to {:?}",
            action.slug()
          )));
        }
        all.push((chord.as_slice(), *action));
      }
    }
    for i in 0..all.len() {
      for j in (i + 1)..all.len() {
        if all[i].0 == all[j].0 {
          if all[i].1 != all[j].1 {
            return Err(GwmError::Config(format!(
              "keymap: chord {:?} bound to both {:?} and {:?} — conflict",
              format_chord(all[i].0),
              all[i].1.slug(),
              all[j].1.slug()
            )));
          }
          continue;
        }
        let (short, long) = if all[i].0.len() < all[j].0.len() {
          (i, j)
        } else {
          (j, i)
        };
        if all[short].0.len() < all[long].0.len() && all[long].0.starts_with(all[short].0) {
          return Err(GwmError::Config(format!(
            "keymap: chord {:?} (action {:?}) is a prefix of {:?} (action {:?}) — refused at load time so the event loop never has to time out",
            format_chord(all[short].0),
            all[short].1.slug(),
            format_chord(all[long].0),
            all[long].1.slug()
          )));
        }
      }
    }
    Ok(())
  }

  /// Resolve a pending-keys buffer against the keymap.
  pub fn lookup(&self, keys: &[KeyStroke]) -> ChordResolution {
    let mut pending = false;
    for entry in &self.entries {
      for chord in &entry.chords {
        if chord.as_slice() == keys {
          return ChordResolution::Matched(entry.action);
        }
        if chord.len() > keys.len() && chord.starts_with(keys) {
          pending = true;
        }
      }
    }
    if pending {
      ChordResolution::PendingPrefix
    } else {
      ChordResolution::NoMatch
    }
  }

  /// Snapshot the resolved keymap for `gwm tui keys` / help overlay.
  /// Order matches the declarative `define_actions!` invocation so the
  /// rendered table stays stable across runs.
  pub fn list(&self) -> Vec<Binding> {
    self.entries.clone()
  }
}

/// Build a default `Binding` from a list of chord literals. Panics
/// if a literal does not parse — that is a programmer error in the
/// defaults table, never user input.
fn def(action: Action, chord_literals: &[&str]) -> Binding {
  let chords = chord_literals
    .iter()
    .map(|s| {
      KeyStroke::parse_chord(s).unwrap_or_else(|e| panic!("default keymap chord {:?} failed to parse: {}", s, e))
    })
    .collect();
  Binding {
    action,
    chords,
    source: Source::Default,
  }
}
