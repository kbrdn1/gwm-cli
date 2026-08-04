//! One invariant, checked by construction rather than stated in a doc
//! comment (issue #507).
//!
//! `std::env::set_var` is `unsafe` because it races a concurrent `getenv`, so
//! a test binary that mutates a process-global variable has to serialise every
//! test that can **observe** it. Each such binary keeps its own mutex for that,
//! and the rule has so far lived in the mutex's doc comment. #503 is what a
//! doc-comment rule is worth: it said "tests that expand a `{home}` pattern"
//! while the real boundary was "tests that call `expand_placeholders`", and
//! five tests were written against the narrower sentence.
//!
//! So the rule is checked here instead. For each audited binary: every
//! function that calls one of the listed **ambient readers** must also take
//! that binary's lock.
//!
//! An *ambient reader* is a function that resolves the variable itself. The
//! distinction is the whole audit, and getting it wrong in either direction is
//! easy:
//!
//! - `Config::load_layered` and `resolved_rows` take the global config path as
//!   a **parameter**, so they read nothing. `Config::load_for_repo` calls
//!   `global_config_path()`, so it does.
//! - `resolve_global_config_path` takes every path as a parameter;
//!   `global_config_path` reads `$XDG_CONFIG_HOME` and `dirs::home_dir()`. One
//!   name contains the other, which is exactly how a substring search reports
//!   five tests that are in fact clean.

/// A test binary that mutates the environment, and what counts as observing it.
struct Audited {
  /// For the failure message.
  file: &'static str,
  /// The variable the binary rewrites with `set_var`.
  var: &'static str,
  /// Source of the binary, pulled in at compile time so this cannot drift
  /// from what is on disk or depend on the working directory.
  source: &'static str,
  /// Functions that resolve `var` themselves, verified in `src/`.
  ambient_readers: &'static [&'static str],
}

const AUDITED: &[Audited] = &[
  Audited {
    file: "tests/config_tests.rs",
    var: "HOME",
    source: include_str!("config_tests.rs"),
    // `expand_placeholders` resolves `dirs::home_dir()` before it looks at a
    // single token, and ends with `shellexpand::tilde`. `load_for_repo`
    // resolves the global config path, which reads `$HOME` on the way.
    ambient_readers: &["expand_placeholders", "Config::load_for_repo"],
  },
  Audited {
    file: "tests/config_global_tests.rs",
    var: "XDG_CONFIG_HOME",
    source: include_str!("config_global_tests.rs"),
    ambient_readers: &["global_config_path", "Config::load_for_repo"],
  },
  Audited {
    file: "tests/trust_tests.rs",
    var: "GWM_TRUST_LEDGER",
    source: include_str!("trust_tests.rs"),
    ambient_readers: &["default_ledger_path"],
  },
  Audited {
    file: "tests/history_tests.rs",
    var: "GWM_HISTORY_FILE",
    source: include_str!("history_tests.rs"),
    ambient_readers: &["default_journal_path"],
  },
];

/// Split a Rust source into `(name, body)` per top-level `fn`.
///
/// CRLF is normalised first: a Windows checkout stores these files with `\r\n`
/// and every line-anchored match would miss otherwise, leaving the guard green
/// on exactly one of the three runners.
fn functions(source: &str) -> Vec<(String, String)> {
  let normalised = source.replace("\r\n", "\n");
  let mut out = Vec::new();
  for chunk in normalised.split("\nfn ").skip(1) {
    let name = chunk.split('(').next().unwrap_or_default().to_string();
    out.push((name, chunk.to_string()));
  }
  out
}

/// Does `body` **call** `name`, as opposed to merely mentioning it?
///
/// Two exclusions, both learned from a search that got this wrong: a comment
/// naming the function is not a call (five of the six `load_for_repo` mentions
/// in `config_tests.rs` are prose), and a longer identifier ending in `name` is
/// a different function (`resolve_global_config_path` is not
/// `global_config_path`).
fn calls(body: &str, name: &str) -> bool {
  body.lines().filter(|l| !l.trim_start().starts_with("//")).any(|line| {
    line.match_indices(name).any(|(at, _)| {
      let before_ok = at == 0
        || !line[..at]
          .chars()
          .next_back()
          .is_some_and(|c| c.is_alphanumeric() || c == '_');
      let after_ok = line[at + name.len()..].starts_with('(');
      before_ok && after_ok
    })
  })
}

#[test]
fn every_test_that_can_observe_a_mutated_env_var_takes_its_lock() {
  let mut offenders = Vec::new();
  for binary in AUDITED {
    for (name, body) in functions(binary.source) {
      let reader = binary.ambient_readers.iter().find(|r| calls(&body, r));
      if let Some(reader) = reader {
        if !body.contains("lock()") {
          offenders.push(format!(
            "{}::{} calls {} (an ambient ${} reader) without taking the binary's lock",
            binary.file, name, reader, binary.var
          ));
        }
      }
    }
  }
  assert!(
    offenders.is_empty(),
    "a test that can observe a rewritten environment variable must be serialised against the rewrite:\n  {}",
    offenders.join("\n  ")
  );
}

#[test]
fn the_guard_can_actually_fire() {
  // A guard that never matches passes vacuously, and this one is built from
  // two exclusions that could each silently swallow every call. So: the same
  // predicate, against a body written here, in both directions.
  let guarded = "  let _g = env_lock().lock().unwrap();\n  expand_placeholders(\"{home}\", ...);\n";
  let bare = "  expand_placeholders(\"{home}\", ...);\n";
  let commented = "  // expand_placeholders(\"{home}\") would read $HOME\n";
  let longer_name = "  resolve_expand_placeholders(...);\n";

  assert!(calls(bare, "expand_placeholders"), "a bare call must be seen");
  assert!(calls(guarded, "expand_placeholders"), "a guarded call is still a call");
  assert!(guarded.contains("lock()"), "and the guard is what makes it acceptable");
  assert!(!calls(commented, "expand_placeholders"), "prose is not a call");
  assert!(
    !calls(longer_name, "expand_placeholders"),
    "a longer identifier is a different function"
  );
}
