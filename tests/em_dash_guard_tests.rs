//! No string literal under `src/` carries an em dash (issue #567).
//!
//! The rule this repo writes under is that published prose does not use the
//! em dash, and the binary's own output is as published as the README. #516
//! swept `docs/` and #543 finished `skills/`; neither could reach `src/`,
//! where the strings a user reads on stderr and in the TUI live. Sweeping
//! them once fixes today; this is what stops the habit coming back, which is
//! the follow-up #516 closed without.
//!
//! # Why a lexer and not a grep
//!
//! `grep -rn '"[^"]*—[^"]*"'` finds 138 lines here and the real figure is
//! 161 literals: it cannot see a literal that spans lines, and it reads the
//! quotes inside a doc comment as a literal's delimiters. The opposite error
//! matters more. Roughly 1900 em dashes in this tree sit in comments and doc
//! comments, which are **not** in scope: a doc comment quoting a spec or a
//! command's real output has to stay verbatim. A guard that cannot tell the
//! two apart is a guard nobody can keep green.
//!
//! # Scope, stated
//!
//! `src/` only. The fixtures below are in `tests/`, so the sweep cannot
//! match its own strings and pass by finding itself; assert prose elsewhere
//! in the suite is not touched either, since a failure message is read by
//! whoever runs the suite, not published.
//!
//! Only U+2014. The en dash appears three times in this tree, all in
//! comments, all of them the range in `3–5×`, which is a numeral and not a
//! connector.
//!
//! # What it does not catch, on purpose
//!
//! A dash assembled at runtime: `format!("{a} {} {b}", '\u{2014}')` reads as
//! an escape here and nothing links it to the character. Nothing in the tree
//! does that, and covering it means evaluating const expressions.

use std::path::{Path, PathBuf};

const EM_DASH: char = '\u{2014}';

fn repo_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Read a source file with CRLF normalised.
///
/// A Windows checkout stores these with `\r\n`. Nothing below is
/// line-anchored, but the reported line numbers are, and an offender list
/// pointing at the wrong lines is worse than no list.
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

/// A literal, with the 1-based line it opens on.
#[derive(Debug, PartialEq, Eq)]
struct Literal {
  line: usize,
  text: String,
}

fn is_ident(c: char) -> bool {
  c.is_alphanumeric() || c == '_'
}

/// Every string and character literal in `source`, in order.
///
/// A hand-written scanner rather than a regex because the three things that
/// have to be skipped, both comment forms and the lifetime tick, are exactly
/// the ones a regex cannot separate from what they contain. It walks `char`s
/// rather than bytes so an em dash inside a literal cannot land the cursor
/// mid-sequence.
fn literals(source: &str) -> Vec<Literal> {
  let src: Vec<char> = source.chars().collect();
  let mut out = Vec::new();
  let mut i = 0usize;
  let mut line = 1usize;

  while i < src.len() {
    let c = src[i];

    if c == '\n' {
      line += 1;
      i += 1;
      continue;
    }

    // `//`, and with it `///` and `//!`.
    if c == '/' && src.get(i + 1) == Some(&'/') {
      while i < src.len() && src[i] != '\n' {
        i += 1;
      }
      continue;
    }

    // `/* */`, which nests in Rust: `/* /* */ */` is one comment, and
    // stopping at the first `*/` would resume the scan inside it.
    if c == '/' && src.get(i + 1) == Some(&'*') {
      let mut depth = 1usize;
      i += 2;
      while i < src.len() && depth > 0 {
        if src[i] == '/' && src.get(i + 1) == Some(&'*') {
          depth += 1;
          i += 2;
        } else if src[i] == '*' && src.get(i + 1) == Some(&'/') {
          depth -= 1;
          i += 2;
        } else {
          if src[i] == '\n' {
            line += 1;
          }
          i += 1;
        }
      }
      continue;
    }

    // A string, with its optional `b` / `c` / `r` prefixes: `"`, `b"`, `r"`,
    // `r#"`, `br#"`. The prefix only counts when the character before it is
    // not part of an identifier, or the `r` ending `for` would open a raw
    // string on the next quote it meets.
    let mut p = i;
    while p < src.len() && p - i < 2 && matches!(src[p], 'b' | 'c' | 'r') {
      p += 1;
    }
    let prefixed = p > i;
    let raw = prefixed && src[p - 1] == 'r';
    let mut hashes = 0usize;
    let mut q = p;
    if raw {
      while src.get(q) == Some(&'#') {
        hashes += 1;
        q += 1;
      }
    }
    let opens = src.get(q) == Some(&'"') && (!prefixed || i == 0 || !is_ident(src[i - 1]));
    if opens {
      let start = line;
      let mut text = String::new();
      let mut j = q + 1;
      loop {
        match src.get(j) {
          None => break,
          Some(&'\\') if !raw => {
            // The escape is kept verbatim: `\"` must not close the literal,
            // and no escape in this tree spells an em dash.
            text.push('\\');
            if let Some(&e) = src.get(j + 1) {
              text.push(e);
              if e == '\n' {
                line += 1;
              }
            }
            j += 2;
          }
          Some(&'"') if closes(&src, j, hashes) => {
            j += 1 + hashes;
            break;
          }
          Some(&ch) => {
            if ch == '\n' {
              line += 1;
            }
            text.push(ch);
            j += 1;
          }
        }
      }
      out.push(Literal { line: start, text });
      i = j;
      continue;
    }

    // A character literal, which shares its opening tick with a lifetime.
    // `'a` and `'static` are told apart by what follows: a literal closes on
    // the character after the one it holds.
    if c == '\'' {
      if src.get(i + 1) == Some(&'\\') {
        // `'\''` closes on the tick after the escape, not on the escaped one.
        let mut j = i + 2;
        while j < src.len() && src[j] != '\'' {
          j += 1;
        }
        if j < src.len() {
          out.push(Literal {
            line,
            text: src[i + 1..j].iter().collect(),
          });
          i = j + 1;
          continue;
        }
      } else if src.get(i + 2) == Some(&'\'') {
        out.push(Literal {
          line,
          text: src[i + 1..i + 2].iter().collect(),
        });
        i += 3;
        continue;
      }
      i += 1;
      continue;
    }

    i += 1;
  }

  out
}

/// Whether the `"` at `at` closes a literal opened with `hashes` hashes.
fn closes(src: &[char], at: usize, hashes: usize) -> bool {
  (1..=hashes).all(|n| src.get(at + n) == Some(&'#'))
}

#[test]
fn no_string_literal_under_src_carries_an_em_dash() {
  let root = repo_root();
  let mut scanned = 0usize;
  let mut offenders = Vec::new();

  for path in rs_files(&root.join("src")) {
    let file = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
    for lit in literals(&read(&path)) {
      scanned += 1;
      if lit.text.contains(EM_DASH) {
        let shown: String = lit.text.chars().take(90).collect();
        offenders.push(format!("{file}:{} {shown}", lit.line));
      }
    }
  }

  // A scanner that desynchronises reports no offenders for the same reason a
  // clean tree does, and the two are indistinguishable from the outside. The
  // tree holds 4867 literals today, so a floor well under it still catches
  // the failure that matters: one mis-parsed construct swallowing the rest of
  // a file, or of every file.
  assert!(
    scanned >= 4000,
    "expected the scan to find the literals in src/, found {scanned}"
  );
  assert!(
    offenders.is_empty(),
    "a string printed by gwm must not carry an em dash (issue #567): use a colon where it \
     introduces an explanation, a comma for an apposition, a full stop where the clause \
     already carries a colon. {} offender(s):\n  {}",
    offenders.len(),
    offenders.join("\n  ")
  );
}

#[test]
fn the_guard_can_actually_fire() {
  // Each construct the scanner has to tell apart, in both directions, since
  // a scanner that skips too much and one that skips nothing both report an
  // empty offender list.
  let found = |s: &str| literals(s).iter().any(|l| l.text.contains(EM_DASH));

  assert!(found(r#"let m = "broke — do this";"#), "a plain literal is in scope");
  assert!(
    found(r##"let m = r#"broke — do this"#;"##),
    "and so is a raw one, hashes and all"
  );
  assert!(
    found("let m = \"says \\\" then — this\";"),
    "an escaped quote does not close the literal, so what follows it is still inside"
  );
  assert!(found("let m = \"first\nsecond — third\";"), "a literal may span lines");
  assert!(found(r#"let c = '—';"#), "a character literal holds one too");

  assert!(!found("// broke — do this"), "a line comment is prose about the code");
  assert!(
    !found("/// broke — do this"),
    "a doc comment quoting output stays verbatim"
  );
  assert!(!found("//! broke — do this"), "and so does a module one");
  assert!(!found("/* broke — do this */"), "a block comment is not output");
  assert!(
    !found("/* outer /* inner — dash */ still outer */ let m = \"clean\";"),
    "block comments nest, and stopping at the first close resumes inside one"
  );
  assert!(
    !found("fn f<'a>(s: &'a str) -> &'a str { s } // — prose"),
    "a lifetime is not an unterminated character literal"
  );
  assert!(!found(r"let c = '\'';"), "an escaped tick closes on the next one");

  // Line numbers are what the offender list is worth.
  let counted = literals("fn a() {}\n// —\nlet m = \"x — y\";\n");
  assert_eq!(
    counted,
    vec![Literal {
      line: 3,
      text: "x — y".into()
    }],
    "the comment is skipped and the literal is reported on its own line"
  );
}

#[test]
fn the_hard_constructs_are_read_off_the_real_tree() {
  // Fixtures prove the scanner handles a construct; only the tree proves it
  // handles the ones actually in it. Both of these were written before this
  // guard and neither is a shape the fixtures above invented: the hook
  // script is a multi-line `r#"..."#` holding `#`, `"` and `'`, and the
  // shell snippet holds an interior `"$@"`.
  let hooks = literals(&read(&repo_root().join("src/hooks.rs")));
  assert!(
    hooks
      .iter()
      .any(|l| l.text.contains("#!/bin/sh") && l.text.contains("gwm commit-prefix --unicode")),
    "the generated commit-msg hook is one raw literal, start to end"
  );

  let cli = literals(&read(&repo_root().join("src/cli.rs")));
  assert!(
    cli.iter().any(|l| l.text.contains("unalias gcd")),
    "and so is the generated shell helper"
  );
  assert!(
    cli.iter().any(|l| l.text.contains("\\\"")),
    "a literal spelling an escaped quote survives the scan"
  );
}
