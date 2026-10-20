//! Issue #688. The reader the workflow sweeps parse YAML with.
//!
//! `serde_yaml_ng` was replaced because `RUSTSEC-2024-0449` marks the
//! `unsafe-libyaml` under it unmaintained. Swapping the parser under every
//! read in these sweeps, 160 lines over four files, is only safe if the new
//! reading answers the old questions the same way, so this file pins the
//! answers rather than trusting that it does.
//!
//! With the old crate briefly re-declared, the two readers were compared head
//! to head through `At::at`, the accessor these sweeps actually use:
//! **187 comparisons over 17 documents and 8 keys, including five-level paths,
//! with zero divergence** on `is_null()`, `as_str()`, `as_bool()`, sequence
//! length and mapping size.
//!
//! Through `At::at` and not through an adapter, because the first run of this
//! measurement covered four helpers that were replaced before any of it
//! shipped: evidence for a path that was not delivered is not evidence. That
//! harness is gone with the dependency; what stays below is the answer it
//! produced, written as literals. A reader swapped again has to reproduce
//! these, or say in a diff why it does not.
//!
//! One behaviour did change and is not in the count above: mapping equality is
//! now order-sensitive, where the old reader compared order-insensitively.
//! `set` states it where it matters and the CHANGELOG records it.

mod common;

use common::{string_keys, yaml_doc, At};
use saphyr::{LoadableYamlNode, YamlOwned};

fn doc(src: &str) -> YamlOwned {
  yaml_doc("<test>", src)
}

/// An absent key and a key whose value is YAML null read alike.
///
/// Not an accident: `serde_yaml_ng` indexed a mapping and answered
/// `is_null() == true` for both, and every `must carry no X:` assertion in
/// these sweeps was written against that. `At::at` hands back a null node for
/// a key it did not find, which keeps the two together on purpose. Telling
/// them apart is now possible and would be a change to make with a test of its
/// own, which is why this one exists to notice it.
#[test]
fn an_absent_key_and_an_explicit_null_read_alike() {
  for src in ["other: 1", "if: ~", "if: null", "if:"] {
    assert!(
      doc(src).at("if").is_null(),
      "`{src}` must read as null for the `if` key"
    );
  }
}

/// A false boolean is not a null, which is the defect fixed at `6bb82758`.
///
/// `if: false` is the canonical way to switch a job off. A reader answering
/// "null" for it would take the "no `if:` at all" arm of
/// `assert_job_is_blocking` and pass the one case that guard exists to catch.
#[test]
fn a_false_boolean_is_not_read_as_absent() {
  let d = doc("if: false");
  assert!(
    !d.at("if").is_null(),
    "`if: false` must not read as null: it is the canonical off switch"
  );
  assert_eq!(d.at("if").as_bool(), Some(false));
  assert_eq!(
    d.at("if").as_str(),
    None,
    "a boolean must read as no string, so a sweep comparing `as_str()` cannot see it"
  );
}

/// Reading past an absent level answers null instead of panicking.
///
/// This is the whole reason `At` exists. `saphyr` panics when `[]` meets a key
/// that is not there or a scalar, where `serde_yaml_ng` answered `Null` and
/// let the next index run. A panic is not an assertion failure: it carries no
/// message about the rule that was broken, so a guard built on it would report
/// the parser instead of the workflow. Three levels, because the shortest real
/// path in these sweeps is a job's `defaults.run.shell`.
#[test]
fn reading_through_an_absent_level_answers_null() {
  let d = doc("jobs:\n  build:\n    id: 1\n");
  assert!(!d.at("jobs").at("build").is_null());
  assert!(d.at("jobs").at("nope").is_null());
  assert!(d.at("nope").at("deeper").at("deepest").is_null());
  assert!(
    d.at("jobs").at("build").at("id").at("x").is_null(),
    "reading through a scalar must answer null, not panic"
  );
}

/// A sequence read off an absent path is empty, not a panic.
#[test]
fn a_sequence_at_an_absent_path_is_empty() {
  let d = doc("strategy:\n  matrix:\n    os: [ubuntu-latest, macos-latest]\n");
  let len = |node: &YamlOwned| node.as_sequence().cloned().unwrap_or_default().len();
  assert_eq!(len(d.at("strategy").at("matrix").at("os")), 2);
  assert_eq!(len(d.at("strategy").at("matrix").at("exclude")), 0);
  assert_eq!(len(d.at("nope")), 0);
  assert_eq!(
    len(d.at("strategy").at("matrix")),
    0,
    "a mapping is not a sequence, and reading one as a sequence must be empty rather than a panic"
  );
}

/// The keys of a mapping come back in document order, not sorted.
///
/// Measured, because the cheaper replacement fails it: deserialising the YAML
/// into `serde_json::Value` would have been a near-mechanical rename, and it
/// sorts mapping keys. `string_keys` promises document order and its own test
/// asserts `["build", "release"]`, which is alphabetical, so the guarantee
/// would have gone while that test kept passing. The sample here is
/// deliberately not alphabetical.
#[test]
fn mapping_keys_keep_document_order() {
  let d = doc("jobs:\n  zebra:\n    id: 1\n  alpha:\n    id: 2\n  middle:\n    id: 3\n");
  assert_eq!(
    string_keys(d.at("jobs").as_mapping().expect("a mapping"), "jobs"),
    ["zebra", "alpha", "middle"],
    "the reader must preserve document order: a sweep reporting the first offending job by \
     position reports the wrong one if the keys are sorted"
  );
}

/// A key the reader does not read as a string is still visible as one.
///
/// `string_keys` panics on it rather than skipping it (#673), and that panic is
/// only reachable if the reader can represent the key at all. The
/// `serde_json::Value` route could not: it turns `true:` into the string
/// `"true"`, which leaves `string_keys` unable to fire and turns a live guard
/// into a vacuous one. Measured before that route was dropped.
#[test]
#[should_panic(expected = "every key must be a string")]
fn a_non_string_key_is_still_a_non_string_key() {
  let d = doc("jobs:\n  true:\n    id: 1\n");
  string_keys(d.at("jobs").as_mapping().expect("a mapping"), "jobs");
}

/// The three `permissions:` shapes the #681 guards compare stay distinct.
///
/// `write-all` is a string, `{}` is an empty mapping and an absent key is
/// nothing. A reader collapsing any two of them would pass one of the
/// mutations those guards were written against. `as_str()` reads `None` for
/// `{}` and for an absent key alike, which is the half of the trap #669 names;
/// what tells them apart is that `{}` is not null.
#[test]
fn the_three_permission_shapes_stay_distinct() {
  let wide = doc("permissions: write-all");
  let empty = doc("permissions: {}");
  let absent = doc("other: 1");

  assert_eq!(wide.at("permissions").as_str(), Some("write-all"));
  assert_eq!(empty.at("permissions").as_str(), None);
  assert_eq!(absent.at("permissions").as_str(), None);

  assert!(
    !empty.at("permissions").is_null(),
    "`permissions: {{}}` is a grant of nothing and must read as present"
  );
  assert!(
    absent.at("permissions").is_null(),
    "an absent `permissions:` must read as null, which is what tells it from `{{}}`"
  );
  assert_ne!(
    empty.at("permissions"),
    wide.at("permissions"),
    "comparing by value is what tells `{{}}` from `write-all`"
  );
}

/// Two parses of the same text compare equal, which is what every by-value
/// `permissions:` assertion rests on.
#[test]
fn equal_documents_compare_equal() {
  assert_eq!(doc("contents: read"), doc("contents: read"));
  assert_ne!(doc("contents: read"), doc("contents: write"));
}

/// CRLF is a line break, so a workflow checked out on Windows parses the same.
///
/// This repo has a standing rule that a test parsing a repo file normalises
/// line endings first, and the reason is not the YAML reader: it came from
/// `msrv_tests` searching the raw text for `"\n  msrv:\n"`, which matched
/// nothing on the Windows runner (#498). A substring search over bytes, with
/// no parser involved.
///
/// Stated because the first draft of this docstring invented a different
/// reason, that the old reader left a trailing `\r` on parsed values.
/// Measured on `dev`: `release_workflow_tests` parsed the workflows with no
/// normalisation at all, compared block-scalar `run:` against Rust literals,
/// and `test (windows-latest)` was green. So neither reader leaves a `\r`,
/// and the normalisation is deliberately not added back here: `saphyr`
/// implements YAML 1.2, where CRLF is a line break.
///
/// What the measurement actually covers, since "the suites pass" overstates
/// it: `msrv_tests::read` and `the_workflows_are_single_document` normalise
/// line endings themselves before parsing, so they cannot see a CRLF
/// workflow whatever the reader does. The suites that do are
/// `release_workflow_tests`, block-scalar `run:` compared against Rust
/// literals included, and `linux_packaging_metadata_tests`. Both pass with
/// all four workflow files converted, and `test (windows-latest)` is green.
/// The rule still holds for every other parser this repo reads a file with.
#[test]
fn crlf_parses_the_same_as_lf() {
  let lf = doc("permissions:\n  contents: read\njobs:\n  build:\n    id: 1\n");
  let crlf = doc("permissions:\r\n  contents: read\r\njobs:\r\n  build:\r\n    id: 1\r\n");
  assert_eq!(
    lf, crlf,
    "a CRLF document must parse to the same tree as its LF twin, or every by-value \
     comparison in these sweeps reads as changed on the Windows runner alone"
  );
  assert_eq!(
    crlf.at("permissions").at("contents").as_str(),
    Some("read"),
    "a scalar read out of a CRLF document must not carry the carriage return"
  );
}

/// Every workflow is a single document.
///
/// Each sweep reads the first one and only the first, so a second would be
/// unaudited. No file carries one today, which is why this is asserted rather
/// than assumed.
#[test]
fn the_workflows_are_single_document() {
  let mut audited = 0usize;
  for entry in std::fs::read_dir(".github/workflows").expect("the workflows directory must exist") {
    let path = entry.expect("a readable entry").path();
    let name = path.display().to_string();
    if !name.ends_with(".yml") && !name.ends_with(".yaml") {
      continue;
    }
    let text = std::fs::read_to_string(&path)
      .unwrap_or_else(|e| panic!("{name} must be readable: {e}"))
      .replace("\r\n", "\n");
    let docs = YamlOwned::load_from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(
      docs.len(),
      1,
      "{name} must hold exactly one YAML document: every sweep reads only the first, so a \
       second one would be unaudited"
    );
    audited += 1;
  }
  // Exact, the policy #649 settled for a set of units the repo owns: a red here
  // is a workflow added or removed, which is an edit to acknowledge.
  assert_eq!(
    audited, 4,
    "expected the four workflows under .github/workflows, parsed {audited}. A glob that \
     stopped matching would report every file single-document by reading none"
  );
}
