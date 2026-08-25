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

use crate::config::{
  ClipboardMode, Config, ConfigRow, ConfigSource, MuxTarget, SidebarOrientation, SidebarPosition, TuiLayout,
};
use crate::multiplexer::SplitDirection;
use crate::tui::keymap::{Action, KeyStroke, Keymap};
use crate::tui::modal_keymap::{ModalAction, ModalKeymap};

/// The Settings categories, in tab order. `Theme`, `Worktree`, `Tui` and
/// `Keys` are editable; `All` is the read-only resolved-config view (the
/// pre-#279 panel).
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
  /// Keymap editor (issue #294): every global action + modal verb, rebound
  /// via live keystroke capture. Rows are dynamic ([`KeyRow`]), not the
  /// `&'static [SettingField]` the other editable tabs use.
  Keys,
  /// The full resolved config, read-only with source attribution.
  All,
}

impl SettingsTab {
  /// Tabs in display order — the navigation cycle.
  pub const ALL: [SettingsTab; 5] = [
    SettingsTab::Theme,
    SettingsTab::Worktree,
    SettingsTab::Tui,
    SettingsTab::Keys,
    SettingsTab::All,
  ];

  /// Short tab label shown in the header strip.
  pub fn label(self) -> &'static str {
    match self {
      SettingsTab::Theme => "Theme",
      SettingsTab::Worktree => "Worktree",
      SettingsTab::Tui => "TUI",
      SettingsTab::Keys => "Keys",
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
        SettingField::Layout,
        SettingField::DimUnfocused,
        SettingField::StatusOneLine,
        SettingField::NoteVim,
        SettingField::MuxOpenIn,
        SettingField::MuxPaneDirection,
        SettingField::SidebarPosition,
        SettingField::SidebarOrientation,
        SettingField::Clipboard,
        SettingField::OpenMode,
        SettingField::ConfirmCountdown,
        SettingField::AutoRefreshSecs,
        SettingField::OpenShellCmd,
        SettingField::OpenEditorCmd,
      ],
      // The Keys tab edits dynamic [`KeyRow`]s, not static fields, and `All`
      // is read-only.
      SettingsTab::Keys | SettingsTab::All => &[],
    }
  }
}

/// What a [`KeyRow`] rebinds: a global `View::List` action ([`Action`], chords
/// allowed) or a contextual modal verb ([`ModalAction`], single-stroke).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTarget {
  /// A global action under `[tui.keys]`.
  Global(Action),
  /// A modal verb under `[tui.keys.modal.<context>]`.
  Modal(ModalAction),
}

impl KeyTarget {
  /// The dotted `.gwm.toml` key the rebind writes (`config_cli::set_array_at`).
  pub fn config_key(self) -> String {
    match self {
      KeyTarget::Global(a) => format!("tui.keys.{}", a.slug()),
      KeyTarget::Modal(m) => format!("tui.keys.modal.{}.{}", m.context().config_path(), m.verb()),
    }
  }

  /// Modal verbs are single-stroke (issue #219); global actions accept
  /// multi-stroke chords. Drives the capture machine's accumulate-vs-commit.
  pub fn single_only(self) -> bool {
    matches!(self, KeyTarget::Modal(_))
  }

  /// Dotted `.gwm.toml` keys for any pre-#290 alias of a global action — to be
  /// stripped from a legacy config when the canonical slug is (re)written so a
  /// stale alias can't shadow the new binding (Codex #297 review). Empty for
  /// modal verbs (they have no compat aliases).
  pub fn compat_alias_keys(self) -> Vec<String> {
    match self {
      KeyTarget::Global(a) => a.compat_alias_slugs().map(|s| format!("tui.keys.{s}")).collect(),
      KeyTarget::Modal(_) => Vec::new(),
    }
  }
}

/// One row of the Keys tab: a bindable target, its display scope/label, the
/// current key(s), and the layer that sourced the binding. Built fresh on
/// panel open by [`build_key_rows`] from the live keymaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
  /// What this row rebinds.
  pub target: KeyTarget,
  /// `"global"` or `"modal.<context-path>"`.
  pub scope: String,
  /// The action slug (global) or context-local verb (modal).
  pub label: String,
  /// Current key(s), comma-joined (`"j, Down"`); empty when unbound.
  pub keys: String,
  /// The layer the binding came from (repo / user / default).
  pub source: ConfigSource,
}

/// Build the full Keys-tab row list: every global action (declaration order),
/// then every modal verb grouped by its context (declaration order). `source_of`
/// maps a dotted config key to its layer — the App passes the same resolved-row
/// attribution the `All` tab uses, so a hand-edited or in-TUI-set binding shows
/// the right `repo`/`user` badge and an untouched one reads `default`.
pub fn build_key_rows(keymap: &Keymap, modal: &ModalKeymap, source_of: impl Fn(&str) -> ConfigSource) -> Vec<KeyRow> {
  let mut rows = Vec::new();
  for action in Action::all() {
    let target = KeyTarget::Global(action);
    rows.push(KeyRow {
      target,
      scope: "global".to_string(),
      label: action.slug().to_string(),
      keys: keymap.keys_display(action),
      source: source_of(&target.config_key()),
    });
  }
  for action in ModalAction::all() {
    let target = KeyTarget::Modal(action);
    rows.push(KeyRow {
      target,
      scope: format!("modal.{}", action.context().config_path()),
      label: action.verb().to_string(),
      keys: modal.keys_display(action),
      source: source_of(&target.config_key()),
    });
  }
  rows
}

/// In-progress live keystroke capture for the selected [`KeyRow`] (issue
/// #294). Pure: the routing layer feeds strokes in, the App reads
/// [`Self::as_config_items`] to persist. `single_only` mirrors the target's
/// kind so the router knows whether to auto-commit (modal) or accumulate a
/// chord until the user confirms (global).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCapture {
  /// Index into [`ConfigPanel::key_rows`] being rebound.
  pub row: usize,
  /// Modal verb → true (one stroke, auto-commit); global → false (chord).
  pub single_only: bool,
  /// The strokes captured so far, in order.
  pub pending: Vec<KeyStroke>,
}

impl KeyCapture {
  /// The TOML array elements to write: one chord, the captured strokes
  /// space-joined (`"g g"`), or an empty list when nothing was captured (an
  /// unbind). Live capture sets a single binding; alternatives stay a
  /// hand-edit.
  pub fn as_config_items(&self) -> Vec<String> {
    if self.pending.is_empty() {
      return Vec::new();
    }
    vec![self.pending.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" ")]
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
  /// Cycle through a fixed set of values (Space / Enter advances),
  /// written to TOML as a **string**.
  Choice,
  /// Cycle through `false` / `true`, written to TOML as a **bare
  /// boolean** (issue #545).
  ///
  /// It is a separate kind from [`Self::Choice`] purely because of how
  /// the value is spelled on disk: quoting it produces
  /// `dim_unfocused = "true"`, which serde refuses as a string where a
  /// bool belongs, so the write fails and the setting never changes
  /// (Codex review, PR #546). Everything else — cycling, rendering —
  /// behaves like a choice.
  Bool,
  /// A numeric (`u32`) value edited character-by-character in a buffer.
  Uint,
  /// A free-text value edited character-by-character in a buffer.
  Text,
}

// Derived from the enums' `const fn label()` rather than restated: the choice
// string is written verbatim into `.gwm.toml`, so a list that drifted from the
// serde spelling would make the panel produce a file that no longer loads. Going
// through `label()` makes that drift impossible to express (#365).
const SIDEBAR_CHOICES: &[&str] = &[SidebarPosition::Right.label(), SidebarPosition::Left.label()];
const LAYOUT_CHOICES: &[&str] = &[TuiLayout::Compact.label(), TuiLayout::Bordered.label()];
// A bool is a two-value cycle; the strings are what TOML spells them, so the
// round-trip test covers them like any other choice list.
const BOOL_CHOICES: &[&str] = &["false", "true"];
const SIDEBAR_ORIENTATION_CHOICES: &[&str] = &[
  SidebarOrientation::Stacked.label(),
  SidebarOrientation::SideBySide.label(),
  SidebarOrientation::Auto.label(),
];
// `TuiOpenMode` has no `label()` to derive from; the round-trip test
// (`every_choice_is_a_value_the_config_can_load_back`) guards it instead.
const OPEN_MODE_CHOICES: &[&str] = &["shell", "editor", "finder"];
const MUX_OPEN_IN_CHOICES: &[&str] = &[
  MuxTarget::Pane.label(),
  MuxTarget::Tab.label(),
  MuxTarget::Workspace.label(),
];
const MUX_PANE_DIRECTION_CHOICES: &[&str] = &[SplitDirection::Right.label(), SplitDirection::Down.label()];
const CLIPBOARD_CHOICES: &[&str] = &[
  ClipboardMode::Auto.label(),
  ClipboardMode::Osc52.label(),
  ClipboardMode::Tools.label(),
];

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
  /// `tui.layout` — compact / bordered (issue #545).
  Layout,
  /// `tui.dim_unfocused` — dim the pane without focus (issue #545).
  DimUnfocused,
  /// `tui.status_one_line` — fold the sidebar Status block (issue #547).
  StatusOneLine,
  /// `tui.note_vim` — the note editor's vim normal mode (issue #557).
  NoteVim,
  /// `tui.mux_open_in` — pane / tab / workspace (issue #608).
  MuxOpenIn,
  /// `tui.mux_pane_direction` — right / down (issue #589).
  MuxPaneDirection,
  /// `tui.sidebar_position` — left / right.
  SidebarPosition,
  /// `tui.sidebar_orientation` — stacked / side-by-side / auto.
  SidebarOrientation,
  /// `tui.clipboard` — auto / osc52 / tools.
  Clipboard,
  /// `tui.open.mode` — shell / editor / finder.
  OpenMode,
  /// `tui.confirm_countdown_secs` — numeric input.
  ConfirmCountdown,
  /// `tui.auto_refresh_secs` — numeric input, 0 disables.
  AutoRefreshSecs,
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
      SettingField::Layout => "layout",
      SettingField::DimUnfocused => "dim unfocused pane",
      SettingField::StatusOneLine => "status on one line",
      SettingField::NoteVim => "note vim mode",
      SettingField::MuxOpenIn => "mux opens in",
      SettingField::MuxPaneDirection => "mux pane side",
      SettingField::SidebarOrientation => "sidebar layout",
      SettingField::Clipboard => "clipboard",
      SettingField::OpenMode => "open mode",
      SettingField::ConfirmCountdown => "confirm countdown (s)",
      SettingField::AutoRefreshSecs => "auto refresh (s)",
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
      SettingField::Layout => "tui.layout",
      SettingField::DimUnfocused => "tui.dim_unfocused",
      SettingField::StatusOneLine => "tui.status_one_line",
      SettingField::NoteVim => "tui.note_vim",
      SettingField::MuxOpenIn => "tui.mux_open_in",
      SettingField::MuxPaneDirection => "tui.mux_pane_direction",
      SettingField::SidebarOrientation => "tui.sidebar_orientation",
      SettingField::Clipboard => "tui.clipboard",
      SettingField::OpenMode => "tui.open.mode",
      SettingField::ConfirmCountdown => "tui.confirm_countdown_secs",
      SettingField::AutoRefreshSecs => "tui.auto_refresh_secs",
      SettingField::OpenShellCmd => "tui.open.shell_cmd",
      SettingField::OpenEditorCmd => "tui.open.editor_cmd",
    }
  }

  /// Whether the field is a cyclable choice, a numeric input, or free text.
  pub fn kind(self) -> FieldKind {
    match self {
      SettingField::ThemePreset
      | SettingField::Layout
      | SettingField::SidebarPosition
      | SettingField::SidebarOrientation
      | SettingField::Clipboard
      | SettingField::MuxOpenIn
      | SettingField::MuxPaneDirection
      | SettingField::OpenMode => FieldKind::Choice,
      SettingField::DimUnfocused | SettingField::StatusOneLine | SettingField::NoteVim => FieldKind::Bool,
      SettingField::ConfirmCountdown | SettingField::AutoRefreshSecs => FieldKind::Uint,
      SettingField::WorktreeBase
      | SettingField::WorktreePathPattern
      | SettingField::WorktreeBranchPattern
      | SettingField::OpenShellCmd
      | SettingField::OpenEditorCmd => FieldKind::Text,
    }
  }

  fn edit_char_limit(self) -> usize {
    match self {
      SettingField::AutoRefreshSecs => 20,
      SettingField::ConfirmCountdown => 3,
      _ => 256,
    }
  }

  /// The fixed choice set for a `Choice` field. Theme presets come from the
  /// theme registry; the rest are static. Empty for non-choice fields.
  pub fn choices(self) -> &'static [&'static str] {
    match self {
      SettingField::ThemePreset => crate::tui::theme::preset_names(),
      SettingField::SidebarPosition => SIDEBAR_CHOICES,
      SettingField::Layout => LAYOUT_CHOICES,
      SettingField::DimUnfocused | SettingField::StatusOneLine | SettingField::NoteVim => BOOL_CHOICES,
      SettingField::SidebarOrientation => SIDEBAR_ORIENTATION_CHOICES,
      SettingField::MuxOpenIn => MUX_OPEN_IN_CHOICES,
      SettingField::MuxPaneDirection => MUX_PANE_DIRECTION_CHOICES,
      SettingField::Clipboard => CLIPBOARD_CHOICES,
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
      SettingField::Layout => cfg.tui.layout.label().into(),
      SettingField::DimUnfocused => cfg.tui.dim_unfocused.to_string(),
      SettingField::StatusOneLine => cfg.tui.status_one_line.to_string(),
      SettingField::NoteVim => cfg.tui.note_vim.to_string(),
      SettingField::MuxOpenIn => cfg.tui.mux_open_in.label().into(),
      SettingField::MuxPaneDirection => cfg.tui.mux_pane_direction.label().into(),
      SettingField::SidebarOrientation => cfg.tui.sidebar_orientation.label().into(),
      SettingField::Clipboard => cfg.tui.clipboard.label().into(),
      SettingField::OpenMode => match cfg.tui.open.mode {
        crate::config::TuiOpenMode::Shell => "shell".into(),
        crate::config::TuiOpenMode::Editor => "editor".into(),
        crate::config::TuiOpenMode::Finder => "finder".into(),
      },
      SettingField::ConfirmCountdown => cfg.tui.confirm_countdown_secs.to_string(),
      SettingField::AutoRefreshSecs => cfg.tui.auto_refresh_secs.to_string(),
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
  /// Keys-tab rows (issue #294): every rebindable global action + modal verb
  /// with its current binding + source. Rebuilt on panel open by the App from
  /// the live keymaps; empty on the other tabs.
  pub key_rows: Vec<KeyRow>,
  /// When `Some`, a live keystroke capture is in progress on the Keys tab;
  /// strokes route into it until commit / cancel.
  pub capture: Option<KeyCapture>,
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

  /// The selected Keys-tab row, if the active tab is `Keys`.
  pub fn selected_key_row(&self) -> Option<&KeyRow> {
    if self.tab == SettingsTab::Keys {
      self.key_rows.get(self.selected)
    } else {
      None
    }
  }

  /// Number of selectable rows in the current tab: the static fields, or the
  /// dynamic key rows on the Keys tab.
  fn selectable_count(&self) -> usize {
    if self.tab == SettingsTab::Keys {
      self.key_rows.len()
    } else {
      self.fields().len()
    }
  }

  /// Move to the next tab, wrapping. Resets the field selection and any
  /// in-progress edit / capture so the new tab starts clean.
  pub fn next_tab(&mut self) {
    let idx = SettingsTab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
    self.tab = SettingsTab::ALL[(idx + 1) % SettingsTab::ALL.len()];
    self.selected = 0;
    self.editing = None;
    self.capture = None;
    self.scroll = 0;
  }

  /// Move to the previous tab, wrapping. Same reset as [`Self::next_tab`].
  pub fn prev_tab(&mut self) {
    let idx = SettingsTab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
    let len = SettingsTab::ALL.len();
    self.tab = SettingsTab::ALL[(idx + len - 1) % len];
    self.selected = 0;
    self.editing = None;
    self.capture = None;
    self.scroll = 0;
  }

  /// Flip the edit target layer (project ↔ global).
  pub fn toggle_layer(&mut self) {
    self.layer = self.layer.toggled();
  }

  /// Select the previous field / key row in the current tab (no-op while
  /// editing or capturing, or on a tab with no rows).
  pub fn select_prev(&mut self) {
    if self.editing.is_some() || self.capture.is_some() {
      return;
    }
    self.selected = self.selected.saturating_sub(1);
  }

  /// Select the next field / key row in the current tab, clamped to the last.
  pub fn select_next(&mut self) {
    if self.editing.is_some() || self.capture.is_some() {
      return;
    }
    let count = self.selectable_count();
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
  /// only (with per-field caps); `Text` fields take any printable character
  /// (capped at 256).
  /// Returns whether the character is of the TYPE the field accepts
  /// (Codex review #456): a numeric field refuses non-digits, and a
  /// refused character is not typing — the caller lets it reach the
  /// modal resolution so a rebound verb on it still fires. A valid
  /// character arriving on a FULL buffer is a consumed no-op (`true`),
  /// never a fall-through: a digit must not suddenly cancel the edit
  /// just because the countdown hit its width limit.
  pub fn push_edit_char(&mut self, c: char) -> bool {
    let field = self.selected_field();
    let uint = matches!(field.map(SettingField::kind), Some(FieldKind::Uint));
    let limit = field.map(SettingField::edit_char_limit).unwrap_or(256);
    if let Some(buf) = self.editing.as_mut() {
      if uint {
        if !c.is_ascii_digit() {
          return false;
        }
        if buf.len() < limit {
          buf.push(c);
        }
        return true;
      }
      if c.is_control() {
        return false;
      }
      if buf.len() < limit {
        buf.push(c);
      }
      return true;
    }
    false
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

  // ── Keys tab: live keystroke capture (issue #294) ──────────────────────

  /// Arm a live capture for the selected Keys-tab row. No-op off the Keys tab
  /// or with no row selected. The capture inherits the row's `single_only`
  /// flag so the router auto-commits a modal verb but accumulates a global
  /// chord.
  pub fn begin_capture(&mut self) {
    if self.tab != SettingsTab::Keys {
      return;
    }
    if let Some(row) = self.key_rows.get(self.selected) {
      self.capture = Some(KeyCapture {
        row: self.selected,
        single_only: row.target.single_only(),
        pending: Vec::new(),
      });
    }
  }

  /// Append a captured stroke to the in-progress capture.
  pub fn capture_push(&mut self, stroke: KeyStroke) {
    if let Some(cap) = self.capture.as_mut() {
      cap.pending.push(stroke);
    }
  }

  /// Drop the last captured stroke (Backspace during a multi-stroke capture).
  pub fn capture_pop(&mut self) {
    if let Some(cap) = self.capture.as_mut() {
      cap.pending.pop();
    }
  }

  /// Cancel the in-progress capture, discarding pending strokes.
  pub fn cancel_capture(&mut self) {
    self.capture = None;
  }

  /// Commit the in-progress capture, returning it for the App to persist.
  pub fn take_capture(&mut self) -> Option<KeyCapture> {
    self.capture.take()
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
    self.capture = None;
  }
}
