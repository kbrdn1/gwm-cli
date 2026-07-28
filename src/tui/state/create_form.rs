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

/// Input state for the create-worktree overlay. `Default` opens the form
/// in the initial state (structured mode, Type field focused, first type
/// selected, every string field empty).
///
/// Both modes' buffers are kept side by side rather than sharing one:
/// toggling is exploratory, and a user flipping across to look at the
/// other form must not lose what they already typed.
#[derive(Debug, Default)]
pub struct CreateForm {
  pub mode: Mode,
  pub field: Field,
  pub type_index: usize,
  pub issue: String,
  pub desc: String,
  pub name: String,
}

impl CreateForm {
  pub fn new() -> Self {
    Self::default()
  }

  /// Return to the freshly-opened state. Called by the orchestrator when
  /// the form opens or cancels.
  pub fn reset(&mut self) {
    self.mode = Mode::Structured;
    self.field = Field::Type;
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
        self.field = Field::Issue;
      }
    }
  }

  /// Rotate field focus forward (Type → Issue → Desc → Type). Free-form
  /// mode has a single field, so rotation stays put rather than walking
  /// focus onto inputs that mode does not present.
  pub fn next_field(&mut self) {
    if self.mode == Mode::Freeform {
      return;
    }
    self.field = match self.field {
      Field::Type => Field::Issue,
      Field::Issue => Field::Desc,
      Field::Desc | Field::Name => Field::Type,
    };
  }

  /// Rotate field focus backward (Type → Desc → Issue → Type).
  pub fn prev_field(&mut self) {
    if self.mode == Mode::Freeform {
      return;
    }
    self.field = match self.field {
      Field::Type => Field::Desc,
      Field::Issue | Field::Name => Field::Type,
      Field::Desc => Field::Issue,
    };
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
  pub fn push_char(&mut self, c: char) {
    match self.field {
      Field::Issue if c.is_ascii_digit() && self.issue.chars().count() < MAX_ISSUE_LEN => self.issue.push(c),
      Field::Desc if self.desc.chars().count() < MAX_DESC_LEN => self.desc.push(c),
      Field::Name if self.name.len() + c.len_utf8() <= MAX_NAME_LEN => self.name.push(c),
      _ => {}
    }
  }

  /// Pop the last character from the currently focused string field.
  /// Type is no-op.
  pub fn pop_char(&mut self) {
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
