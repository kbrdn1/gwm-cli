//! Unit tests for the pure clipboard planner (issue #367).
//!
//! `plan_clipboard_write` decides *how* to put text on the clipboard —
//! OSC52 escape sequence vs the host binaries — and builds the exact bytes
//! for the OSC52 path. It is pure: the environment (SSH / tmux / screen) is
//! injected as parameters, mirroring `multiplexer::detect_tmux`, so the whole
//! matrix is testable without an SSH session or a multiplexer.
//!
//! The bytes matter here in a way they rarely do: nothing acknowledges an
//! OSC52 write. A malformed sequence does not error — it silently prints
//! garbage or vanishes. So the framing is pinned byte-for-byte.

use gwm::clipboard::{plan_clipboard_write, ClipboardPlan, MAX_OSC52_BYTES};
use gwm::config::ClipboardMode;

/// The env combination for a plain local terminal.
const LOCAL: Env = Env {
  ssh: false,
  tmux: false,
  screen: false,
};
/// A bare SSH session, no multiplexer.
const SSH: Env = Env {
  ssh: true,
  tmux: false,
  screen: false,
};

#[derive(Clone, Copy)]
struct Env {
  ssh: bool,
  tmux: bool,
  screen: bool,
}

fn plan(text: &str, mode: ClipboardMode, env: Env) -> ClipboardPlan {
  plan_clipboard_write(text, mode, env.ssh, env.tmux, env.screen)
}

fn osc_bytes(p: &ClipboardPlan) -> &[u8] {
  match p {
    ClipboardPlan::Osc52(b) => b,
    other => panic!("expected an OSC52 plan, got {other:?}"),
  }
}

// ---------------------------------------------------------------------------
// `auto` — the decision the whole issue turns on
// ---------------------------------------------------------------------------

#[test]
fn auto_uses_host_tools_when_not_over_ssh() {
  // Local terminal: the host tools work and own the clipboard. Emitting OSC52
  // here would regress users whose terminal ignores it.
  assert_eq!(plan("x", ClipboardMode::Auto, LOCAL), ClipboardPlan::Tools);
}

#[test]
fn auto_uses_osc52_over_ssh() {
  // The reported bug (#363): over SSH the host tools "succeed" while writing
  // to the wrong machine's clipboard, so `auto` must route through the
  // terminal instead.
  assert!(matches!(plan("x", ClipboardMode::Auto, SSH), ClipboardPlan::Osc52(_)));
}

// ---------------------------------------------------------------------------
// Explicit overrides beat detection, both ways
// ---------------------------------------------------------------------------

#[test]
fn explicit_osc52_applies_even_locally() {
  assert!(matches!(
    plan("x", ClipboardMode::Osc52, LOCAL),
    ClipboardPlan::Osc52(_)
  ));
}

#[test]
fn explicit_tools_applies_even_over_ssh() {
  // The escape hatch: a terminal that mangles OSC52 must be able to opt out,
  // even though `auto` would have picked OSC52 here.
  assert_eq!(plan("x", ClipboardMode::Tools, SSH), ClipboardPlan::Tools);
}

// ---------------------------------------------------------------------------
// Framing — pinned byte-for-byte against crossterm's own contract
// ---------------------------------------------------------------------------

#[test]
fn osc52_sequence_is_base64_in_an_osc_52_frame() {
  // "foo" → "Zm9v" is crossterm's own documented example, so this also pins
  // that we build on its framing rather than reinventing it.
  let p = plan("foo", ClipboardMode::Osc52, LOCAL);
  assert_eq!(osc_bytes(&p), b"\x1b]52;c;Zm9v\x1b\\");
}

#[test]
fn osc52_encodes_utf8_payloads() {
  // Branch names are not ASCII-only; base64 operates on the UTF-8 bytes.
  let p = plan("é", ClipboardMode::Osc52, LOCAL);
  assert_eq!(osc_bytes(&p), b"\x1b]52;c;w6k=\x1b\\");
}

#[test]
fn osc52_handles_empty_text() {
  // Yanking an empty string must stay a well-formed sequence, not a truncated
  // frame the terminal would swallow the next keystrokes into.
  let p = plan("", ClipboardMode::Osc52, LOCAL);
  assert_eq!(osc_bytes(&p), b"\x1b]52;c;\x1b\\");
}

// ---------------------------------------------------------------------------
// Multiplexer passthrough
// ---------------------------------------------------------------------------

#[test]
fn tmux_wraps_the_sequence_in_dcs_passthrough_with_doubled_escapes() {
  // tmux eats a bare OSC52. The DCS passthrough form is `\x1bPtmux;<inner>\x1b\\`
  // with every ESC of the inner sequence doubled — both of them here (the OSC
  // introducer and the ST terminator).
  let env = Env {
    ssh: true,
    tmux: true,
    screen: false,
  };
  let p = plan("foo", ClipboardMode::Osc52, env);
  assert_eq!(osc_bytes(&p), b"\x1bPtmux;\x1b\x1b]52;c;Zm9v\x1b\x1b\\\x1b\\");
}

#[test]
fn no_tmux_wrapping_outside_tmux() {
  let p = plan("foo", ClipboardMode::Osc52, SSH);
  assert!(
    !osc_bytes(&p).starts_with(b"\x1bPtmux;"),
    "a bare terminal must not get the tmux wrapper"
  );
}

#[test]
fn screen_falls_back_to_tools_rather_than_emitting_a_sequence_it_would_eat() {
  // GNU screen needs its own chunked DCS form; emitting an unwrapped sequence
  // inside screen is a silent no-op. Degrading to the host tools is honest —
  // it either works or reports a real error — where a swallowed sequence gives
  // the user a success message and an empty clipboard.
  let env = Env {
    ssh: true,
    tmux: false,
    screen: true,
  };
  assert_eq!(plan("foo", ClipboardMode::Auto, env), ClipboardPlan::Tools);
}

// ---------------------------------------------------------------------------
// Size ceiling
// ---------------------------------------------------------------------------

#[test]
fn oversized_payload_is_refused_not_truncated() {
  // Terminals cap OSC52 length. A truncated sequence pastes as corrupt text
  // (or eats the following output), so an oversized payload must fail loudly
  // instead — the Command Logs copy can realistically hit this.
  let huge = "a".repeat(MAX_OSC52_BYTES * 2);
  assert_eq!(
    plan(&huge, ClipboardMode::Osc52, LOCAL),
    ClipboardPlan::TooLarge {
      bytes: MAX_OSC52_BYTES * 2
    }
  );
}

#[test]
fn payload_just_under_the_ceiling_is_still_emitted() {
  // The boundary is a real one: pin that the check is on the input length and
  // does not reject a payload that fits.
  let text = "a".repeat(MAX_OSC52_BYTES);
  assert!(matches!(
    plan(&text, ClipboardMode::Osc52, LOCAL),
    ClipboardPlan::Osc52(_)
  ));
}

#[test]
fn tools_mode_ignores_the_size_ceiling() {
  // The ceiling is a property of the escape sequence, not of the clipboard.
  // Piping a huge string into pbcopy is fine, so `tools` must not inherit it.
  let huge = "a".repeat(MAX_OSC52_BYTES * 2);
  assert_eq!(plan(&huge, ClipboardMode::Tools, LOCAL), ClipboardPlan::Tools);
}
