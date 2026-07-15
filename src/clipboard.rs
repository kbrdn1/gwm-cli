//! Clipboard routing (issue #367): decide whether yanked text goes to the
//! host clipboard binaries or to the terminal via an OSC52 escape sequence,
//! and build the exact bytes for the OSC52 path.
//!
//! # Why this exists
//!
//! [`crate::tui::clipboard_candidates`] shells out to `pbcopy` / `wl-copy` /
//! `xclip` / `clip.exe`. Those write to the clipboard of the machine gwm runs
//! on. Over SSH that is the wrong machine — and the failure is silent rather
//! than loud: on a remote macOS host `pbcopy` exists, runs, exits 0, so gwm
//! reports `yanked branch name (pbcopy)` while the user's actual clipboard is
//! untouched. OSC52 hands the text to the terminal emulator instead, which is
//! the process that owns the clipboard the user pastes from.
//!
//! # Purity
//!
//! [`plan_clipboard_write`] takes the environment (SSH, tmux, screen) as
//! parameters rather than reading `std::env` itself, mirroring
//! [`crate::multiplexer::detect_tmux`]. The whole decision matrix is then
//! unit-testable without an SSH session or a live multiplexer — which matters
//! more than usual here, because OSC52 has no acknowledgement: a wrong
//! sequence cannot be caught at runtime, only printed as garbage.
//!
//! # Known limits (not bugs — inherent to OSC52)
//!
//! - **No confirmation.** Nothing reports back whether the terminal accepted
//!   the sequence. A success message means "emitted", not "copied".
//! - **tmux needs `allow-passthrough`.** Off by default since tmux 3.3. The
//!   DCS wrapper below is necessary but not sufficient; the user must enable
//!   the option. gwm cannot detect or force this.
//! - **Terminal support varies.** kitty, WezTerm, Alacritty and iTerm2 (with
//!   the setting on) honour OSC52; Terminal.app does not. Hence
//!   [`ClipboardMode::Tools`] as an escape hatch.

use crate::config::ClipboardMode;
use crossterm::clipboard::CopyToClipboard;
use crossterm::Command;

/// Maximum input length, in bytes, gwm will put in an OSC52 sequence.
///
/// Terminals cap the length they will accept and the ceiling is not
/// standardised — 100k is a common one, several are far lower. This is a
/// deliberately conservative bound on the *input*: base64 inflates by 4/3, so
/// 64 KiB of text becomes ~87k of sequence, which stays under the usual caps.
///
/// The alternative to a ceiling is emitting a sequence the terminal truncates,
/// which pastes as corrupt text or swallows subsequent output. Refusing is the
/// honest failure — the Command Logs copy can realistically reach this size.
pub const MAX_OSC52_BYTES: usize = 64 * 1024;

/// What the caller should do to put the text on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPlan {
  /// Write these exact bytes to the terminal. Already framed, and already
  /// wrapped for the multiplexer when one was detected.
  Osc52(Vec<u8>),
  /// Use the host binaries ([`crate::tui::clipboard_candidates`]).
  Tools,
  /// The text is too long for an OSC52 sequence. Carries the input length so
  /// the caller can say so; `bytes` is the input, not the encoded size.
  TooLarge { bytes: usize },
}

/// Decide how to copy `text`, and build the sequence when the answer is OSC52.
///
/// `is_ssh` / `in_tmux` / `in_screen` are injected (see the module note). The
/// caller derives them from `$SSH_TTY`/`$SSH_CONNECTION`, `$TMUX` and `$STY`.
pub fn plan_clipboard_write(
  text: &str,
  mode: ClipboardMode,
  is_ssh: bool,
  in_tmux: bool,
  in_screen: bool,
) -> ClipboardPlan {
  let wants_osc52 = match mode {
    ClipboardMode::Tools => false,
    ClipboardMode::Osc52 => true,
    ClipboardMode::Auto => is_ssh,
  };
  if !wants_osc52 {
    return ClipboardPlan::Tools;
  }

  // GNU screen wants its own chunked DCS form, with a per-sequence length limit
  // well below tmux's. An unwrapped sequence inside screen is silently eaten,
  // so the user would get a success message and an empty clipboard. Degrading
  // to the host tools is the honest option: they either work, or report a real
  // error. Doing screen properly is a follow-up, not a silent half-measure.
  if in_screen {
    return ClipboardPlan::Tools;
  }

  if text.len() > MAX_OSC52_BYTES {
    return ClipboardPlan::TooLarge { bytes: text.len() };
  }

  let mut seq = String::new();
  // Framing (`\x1b]52;c;<base64>\x1b\\`) comes from crossterm's own
  // `CopyToClipboard`, whose tests pin those bytes — no reason to hand-roll it,
  // or the base64, here. `write_ansi` renders into a buffer rather than the
  // terminal, which is what keeps this function pure.
  if CopyToClipboard::to_clipboard_from(text).write_ansi(&mut seq).is_err() {
    // Writing into a String cannot fail in practice; fall back to the tools
    // rather than unwrap on a user-facing path (CLAUDE.md).
    return ClipboardPlan::Tools;
  }

  let bytes = if in_tmux {
    wrap_tmux_passthrough(&seq)
  } else {
    seq.into_bytes()
  };
  ClipboardPlan::Osc52(bytes)
}

/// Wrap `seq` in tmux's DCS passthrough so tmux forwards it to the outer
/// terminal instead of interpreting it: `\x1bPtmux;<seq with ESC doubled>\x1b\\`.
///
/// Every ESC inside the payload must be doubled — the OSC52 sequence has two
/// (the `\x1b]` introducer and the `\x1b\\` terminator).
///
/// Requires `set -g allow-passthrough on` in the user's tmux config; it is off
/// by default since tmux 3.3 and nothing here can change that.
fn wrap_tmux_passthrough(seq: &str) -> Vec<u8> {
  let mut out = Vec::with_capacity(seq.len() + 16);
  out.extend_from_slice(b"\x1bPtmux;");
  for b in seq.bytes() {
    if b == 0x1b {
      out.push(0x1b);
    }
    out.push(b);
  }
  out.extend_from_slice(b"\x1b\\");
  out
}
