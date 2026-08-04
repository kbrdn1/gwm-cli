//! One invariant, checked by construction rather than stated in a doc
//! comment (issue #507).
//!
//! `std::env::set_var` is `unsafe` because it races a concurrent `getenv`, so
//! a test binary that rewrites a process-global variable has to serialise
//! every test that can **observe** it. Each such binary keeps its own mutex
//! for that, and the rule has so far lived in the mutex's doc comment. #503 is
//! what a doc-comment rule is worth: it said "tests that expand a `{home}`
//! pattern" while the real boundary was "tests that call
//! `expand_placeholders`", and five tests were written against the sentence.
//!
//! Nothing here is a hand-maintained list, because every hand-maintained list
//! written while fixing #507 was wrong within the hour:
//!
//! - naming four binaries missed the eight others that also call `set_var`;
//! - naming the reader functions missed the wrappers that reach them
//!   (`Config::load_exec_config` and `global_forge_host` both call
//!   `global_config_path`, `history::record` calls `default_journal_path`);
//! - looking for the text `lock()` accepts a lock taken *after* the read,
//!   which serialises nothing.
//!
//! So all three are derived: the binaries from their own `set_var` calls, the
//! readers from a transitive walk of `src/`, and the ordering from where the
//! guard sits relative to the call.
//!
//! # What it does not catch, on purpose
//!
//! A reader that takes the variable *name* as a parameter. `trust::env_truthy`
//! calls `std::env::var(key)`, so nothing links it to any particular variable
//! by reading the source. Seeding on non-literal `env::var` calls was tried
//! and measured: it marks `env_truthy` a reader of everything, which through
//! the transitive walk reaches `read_link` and `enter_open_menu`, and demands
//! a lock from about a third of `forge_tests` and `tui_app_tests` including
//! binaries whose own rewritten-variable set is empty. That trades a targeted
//! guard for one whose failures carry no information, so the bound stays and
//! is written here instead.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Read a source file with CRLF normalised.
///
/// A Windows checkout stores these with `\r\n`, and every line-anchored match
/// below would miss, leaving this guard green on exactly one of the three
/// runners.
fn read(path: &Path) -> String {
  std::fs::read_to_string(path)
    .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    .replace("\r\n", "\n")
}

fn rs_files(dir: &Path) -> Vec<PathBuf> {
  let mut out = Vec::new();
  let mut stack = vec![dir.to_path_buf()];
  while let Some(d) = stack.pop() {
    for entry in std::fs::read_dir(&d).unwrap_or_else(|e| panic!("cannot list {}: {e}", d.display())) {
      let path = entry.unwrap().path();
      if path.is_dir() {
        stack.push(path);
      } else if path.extension().is_some_and(|e| e == "rs") {
        out.push(path);
      }
    }
  }
  out.sort();
  out
}

/// Every `(name, body)` pair in a Rust source, where the body runs to the next
/// `fn`.
///
/// Deliberately over-approximating: a body that swallows a nested `fn` makes
/// the outer one look like it calls what the inner one calls, which can only
/// ever ask for *more* locking, never less. A guard that errs has to err
/// towards failing.
fn functions(source: &str) -> Vec<(String, String)> {
  let mut out: Vec<(String, String)> = Vec::new();
  let bytes: Vec<usize> = source
    .match_indices("fn ")
    .filter(|(at, _)| {
      *at == 0
        || !source[..*at]
          .chars()
          .next_back()
          .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
    .map(|(at, _)| at)
    .collect();
  for (i, start) in bytes.iter().enumerate() {
    let end = bytes.get(i + 1).copied().unwrap_or(source.len());
    let chunk = &source[*start..end];
    let name = chunk[3..].split('(').next().unwrap_or_default().trim().to_string();
    if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
      out.push((name, chunk.to_string()));
    }
  }
  out
}

/// Lines that are not comments. A function named in prose is not called by it,
/// which is how a first pass counted five comments as calls to
/// `Config::load_for_repo`.
fn code_lines(body: &str) -> impl Iterator<Item = &str> {
  body.lines().filter(|l| !l.trim_start().starts_with("//"))
}

/// Byte offset of the first **call** to `name` in `body`, if any.
///
/// A call, not a mention: the character before must not be part of an
/// identifier (so `resolve_global_config_path` is not `global_config_path`)
/// and the next must be `(`.
fn call_at(body: &str, name: &str) -> Option<usize> {
  let mut offset = 0usize;
  for line in body.lines() {
    let line_len = line.len() + 1;
    if !line.trim_start().starts_with("//") {
      for (at, _) in line.match_indices(name) {
        let before_ok = at == 0
          || !line[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if before_ok && line[at + name.len()..].starts_with('(') {
          return Some(offset + at);
        }
      }
    }
    offset += line_len;
  }
  None
}

/// Environment variables `source` rewrites with `set_var` / `remove_var`.
fn mutated_vars(source: &str) -> BTreeSet<String> {
  let mut out = BTreeSet::new();
  for line in code_lines(source) {
    for call in ["set_var(\"", "remove_var(\""] {
      let mut rest = line;
      while let Some(at) = rest.find(call) {
        rest = &rest[at + call.len()..];
        if let Some(end) = rest.find('"') {
          out.insert(rest[..end].to_string());
        }
      }
    }
  }
  out
}

/// Names carried by exactly one function in `src/`.
///
/// Matching happens by name, with no scope resolution, so a name several
/// functions share cannot be attributed: `new` and `default` exist on dozens
/// of types, one of which reaches `dirs::home_dir()`, and treating that as a
/// reader made every `TempDir::new()` in the suite an offender. Restricting to
/// unambiguous names is the honest bound, and it is a bound: a reader whose
/// name collides with anything is invisible here, so the doc comments still
/// carry the rule for those.
fn unambiguous(src: &[(String, String)]) -> BTreeSet<String> {
  let mut seen: std::collections::BTreeMap<&str, usize> = Default::default();
  for (name, _) in src {
    *seen.entry(name.as_str()).or_default() += 1;
  }
  seen
    .into_iter()
    .filter(|(_, n)| *n == 1)
    .map(|(name, _)| name.to_string())
    .collect()
}

/// Functions in `src/` that can observe any of `vars`, transitively.
///
/// Seeded with the ones that read the variable themselves, then closed over
/// callers until it stops growing: `default_journal_path` reads
/// `$GWM_HISTORY_FILE`, so `history::record`, which calls it, observes it too,
/// and so does anything calling *that*.
///
/// `$HOME` is seeded differently because nothing reads it by name: it is
/// reached through `dirs::home_dir()`, and through the `shellexpand::tilde`
/// pass `expand_placeholders` ends with.
fn ambient_readers(src: &[(String, String)], vars: &BTreeSet<String>) -> BTreeSet<String> {
  let unique = unambiguous(src);
  let mut seeds: Vec<String> = Vec::new();
  for var in vars {
    if var == "HOME" {
      seeds.push("dirs::home_dir(".into());
      seeds.push("shellexpand::tilde(".into());
    } else {
      seeds.push(format!("env::var(\"{var}\""));
      seeds.push(format!("env::var_os(\"{var}\""));
    }
  }

  let mut readers: BTreeSet<String> = src
    .iter()
    .filter(|(name, _)| unique.contains(name.as_str()))
    .filter(|(_, body)| code_lines(body).any(|l| seeds.iter().any(|s| l.contains(s.as_str()))))
    .map(|(name, _)| name.clone())
    .collect();

  loop {
    let mut grew = false;
    for (name, body) in src {
      if readers.contains(name) || !unique.contains(name.as_str()) {
        continue;
      }
      // `contains` first: `call_at` walks the body line by line, and skipping
      // it for the overwhelming majority that do not mention the name at all
      // is the difference between a second and a minute.
      if readers
        .iter()
        .any(|r| body.contains(r.as_str()) && call_at(body, r).is_some())
      {
        readers.insert(name.clone());
        grew = true;
      }
    }
    if !grew {
      break;
    }
  }
  readers
}

/// The two shapes a test binary uses to take its lock.
///
/// A `-> &'static Mutex<()>` helper hands back the mutex, so the acquisition
/// is `helper().lock()`. A `-> MutexGuard` helper takes the lock itself and
/// hands back the guard, so calling it *is* the acquisition. Both are in the
/// tree (`env_lock` and `clean_env`), and reading only the first shape reports
/// every `clean_env()` test in `gitlab_tests` and `forge_tests` as unguarded.
struct Locks {
  /// Needs `.lock()` chained onto the call.
  mutexes: Vec<String>,
  /// The call alone acquires, because the guard is the return value.
  providers: Vec<String>,
}

fn lock_helpers(source: &str) -> Locks {
  let mut mutexes = Vec::new();
  let mut providers = Vec::new();
  for (name, body) in functions(source) {
    let head = body.lines().next().unwrap_or_default();
    if head.contains("MutexGuard") {
      providers.push(name);
    } else if head.contains("Mutex") {
      mutexes.push(name);
    }
  }
  Locks { mutexes, providers }
}

/// Byte offset of the first acquisition of one of `helpers`, if any.
///
/// The `.lock()` has to be chained to the helper call, not merely present
/// somewhere after it: `env_lock(); reader(); other.lock();` returns a
/// reference, reads unprotected, and then locks something unrelated, which an
/// "is there a `.lock()` later" test reads as a guarded body.
fn guard_at(body: &str, locks: &Locks) -> Option<usize> {
  let chained = locks.mutexes.iter().filter_map(|h| {
    let at = call_at(body, h)?;
    let after = &body[at + h.len()..];
    let after = after.strip_prefix('(')?;
    let after = after.trim_start().strip_prefix(')')?;
    // Allow the line break rustfmt inserts on a long chain.
    after.trim_start().starts_with(".lock()").then_some(at)
  });
  let provided = locks.providers.iter().filter_map(|p| call_at(body, p));
  chained.chain(provided).min()
}

#[test]
fn every_test_that_can_observe_a_rewritten_env_var_locks_first() {
  let root = repo_root();
  let src: Vec<(String, String)> = rs_files(&root.join("src"))
    .iter()
    .flat_map(|p| functions(&read(p)))
    .collect();

  let mut audited = 0usize;
  let mut offenders = Vec::new();

  for path in rs_files(&root.join("tests")) {
    let source = read(&path);
    let file = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
    if file.contains("env_guard_invariant") {
      continue; // its own `set_var` strings are the fixture below
    }
    // Membership is "does this binary rewrite the environment at all", not
    // "does it name a variable we could parse". `forge_tests::clean_env` loops
    // over a list and calls `remove_var(v)`, so the literal scan finds nothing
    // and the binary would drop out of the audit entirely while the count
    // stayed plausible.
    if !source.contains("set_var(") && !source.contains("remove_var(") {
      continue;
    }
    audited += 1;

    // Empty when every key is indirect; the readers derived from it are then
    // empty too, and what still holds for the binary is the rule that the
    // rewrite itself must be locked.
    let vars = mutated_vars(&source);
    let readers = ambient_readers(&src, &vars);
    let helpers = lock_helpers(&source);
    for (name, body) in functions(&source) {
      // The rewrite itself has to be serialised, not only the reads: a test
      // that only calls `set_var` (seeding the environment for a child
      // process, say) still races every concurrent reader, and matching no
      // reader name would otherwise excuse it.
      let mutation = ["set_var(", "remove_var("]
        .iter()
        .filter_map(|m| call_at(&body, m.trim_end_matches('(')))
        .min();
      let reader_hit = readers
        .iter()
        .filter(|r| body.contains(r.as_str()))
        .filter_map(|r| call_at(&body, r).map(|at| (r, at)))
        .min_by_key(|(_, at)| *at);
      let (reader, read_at) = match (reader_hit, mutation) {
        (Some((_, at)), Some(m)) if m < at => ("set_var/remove_var", m),
        (Some((r, at)), _) => (r.as_str(), at),
        (None, Some(m)) => ("set_var/remove_var", m),
        (None, None) => continue,
      };
      match guard_at(&body, &helpers) {
        Some(lock_at) if lock_at < read_at => {}
        Some(_) => offenders.push(format!(
          "{file}::{name} takes its lock AFTER calling {reader}, which serialises nothing (vars: {vars:?})"
        )),
        None => offenders.push(format!(
          "{file}::{name} calls {reader}, which can observe {vars:?}, without taking the binary's lock"
        )),
      }
    }
  }

  // A sweep that silently stops finding binaries reports "no offenders" for
  // the same reason an empty one does. Ten rewrite the environment today,
  // eleven counting this file, which is skipped because its own fixtures
  // contain the strings being searched for. The floor moves up when one is
  // added, never down without saying why.
  assert!(
    audited >= 10,
    "expected the sweep to find the env-rewriting binaries, found {audited}"
  );
  assert!(
    offenders.is_empty(),
    "a test that can observe a rewritten environment variable must hold the lock across the read:\n  {}",
    offenders.join("\n  ")
  );
}

#[test]
fn the_guard_can_actually_fire() {
  // A guard built from three exclusions can pass by matching nothing, so each
  // one is exercised in both directions against bodies written here.
  let src = vec![
    (
      "default_journal_path".to_string(),
      "fn default_journal_path() {\n  std::env::var(\"GWM_HISTORY_FILE\")\n}\n".to_string(),
    ),
    (
      "record".to_string(),
      "fn record() {\n  default_journal_path();\n}\n".to_string(),
    ),
    (
      "unrelated".to_string(),
      "fn unrelated() {\n  something_else();\n}\n".to_string(),
    ),
  ];
  let vars: BTreeSet<String> = ["GWM_HISTORY_FILE".to_string()].into_iter().collect();
  let readers = ambient_readers(&src, &vars);

  assert!(
    readers.contains("default_journal_path"),
    "the direct reader is a reader"
  );
  assert!(
    readers.contains("record"),
    "a wrapper reaching it transitively is one too, which a list of direct names misses"
  );
  assert!(!readers.contains("unrelated"), "and nothing else is");

  let helpers = Locks {
    mutexes: vec!["env_lock".to_string()],
    providers: vec!["clean_env".to_string()],
  };
  let before = "fn t() {\n  let _g = env_lock().lock().unwrap();\n  record();\n}\n";
  let after = "fn t() {\n  record();\n  let _g = env_lock().lock().unwrap();\n}\n";
  let none = "fn t() {\n  record();\n}\n";
  let prose = "fn t() {\n  // record() would read the journal path\n}\n";
  let longer = "fn t() {\n  wrapped_record();\n}\n";
  let unchained = "fn t() {\n  env_lock();\n  record();\n  other.lock();\n}\n";
  let provided = "fn t() {\n  let _g = clean_env();\n  record();\n}\n";

  assert!(
    guard_at(before, &helpers) < call_at(before, "record"),
    "locked first is fine"
  );
  assert!(
    guard_at(after, &helpers) > call_at(after, "record"),
    "locked afterwards serialises nothing and must be caught"
  );
  assert!(guard_at(none, &helpers).is_none(), "no guard at all is caught");
  assert!(
    guard_at(unchained, &helpers).is_none(),
    "the helper without a chained .lock() returns a reference and protects nothing, whatever locks later"
  );
  assert!(
    guard_at(provided, &helpers) < call_at(provided, "record"),
    "a helper that returns the guard acquires on call"
  );
  assert!(call_at(prose, "record").is_none(), "prose is not a call");
  assert!(
    call_at(longer, "record").is_none(),
    "a longer identifier is a different function"
  );
  assert_eq!(
    mutated_vars("  std::env::set_var(\"GWM_HISTORY_FILE\", p);\n"),
    vars,
    "the binary's own set_var is what says which variable it rewrites"
  );
}
