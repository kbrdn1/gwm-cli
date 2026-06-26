//! Unit tests for the built-in `.gwm.toml` presets (issue #37).
//!
//! The "loads as Config" test is the real guard: it runs every embedded
//! preset through the same validation path as the runtime
//! (`Config::load_layered`), so a preset that deserialises but fails
//! `validate_bootstrap_guards` (bad regex), references an undefined guard,
//! or trips a path check is caught here, not at `gwm create` time.

use gwm::config::Config;
use gwm::presets;
use tempfile::TempDir;

#[test]
fn lookup_resolves_canonical_names() {
  for name in ["generic", "laravel", "node", "rust", "go", "python-uv"] {
    assert!(presets::lookup(name).is_some(), "missing preset {name}");
  }
}

#[test]
fn lookup_resolves_aliases() {
  // `nuxt` is an alias of the `node` preset — they share the same body.
  let nuxt = presets::lookup("nuxt").expect("nuxt alias resolves");
  let node = presets::lookup("node").expect("node preset");
  assert_eq!(nuxt.name, node.name, "nuxt must resolve to the node preset");
  assert_eq!(nuxt.body, node.body);
}

#[test]
fn lookup_rejects_unknown() {
  assert!(presets::lookup("cobol").is_none());
}

#[test]
fn generic_preset_is_the_documented_example() {
  // `gwm init` (no preset) must stay byte-identical to the shipped example,
  // so the generic preset's body IS that file.
  let generic = presets::lookup("generic").expect("generic preset");
  assert_eq!(generic.body, include_str!("../examples/gwm.toml.example"));
}

#[test]
fn every_preset_has_a_nonempty_description() {
  for p in presets::all() {
    assert!(
      !p.description.trim().is_empty(),
      "preset {} lacks a one-line description for --list-presets",
      p.name
    );
  }
}

#[test]
fn every_preset_loads_as_valid_config() {
  // The strong invariant: each embedded preset must pass the full runtime
  // validation chain (regex compilation, path checks, guard references),
  // not merely serde deserialisation.
  for p in presets::all() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".gwm.toml"), p.body).unwrap();
    // No global layer: validate the repo file in isolation.
    let res = Config::load_layered(dir.path(), None);
    assert!(
      res.is_ok(),
      "preset {} failed to load as Config: {:?}",
      p.name,
      res.err()
    );
  }
}
