//! Settings panel modal state (issue #232; editable in #279).
//!
//! Started life as a read-only Configuration overlay (issue #232): the
//! scroll cursor plus the owned, already-resolved [`crate::config::ConfigRow`]
//! snapshot the renderer paints. Issue #279 turns it into an editable
//! **Settings** panel — herdr-style — without dropping that read-only view:
//!
//! - **Tabs** ([`SettingsTab`]) split the surface into the editable `Theme`
//!   and `Tui` categories plus the read-only `All` resolved-config view.
//! - **A layer selector** ([`SettingsLayer`]) chooses whether an edit lands
//!   in the per-project `.gwm.toml` or the user-global `config.toml`.
//! - **Fields** ([`SettingField`]) are real toggles / choices / numeric
//!   inputs, resolved live against the loaded [`crate::config::Config`].
//!
//! The state here stays pure (no I/O): navigation, selection and the input
//! edit buffer live here and are unit-tested ratatui-free; the actual write
//! (`config_cli::set_value_at`) and the apply-live reload are orchestrated
//! by [`crate::tui::App`]. Scroll mirrors the help / Command Logs overlays:
//! the cursor lives here, `max_scroll` / `max_x_scroll` are republished by
//! the renderer each frame against the live viewport.

use crate::config::{Config, ConfigRow, ConfigSource};

/// The Settings categories, in tab order. `Theme` and `Tui` are editable;
/// `All` is the read-only resolved-config view (the pre-#279 panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
  /// Theme preset selection (editable).
  #[default]
  Theme,
  /// Worktree naming knobs: base dir + path / branch patterns.
  Worktree,
  /// TUI behaviour knobs: sidebar side, open mode + commands, confirm
  /// countdown.
  Tui,
  /// The full resolved config, read-only with source attribution.
  All,
}

impl SettingsTab {
  /// Tabs in display order — the navigation cycle.
  pub const ALL: [SettingsTab; 4] = [
    SettingsTab::Theme,
    SettingsTab::Worktree,
    SettingsTab::Tui,
    SettingsTab::All,
  ];

  /// Short tab label shown in the header strip.
  pub fn label(self) -> &'static str {
    match self {
      SettingsTab::Theme => "Theme",
      SettingsTab::Worktree => "Worktree",
      SettingsTab::Tui => "TUI",
      SettingsTab::All => "All",
    }
  }

  /// The editable fields under this tab, in display order. `All` has none
  /// (it is the read-only resolved view).
  pub fn fields(self) -> &'static [SettingField] {
    match self {
      SettingsTab::Theme => &[SettingField::ThemePreset],
      SettingsTab::Worktree => &[
        SettingField::WorktreeBase,
        SettingField::WorktreePathPattern,
        SettingField::WorktreeBranchPattern,
      ],
      SettingsTab::Tui => &[
        SettingField::SidebarPosition,
        SettingField::OpenMode,
        SettingField::ConfirmCountdown,
        SettingField::OpenShellCmd,
        SettingField::OpenEditorCmd,
      ],
      SettingsTab::All => &[],
    }
  }
}

/// Which config layer an edit targets. Both are editable (issue #279):
/// `Project` writes the repo `.gwm.toml`, `Global` writes the user-level
/// `~/.config/gwm/config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsLayer {
  /// The repo-local `.gwm.toml` — the default (matches `gwm config set`).
  #[default]
  Project,
  /// The user-global `~/.config/gwm/config.toml`.
  Global,
}

impl SettingsLayer {
  /// Label for the header indicator.
  pub fn label(self) -> &'static str {
    match self {
      SettingsLayer::Project => "project (.gwm.toml)",
      SettingsLayer::Global => "global (~/.config/gwm)",
    }
  }

  /// The [`ConfigSource`] an edit on this layer writes — used to decide
  /// whether the edit will actually take effect or be shadowed by a
  /// higher-precedence layer.
  pub fn source(self) -> ConfigSource {
    match self {
      SettingsLayer::Project => ConfigSource::Repo,
      SettingsLayer::Global => ConfigSource::User,
    }
  }

  /// Flip to the other layer.
  pub fn toggled(self) -> Self {
    match self {
      SettingsLayer::Project => SettingsLayer::Global,
      SettingsLayer::Global => SettingsLayer::Project,
    }
  }
}

/// How a [`SettingField`] is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
  /// Cycle through a fixed set of values (Space / Enter advances).
  Choice,
  /// A numeric (`u32`) value edited character-by-character in a buffer.
  Uint,
  /// A free-text value edited character-by-character in a buffer.
  Text,
}

const SIDEBAR_CHOICES: &[&str] = &["right", "left"];
const OPEN_MODE_CHOICES: &[&str] = &["shell", "editor", "finder"];

/// One editable setting, resolved live against the loaded [`Config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingField {
  /// `theme.preset` — cycle the built-in palettes.
  ThemePreset,
  /// `worktree.base` — the worktree base directory (text).
  WorktreeBase,
  /// `worktree.path_pattern` — the worktree dir-name pattern (text).
  WorktreePathPattern,
  /// `worktree.branch_pattern` — the branch-name pattern (text).
  WorktreeBranchPattern,
  /// `tui.sidebar_position` — left / right.
  SidebarPosition,
  /// `tui.open.mode` — shell / editor / finder.
  OpenMode,
  /// `tui.confirm_countdown_secs` — numeric input.
  ConfirmCountdown,
  /// `tui.open.shell_cmd` — `$SHELL` override (text).
  OpenShellCmd,
  /// `tui.open.editor_cmd` — `$EDITOR` override (text).
  OpenEditorCmd,
}

impl SettingField {
  /// Human label shown in the panel.
  pub fn label(self) -> &'static str {
    match self {
      SettingField::ThemePreset => "theme preset",
      SettingField::WorktreeBase => "base directory",
      SettingField::WorktreePathPattern => "path pattern",
      SettingField::WorktreeBranchPattern => "branch pattern",
      SettingField::SidebarPosition => "sidebar position",
      SettingField::OpenMode => "open mode",
      SettingField::ConfirmCountdown => "confirm countdown (s)",
      SettingField::OpenShellCmd => "open shell cmd",
      SettingField::OpenEditorCmd => "open editor cmd",
    }
  }

  /// Dotted config key path the edit writes (`config_cli::set_value_at`).
  pub fn key_path(self) -> &'static str {
    match self {
      SettingField::ThemePreset => "theme.preset",
      SettingField::WorktreeBase => "worktree.base",
      SettingField::WorktreePathPattern => "worktree.path_pattern",
      SettingField::WorktreeBranchPattern => "worktree.branch_pattern",
      SettingField::SidebarPosition => "tui.sidebar_position",
      SettingField::OpenMode => "tui.open.mode",
      SettingField::ConfirmCountdown => "tui.confirm_countdown_secs",
      SettingField::OpenShellCmd => "tui.open.shell_cmd",
      SettingField::OpenEditorCmd => "tui.open.editor_cmd",
    }
  }

  /// Whether the field is a cyclable choice, a numeric input, or free text.
  pub fn kind(self) -> FieldKind {
    match self {
      SettingField::ThemePreset | SettingField::SidebarPosition | SettingField::OpenMode => FieldKind::Choice,
      SettingField::ConfirmCountdown => FieldKind::Uint,
      SettingField::WorktreeBase
      | SettingField::WorktreePathPattern
      | SettingField::WorktreeBranchPattern
      | SettingField::OpenShellCmd
      | SettingField::OpenEditorCmd => FieldKind::Text,
    }
  }

  /// The fixed choice set for a `Choice` field. Theme presets come from the
  /// theme registry; the rest are static. Empty for non-choice fields.
  pub fn choices(self) -> &'static [&'static str] {
    match self {
      SettingField::ThemePreset => crate::tui::theme::preset_names(),
      SettingField::SidebarPosition => SIDEBAR_CHOICES,
      SettingField::OpenMode => OPEN_MODE_CHOICES,
      _ => &[],
    }
  }

  /// The current value as a display string, read from the resolved config.
  pub fn current(self, cfg: &Config) -> String {
    match self {
      SettingField::ThemePreset => cfg.theme.preset.clone().unwrap_or_else(|| "default".into()),
      SettingField::WorktreeBase => cfg.worktree.base.clone(),
      SettingField::WorktreePathPattern => cfg.worktree.path_pattern.clone(),
      SettingField::WorktreeBranchPattern => cfg.worktree.branch_pattern.clone(),
      SettingField::SidebarPosition => cfg.tui.sidebar_position.label().into(),
      SettingField::OpenMode => match cfg.tui.open.mode {
        crate::config::TuiOpenMode::Shell => "shell".into(),
        crate::config::TuiOpenMode::Editor => "editor".into(),
        crate::config::TuiOpenMode::Finder => "finder".into(),
      },
      SettingField::ConfirmCountdown => cfg.tui.confirm_countdown_secs.to_string(),
      SettingField::OpenShellCmd => cfg.tui.open.shell_cmd.clone().unwrap_or_default(),
      SettingField::OpenEditorCmd => cfg.tui.open.editor_cmd.clone().unwrap_or_default(),
    }
  }

  /// The next value for a `Choice` field, wrapping. If the current value is
  /// not one of the choices (e.g. theme preset is `None`/"default"), the
  /// first choice is returned. `None` for `Uint` fields.
  pub fn next_choice(self, cfg: &Config) -> Option<String> {
    let choices = self.choices();
    if choices.is_empty() {
      return None;
    }
    let current = self.current(cfg);
    let idx = choices.iter().position(|c| *c == current);
    let next = match idx {
      Some(i) => choices[(i + 1) % choices.len()],
      None => choices[0],
    };
    Some(next.to_string())
  }
}

/// Owned state for the Settings overlay: the read-only resolved rows, the
/// active tab / layer / selection, the optional numeric-input edit buffer,
/// and the (vertical + horizontal) scroll cursor with renderer-published
/// bounds.
#[derive(Debug, Default)]
pub struct ConfigPanel {
  /// Resolved config rows (key, value, source) for the read-only `All` tab,
  /// grouped section-first by the renderer. Also the source-attribution
  /// lookup behind the editable tabs. Assigned by
  /// [`crate::tui::App::enter_config_panel`].
  pub rows: Vec<ConfigRow>,
  /// Active settings tab (issue #279).
  pub tab: SettingsTab,
  /// Which config layer edits target (issue #279).
  pub layer: SettingsLayer,
  /// Selected field index within the current editable tab.
  pub selected: usize,
  /// When `Some`, the numeric-input edit buffer for the selected `Uint`
  /// field; keystrokes route here until commit (Enter) or cancel (Esc).
  pub editing: Option<String>,
  /// Vertical scroll offset, in rows. Clamped to `max_scroll`.
  pub scroll: u16,
  /// Maximum vertical scroll offset, republished by the renderer each
  /// frame as `content_rows.saturating_sub(viewport_rows)`.
  pub max_scroll: u16,
  /// Horizontal scroll offset, in columns. Clamped to `max_x_scroll`.
  pub x_scroll: u16,
  /// Maximum horizontal scroll offset, republished by the renderer.
  pub max_x_scroll: u16,
}

impl ConfigPanel {
  /// An empty overlay at the origin.
  pub fn new() -> Self {
    Self::default()
  }

  /// The editable fields under the active tab (empty on the `All` tab).
  pub fn fields(&self) -> &'static [SettingField] {
    self.tab.fields()
  }

  /// The selected field, if the active tab has any.
  pub fn selected_field(&self) -> Option<SettingField> {
    self.fields().get(self.selected).copied()
  }

  /// Move to the next tab, wrapping. Resets the field selection and any
  /// in-progress edit so the new tab starts clean.
  pub fn next_tab(&mut self) {
    let idx = SettingsTab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
    self.tab = SettingsTab::ALL[(idx + 1) % SettingsTab::ALL.len()];
    self.selected = 0;
    self.editing = None;
    self.scroll = 0;
  }

  /// Move to the previous tab, wrapping. Same reset as [`Self::next_tab`].
  pub fn prev_tab(&mut self) {
    let idx = SettingsTab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
    let len = SettingsTab::ALL.len();
    self.tab = SettingsTab::ALL[(idx + len - 1) % len];
    self.selected = 0;
    self.editing = None;
    self.scroll = 0;
  }

  /// Flip the edit target layer (project ↔ global).
  pub fn toggle_layer(&mut self) {
    self.layer = self.layer.toggled();
  }

  /// Select the previous field in the current tab (no-op while editing or on
  /// a tab with no fields).
  pub fn select_prev(&mut self) {
    if self.editing.is_some() {
      return;
    }
    self.selected = self.selected.saturating_sub(1);
  }

  /// Select the next field in the current tab, clamped to the last field.
  pub fn select_next(&mut self) {
    if self.editing.is_some() {
      return;
    }
    let count = self.fields().len();
    if count > 0 {
      self.selected = (self.selected + 1).min(count - 1);
    }
  }

  /// Begin editing the selected input field (`Uint` or `Text`), seeding the
  /// buffer with its current value. No-op if the selected field is a
  /// `Choice` (those cycle, they are not text-edited).
  pub fn begin_edit(&mut self, current: &str) {
    if matches!(
      self.selected_field().map(SettingField::kind),
      Some(FieldKind::Uint | FieldKind::Text)
    ) {
      self.editing = Some(current.to_string());
    }
  }

  /// Append a character to the edit buffer. `Uint` fields take ASCII digits
  /// only (capped at 3 chars — the countdown clamps to single seconds
  /// anyway); `Text` fields take any printable character (capped at 256).
  pub fn push_edit_char(&mut self, c: char) {
    let uint = matches!(self.selected_field().map(SettingField::kind), Some(FieldKind::Uint));
    if let Some(buf) = self.editing.as_mut() {
      if uint {
        if c.is_ascii_digit() && buf.len() < 3 {
          buf.push(c);
        }
      } else if !c.is_control() && buf.len() < 256 {
        buf.push(c);
      }
    }
  }

  /// Delete the last character of the edit buffer.
  pub fn pop_edit_char(&mut self) {
    if let Some(buf) = self.editing.as_mut() {
      buf.pop();
    }
  }

  /// Cancel the in-progress edit, discarding the buffer.
  pub fn cancel_edit(&mut self) {
    self.editing = None;
  }

  /// Commit the in-progress edit, returning the raw buffer (the caller
  /// coerces an empty numeric buffer to `"0"`; an empty text buffer is a
  /// legitimate "unset" value).
  pub fn take_edit(&mut self) -> Option<String> {
    self.editing.take()
  }

  /// The source layer that currently provides `field`'s value, looked up in
  /// the resolved rows. Drives the "shadowed edit" guidance: editing the
  /// global layer for a field the repo overrides won't change the effective
  /// value (repo wins).
  pub fn field_source(&self, field: SettingField) -> Option<ConfigSource> {
    self.rows.iter().find(|r| r.key == field.key_path()).map(|r| r.source)
  }

  /// Scroll down one row, never past the last line.
  pub fn scroll_down(&mut self) {
    self.scroll = (self.scroll + 1).min(self.max_scroll);
  }

  /// Scroll up one row, never above the top.
  pub fn scroll_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(1);
  }

  /// Scroll right one column, never past the widest line.
  pub fn scroll_right(&mut self) {
    self.x_scroll = (self.x_scroll + 1).min(self.max_x_scroll);
  }

  /// Scroll left one column, never before the first.
  pub fn scroll_left(&mut self) {
    self.x_scroll = self.x_scroll.saturating_sub(1);
  }

  /// Jump to the first row (`g`).
  pub fn scroll_to_top(&mut self) {
    self.scroll = 0;
  }

  /// Jump to the last row (`G`).
  pub fn scroll_to_bottom(&mut self) {
    self.scroll = self.max_scroll;
  }

  /// Reset the cursor + selection + edit buffer to the origin, keeping the
  /// resolved rows and the active tab/layer. Called when the overlay opens.
  pub fn reset(&mut self) {
    self.scroll = 0;
    self.x_scroll = 0;
    self.selected = 0;
    self.editing = None;
  }
}
