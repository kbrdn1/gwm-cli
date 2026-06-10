//! PTY overlay state for embedded lazygit / native terminal (issue #35).
//!
//! [`PtyOverlay`] owns a `portable-pty` master + child pair and a `vt100`
//! parser. The event loop spawns a reader thread that feeds PTY output back
//! via an `mpsc` channel; every iteration drains the channel and calls
//! [`PtyOverlay::poll_bytes`] so the parser stays current before the next
//! ratatui frame.
//!
//! [`key_to_bytes`] is a pure conversion function, `pub` for state-machine
//! tests that want to pin the byte contract without spawning a real PTY.

use crate::error::{GwmError, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;

/// Discriminates the program running inside the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyKind {
  /// `lazygit -p <worktree-path>` (or whatever `[git_tui]` resolves to).
  LazyGit,
  /// The user's `$SHELL` (or `/bin/sh`) rooted at the selected worktree.
  Terminal,
}

/// Live PTY overlay — one spawned process, one vt100 parser, one mpsc reader
/// channel. Dropping kills the child; the reader thread exits on the next
/// failed `send` after the `Receiver` is dropped with the struct.
pub struct PtyOverlay {
  pub kind: PtyKind,
  master: Box<dyn portable_pty::MasterPty + Send>,
  child: Box<dyn portable_pty::Child + Send + Sync>,
  writer: Box<dyn Write + Send>,
  /// vt100 parser — updated by [`poll_bytes`], read by the renderer.
  pub parser: tui_term::vt100::Parser,
  rx: mpsc::Receiver<Vec<u8>>,
  /// Current PTY column count (kept in sync by [`resize`]).
  pub cols: u16,
  /// Current PTY row count (kept in sync by [`resize`]).
  pub rows: u16,
}

impl std::fmt::Debug for PtyOverlay {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("PtyOverlay")
      .field("kind", &self.kind)
      .field("cols", &self.cols)
      .field("rows", &self.rows)
      .finish_non_exhaustive()
  }
}

impl PtyOverlay {
  /// Spawn `argv[0] argv[1..]` in a fresh PTY of `cols × rows`, with the
  /// working directory set to `cwd`. Sets `TERM=xterm-256color` so
  /// interactive programs (lazygit, neovim, shells) know they have colour.
  pub fn spawn(kind: PtyKind, argv: &[&str], cwd: &Path, cols: u16, rows: u16) -> Result<Self> {
    let Some((bin, args)) = argv.split_first() else {
      return Err(GwmError::Other("empty argv for PTY overlay".into()));
    };

    let pty_system = native_pty_system();
    let pair = pty_system
      .openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
      })
      .map_err(|e| GwmError::Other(e.to_string()))?;

    let mut cmd = CommandBuilder::new(bin);
    for arg in args {
      cmd.arg(*arg);
    }
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");

    let child = pair
      .slave
      .spawn_command(cmd)
      .map_err(|e| GwmError::Other(e.to_string()))?;
    // Slave fd is no longer needed once the child holds its own copy.
    drop(pair.slave);

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut reader = pair
      .master
      .try_clone_reader()
      .map_err(|e| GwmError::Other(e.to_string()))?;

    std::thread::spawn(move || {
      let mut buf = [0u8; 4096];
      loop {
        match reader.read(&mut buf) {
          Ok(0) | Err(_) => break,
          Ok(n) => {
            if tx.send(buf[..n].to_vec()).is_err() {
              break;
            }
          }
        }
      }
    });

    let writer = pair.master.take_writer().map_err(|e| GwmError::Other(e.to_string()))?;

    let parser = tui_term::vt100::Parser::new(rows, cols, 0);

    Ok(Self {
      kind,
      master: pair.master,
      child,
      writer,
      parser,
      rx,
      cols,
      rows,
    })
  }

  /// Drain the reader channel and feed pending bytes into the vt100 parser.
  /// Call once per event-loop tick, before the ratatui draw.
  pub fn poll_bytes(&mut self) {
    while let Ok(bytes) = self.rx.try_recv() {
      self.parser.process(&bytes);
    }
  }

  /// Convert a crossterm [`KeyEvent`] to terminal bytes and write them to
  /// the PTY master. The child process receives them on its stdin.
  pub fn write_key(&mut self, key: KeyEvent) -> std::io::Result<()> {
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
      self.writer.write_all(&bytes)?;
      self.writer.flush()?;
    }
    Ok(())
  }

  /// Resize the PTY and the vt100 parser. Call on every terminal `Resize`
  /// event so the child program sees the updated dimensions.
  pub fn resize(&mut self, cols: u16, rows: u16) {
    self.cols = cols;
    self.rows = rows;
    self.parser.screen_mut().set_size(rows, cols);
    let _ = self.master.resize(PtySize {
      rows,
      cols,
      pixel_width: 0,
      pixel_height: 0,
    });
  }

  /// Returns `true` while the child process is still running.
  pub fn is_alive(&mut self) -> bool {
    matches!(self.child.try_wait(), Ok(None))
  }

  /// Send SIGKILL (or the platform equivalent) to the child process.
  pub fn kill(&mut self) {
    let _ = self.child.kill();
  }
}

/// Convert a crossterm [`KeyEvent`] to the terminal-protocol bytes that the
/// PTY child expects to receive on its stdin. Handles ASCII, Unicode, Ctrl
/// combos, arrows, function keys, and common editing keys.
///
/// `pub` so `tests/tui_state_pty_overlay_tests.rs` can pin the byte
/// contract without spawning a real PTY.
pub fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
  match key.code {
    KeyCode::Char(c) => {
      if key.modifiers.contains(KeyModifiers::CONTROL) {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() {
          return vec![(c as u8) - b'a' + 1];
        }
        return vec![];
      }
      let mut buf = [0u8; 4];
      let s = c.encode_utf8(&mut buf);
      let bytes = s.as_bytes().to_vec();
      // Alt+<char>: readline/bash Meta convention — prefix with ESC (0x1b).
      if key.modifiers.contains(KeyModifiers::ALT) {
        let mut out = vec![27u8];
        out.extend_from_slice(&bytes);
        return out;
      }
      bytes
    }
    KeyCode::Enter => vec![b'\r'],
    KeyCode::Backspace => vec![127],
    KeyCode::Esc => vec![27],
    KeyCode::Tab => vec![b'\t'],
    KeyCode::BackTab => vec![27, b'[', b'Z'],
    KeyCode::Up => vec![27, b'[', b'A'],
    KeyCode::Down => vec![27, b'[', b'B'],
    KeyCode::Right => vec![27, b'[', b'C'],
    KeyCode::Left => vec![27, b'[', b'D'],
    KeyCode::Home => vec![27, b'[', b'H'],
    KeyCode::End => vec![27, b'[', b'F'],
    KeyCode::Delete => vec![27, b'[', b'3', b'~'],
    KeyCode::Insert => vec![27, b'[', b'2', b'~'],
    KeyCode::PageUp => vec![27, b'[', b'5', b'~'],
    KeyCode::PageDown => vec![27, b'[', b'6', b'~'],
    KeyCode::F(n) => f_key_bytes(n),
    _ => vec![],
  }
}

fn f_key_bytes(n: u8) -> Vec<u8> {
  match n {
    1 => vec![27, b'O', b'P'],
    2 => vec![27, b'O', b'Q'],
    3 => vec![27, b'O', b'R'],
    4 => vec![27, b'O', b'S'],
    5 => vec![27, b'[', b'1', b'5', b'~'],
    6 => vec![27, b'[', b'1', b'7', b'~'],
    7 => vec![27, b'[', b'1', b'8', b'~'],
    8 => vec![27, b'[', b'1', b'9', b'~'],
    9 => vec![27, b'[', b'2', b'0', b'~'],
    10 => vec![27, b'[', b'2', b'1', b'~'],
    11 => vec![27, b'[', b'2', b'3', b'~'],
    12 => vec![27, b'[', b'2', b'4', b'~'],
    _ => vec![],
  }
}
