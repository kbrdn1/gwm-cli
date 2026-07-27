use gwm::config::{BranchType, WorktreeConfig};
use gwm::naming::{branch_pattern_warning, default_branch_types, kebab, parse_branch, BranchSpec, BRANCH_TYPES};

#[test]
fn naming_regexes_compile_at_first_use() {
  // Issue #97. The three regexes in `src/naming.rs` are lifted to
  // module statics via `LazyLock`. The `expect("static <NAME> compiles")`
  // inside each `LazyLock::new` makes a developer-introduced regex typo
  // surface AT THIS TEST instead of in an unrelated downstream call
  // site (which historically used `Regex::new(...).unwrap()` per call
  // and would have shifted the blast radius to the user). Each line
  // below forces the init path for one of the three statics — a panic
  // here is a developer bug in `naming.rs`, not a user error.
  let _ = BranchSpec::new("feat", "1", "x"); // ISSUE_RE + DESC_RE via validate
  let _ = parse_branch("feat/#1-x"); // BRANCH_RE via parse_branch
}

#[test]
fn kebab_normalizes() {
  assert_eq!(kebab("Hello World"), "hello-world");
  assert_eq!(kebab("Foo_BAR  baz"), "foo-bar-baz");
  assert_eq!(kebab("--leading--"), "leading");
  assert_eq!(kebab("  spaces  "), "spaces");
  assert_eq!(kebab("ALL CAPS"), "all-caps");
  assert_eq!(kebab(""), "");
  assert_eq!(kebab("__"), "");
}

#[test]
fn kebab_treats_punctuation_as_separator() {
  assert_eq!(kebab("foo!@#bar"), "foo-bar");
  assert_eq!(kebab("hello.world"), "hello-world");
  assert_eq!(kebab("v1.2.3"), "v1-2-3");
}

#[test]
fn branch_validation() {
  assert!(BranchSpec::new("feat", "123", "user-auth").is_ok());
  assert!(BranchSpec::new("nope", "123", "x").is_err());
  assert!(BranchSpec::new("feat", "abc", "x").is_err());
  assert!(BranchSpec::new("feat", "123", "").is_err());
}

#[test]
fn all_branch_types_accepted() {
  for (t, _) in BRANCH_TYPES {
    assert!(BranchSpec::new(*t, "1", "x").is_ok(), "type {} should be valid", t);
  }
}

#[test]
fn invalid_issue_must_be_digits() {
  assert!(BranchSpec::new("feat", "abc", "x").is_err());
  assert!(BranchSpec::new("feat", "12a", "x").is_err());
  assert!(BranchSpec::new("feat", "", "x").is_err());
}

#[test]
fn description_normalized_before_validation() {
  let spec = BranchSpec::new("feat", "1", "My New Feature").unwrap();
  assert_eq!(spec.desc, "my-new-feature");
}

#[test]
fn parse_roundtrip() {
  let parsed = parse_branch("feat/#42-cool-feature").unwrap();
  assert_eq!(parsed.type_, "feat");
  assert_eq!(parsed.issue, "42");
  assert_eq!(parsed.desc, "cool-feature");
}

#[test]
fn parse_rejects_garbage() {
  assert!(parse_branch("garbage").is_none());
  assert!(parse_branch("feat/no-issue").is_none());
  assert!(parse_branch("FEAT/#1-x").is_none()); // uppercase type
}

#[test]
fn renders_paths() {
  let cfg = WorktreeConfig::default();
  let spec = BranchSpec::new("feat", "10", "x").unwrap();
  assert_eq!(spec.branch_name(&cfg, "myrepo").unwrap(), "feat/#10-x");
  assert_eq!(spec.worktree_dirname(&cfg, "myrepo").unwrap(), "feat-10-x");
  let p = spec
    .worktree_path(&cfg, "myrepo", std::path::Path::new("/repos/myrepo"))
    .unwrap();
  assert!(p.ends_with(std::path::Path::new("cc-worktree").join("myrepo").join("feat-10-x")));
}

#[test]
fn default_branch_types_matches_const_table() {
  let runtime = default_branch_types();
  assert_eq!(runtime.len(), BRANCH_TYPES.len());
  for ((cname, cdesc), bt) in BRANCH_TYPES.iter().zip(runtime.iter()) {
    assert_eq!(*cname, bt.name);
    assert_eq!(*cdesc, bt.description);
  }
}

#[test]
fn new_with_custom_types_rejects_default_built_in() {
  let custom = vec![BranchType {
    name: "migration".into(),
    description: "Database migration".into(),
  }];
  // `feat` is a built-in default but is NOT in the custom override.
  let err = BranchSpec::new_with_types("feat", "1", "x", &custom).unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("invalid branch type 'feat'"), "got: {msg}");
  assert!(
    msg.contains("migration"),
    "error must list the allowed types — got: {msg}"
  );
  assert!(
    !msg.contains("feat, fix"),
    "error must not leak the built-in default list — got: {msg}"
  );
}

#[test]
fn new_with_custom_types_accepts_listed_name() {
  let custom = vec![
    BranchType {
      name: "feat".into(),
      description: "Feature".into(),
    },
    BranchType {
      name: "migration".into(),
      description: "Database migration".into(),
    },
  ];
  let spec = BranchSpec::new_with_types("migration", "42", "users-table", &custom).expect("ok");
  assert_eq!(spec.type_, "migration");
}

#[test]
fn invalid_type_error_lists_allowed_names_from_defaults() {
  let err = BranchSpec::new("nope", "1", "x").unwrap_err();
  let msg = format!("{}", err);
  // Every built-in name must be enumerated so the user knows what's
  // accepted in this repo without having to re-read the docs.
  for (name, _) in BRANCH_TYPES {
    assert!(msg.contains(name), "expected {name} in error message, got: {msg}");
  }
}

#[test]
fn renders_with_custom_patterns() {
  let cfg = WorktreeConfig {
    base: "/tmp/{repo}".into(),
    path_pattern: "{type}_{issue}_{desc}".into(),
    branch_pattern: "release/{type}-{issue}".into(),
  };
  let spec = BranchSpec::new("fix", "7", "foo-bar").unwrap();
  assert_eq!(spec.branch_name(&cfg, "r").unwrap(), "release/fix-7");
  assert_eq!(spec.worktree_dirname(&cfg, "r").unwrap(), "fix_7_foo-bar");
  let p = spec.worktree_path(&cfg, "r", std::path::Path::new("/repos/r")).unwrap();
  assert_eq!(p, std::path::Path::new("/tmp/r").join("fix_7_foo-bar"));
}

#[test]
fn worktree_path_resolves_repo_parent_base() {
  // `{repo_parent}/worktrees` must land in the repo's sibling directory,
  // matching an editor's `../worktrees` convention (Zed git.worktree_directory).
  let cfg = WorktreeConfig {
    base: "{repo_parent}/worktrees".into(),
    path_pattern: "{type}-{issue}-{desc}".into(),
    branch_pattern: "{type}/#{issue}-{desc}".into(),
  };
  let spec = BranchSpec::new("feat", "175", "repo-path").unwrap();
  let repo_path = std::path::Path::new("/Users/me/Projects/Perso/gwm-cli");
  let p = spec.worktree_path(&cfg, "gwm-cli", repo_path).unwrap();
  assert_eq!(
    p,
    std::path::Path::new("/Users/me/Projects/Perso/worktrees/feat-175-repo-path")
  );
}

// ---------------------------------------------------------------------
// Issue #415 — `branch_pattern_warning` probes an actual format/parse
// round-trip rather than comparing the pattern to the default string.
// A pattern can differ from the default and still be readable, and the
// warning must not claim otherwise.
// ---------------------------------------------------------------------

#[test]
fn the_default_pattern_round_trips_so_no_warning() {
  assert_eq!(branch_pattern_warning("{type}/#{issue}-{desc}"), None);
}

#[test]
fn an_unparseable_pattern_warns_that_everything_is_inactive() {
  let w = branch_pattern_warning("{type}-{issue}-{desc}").expect("an unparseable pattern must warn");
  assert!(w.contains("branch_pattern"), "the warning must name the key: {}", w);
  for expected in ["auto-linking", "gitmoji", "branch-convention"] {
    assert!(
      w.contains(expected),
      "warning should name the '{}' consequence: {}",
      expected,
      w
    );
  }
}

/// The finding that killed the string-equality version: this pattern
/// differs from the default, yet `feat/#42-…` still matches `BRANCH_RE`,
/// so `type` and `issue` ARE recovered. Only `desc` comes back wrong.
/// Claiming auto-linking and gitmoji are inactive here would be false.
#[test]
fn a_pattern_whose_type_and_issue_survive_warns_only_about_desc() {
  let w = branch_pattern_warning("{type}/#{issue}-prefix-{desc}").expect("a lossy pattern must still warn");
  assert!(
    w.contains("desc"),
    "the warning must name the segment that breaks: {}",
    w
  );
  assert!(
    !w.contains("auto-linking") && !w.contains("gitmoji"),
    "type and issue are recovered from this pattern — the warning must not claim otherwise: {}",
    w
  );
}

#[test]
fn a_pattern_that_drops_the_issue_warns_about_auto_linking() {
  let w = branch_pattern_warning("{type}/#1-{desc}").expect("a pattern with a frozen issue must warn");
  assert!(
    w.contains("auto-linking"),
    "a pattern that hardcodes the issue breaks auto-linking: {}",
    w
  );
}

/// A single probe value collides with a pattern that hardcodes that same
/// value: `feat/#{issue}-{desc}` formats `feat/#42-…` and parses back
/// `type = "feat"`, which matches a probe that also used `feat`. But
/// `gwm create fix 42 …` writes a `feat/` branch and reads back the wrong
/// type. Two probes with distinct values close that false negative.
#[test]
fn a_pattern_that_hardcodes_the_type_is_not_a_false_negative() {
  let w = branch_pattern_warning("feat/#{issue}-{desc}").expect("a hardcoded type must warn");
  assert!(
    w.contains("type"),
    "the warning must name `type` as the broken segment: {}",
    w
  );
}

#[test]
fn a_pattern_that_hardcodes_the_desc_is_not_a_false_negative() {
  let w = branch_pattern_warning("{type}/#{issue}-fixed").expect("a hardcoded desc must warn");
  assert!(
    w.contains("desc"),
    "the warning must name `desc` as the broken segment: {}",
    w
  );
}

/// Issue #415 (Codex review): `type` and `issue` also feed lifecycle hook
/// placeholders and the TUI rename, not only gitmoji / auto-linking.
#[test]
fn the_warning_names_every_consumer_of_a_broken_segment() {
  let w = branch_pattern_warning("{type}/#1-{desc}").expect("a frozen issue must warn");
  assert!(w.contains("auto-linking"), "issue feeds auto-linking: {}", w);
  assert!(
    w.contains("hook placeholders") && w.contains("rename"),
    "issue also feeds lifecycle hook placeholders and the TUI rename: {}",
    w
  );
}
