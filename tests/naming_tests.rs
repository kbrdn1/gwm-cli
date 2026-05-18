use gwm::config::WorktreeConfig;
use gwm::naming::{kebab, parse_branch, BranchSpec, BRANCH_TYPES};

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
  let p = spec.worktree_path(&cfg, "myrepo").unwrap();
  assert!(p.to_string_lossy().ends_with("/cc-worktree/myrepo/feat-10-x"));
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
  let p = spec.worktree_path(&cfg, "r").unwrap();
  assert_eq!(p.to_string_lossy(), "/tmp/r/fix_7_foo-bar");
}
