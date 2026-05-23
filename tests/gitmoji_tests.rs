//! Unit tests for the gitmoji module (issue #85). The module exposes a
//! built-in `branch_type → emoji shortcode` table baked into the binary,
//! merged at load time with optional `[gitmoji]` overrides from
//! `.gwm.toml`. Resolution then composes the prefix expected by every
//! commit in this repo: `<emoji> <type>(#<issue>):`.

use gwm::gitmoji::{default_map, load, resolve_prefix, shortcode_to_unicode};
use gwm::naming::BranchSpec;

#[test]
fn default_map_contains_built_in_branch_types() {
  // All ten built-in branch types declared in CONTRIBUTING.md must be
  // resolvable out of the box — defaults are the contract for users
  // who never write a `[gitmoji]` block. Spot-check the four canonical
  // pairings; the full sweep is implied by the iteration below.
  let map = default_map();
  assert_eq!(map.get("feat"), Some(":sparkles:"));
  assert_eq!(map.get("fix"), Some(":bug:"));
  assert_eq!(map.get("hotfix"), Some(":ambulance:"));
  assert_eq!(map.get("chore"), Some(":wrench:"));

  // Hard-pin the full set so a future "let's drop one" decision shows up
  // in this test rather than silently breaking shell prompts.
  for ty in [
    "feat", "fix", "hotfix", "docs", "test", "refactor", "chore", "perf", "ci", "build",
  ] {
    assert!(
      map.get(ty).is_some(),
      "default gitmoji map must include built-in branch type {:?}",
      ty
    );
  }
}

#[test]
fn load_without_dot_gwm_toml_yields_defaults() {
  // The repo-less path: no `.gwm.toml` means defaults verbatim.
  let dir = tempfile::TempDir::new().expect("tempdir");
  let map = load(Some(dir.path())).expect("load defaults");
  assert_eq!(map.get("feat"), Some(":sparkles:"));
  assert_eq!(map.get("fix"), Some(":bug:"));
}

#[test]
fn load_merges_dot_gwm_toml_overrides_on_top_of_defaults() {
  // `[gitmoji]` is additive on top of the built-in table: overriding
  // `feat` must not erase `fix` / `chore` / etc. This is the contract
  // that lets a team customise one entry without re-declaring the whole
  // table.
  let dir = tempfile::TempDir::new().expect("tempdir");
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[gitmoji]
feat = ":rocket:"
"#,
  )
  .expect("write .gwm.toml");

  let map = load(Some(dir.path())).expect("load with override");
  // Overridden entry wins.
  assert_eq!(map.get("feat"), Some(":rocket:"));
  // Other defaults survive intact — this is the bug we explicitly do
  // NOT want: a partial override wiping the rest of the table.
  assert_eq!(map.get("fix"), Some(":bug:"));
  assert_eq!(map.get("chore"), Some(":wrench:"));
}

#[test]
fn load_accepts_custom_branch_types_not_in_defaults() {
  // A repo using a non-built-in branch type (`migration`, `release`, …)
  // must be able to declare its emoji without `gwm` rejecting the key.
  // Gitmoji map shape is "open by design"; validation against
  // `BRANCH_TYPES` is `BranchSpec::validate`'s job, not ours.
  let dir = tempfile::TempDir::new().expect("tempdir");
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[gitmoji]
migration = ":truck:"
"#,
  )
  .expect("write .gwm.toml");

  let map = load(Some(dir.path())).expect("load with custom type");
  assert_eq!(map.get("migration"), Some(":truck:"));
}

#[test]
fn resolve_prefix_renders_shortcode_form_by_default() {
  // The canonical form on this repo's commits: `:sparkles: feat(#41):`.
  // The trailing colon AND space matters — every commit subject in the
  // history is `:emoji: type(#N): <subject>`.
  let map = default_map();
  let spec = BranchSpec::new("feat", "41", "tui-search").expect("valid branch");
  let prefix = resolve_prefix(&map, &spec, false);
  assert_eq!(prefix, ":sparkles: feat(#41):");
}

#[test]
fn resolve_prefix_with_unicode_emits_the_real_emoji() {
  // Same shape, but `:sparkles:` becomes ✨ — useful for shell prompts
  // and commit-msg hooks that want a single byte sequence in the
  // commit message body rather than a shortcode.
  let map = default_map();
  let spec = BranchSpec::new("feat", "41", "tui-search").expect("valid branch");
  let prefix = resolve_prefix(&map, &spec, true);
  assert_eq!(prefix, "✨ feat(#41):");
}

#[test]
fn resolve_prefix_for_fix_branch_uses_bug_emoji() {
  // Second branch type, mostly to pin `fix` ↔ `:bug:` ↔ `🐛` since it
  // is the second most-used emoji in the repo's history.
  let map = default_map();
  let spec = BranchSpec::new("fix", "10", "bar-baz").expect("valid branch");
  assert_eq!(resolve_prefix(&map, &spec, false), ":bug: fix(#10):");
  assert_eq!(resolve_prefix(&map, &spec, true), "🐛 fix(#10):");
}

#[test]
fn resolve_prefix_unknown_type_falls_back_to_question_mark() {
  // A branch like `release/#1-…` with no matching `[gitmoji]` entry
  // and no built-in default must still produce a syntactically valid
  // commit prefix — we degrade to `:question:` / ❓ rather than panic
  // or return an error. This keeps the CLI safe to call from a
  // commit-msg hook on any branch.
  let map = default_map();
  // Build a spec directly (bypassing `BranchSpec::new`'s validation)
  // because `release` is not in the built-in branch-types list.
  let spec = BranchSpec {
    type_: "release".into(),
    issue: "7".into(),
    desc: "v0-8".into(),
  };
  assert_eq!(resolve_prefix(&map, &spec, false), ":question: release(#7):");
  assert_eq!(resolve_prefix(&map, &spec, true), "❓ release(#7):");
}

#[test]
fn shortcode_to_unicode_covers_every_default_entry() {
  // Every shortcode shipped in the built-in table must have a unicode
  // mapping — otherwise `--unicode` silently falls back to the
  // shortcode form on a known emoji, which is the user-confusing
  // failure mode we want to prevent at the type-system / test level.
  let map = default_map();
  for (_, shortcode) in map.iter() {
    let unicode = shortcode_to_unicode(shortcode);
    assert!(
      !unicode.is_empty() && unicode != shortcode,
      "shortcode {:?} has no unicode mapping",
      shortcode
    );
  }
}
