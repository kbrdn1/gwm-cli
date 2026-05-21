//! Per-row commit graph renderer for the Recent Commits sidebar block.
//!
//! Direct Rust port of lazygit's `pkg/gui/presentation/graph/` package
//! (`graph.go` + `cell.go`). The algorithm walks the commit list once,
//! maintains a set of active "pipes" (vertical line segments) between
//! consecutive rows, and renders each row as a fixed sequence of 2-char
//! cells (`<glyph><filler>`).
//!
//! The output is a `Vec<Line<'static>>` ready to drop into a ratatui
//! `Paragraph`. Each row's width is exactly `2 * (max_pos + 1)` chars,
//! independent of terminal width — the graph is width-deterministic on
//! the input commit list (lazygit caches on `(head_hash, count)` only).
//!
//! Differences from lazygit, all intentional:
//!
//! - **Single muted colour** (`Color::DarkGray`) for connectors and a
//!   neutral `Color::Blue` for `○` / `◎` nodes. Lazygit uses per-author
//!   MD5→HSL→RGB colours; we skip that for now — every commit on `gwm`
//!   is authored by the same person, so the rainbow is wasted ink.
//! - **No selected-commit override** — lazygit highlights the pipes
//!   originating from the cursor; our sidebar's selection lives on the
//!   *worktree* list, not on a specific commit.
//! - **No empty-tree sentinel** — lazygit emits an `EmptyTreeCommitHash`
//!   target for the first commit so its `○` doesn't appear orphaned;
//!   here we simply skip the seeded `Starts` pipe in that case.

use crate::worktree::CommitRow;
use ratatui::{
  style::{Color, Modifier, Style},
  text::Span,
};
use std::collections::HashSet;

/// Lifecycle of a pipe across the row it sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PipeKind {
  /// A pipe arriving from above that ends on the current row's commit.
  Terminates,
  /// A pipe starting from the current row's commit, heading downward
  /// toward one of its parents.
  Starts,
  /// A pipe that arrived from above and continues below — passing
  /// through the current row, possibly shifting columns left/right.
  Continues,
}

#[derive(Debug, Clone)]
pub struct Pipe {
  pub from_pos: i16,
  pub to_pos: i16,
  pub from_hash: String,
  pub to_hash: String,
  pub kind: PipeKind,
}

impl Pipe {
  #[inline]
  fn left(&self) -> i16 {
    self.from_pos.min(self.to_pos)
  }
  #[inline]
  fn right(&self) -> i16 {
    self.from_pos.max(self.to_pos)
  }
}

/// What a cell represents at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellType {
  Connection,
  Commit,
  Merge,
}

#[derive(Debug, Clone)]
struct Cell {
  up: bool,
  down: bool,
  left: bool,
  right: bool,
  cell_type: CellType,
}

impl Cell {
  fn new_connection() -> Self {
    Self {
      up: false,
      down: false,
      left: false,
      right: false,
      cell_type: CellType::Connection,
    }
  }
  fn set_up(&mut self) {
    self.up = true;
  }
  fn set_down(&mut self) {
    self.down = true;
  }
  fn set_left(&mut self) {
    self.left = true;
  }
  fn set_right(&mut self) {
    self.right = true;
  }
  fn set_type(&mut self, t: CellType) {
    self.cell_type = t;
  }
}

/// Port of lazygit's 16-case `getBoxDrawingChars` truth table
/// (`cell.go:147-183`). Returns `(glyph, right_filler)` — each cell emits
/// two characters so a continuous horizontal stroke can be drawn by
/// chaining `─` fillers across columns.
///
/// Glyphs are all in the U+2500 light-box-drawing block.
pub fn box_drawing_chars(up: bool, down: bool, left: bool, right: bool) -> (char, char) {
  match (up, down, left, right) {
    (true, true, true, true) => ('│', '─'),
    (true, true, true, false) => ('│', ' '),
    (true, true, false, true) => ('│', '─'),
    (true, true, false, false) => ('│', ' '),
    (true, false, true, true) => ('┴', '─'),
    (true, false, true, false) => ('╯', ' '),
    (true, false, false, true) => ('╰', '─'),
    (true, false, false, false) => ('╵', ' '),
    (false, true, true, true) => ('┬', '─'),
    (false, true, true, false) => ('╮', ' '),
    (false, true, false, true) => ('╭', '─'),
    (false, true, false, false) => ('╷', ' '),
    (false, false, true, true) => ('─', '─'),
    (false, false, true, false) => ('─', ' '),
    (false, false, false, true) => ('╶', '─'),
    (false, false, false, false) => (' ', ' '),
  }
}

/// Seed sentinel hash used by lazygit for the pipe that ends on the
/// very first commit (`graph.go:36`). The actual value is opaque — it
/// just must never equal any real SHA-1.
const START_HASH: &str = "__GWM_GRAPH_START__";

/// Walk `commits` once, producing the per-row pipe sets. This is the
/// Rust translation of lazygit's `GetPipeSets` (`graph.go:60-69`).
pub fn build_pipe_sets(commits: &[CommitRow]) -> Vec<Vec<Pipe>> {
  if commits.is_empty() {
    return Vec::new();
  }
  let mut pipes = vec![Pipe {
    from_pos: 0,
    to_pos: 0,
    from_hash: START_HASH.to_string(),
    to_hash: commits[0].hash.clone(),
    kind: PipeKind::Starts,
  }];
  let mut out = Vec::with_capacity(commits.len());
  for commit in commits {
    pipes = get_next_pipes(&pipes, commit);
    out.push(pipes.clone());
  }
  out
}

/// Per-row transition. Port of lazygit's `getNextPipes`
/// (`graph.go:109-273`). Given the previous row's pipes and the current
/// commit, produces the current row's pipe set.
fn get_next_pipes(prev_pipes: &[Pipe], commit: &CommitRow) -> Vec<Pipe> {
  let max_pos: i16 = prev_pipes.iter().map(|p| p.to_pos).max().unwrap_or(0);

  // Pipes that terminated last row do not carry into this one.
  let current_pipes: Vec<&Pipe> = prev_pipes.iter().filter(|p| p.kind != PipeKind::Terminates).collect();

  // Default to a brand-new commit column. Falls back to the column of
  // any descendant pipe pointing at us.
  let mut pos: i16 = max_pos + 1;
  for pipe in &current_pipes {
    if pipe.to_hash == commit.hash {
      pos = pipe.to_pos;
      break;
    }
  }

  let mut new_pipes: Vec<Pipe> = Vec::with_capacity(current_pipes.len() + commit.parents.len());

  // Emit the STARTS pipe for the *first* parent (or empty-tree sentinel
  // when this is the root commit).
  let first_parent = commit.parents.first().cloned().unwrap_or_else(|| String::from(""));
  new_pipes.push(Pipe {
    from_pos: pos,
    to_pos: pos,
    from_hash: commit.hash.clone(),
    to_hash: first_parent,
    kind: PipeKind::Starts,
  });

  // Shared mutable state for the per-pipe loops below.
  let mut taken_spots: HashSet<i16> = HashSet::new();
  let mut traversed_spots: HashSet<i16> = HashSet::new();

  // Pre-compute the spots that continuing pipes already occupy — a new
  // merge-parent pipe must not land on top of them.
  let mut traversed_spots_for_continuing: HashSet<i16> = HashSet::new();
  for pipe in &current_pipes {
    if pipe.to_hash != commit.hash {
      traversed_spots_for_continuing.insert(pipe.to_pos);
    }
  }

  // Helper closures need shared borrows of the spot sets, so inline
  // them as plain `fn`-style helpers operating on locals here.
  fn next_free(spots: &HashSet<i16>) -> i16 {
    let mut i: i16 = 0;
    while spots.contains(&i) {
      i += 1;
    }
    i
  }
  fn next_free_for_new(taken: &HashSet<i16>, traversed_for_continuing: &HashSet<i16>) -> i16 {
    let mut i: i16 = 0;
    while taken.contains(&i) || traversed_for_continuing.contains(&i) {
      i += 1;
    }
    i
  }
  fn traverse(taken: &mut HashSet<i16>, traversed: &mut HashSet<i16>, from: i16, to: i16) {
    let (l, r) = if from <= to { (from, to) } else { (to, from) };
    for i in l..=r {
      traversed.insert(i);
    }
    taken.insert(to);
  }

  // Phase 1: terminating + leftward-continuing pipes from the previous row.
  for pipe in &current_pipes {
    if pipe.to_hash == commit.hash {
      // pipe ends on this commit
      new_pipes.push(Pipe {
        from_pos: pipe.to_pos,
        to_pos: pos,
        from_hash: pipe.from_hash.clone(),
        to_hash: pipe.to_hash.clone(),
        kind: PipeKind::Terminates,
      });
      traverse(&mut taken_spots, &mut traversed_spots, pipe.to_pos, pos);
    } else if pipe.to_pos < pos {
      // pipe continues; pick the next free column to its right
      let avail = next_free(&traversed_spots);
      new_pipes.push(Pipe {
        from_pos: pipe.to_pos,
        to_pos: avail,
        from_hash: pipe.from_hash.clone(),
        to_hash: pipe.to_hash.clone(),
        kind: PipeKind::Continues,
      });
      traverse(&mut taken_spots, &mut traversed_spots, pipe.to_pos, avail);
    }
  }

  // Phase 2: extra parents of a merge commit each open a new column.
  if commit.parents.len() >= 2 {
    for parent in commit.parents.iter().skip(1) {
      let avail = next_free_for_new(&taken_spots, &traversed_spots_for_continuing);
      new_pipes.push(Pipe {
        from_pos: pos,
        to_pos: avail,
        from_hash: commit.hash.clone(),
        to_hash: parent.clone(),
        kind: PipeKind::Starts,
      });
      taken_spots.insert(avail);
    }
  }

  // Phase 3: continuing pipes from the *right* of the commit. They may
  // shift leftward to fill blank columns.
  for pipe in &current_pipes {
    if pipe.to_hash != commit.hash && pipe.to_pos > pos {
      let mut last = pipe.to_pos;
      let mut i = pipe.to_pos;
      while i > pos {
        i -= 1;
        if taken_spots.contains(&i) || traversed_spots.contains(&i) {
          break;
        }
        last = i;
      }
      new_pipes.push(Pipe {
        from_pos: pipe.to_pos,
        to_pos: last,
        from_hash: pipe.from_hash.clone(),
        to_hash: pipe.to_hash.clone(),
        kind: PipeKind::Continues,
      });
      traverse(&mut taken_spots, &mut traversed_spots, pipe.to_pos, last);
    }
  }

  // Stable ordering by to_pos, then kind (matches lazygit's sort).
  new_pipes.sort_by(|a, b| {
    if a.to_pos == b.to_pos {
      a.kind.cmp(&b.kind)
    } else {
      a.to_pos.cmp(&b.to_pos)
    }
  });

  new_pipes
}

/// Render a single pipe set into ratatui spans, two spans per cell. Port
/// of lazygit's `renderPipeSet` (`graph.go:275-385`).
pub fn render_pipe_set(pipes: &[Pipe]) -> Vec<Span<'static>> {
  let mut max_pos: i16 = 0;
  let mut commit_pos: i16 = 0;
  let mut start_count: usize = 0;
  for pipe in pipes {
    if pipe.kind == PipeKind::Starts {
      start_count += 1;
      commit_pos = pipe.from_pos;
    } else if pipe.kind == PipeKind::Terminates {
      commit_pos = pipe.to_pos;
    }
    if pipe.right() > max_pos {
      max_pos = pipe.right();
    }
  }
  let is_merge = start_count > 1;

  let mut cells: Vec<Cell> = (0..=max_pos).map(|_| Cell::new_connection()).collect();

  // First pass: STARTS pipes paint their downward stroke + any leftward
  // continuation. Done first so subsequent passes can layer on top.
  for pipe in pipes {
    if pipe.kind == PipeKind::Starts {
      apply_pipe(&mut cells, pipe);
    }
  }
  // Second pass: TERMINATES and CONTINUES (except the trivial commit-
  // on-commit terminate that would erase the commit cell glyph).
  for pipe in pipes {
    if pipe.kind != PipeKind::Starts
      && !(pipe.kind == PipeKind::Terminates && pipe.from_pos == commit_pos && pipe.to_pos == commit_pos)
    {
      apply_pipe(&mut cells, pipe);
    }
  }

  // Mark the commit cell.
  if let Some(c) = cells.get_mut(commit_pos as usize) {
    c.set_type(if is_merge { CellType::Merge } else { CellType::Commit });
  }

  let connector_style = Style::default().fg(Color::DarkGray);
  let node_style = Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD);

  let mut out: Vec<Span<'static>> = Vec::with_capacity(cells.len() * 2);
  for cell in &cells {
    let (glyph, filler) = box_drawing_chars(cell.up, cell.down, cell.left, cell.right);
    let render_glyph: String = match cell.cell_type {
      CellType::Connection => glyph.to_string(),
      CellType::Commit => '○'.to_string(),
      CellType::Merge => '◎'.to_string(),
    };
    let style = if matches!(cell.cell_type, CellType::Commit | CellType::Merge) {
      node_style
    } else {
      connector_style
    };
    out.push(Span::styled(render_glyph, style));
    // The right filler is always a connector (never a node), so it
    // keeps the connector style.
    out.push(Span::styled(filler.to_string(), connector_style));
  }
  out
}

fn apply_pipe(cells: &mut [Cell], pipe: &Pipe) {
  let left = pipe.left();
  let right = pipe.right();

  if left != right {
    for i in (left + 1)..right {
      if let Some(c) = cells.get_mut(i as usize) {
        c.set_left();
        c.set_right();
      }
    }
    if let Some(c) = cells.get_mut(left as usize) {
      c.set_right();
    }
    if let Some(c) = cells.get_mut(right as usize) {
      c.set_left();
    }
  }

  match pipe.kind {
    PipeKind::Starts | PipeKind::Continues => {
      if let Some(c) = cells.get_mut(pipe.to_pos as usize) {
        c.set_down();
      }
    }
    PipeKind::Terminates => {}
  }
  match pipe.kind {
    PipeKind::Terminates | PipeKind::Continues => {
      if let Some(c) = cells.get_mut(pipe.from_pos as usize) {
        c.set_up();
      }
    }
    PipeKind::Starts => {}
  }
}

/// One-shot helper: compute pipe sets for the commit list and render
/// each row's spans. Output length matches `commits.len()`.
pub fn render_commits(commits: &[CommitRow]) -> Vec<Vec<Span<'static>>> {
  build_pipe_sets(commits)
    .into_iter()
    .map(|pipes| render_pipe_set(&pipes))
    .collect()
}

/// Convenience constructor for tests — builds a `CommitRow` with the
/// minimum data the graph algorithm needs.
#[doc(hidden)]
pub fn test_row(hash: &str, parents: &[&str]) -> CommitRow {
  CommitRow {
    hash: hash.into(),
    author: String::new(),
    parents: parents.iter().map(|s| s.to_string()).collect(),
    subject: String::new(),
  }
}
