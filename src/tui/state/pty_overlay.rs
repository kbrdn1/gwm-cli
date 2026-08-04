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
#[cfg(unix)]
use libc;
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
  /// The `[review]` launcher (e.g. `codex review --base dev`) in PTY overlay.
  Review,
  /// An `[exec.profiles.<name>]` profile command, run in the selected
  /// worktree via the exec picker overlay (issue #325).
  Exec,
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
  /// Optional diff tempfile whose lifetime must match the overlay's (issue #291).
  /// Set by the `ReviewOverlay` dispatcher when `[review].command` uses `{diff}`.
  /// Dropped (and thus unlinked) when the overlay closes.
  pub diff_file: Option<tempfile::NamedTempFile>,
  /// Argv to run, best-effort, once the overlay is killed (issue #421).
  /// A containerised exec profile is spawned through `docker run`, and
  /// killing that client does NOT stop the container: the daemon owns it, and
  /// `--rm` only fires when it exits. So a long command would keep writing to
  /// the worktree after the overlay visibly closed. This tears it down by
  /// name. `None` for every other overlay (lazygit, review, a host command),
  /// which the process-group signal already covers.
  teardown: Option<Vec<String>>,
  /// Set by the run loop when a [`PtyKind::Exec`] child has exited and the
  /// overlay is *lingering* so its final output stays on screen (issue #325).
  /// Unlike lazygit / a shell — which close the overlay the instant the child
  /// dies — a one-shot exec command (`cargo test`, `npm run build`) exits as
  /// soon as it finishes, so the overlay must persist until the user
  /// dismisses it. While `true`, any keystroke closes the overlay.
  pub finished: bool,
  /// `true` once the child leader has been observed exited and reaped (by
  /// [`Self::is_alive`] or [`Self::kill`]). Guards [`Self::kill`] from blocking
  /// on a second `wait()` for an already-reaped leader.
  reaped: bool,
  /// The child leader's PID, captured at spawn so the process-group SIGKILL
  /// can target it even after the leader is reaped (its live `process_id()`
  /// may then be `None`). On Unix the child is a session leader (PGID == PID),
  /// so `kill(-pid)` reaps the whole pipeline. Read only inside the
  /// `#[cfg(unix)]` arm of [`Self::signal_group`]; Windows has no process
  /// groups (termination goes through `Child::kill`), so the field is unused
  /// there.
  #[cfg_attr(not(unix), allow(dead_code))]
  spawn_pid: Option<u32>,
  /// `true` once the process-group SIGKILL has been sent (issue #325 / Codex
  /// #333 review). Sent exactly once — promptly when an exec leader exits
  /// (so backgrounded descendants like `sh -c "sleep 60 &"` are cleaned in
  /// the safe window, while the PGID is still valid), so the later dismissal
  /// of a lingering overlay never re-signals a possibly-recycled PGID.
  signalled: bool,
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
    // Capture the leader PID now, while it is valid: after the leader is
    // reaped, `process_id()` may return `None`, but we still need its PID to
    // SIGKILL the process group and clean backgrounded descendants (#333).
    let spawn_pid = child.process_id();
    // Slave fd is no longer needed once the child holds its own copy.
    drop(pair.slave);

    // Bounded channel: the reader thread blocks when the buffer is full,
    // preventing unbounded memory growth when PTY output outpaces consumption.
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(128);
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
      diff_file: None,
      teardown: None,
      finished: false,
      reaped: false,
      spawn_pid,
      signalled: false,
    })
  }

  /// Attach a teardown command run once on [`Self::kill`] (issue #421).
  /// Builder form, so no existing `spawn` call site changes.
  pub fn with_teardown(mut self, argv: Vec<String>) -> Self {
    self.teardown = Some(argv);
    self
  }

  /// The attached teardown argv, if any. Exposed for the state tests.
  pub fn teardown_argv(&self) -> Option<&[String]> {
    self.teardown.as_deref()
  }

  /// Drain the reader channel and feed pending bytes into the vt100 parser.
  /// Call once per event-loop tick, before the ratatui draw.
  ///
  /// Capped at 64 chunks per frame so continuous-output programs (e.g.
  /// `yes`, verbose builds) cannot stall the TUI event loop or allocate
  /// without bound. Residual bytes remain in the channel and are consumed
  /// on the next tick.
  pub fn poll_bytes(&mut self) {
    for _ in 0..64 {
      match self.rx.try_recv() {
        Ok(bytes) => self.parser.process(&bytes),
        Err(_) => break,
      }
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

  /// Returns `true` while the child leader is still running. Once it has
  /// exited, records the reap so [`Self::kill`] skips a second blocking
  /// `wait()`.
  pub fn is_alive(&mut self) -> bool {
    match self.child.try_wait() {
      Ok(None) => true,
      _ => {
        self.reaped = true;
        false
      }
    }
  }

  /// Send `SIGKILL` to the child's process group exactly once, using the PID
  /// captured at spawn so it works even after the leader was reaped. On Unix
  /// the child is a session leader (PGID == PID), so this also terminates any
  /// backgrounded descendants (`sh -c "sleep 60 &"`). No-op on a second call,
  /// and on non-Unix.
  fn signal_group(&mut self) {
    if self.signalled {
      return;
    }
    self.signalled = true;
    #[cfg(unix)]
    if let Some(pid) = self.spawn_pid {
      unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
    }
  }

  /// Mark a finished (exited) exec overlay as lingering and PROMPTLY clean its
  /// process group while the PGID is still valid (#333 review). Sending the
  /// group SIGKILL the instant the leader exits — rather than deferring it to
  /// the dismissal keystroke — both reaps backgrounded descendants and keeps
  /// the signal out of the long linger window where the kernel could recycle
  /// the PGID. Called by the run loop when it detects a [`PtyKind::Exec`]
  /// child has died.
  pub fn mark_finished(&mut self) {
    self.signal_group();
    self.finished = true;
  }

  /// Send SIGKILL to the child's process group and wait until the leader is
  /// reaped. On Unix the entire group is killed so sub-processes spawned by
  /// the shell (e.g. `yes | head`, or a backgrounded `sleep`) are terminated
  /// too — even if the leader already exited (the group signal goes through
  /// [`Self::signal_group`], which is a no-op once already sent, e.g. by
  /// [`Self::mark_finished`] for a lingering exec).
  ///
  /// On macOS a PTY child that is blocked in a kernel write (D-state) will
  /// not react to SIGKILL until the PTY master drains enough data to allow
  /// the write to complete. We therefore keep draining the reader channel
  /// while polling `try_wait`, breaking any such deadlock. A 500 ms timeout
  /// guards against the unexpected: after that we fall back to a blocking
  /// `wait()`.
  pub fn kill(&mut self) {
    self.kill_client();
    // Always, including the early returns of `kill_client`: the client being
    // already reaped says nothing about the container, which outlives it.
    self.run_teardown();
  }

  /// Kill the pty leader and its process group, then reap it.
  fn kill_client(&mut self) {
    // Clean the process group (kills backgrounded descendants). Sent at most
    // once: for a lingering exec overlay `mark_finished` already sent it when
    // the leader exited, so this is a no-op here and the long-linger PGID is
    // never re-signalled (#333 review).
    self.signal_group();
    // Leader already reaped (by is_alive / mark_finished): nothing to wait on.
    if self.reaped {
      return;
    }
    let _ = self.child.kill();
    // Drain up to 128 buffered chunks per tick so the reader thread can keep
    // consuming from the PTY master fd, preventing a kernel D-state deadlock.
    for _ in 0..100 {
      match self.child.try_wait() {
        Ok(Some(_)) => {
          self.reaped = true;
          return;
        }
        _ => {
          for _ in 0..128 {
            if self.rx.try_recv().is_err() {
              break;
            }
          }
          std::thread::sleep(std::time::Duration::from_millis(5));
        }
      }
    }
    let _ = self.child.wait();
    self.reaped = true;
  }

  /// Run the teardown command once, ignoring its outcome. Best-effort by
  /// design: the container may already be gone (the command finished on its
  /// own), and a TUI has no channel for the error of a cleanup the user did
  /// not ask about. Output is discarded so nothing can corrupt the frame.
  fn run_teardown(&mut self) {
    let Some(argv) = self.teardown.take() else {
      return;
    };
    let Some((bin, args)) = argv.split_first() else {
      return;
    };
    let _ = std::process::Command::new(bin)
      .args(args)
      .stdin(std::process::Stdio::null())
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .status();
  }

  /// `true` once the child leader has been observed exited and reaped.
  /// Exposed for the state-machine tests (#333 review).
  pub fn is_reaped(&self) -> bool {
    self.reaped
  }

  /// `true` once the process-group SIGKILL has been sent (descendant cleanup).
  /// Exposed for the state-machine tests (#333 review).
  pub fn group_signalled(&self) -> bool {
    self.signalled
  }

  /// Poll the exit status without blocking. Exposed for tests that need to
  /// assert reap state without re-entering the blocking path of [`kill`].
  pub fn try_wait_after_kill(&mut self) -> Option<portable_pty::ExitStatus> {
    self.child.try_wait().ok().flatten()
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
