//! Configurable TUI theme (issue #33).
//!
//! Role-based colours: every visual signal in the TUI maps to a
//! semantic role (`focus`, `accent`, `branch`, `clean`, `dirty`,
//! `main`, `locked`, `prunable`, `muted`, `selection_bg`, `name`,
//! `path`) rather than a hard-coded `Color::Cyan`. Users override
//! roles in `.gwm.toml`:
//!
//! ```toml
//! [theme]
//! preset = "catppuccin"
//! focus  = "#89b4fa"   # mocha blue override on top of the preset
//! ```
//!
//! Three layers, in order of authority:
//!
//! 1. [`Theme::default()`] — the pre-#33 hardcoded scheme. Users
//!    who omit `[theme]` see no change.
//! 2. [`Theme::preset(name)`] — built-in palette. The preset
//!    replaces every role; partial presets are not supported.
//! 3. [`Theme::apply_override`] — per-role override fed from
//!    `[theme]` entries. Overrides land on top of (1) or (2).
//!
//! Colour values accept three forms:
//!   - Named (`cyan`, `Cyan`, `dark_gray`, `bright_blue`) — case-
//!     insensitive.
//!   - 256-palette index (`0`..=`255`).
//!   - Hex (`#89b4fa`, `#0ff` short form is **not** supported in
//!     v1; the parser refuses to guess).
//!
//! Errors surface as [`GwmError::Config`] so the config loader can
//! attribute them to the right TOML coordinate.

use crate::error::{GwmError, Result};
use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Theme struct
// ---------------------------------------------------------------------------

/// Role-based colour scheme for the TUI. Every field is a
/// [`ratatui::style::Color`]; the renderer reads them via
/// `App.theme` rather than hard-coding palette values.
///
/// Order of fields here matches the documented `[theme]` key order
/// in `examples/gwm.toml.example` so the doc and the struct stay
/// readable side-by-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
  /// Focused border / cursor / active overlay highlight. Pre-#33
  /// hardcoded as `Color::Cyan`.
  pub focus: Color,
  /// General accent: header title, key hints in the help overlay,
  /// the palette input bar prompt. Pre-#33 hardcoded as `Cyan`.
  pub accent: Color,
  /// Branch name in lists and the sidebar identity card. Pre-#33
  /// hardcoded as `Green` when the branch is in a healthy state.
  pub branch: Color,
  /// "Working tree is clean" status indicator. Pre-#33 `Green`.
  pub clean: Color,
  /// "Working tree is dirty" status indicator. Pre-#33 `Yellow`.
  pub dirty: Color,
  /// Main / trunk worktree badge. Pre-#33 `Yellow`.
  pub main: Color,
  /// Locked worktree badge (`🔒`). Pre-#33 `Magenta`.
  pub locked: Color,
  /// Prunable worktree badge (`⚠`). Pre-#33 `Red`.
  pub prunable: Color,
  /// De-emphasised text: hints, footers, placeholders. Pre-#33
  /// `DarkGray`.
  pub muted: Color,
  /// Selection highlight background. Pre-#33 `DarkGray`. Kept as a
  /// separate role so themes that prefer a coloured selection
  /// background (e.g. catppuccin's surface tone) can move it
  /// independently of `muted`.
  pub selection_bg: Color,
  /// Primary identity text: the worktree *name* in the table and the
  /// sidebar header, plus the `Issue #N` / `PR #N` heads of the link
  /// summary lines. Pre-#170 these were a hard-coded `Color::White`
  /// with no semantic role (issue #210). Rendered bold at the name
  /// sites; the role only carries the foreground colour.
  pub name: Color,
  /// Worktree path column in the table. Pre-#170 a hard-coded
  /// `Color::Gray`, distinct from `muted` (`DarkGray`) so the path
  /// reads as a structural mid-grey rather than dimmed text (issue
  /// #210). The sidebar identity-card path stays on `muted` and is
  /// intentionally *not* moved here — doing so would shift its
  /// default appearance.
  pub path: Color,
}

impl Default for Theme {
  /// Pre-#33 hardcoded scheme. Documented field-by-field above. A
  /// non-default look (e.g. the Claude orange) is opt-in via a
  /// `[theme] preset = "claude-dark"` block — not baked into the
  /// default, so users who never write a `[theme]` block see no change.
  fn default() -> Self {
    Self {
      focus: Color::Cyan,
      accent: Color::Cyan,
      branch: Color::Green,
      clean: Color::Green,
      dirty: Color::Yellow,
      main: Color::Yellow,
      locked: Color::Magenta,
      prunable: Color::Red,
      muted: Color::DarkGray,
      selection_bg: Color::DarkGray,
      name: Color::White,
      path: Color::Gray,
    }
  }
}

impl Theme {
  /// Resolve a built-in preset by name. Returns `None` for unknown
  /// names — the config loader translates `None` into a
  /// `GwmError::Config` with the candidate name + the [`preset_names`]
  /// list for the user to crib from.
  pub fn preset(name: &str) -> Option<Self> {
    match name {
      "catppuccin" | "catppuccin-mocha" => Some(Self::catppuccin_mocha()),
      "gruvbox" | "gruvbox-dark" => Some(Self::gruvbox_dark()),
      "tokyo-night" | "tokyonight" => Some(Self::tokyo_night()),
      "claude-dark" | "claude" => Some(Self::claude_dark()),
      _ => None,
    }
  }

  /// Claude dark palette, ported from the Anthropic "Pure Dark"
  /// reference scheme. The signature orange (`#D4825D` primary,
  /// `#C15F3C` for focused borders) drives focus/accent; the semantic
  /// colours map green→branch/clean, yellow→dirty/main, purple→locked,
  /// red→prunable, and the warm overlay greys to muted/selection.
  fn claude_dark() -> Self {
    Self {
      focus: Color::Rgb(0xC1, 0x5F, 0x3C),        // Orange Dark (focused borders)
      accent: Color::Rgb(0xD4, 0x82, 0x5D),       // Orange (primary accent)
      branch: Color::Rgb(0x86, 0xE8, 0x9A),       // Success green
      clean: Color::Rgb(0x86, 0xE8, 0x9A),        // Success green
      dirty: Color::Rgb(0xFF, 0xDF, 0x61),        // Warning yellow
      main: Color::Rgb(0xFF, 0xDF, 0x61),         // Warning yellow
      locked: Color::Rgb(0xC7, 0x9B, 0xFF),       // Special purple
      prunable: Color::Rgb(0xFF, 0x7A, 0x7A),     // Error red
      muted: Color::Rgb(0x99, 0x99, 0x99),        // Text muted
      selection_bg: Color::Rgb(0x38, 0x38, 0x38), // Surface 1 (active)
      name: Color::Rgb(0xF5, 0xF4, 0xEF),         // Text (warm near-white)
      path: Color::Rgb(0xC2, 0xBD, 0xB4),         // Text dim (warm path grey)
    }
  }

  /// Catppuccin Mocha palette. Hex values taken from the upstream
  /// flavour reference (`https://github.com/catppuccin/catppuccin`).
  fn catppuccin_mocha() -> Self {
    Self {
      focus: Color::Rgb(0x89, 0xb4, 0xfa),        // Blue
      accent: Color::Rgb(0xcb, 0xa6, 0xf7),       // Mauve
      branch: Color::Rgb(0xa6, 0xe3, 0xa1),       // Green
      clean: Color::Rgb(0xa6, 0xe3, 0xa1),        // Green
      dirty: Color::Rgb(0xf9, 0xe2, 0xaf),        // Yellow
      main: Color::Rgb(0xf9, 0xe2, 0xaf),         // Yellow
      locked: Color::Rgb(0xcb, 0xa6, 0xf7),       // Mauve
      prunable: Color::Rgb(0xf3, 0x8b, 0xa8),     // Red
      muted: Color::Rgb(0x6c, 0x70, 0x86),        // Overlay 0
      selection_bg: Color::Rgb(0x31, 0x32, 0x44), // Surface 0
      name: Color::Rgb(0xcd, 0xd6, 0xf4),         // Text
      path: Color::Rgb(0xa6, 0xad, 0xc8),         // Subtext 0
    }
  }

  /// Gruvbox dark palette (medium contrast).
  fn gruvbox_dark() -> Self {
    Self {
      focus: Color::Rgb(0x83, 0xa5, 0x98),        // Bright blue
      accent: Color::Rgb(0xfa, 0xbd, 0x2f),       // Bright yellow
      branch: Color::Rgb(0xb8, 0xbb, 0x26),       // Bright green
      clean: Color::Rgb(0xb8, 0xbb, 0x26),        // Bright green
      dirty: Color::Rgb(0xfa, 0xbd, 0x2f),        // Bright yellow
      main: Color::Rgb(0xfe, 0x80, 0x19),         // Bright orange
      locked: Color::Rgb(0xd3, 0x86, 0x9b),       // Bright purple
      prunable: Color::Rgb(0xfb, 0x49, 0x34),     // Bright red
      muted: Color::Rgb(0x92, 0x83, 0x74),        // Light4
      selection_bg: Color::Rgb(0x3c, 0x38, 0x36), // Dark1
      name: Color::Rgb(0xeb, 0xdb, 0xb2),         // fg / Light1
      path: Color::Rgb(0xa8, 0x99, 0x84),         // Light4 (brighter than the muted gray)
    }
  }

  /// Tokyo Night palette (storm variant).
  fn tokyo_night() -> Self {
    Self {
      focus: Color::Rgb(0x7a, 0xa2, 0xf7),        // Blue
      accent: Color::Rgb(0xbb, 0x9a, 0xf7),       // Purple
      branch: Color::Rgb(0x9e, 0xce, 0x6a),       // Green
      clean: Color::Rgb(0x9e, 0xce, 0x6a),        // Green
      dirty: Color::Rgb(0xe0, 0xaf, 0x68),        // Orange
      main: Color::Rgb(0xe0, 0xaf, 0x68),         // Orange
      locked: Color::Rgb(0xbb, 0x9a, 0xf7),       // Purple
      prunable: Color::Rgb(0xf7, 0x76, 0x8e),     // Red
      muted: Color::Rgb(0x56, 0x5f, 0x89),        // Comment
      selection_bg: Color::Rgb(0x33, 0x3a, 0x55), // Selection
      name: Color::Rgb(0xc0, 0xca, 0xf5),         // fg
      path: Color::Rgb(0x73, 0x7a, 0xa2),         // dark5 (brighter than comment muted)
    }
  }

  /// Apply a single role override. Returns `Err` when `role` is
  /// unknown or `value` does not parse as a color. On `Ok` the
  /// targeted field is mutated in place.
  pub fn apply_override(&mut self, role: &str, value: &str) -> Result<()> {
    // Unwrap an inner `GwmError::Config` before re-wrapping so the
    // user-visible error reads `config error: theme.<role>: …`
    // rather than the duplicated `config error: theme.<role>:
    // config error: …` that the naive `format!("…: {}", e)` would
    // produce — the inner Display already carries the "config
    // error:" prefix. Same pattern as `Config::validate_labels` /
    // `validate_tui_keys` use elsewhere.
    let color = parse_color(value).map_err(|e| {
      let inner = match e {
        GwmError::Config(msg) => msg,
        other => other.to_string(),
      };
      GwmError::Config(format!("theme.{}: {}", role, inner))
    })?;
    let slot = self
      .role_mut(role)
      .ok_or_else(|| GwmError::Config(format!("theme: unknown role {:?}", role)))?;
    *slot = color;
    Ok(())
  }

  fn role_mut(&mut self, role: &str) -> Option<&mut Color> {
    match role {
      "focus" => Some(&mut self.focus),
      "accent" => Some(&mut self.accent),
      "branch" => Some(&mut self.branch),
      "clean" => Some(&mut self.clean),
      "dirty" => Some(&mut self.dirty),
      "main" => Some(&mut self.main),
      "locked" => Some(&mut self.locked),
      "prunable" => Some(&mut self.prunable),
      "muted" => Some(&mut self.muted),
      "selection_bg" => Some(&mut self.selection_bg),
      "name" => Some(&mut self.name),
      "path" => Some(&mut self.path),
      _ => None,
    }
  }
}

/// Names of every built-in preset. Surfaced by `gwm theme list`.
pub fn preset_names() -> &'static [&'static str] {
  &["catppuccin", "gruvbox", "tokyo-night", "claude-dark"]
}

// ---------------------------------------------------------------------------
// Color parsing
// ---------------------------------------------------------------------------

/// Parse a colour string in any of the three documented forms
/// (named, indexed, hex). Returns `Err` with a user-facing message
/// on garbage input.
pub fn parse_color(s: &str) -> Result<Color> {
  let trimmed = s.trim();
  if trimmed.is_empty() {
    return Err(GwmError::Config("empty color string".into()));
  }
  // Hex form: `#RRGGBB` (six hex digits + leading `#`). Short
  // (#RGB) form is intentionally not supported — the parser refuses
  // to guess on a 3-char user input.
  if let Some(stripped) = trimmed.strip_prefix('#') {
    if stripped.len() != 6 || !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
      return Err(GwmError::Config(format!(
        "invalid hex color {:?} (expected #RRGGBB)",
        s
      )));
    }
    let r = u8::from_str_radix(&stripped[0..2], 16).unwrap();
    let g = u8::from_str_radix(&stripped[2..4], 16).unwrap();
    let b = u8::from_str_radix(&stripped[4..6], 16).unwrap();
    return Ok(Color::Rgb(r, g, b));
  }
  // Indexed form: bare digits `0..=255`.
  if trimmed.chars().all(|c| c.is_ascii_digit()) {
    let n: u16 = trimmed
      .parse()
      .map_err(|_| GwmError::Config(format!("invalid indexed color {:?}", s)))?;
    if n > 255 {
      return Err(GwmError::Config(format!("indexed color {} out of range (0..=255)", n)));
    }
    return Ok(Color::Indexed(n as u8));
  }
  // Named form: case-insensitive match against the ratatui color
  // enum. Centralised here so a new ratatui release that adds a
  // colour does not require touching every call site.
  let lower = trimmed.to_ascii_lowercase();
  match lower.as_str() {
    "black" => Ok(Color::Black),
    "red" => Ok(Color::Red),
    "green" => Ok(Color::Green),
    "yellow" => Ok(Color::Yellow),
    "blue" => Ok(Color::Blue),
    "magenta" => Ok(Color::Magenta),
    "cyan" => Ok(Color::Cyan),
    "gray" | "grey" => Ok(Color::Gray),
    "dark_gray" | "darkgray" | "dark_grey" | "darkgrey" => Ok(Color::DarkGray),
    "light_red" | "lightred" | "bright_red" | "brightred" => Ok(Color::LightRed),
    "light_green" | "lightgreen" | "bright_green" | "brightgreen" => Ok(Color::LightGreen),
    "light_yellow" | "lightyellow" | "bright_yellow" | "brightyellow" => Ok(Color::LightYellow),
    "light_blue" | "lightblue" | "bright_blue" | "brightblue" => Ok(Color::LightBlue),
    "light_magenta" | "lightmagenta" | "bright_magenta" | "brightmagenta" => Ok(Color::LightMagenta),
    "light_cyan" | "lightcyan" | "bright_cyan" | "brightcyan" => Ok(Color::LightCyan),
    "white" => Ok(Color::White),
    "reset" => Ok(Color::Reset),
    _ => Err(GwmError::Config(format!(
      "unknown color name {:?} (try `cyan` / `#89b4fa` / `220`)",
      s
    ))),
  }
}
