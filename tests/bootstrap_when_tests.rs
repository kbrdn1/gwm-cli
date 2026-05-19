//! Unit tests for `bootstrap::evaluate_when`, the `[[bootstrap.command]].when`
//! predicate evaluator. The single legacy keyword `file_exists:` is covered
//! alongside the four extended keywords introduced for issue #25
//! (`cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`) plus the boolean
//! composition operators `!`, `&&`, `||`.
//!
//! These tests deliberately exercise the evaluator directly (rather than
//! going through `bootstrap::run`) so the predicate grammar can be pinned
//! down without dragging in a shell, a temp `Config`, and the rest of the
//! bootstrap pipeline. A handful of integration tests through
//! `bootstrap::run` live next door in `bootstrap_tests.rs` to keep the
//! end-to-end wiring covered.

use gwm::bootstrap::evaluate_when;
use std::path::Path;
use tempfile::TempDir;

// Sentinel name used wherever a test needs an env var that is guaranteed
// not to be set. The suffix is random enough that no real environment
// would ever define it.
const UNSET_VAR: &str = "GWM_TEST_PREDICATE_DEFINITELY_UNSET_8c4f9a";

// --------------------------------------------------------------------------
// file_exists: — regression coverage for the legacy keyword
// --------------------------------------------------------------------------

#[test]
fn file_exists_true_when_target_present() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
  assert!(evaluate_when("file_exists:Cargo.toml", dir.path()));
}

#[test]
fn file_exists_false_when_target_missing() {
  let dir = TempDir::new().unwrap();
  assert!(!evaluate_when("file_exists:Cargo.toml", dir.path()));
}

// --------------------------------------------------------------------------
// cmd_exists: — binary resolves on $PATH
// --------------------------------------------------------------------------

#[test]
fn cmd_exists_true_for_sh() {
  // `sh` is guaranteed on every Unix host the project supports. On Windows
  // CI we'd substitute `cmd.exe`, but the integration matrix is Unix-only
  // today so this assertion is portable enough.
  let dir = TempDir::new().unwrap();
  assert!(evaluate_when("cmd_exists:sh", dir.path()));
}

#[test]
fn cmd_exists_false_for_made_up_binary() {
  let dir = TempDir::new().unwrap();
  assert!(!evaluate_when(
    "cmd_exists:gwm_test_definitely_no_such_binary_4f7a3b",
    dir.path()
  ));
}

// --------------------------------------------------------------------------
// env_set: — variable is defined (any value)
// --------------------------------------------------------------------------

#[test]
fn env_set_true_for_path() {
  let dir = TempDir::new().unwrap();
  // PATH is guaranteed set by every reasonable test runner.
  assert!(evaluate_when("env_set:PATH", dir.path()));
}

#[test]
fn env_set_false_for_unset_var() {
  let dir = TempDir::new().unwrap();
  let expr = format!("env_set:{}", UNSET_VAR);
  assert!(!evaluate_when(&expr, dir.path()));
}

// --------------------------------------------------------------------------
// env_eq: — variable exactly matches the provided literal
// --------------------------------------------------------------------------

#[test]
fn env_eq_true_when_value_matches() {
  // Picking a known env var whose value contains spaces or `&&` would
  // make the tokenizer split the atom mid-value, so we cherry-pick any
  // currently-set variable with a clean value at runtime. This keeps the
  // test portable across CI environments without relying on a specific
  // variable being defined.
  let dir = TempDir::new().unwrap();
  let (name, value) = std::env::vars()
    .find(|(_, v)| !v.is_empty() && !v.contains(char::is_whitespace) && !v.contains("&&") && !v.contains("||"))
    .expect("at least one env var should have a simple value at test time");
  let expr = format!("env_eq:{}={}", name, value);
  assert!(
    evaluate_when(&expr, dir.path()),
    "expected env_eq:{}={} to evaluate true",
    name,
    value
  );
}

#[test]
fn env_eq_false_when_value_mismatch() {
  let dir = TempDir::new().unwrap();
  assert!(!evaluate_when(
    "env_eq:PATH=/definitely/not/the/real/path/xyz",
    dir.path()
  ));
}

#[test]
fn env_eq_false_when_var_missing() {
  let dir = TempDir::new().unwrap();
  let expr = format!("env_eq:{}=whatever", UNSET_VAR);
  assert!(!evaluate_when(&expr, dir.path()));
}

// --------------------------------------------------------------------------
// glob_exists: — at least one path under cwd matches the glob
// --------------------------------------------------------------------------

#[test]
fn glob_exists_true_when_pattern_matches_in_root() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
  assert!(evaluate_when("glob_exists:*.rs", dir.path()));
}

#[test]
fn glob_exists_true_for_recursive_pattern() {
  let dir = TempDir::new().unwrap();
  std::fs::create_dir_all(dir.path().join("src/inner")).unwrap();
  std::fs::write(dir.path().join("src/inner/lib.rs"), "// nested").unwrap();
  assert!(evaluate_when("glob_exists:**/*.rs", dir.path()));
}

#[test]
fn glob_exists_false_when_no_match() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("README.md"), "no rust").unwrap();
  assert!(!evaluate_when("glob_exists:*.rs", dir.path()));
}

// --------------------------------------------------------------------------
// Boolean composition: ! (NOT)
// --------------------------------------------------------------------------

#[test]
fn not_inverts_file_exists_true() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("a"), "").unwrap();
  assert!(!evaluate_when("!file_exists:a", dir.path()));
}

#[test]
fn not_inverts_file_exists_false() {
  let dir = TempDir::new().unwrap();
  assert!(evaluate_when("!file_exists:missing", dir.path()));
}

// --------------------------------------------------------------------------
// Boolean composition: && (AND)
// --------------------------------------------------------------------------

#[test]
fn and_true_when_both_sides_true() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("a"), "").unwrap();
  std::fs::write(dir.path().join("b"), "").unwrap();
  assert!(evaluate_when("file_exists:a && file_exists:b", dir.path()));
}

#[test]
fn and_false_when_left_false() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("b"), "").unwrap();
  assert!(!evaluate_when("file_exists:a && file_exists:b", dir.path()));
}

#[test]
fn and_false_when_right_false() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("a"), "").unwrap();
  assert!(!evaluate_when("file_exists:a && file_exists:b", dir.path()));
}

// --------------------------------------------------------------------------
// Boolean composition: || (OR)
// --------------------------------------------------------------------------

#[test]
fn or_true_when_left_true() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("a"), "").unwrap();
  assert!(evaluate_when("file_exists:a || file_exists:b", dir.path()));
}

#[test]
fn or_true_when_right_true() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("b"), "").unwrap();
  assert!(evaluate_when("file_exists:a || file_exists:b", dir.path()));
}

#[test]
fn or_false_when_both_sides_false() {
  let dir = TempDir::new().unwrap();
  assert!(!evaluate_when("file_exists:a || file_exists:b", dir.path()));
}

// --------------------------------------------------------------------------
// Precedence: ! > && > ||
// --------------------------------------------------------------------------

#[test]
fn precedence_not_binds_tighter_than_and() {
  // Parsed as `(!file_exists:a) && file_exists:b`. With `a` missing and
  // `b` present, the result is `true && true == true`.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("b"), "").unwrap();
  assert!(evaluate_when("!file_exists:a && file_exists:b", dir.path()));
}

#[test]
fn precedence_and_binds_tighter_than_or() {
  // Parsed as `file_exists:a || (file_exists:b && file_exists:c)`. With
  // only `a` present, the disjunction short-circuits to true.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("a"), "").unwrap();
  assert!(evaluate_when(
    "file_exists:a || file_exists:b && file_exists:c",
    dir.path()
  ));
}

#[test]
fn precedence_and_binds_tighter_than_or_negative() {
  // Same expression, but `a` is missing and `b` is present without `c`.
  // The RHS group `b && c` is false, so the OR is false too.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("b"), "").unwrap();
  assert!(!evaluate_when(
    "file_exists:a || file_exists:b && file_exists:c",
    dir.path()
  ));
}

// --------------------------------------------------------------------------
// Whitespace tolerance — operators may be flanked by any amount of spaces
// --------------------------------------------------------------------------

#[test]
fn extra_whitespace_around_operators_is_tolerated() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("a"), "").unwrap();
  std::fs::write(dir.path().join("b"), "").unwrap();
  assert!(evaluate_when("  file_exists:a   &&   file_exists:b  ", dir.path()));
}

#[test]
fn unicode_whitespace_inside_atom_is_trimmed() {
  // The tokenizer only splits on ASCII whitespace, so a non-ASCII Unicode
  // whitespace character (here NBSP `\u{00A0}`) stays glued to the atom.
  // Pre-fix `evaluate_when` would forward the whitespace-prefixed string
  // straight to `Path::join`, producing a path that doesn't exist. The
  // restored `.trim()` on the keyword remainder keeps the legacy
  // `file_exists:` tolerance intact.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("payload"), "").unwrap();
  let expr = "file_exists:\u{00A0}payload";
  assert!(
    evaluate_when(expr, dir.path()),
    "expected `file_exists:` to strip leading Unicode whitespace"
  );
}

#[test]
fn unicode_path_does_not_panic() {
  // Regression guard for tokenize_when: it slices `expr` by byte index.
  // The delimiters (`!`, `&`, `|`, ASCII whitespace) are all single-byte
  // ASCII codepoints, so the loop can never break mid-character of a
  // multi-byte UTF-8 sequence. This test pins that invariant down.
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("σ.rs"), "// greek lowercase sigma").unwrap();
  assert!(evaluate_when("file_exists:σ.rs", dir.path()));
  assert!(!evaluate_when("file_exists:λ.rs", dir.path()));
}

// --------------------------------------------------------------------------
// Backward compatibility — unknown keyword still defaults to true
// --------------------------------------------------------------------------

#[test]
fn unknown_keyword_still_defaults_to_true() {
  // Matches the long-standing contract that lets old configs with future
  // predicates keep running while the doctor surfaces the unknown keyword.
  let dir = TempDir::new().unwrap();
  assert!(evaluate_when("totally_made_up:whatever", dir.path()));
}

// --------------------------------------------------------------------------
// Sanity probe — evaluator is a pure function of (expr, cwd)
// --------------------------------------------------------------------------

#[test]
fn evaluator_is_pure_for_a_given_cwd() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("a"), "").unwrap();
  let cwd: &Path = dir.path();
  let first = evaluate_when("file_exists:a && cmd_exists:sh", cwd);
  let second = evaluate_when("file_exists:a && cmd_exists:sh", cwd);
  assert_eq!(first, second);
}
