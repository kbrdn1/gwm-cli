//! Create-worktree input form state (extracted from `tui::app::App` per
//! #123 / #102).
//!
//! Pure state: holds the four user-editable values (field focus + type
//! index + issue number buffer + slug buffer) and exposes the rotation /
//! push-pop / reset primitives. The `App` orchestrator owns the side
//! effects in `submit_create` (it composes `BranchSpec` from the form's
//! values, then drives `worktree::add` + `bootstrap::run`).

/// Which input is currently focused inside the create overlay. Selected
/// via Tab / Shift-Tab; the Type field is special — it's cycled via
/// `next_type` / `prev_type` rather than typed into.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum Field {
  #[default]
  Type,
  Issue,
  Desc,
}

/// Input state for the create-worktree overlay. `Default` opens the form
/// in the initial state (Type field focused, first type selected, both
/// string fields empty).
#[derive(Debug, Default)]
pub struct CreateForm {
  pub field: Field,
  pub type_index: usize,
  pub issue: String,
  pub desc: String,
}

impl CreateForm {
  pub fn new() -> Self {
    Self::default()
  }

  /// Return to the freshly-opened state. Called by the orchestrator when
  /// the form opens or cancels.
  pub fn reset(&mut self) {
    self.field = Field::Type;
    self.type_index = 0;
    self.issue.clear();
    self.desc.clear();
  }

  /// Rotate field focus forward (Type → Issue → Desc → Type).
  pub fn next_field(&mut self) {
    self.field = match self.field {
      Field::Type => Field::Issue,
      Field::Issue => Field::Desc,
      Field::Desc => Field::Type,
    };
  }

  /// Rotate field focus backward (Type → Desc → Issue → Type).
  pub fn prev_field(&mut self) {
    self.field = match self.field {
      Field::Type => Field::Desc,
      Field::Issue => Field::Type,
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
  pub fn push_char(&mut self, c: char) {
    match self.field {
      Field::Issue if c.is_ascii_digit() => self.issue.push(c),
      Field::Desc => self.desc.push(c),
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
      _ => {}
    }
  }
}
