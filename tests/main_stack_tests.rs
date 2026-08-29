//! The CLI must not run on the stack Windows gives a process's main thread.
//!
//! Fallout of #617. `Cli::parse` alone sits near 1 MiB in a debug build:
//! clap's derive expands one `Command` builder per subcommand and per argument
//! into a single frame, and every `///` in `cli.rs` is a `long_help` string in
//! it. Adding three arguments to `Create` took the binary from "survives a
//! 1024 KiB stack, dies at 512" to "dies at 1024, survives 2048", so every
//! `gwm.exe` invocation on Windows aborted with `STATUS_STACK_OVERFLOW`
//! (`-1073741571`) while every Unix runner stayed green: macOS and Linux give
//! main 8 MiB.
//!
//! Trimming doc comments back under the ceiling buys one release. Choosing the
//! stack takes the ceiling out of the picture, and this pins that choice.
//!
//! Own integration target on purpose: the failure mode is a stack overflow,
//! which aborts the whole test binary rather than failing one assertion, so
//! the blast radius is this file alone and the abort is unambiguous.

use gwm::cli::{on_own_stack, STACK_SIZE};

/// What Windows gives a process's main thread.
///
/// The probe runs from a thread of exactly this size so the ambient stack is
/// Windows-sized on every runner. Run from the test's own main thread it would
/// pass on macOS and Linux whether or not `on_own_stack` spawns anything,
/// which is the vacuous version of this guard.
const WINDOWS_MAIN_STACK: usize = 1024 * 1024;

/// Deep enough to be impossible on `WINDOWS_MAIN_STACK`, shallow enough to sit
/// comfortably inside `STACK_SIZE`. Checked at compile time so shrinking
/// `STACK_SIZE` past the probe fails the build rather than turning this test
/// into a coin flip.
const PROBE_BYTES: usize = 4 * 1024 * 1024;
const _: () = assert!(PROBE_BYTES > WINDOWS_MAIN_STACK);
const _: () = assert!(STACK_SIZE >= PROBE_BYTES * 2);

/// One 64 KiB frame per call, `black_box`ed so the optimiser cannot elide the
/// array or turn the recursion into a loop.
#[inline(never)]
fn burn(remaining: usize) -> u8 {
  let frame = std::hint::black_box([0u8; 64 * 1024]);
  if remaining <= frame.len() {
    return std::hint::black_box(frame[0]);
  }
  burn(remaining - frame.len())
}

#[test]
fn the_cli_runs_on_a_stack_deeper_than_the_one_windows_gives_main() {
  let probed = std::thread::Builder::new()
    .stack_size(WINDOWS_MAIN_STACK)
    .spawn(|| on_own_stack(|| burn(PROBE_BYTES)).expect("spawn the gwm stack"))
    .expect("spawn the Windows-sized probe thread")
    .join()
    .expect("the probe thread must not panic");

  assert_eq!(probed, 0, "the probe must have run to its base case");
}
