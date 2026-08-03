//! Create-worktree input form state (extracted from `tui::app::App` per
//! #123 / #102).
//!
//! Pure state: holds the four user-editable values (field focus + type
//! index + issue number buffer + slug buffer) and exposes the rotation /
//! push-pop / reset primitives. The `App` orchestrator owns the side
//! effects in `submit_create` (it composes `BranchSpec` from the form's
//! values, then dispatches `worktree::add` + `bootstrap::run` on the async
//! task spine).

/// Max digits accepted in the issue-number field. Seven digits covers any
/// realistic GitHub issue/PR number (up to 9,999,999) while keeping the
/// resolved branch name well within git's 255-byte ref limit (#217).
pub const MAX_ISSUE_LEN: usize = 7;

/// Max characters accepted in the description (slug) field. Bounded so the
/// `<type>/#<issue>-<desc>` branch name cannot exceed git's 255-byte ref
/// limit even with the longest configured branch type (#217).
pub const MAX_DESC_LEN: usize = 200;

/// Max **bytes** accepted in the free-form name field (issue #416).
///
/// Deliberately the validator's own limit rather than a form-local number:
/// the name IS the branch, so a form that stopped short of what
/// `WorktreeName::freeform` accepts would silently truncate a legal name
/// and submit a different branch than the one typed (Codex review on
/// PR #474). Counted in bytes for the same reason the validator does —
/// free-form names are not restricted to ASCII, and it is the worktree
/// directory's byte length that has to fit.
pub const MAX_NAME_LEN: usize = crate::naming::MAX_DIR_COMPONENT_BYTES;

/// Which naming shape the form is collecting (issue #416). Toggled with
/// the `toggle_mode` verb; `Structured` is the default so the canonical
/// triple stays the path of least resistance.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
  #[default]
  Structured,
  Freeform,
}

/// Which input is currently focused inside the create overlay. Selected
/// via Tab / Shift-Tab; the Type field is special — it's cycled via
/// `next_type` / `prev_type` rather than typed into. `Name` is the sole
/// field of [`Mode::Freeform`] and is never reachable from the structured
/// rotation.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum Field {
  #[default]
  Type,
  Issue,
  Desc,
  Name,
}

impl Field {
  /// The field that collects the given editable segment (issue #418).
  /// `None` for anything outside the triple — `Name` has no segment, it *is*
  /// the branch.
  fn from_segment(segment: &str) -> Option<Self> {
    match segment {
      "type" => Some(Field::Type),
      "issue" => Some(Field::Issue),
      "desc" => Some(Field::Desc),
      _ => None,
    }
  }
}

/// The structured fields the given patterns ask the user to supply, in the
/// order the patterns write them (issue #418).
///
/// Pass every pattern the triple feeds — `branch_pattern`, `path_pattern` and
/// `base` — because [`crate::naming::BranchSpec::worktree_path`] expands the
/// three tokens in `base` too, so a segment only `base` carries still names a
/// real directory on disk.
pub fn fields_for(patterns: &[&str]) -> Vec<Field> {
  crate::naming::editable_segments(patterns)
    .into_iter()
    .filter_map(Field::from_segment)
    .collect()
}

/// The field set of the canonical `<type>/#<issue>-<desc>` triple — what a form
/// presents until [`CreateForm::set_fields`] tells it what the repo configured.
const DEFAULT_FIELDS: [Field; 3] = [Field::Type, Field::Issue, Field::Desc];

/// Input state for the create-worktree overlay. `Default` opens the form
/// in the initial state (structured mode, Type field focused, first type
/// selected, every string field empty).
///
/// Both modes' buffers are kept side by side rather than sharing one:
/// toggling is exploratory, and a user flipping across to look at the
/// other form must not lose what they already typed.
#[derive(Debug)]
pub struct CreateForm {
  pub mode: Mode,
  pub field: Field,
  pub type_index: usize,
  pub issue: String,
  pub desc: String,
  pub name: String,
  /// The structured fields the repo's patterns ask for, in pattern order
  /// (issue #418). Private, because the form's invariant is that [`Self::field`]
  /// is always one of these (or `Name` in free-form mode) — a focused field the
  /// renderer does not draw is an input the user cannot see and cannot correct.
  fields: Vec<Field>,
}

impl Default for CreateForm {
  fn default() -> Self {
    Self {
      mode: Mode::default(),
      field: Field::default(),
      type_index: 0,
      issue: String::new(),
      desc: String::new(),
      name: String::new(),
      fields: DEFAULT_FIELDS.to_vec(),
    }
  }
}

impl CreateForm {
  pub fn new() -> Self {
    Self::default()
  }

  /// Point the form at the fields the repo's patterns ask for (issue #418),
  /// re-anchoring focus so the invariant holds even if the caller had already
  /// focused a field the new set drops.
  pub fn set_fields(&mut self, fields: Vec<Field>) {
    self.fields = fields;
    if self.mode == Mode::Structured && !self.fields.contains(&self.field) {
      self.field = self.fields.first().copied().unwrap_or_default();
    }
  }

  /// The structured fields this form presents, in pattern order. The renderer
  /// walks this rather than the canonical triple.
  pub fn fields(&self) -> &[Field] {
    &self.fields
  }

  /// The field the form opens on: the first one the user actually *types* into.
  ///
  /// Type is cycled rather than typed, so opening there makes the first
  /// keypress a silent no-op (#217 UX) — skip it whenever the pattern gives us
  /// something else. A pattern whose only editable token is `{type}` has
  /// nowhere else to go, and lands on it.
  pub fn entry_field(&self) -> Field {
    self
      .fields
      .iter()
      .copied()
      .find(|f| *f != Field::Type)
      .or_else(|| self.fields.first().copied())
      .unwrap_or_default()
  }

  /// The last field in pattern order — where the rename modal opens, because
  /// the usual rename edits the trailing description rather than the type.
  pub fn last_field(&self) -> Field {
    self.fields.last().copied().unwrap_or_default()
  }

  /// Return to the freshly-opened state. Called by the orchestrator when
  /// the form opens or cancels.
  ///
  /// The field set survives: it is the repo's configuration, not something the
  /// user typed.
  pub fn reset(&mut self) {
    self.mode = Mode::Structured;
    self.field = self.fields.first().copied().unwrap_or_default();
    self.type_index = 0;
    self.issue.clear();
    self.desc.clear();
    self.name.clear();
  }

  /// Flip between the structured triple and a free-form name (issue #416),
  /// landing focus on the target mode's entry field: `Name` is free-form's
  /// only field, `Issue` is where `enter_create` opens the structured form.
  /// Typed values on both sides survive, so a round trip loses nothing.
  pub fn toggle_mode(&mut self) {
    match self.mode {
      Mode::Structured => {
        self.mode = Mode::Freeform;
        self.field = Field::Name;
      }
      Mode::Freeform => {
        self.mode = Mode::Structured;
        self.field = self.entry_field();
      }
    }
  }

  /// Rotate field focus forward through the fields the pattern presents, in
  /// pattern order. Free-form mode has a single field, so rotation stays put
  /// rather than walking focus onto inputs that mode does not present.
  pub fn next_field(&mut self) {
    self.rotate(1);
  }

  /// Rotate field focus backward through the same list.
  pub fn prev_field(&mut self) {
    self.rotate(-1);
  }

  /// Step focus by `step` positions within [`Self::fields`], wrapping.
  ///
  /// Walking the configured list rather than a hardcoded `match` is the whole
  /// point of #418: a pattern that writes no issue number must not be able to
  /// put focus on an Issue field the renderer never draws.
  fn rotate(&mut self, step: isize) {
    if self.mode == Mode::Freeform || self.fields.is_empty() {
      return;
    }
    let at = self.fields.iter().position(|f| *f == self.field).unwrap_or(0) as isize;
    let len = self.fields.len() as isize;
    self.field = self.fields[(at + step).rem_euclid(len) as usize];
  }

  /// Advance to the next branch type. `types_len` = the number of
  /// declared types (from `Config::resolved_branch_types().types.len()`);
  /// passing 0 is a no-op so the form survives an empty allow-list
  /// rather than panicking on `% 0`.
  pub fn next_type(&mut self, types_len: usize) {
    if types_len == 0 {
      return;
    }
    self.type_index = (self.type_index + 1) % types_len;
  }

  /// Step back to the previous branch type, wrapping at zero.
  pub fn prev_type(&mut self, types_len: usize) {
    if types_len == 0 {
      return;
    }
    if self.type_index == 0 {
      self.type_index = types_len - 1;
    } else {
      self.type_index -= 1;
    }
  }

  /// Append a character to the currently focused string field. Issue
  /// drops non-digits to match the `<type>/#<digits>-<slug>` branch
  /// convention; Desc accepts any character (slug normalisation happens
  /// downstream in `BranchSpec`). Type is no-op (cycled, not typed).
  /// Name accepts anything printable: git-ref and path legality are
  /// checked once on submit by `WorktreeName::freeform`, not per keystroke,
  /// so the user can type through an intermediate state (issue #416).
  ///
  /// A field the pattern does not present takes nothing (issue #418): the
  /// buffer behind it is never drawn, so a value typed into it is one the user
  /// can neither see nor correct.
  pub fn push_char(&mut self, c: char) {
    if !self.accepts_input() {
      return;
    }
    match self.field {
      Field::Issue if c.is_ascii_digit() && self.issue.chars().count() < MAX_ISSUE_LEN => self.issue.push(c),
      Field::Desc if self.desc.chars().count() < MAX_DESC_LEN => self.desc.push(c),
      Field::Name if self.name.len() + c.len_utf8() <= MAX_NAME_LEN => self.name.push(c),
      _ => {}
    }
  }

  /// Whether the focused field is one this form presents. `Name` belongs to
  /// free-form mode, which has no configured field list of its own.
  fn accepts_input(&self) -> bool {
    match self.mode {
      Mode::Freeform => self.field == Field::Name,
      Mode::Structured => self.fields.contains(&self.field),
    }
  }

  /// Pop the last character from the currently focused string field.
  /// Type is no-op, and so is a field the pattern does not present.
  pub fn pop_char(&mut self) {
    if !self.accepts_input() {
      return;
    }
    match self.field {
      Field::Issue => {
        self.issue.pop();
      }
      Field::Desc => {
        self.desc.pop();
      }
      Field::Name => {
        self.name.pop();
      }
      _ => {}
    }
  }
}
