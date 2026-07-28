use gwm::config::{BranchType, WorktreeConfig};
use gwm::naming::{
  branch_pattern_warning, default_branch_types, kebab, BranchParser, BranchSpec, WorktreeName, BRANCH_TYPES,
};

#[test]
fn naming_regexes_compile_at_first_use() {
  // Issue #97. The literal regexes in `src/naming.rs` are lifted to
  // module statics via `LazyLock`. The `expect("static <NAME> compiles")`
  // inside each `LazyLock::new` makes a developer-introduced regex typo
  // surface AT THIS TEST instead of in an unrelated downstream call
  // site (which historically used `Regex::new(...).unwrap()` per call
  // and would have shifted the blast radius to the user). Each line
  // below forces one init path — a panic here is a developer bug in
  // `naming.rs`, not a user error. Since #417 the branch parser is no
  // longer a literal static, so the second line forces the built-in
  // pattern's *compilation* instead, which carries the same `expect`.
  let _ = BranchSpec::new("feat", "1", "x"); // ISSUE_RE + DESC_RE via validate
  let _ = BranchParser::builtin().parse("feat/#1-x"); // the built-in parser, compiled from the default pattern
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
  let parsed = BranchParser::builtin().parse("feat/#42-cool-feature").unwrap();
  assert_eq!(parsed.type_, "feat");
  assert_eq!(parsed.issue, "42");
  assert_eq!(parsed.desc, "cool-feature");
}

#[test]
fn parse_rejects_garbage() {
  assert!(BranchParser::builtin().parse("garbage").is_none());
  assert!(BranchParser::builtin().parse("feat/no-issue").is_none());
  assert!(BranchParser::builtin().parse("FEAT/#1-x").is_none()); // uppercase type
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
  assert_eq!(
    branch_pattern_warning("{type}/#{issue}-{desc}", "gwm-cli", &default_branch_types()),
    None
  );
}

/// A pattern the compiler cannot mirror. `expand_placeholders` ends with
/// `shellexpand::tilde`, which the reader has no way to undo, so a pattern
/// starting with `~` writes `/Users/…/feat/#7-probe` and nothing reads it
/// back. This is the one shape that still reaches the "everything inactive"
/// verdict, and it is the reason the probe survives #417 as a backstop rather
/// than being replaced by a purely syntactic check.
const UNREADABLE: &str = "~/{type}/#{issue}-{desc}";

#[test]
fn a_pattern_the_compiler_cannot_mirror_warns_that_everything_is_inactive() {
  let w = branch_pattern_warning(UNREADABLE, "gwm-cli", &default_branch_types())
    .expect("a pattern nothing reads back must warn");
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

/// Issue #417 flipping #415's findings. Every pattern below was documented as
/// broken while the parser was hardcoded, and each one is a convention someone
/// actually uses. They round-trip now, so the warning has nothing to say about
/// them — asserting `None` is asserting the feature.
#[test]
fn the_patterns_415_called_broken_now_round_trip() {
  for pattern in [
    // "matches nothing at all": the whole `<type>/#<issue>-<desc>` skeleton
    // was load-bearing for the hardcoded regex, and now none of it is.
    "{type}-{issue}-{desc}",
    "{type}/{issue}-{desc}",
    "{type}/#{issue}_{desc}",
    "{type}_{issue}_{desc}",
    "{repo}/{type}/#{issue}-{desc}",
    "wt/{type}/#{issue}-{desc}",
    // "parses, wrong desc": a literal wedged in, or anything after `{desc}`.
    "{type}/#{issue}-prefix-{desc}",
    "{type}/#{issue}-{desc}-{repo}",
    // "partially parseable": segment order is the user's business.
    "{desc}/#{issue}-{type}",
    "{type}/#{desc}-{issue}",
  ] {
    assert_eq!(
      branch_pattern_warning(pattern, "gwm-cli", &default_branch_types()),
      None,
      "`{}` round-trips since #417 and must not warn",
      pattern
    );
  }
}

#[test]
fn a_pattern_that_drops_the_issue_warns_about_auto_linking() {
  let w = branch_pattern_warning("{type}/#1-{desc}", "gwm-cli", &default_branch_types())
    .expect("a pattern with a frozen issue must warn");
  assert!(
    w.contains("auto-linking"),
    "a pattern that hardcodes the issue breaks auto-linking: {}",
    w
  );
}

/// Issue #417: a literal in the type's position is text, not a type. The
/// derived parser declines to guess that `feat/` denotes `feat`, so nothing
/// reads a branch type back and the warning names the placeholder that would
/// fix it.
///
/// This is the one capability #417 narrows. Under the hardcoded `[a-z]+` the
/// literal happened to land in the type group, so gitmoji worked; #415's
/// review pinned that as intended behaviour for a single-type repo. Deriving
/// the parser cannot preserve it without inferring intent from literal text,
/// which is guesswork on any repo where a type name is also a normal word.
/// The warning is on-demand (`doctor` / `config validate`), not a per-command
/// nag, and it is actionable, so stating the loss beats guessing.
#[test]
fn a_literal_in_the_type_position_is_not_read_back_as_a_type() {
  for types in [
    default_branch_types(),
    vec![BranchType {
      name: "feat".into(),
      description: "the only configured type".into(),
    }],
  ] {
    let w = branch_pattern_warning("feat/#{issue}-{desc}", "gwm-cli", &types)
      .expect("a hardcoded type must warn, however many types are configured");
    assert!(
      w.contains("`{type}`") && w.contains("gitmoji"),
      "the warning must name the missing placeholder and what it costs: {}",
      w
    );
  }
}

#[test]
fn a_pattern_that_hardcodes_the_desc_is_not_a_false_negative() {
  let w = branch_pattern_warning("{type}/#{issue}-fixed", "gwm-cli", &default_branch_types())
    .expect("a hardcoded desc must warn");
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
  let w =
    branch_pattern_warning("{type}/#1-{desc}", "gwm-cli", &default_branch_types()).expect("a frozen issue must warn");
  assert!(w.contains("auto-linking"), "issue feeds auto-linking: {}", w);
  assert!(
    w.contains("hook placeholders") && w.contains("rename"),
    "issue also feeds lifecycle hook placeholders and the TUI rename: {}",
    w
  );
}

/// Issue #415 (Codex review), still load-bearing after #417: `{repo}` is a
/// supported placeholder, so it has to be resolved with the *real* repo name
/// on both sides. The parser compiles it into a literal, so feeding the probe
/// a dummy would build a regex expecting `repo/…` while the formatter writes
/// `gwm-cli/…`, and a pattern that works would be reported as reading nothing.
#[test]
fn the_probe_expands_repo_with_the_real_repo_name() {
  assert_eq!(
    branch_pattern_warning("{repo}/{type}/#{issue}-{desc}", "gwm-cli", &default_branch_types()),
    None,
    "both sides must resolve `{{repo}}` to the same name"
  );
  // A dash in the repo name used to break outright: the hardcoded `[a-z]+`
  // could not match `gwm-cli` in the type position, so the whole pattern read
  // back as nothing. It is an escaped literal now, so the only thing this
  // pattern loses is the `{type}` it never had.
  let w = branch_pattern_warning("{repo}-{issue}-{desc}", "gwm-cli", &default_branch_types())
    .expect("this pattern carries no `{type}`");
  assert!(
    w.contains("carries no `{type}`") && !w.contains("match nothing at all"),
    "a dashed repo name is a literal now, not a parse failure: {}",
    w
  );
}

/// Issue #415 (Codex review): the per-segment flags accumulate across probes,
/// so reporting them as if they held for every parsed branch over-claims. Any
/// verdict derived from probing must carry its count.
///
/// The `missing` verdict is deliberately exempt and is checked separately
/// below: a placeholder the pattern does not contain is absent from every name
/// it will ever write, so quantifying it over a probe set would understate a
/// fact that needs no probing.
#[test]
fn every_probe_derived_verdict_is_scoped_to_the_shapes_actually_probed() {
  let w = branch_pattern_warning(UNREADABLE, "gwm-cli", &default_branch_types()).expect("must warn");
  assert!(
    w.contains("of the ") && w.contains("branch shapes probed"),
    "a probe-derived verdict must count the shapes it probed: {}",
    w
  );

  let w = branch_pattern_warning("{type}/#1-{desc}", "gwm-cli", &default_branch_types()).expect("must warn");
  assert!(
    !w.contains("branch shapes probed") && w.contains("carries no"),
    "a missing placeholder is not a probe result and must not borrow its hedging: {}",
    w
  );
}

/// Issue #417: the two patterns no parser can read are refused by the compiler
/// rather than reported by the probe, so the warning is the compile error and
/// it names the fix.
#[test]
fn an_ambiguous_pattern_is_reported_as_refused_not_as_lossy() {
  let w = branch_pattern_warning("{type}/#{issue}{desc}", "gwm-cli", &default_branch_types())
    .expect("adjacent placeholders must warn");
  assert!(
    w.contains("nothing between them") && w.contains("separate them with a literal"),
    "the warning must say what is wrong and how to fix it: {}",
    w
  );
}

/// Guard for the "which patterns work" table in
/// `docs/4.configuration/1.gwm-toml.md` (EN + FR). The docs make concrete
/// promises about specific patterns; this pins them so the table cannot drift
/// away from the code. Every expectation below was read off the real
/// `branch_pattern_warning` output, not assumed.
#[test]
fn the_documented_pattern_table_matches_reality() {
  // Documented as "round-trips fully".
  for pattern in [
    "{type}/#{issue}-{desc}",
    "{type}-{issue}-{desc}",
    "{type}_{issue}_{desc}",
    "{type}/{issue}-{desc}",
    "{type}/#{issue}_{desc}",
    "{repo}/{type}/#{issue}-{desc}",
    "wt/{type}/#{issue}-{desc}",
    "{type}/#{issue}-prefix-{desc}",
    "{type}/#{issue}-{desc}-{repo}",
    "{desc}/#{issue}-{type}",
  ] {
    assert_eq!(
      branch_pattern_warning(pattern, "gwm-cli", &default_branch_types()),
      None,
      "`{}` is documented as round-tripping",
      pattern
    );
  }

  // Documented as "refused as unreadable".
  for pattern in ["{issue}{desc}", "{desc}{issue}", "{type}{desc}", "{desc}-{desc}"] {
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` is documented as refused but did not warn", pattern));
    assert!(
      w.contains("nothing between them") || w.contains("more than once"),
      "`{}` is documented as refused by the compiler: {}",
      pattern,
      w
    );
  }

  // Documented as "compiles, but drops a segment".
  for (pattern, token) in [
    ("feat/#{issue}-{desc}", "`{type}`"),
    ("{type}/#1-{desc}", "`{issue}`"),
    ("{type}/#{issue}", "`{desc}`"),
  ] {
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` is documented as dropping a segment but did not warn", pattern));
    assert!(
      w.contains(&format!("carries no {}", token)),
      "`{}` is documented as dropping {}: {}",
      pattern,
      token,
      w
    );
  }

  // Documented as the one shape the compiler does not mirror.
  let w = branch_pattern_warning(UNREADABLE, "gwm-cli", &default_branch_types())
    .expect("a `~`-leading pattern is documented as unreadable");
  assert!(w.contains("match nothing at all"), "got: {}", w);
}

/// Issue #415 (Codex review): `Config::merge_layered` deserialises
/// `[[branch_types]]` without running `validate_branch_types`, so the
/// effective list can carry a name `gwm create` would refuse. Probing it
/// would report the *default* pattern as broken — `BRANCH_RE`'s `[a-z]+`
/// cannot match `Feat` — which blames the pattern for a config error that
/// has its own diagnostic.
#[test]
fn an_unusable_branch_type_never_makes_the_default_pattern_look_broken() {
  let invalid = vec![
    BranchType {
      name: "Feat".into(), // rejected by `validate_branch_types` (^[a-z]+$)
      description: String::new(),
    },
    BranchType {
      name: "fix".into(),
      description: String::new(),
    },
  ];
  assert_eq!(
    branch_pattern_warning("{type}/#{issue}-{desc}", "gwm-cli", &invalid),
    None
  );

  // Nothing probeable at all: stay quiet rather than guess.
  let all_invalid = vec![BranchType {
    name: "Feat".into(),
    description: String::new(),
  }];
  assert_eq!(
    branch_pattern_warning("{type}-{issue}-{desc}", "gwm-cli", &all_invalid),
    None
  );
}

/// Issue #415 (Codex review, P1): `branch_pattern` is repo-supplied and
/// neither `gwm doctor` nor `gwm config validate` goes through the TOFU
/// trust gate, so running either in an unvetted repo must not hand its
/// `.gwm.toml` a terminal escape channel (OSC 52 clipboard write, title
/// rewrite). Every path that echoes the pattern has to neutralise it.
///
/// #417 added a third such path and moved the other two, so all three are
/// exercised here rather than only the one that happened to fire in 1.5.0:
/// the compile error, the missing-placeholder verdict, and the probe's `e.g.`
/// example. A pattern that merely appends the payload is no longer a vehicle
/// for any of them, because trailing literals round-trip now.
#[test]
fn control_characters_in_the_pattern_never_reach_the_terminal() {
  // OSC 52 clipboard-write shape: ESC ] 52 ; c ; <payload> BEL
  const OSC52: &str = "\u{1b}]52;c;cHduZWQ=\u{7}";

  // Path 1: the compile error, which quotes the pattern.
  let w = branch_pattern_warning(
    &format!("{{issue}}{{desc}}{}", OSC52),
    "gwm-cli",
    &default_branch_types(),
  )
  .expect("adjacent placeholders must warn");
  assert!(
    !w.chars().any(|c| c.is_control()),
    "no control character may survive the compile error: {:?}",
    w
  );
  assert!(w.contains("{issue}{desc}"), "the value stays recognisable: {}", w);

  // Path 2: the missing-placeholder verdict, which quotes the pattern too.
  let w = branch_pattern_warning("{type}\u{1b}[2J{issue}", "gwm-cli", &default_branch_types())
    .expect("a pattern with no `{desc}` must warn");
  assert!(
    !w.chars().any(|c| c.is_control()),
    "no control character may survive the missing-placeholder verdict: {:?}",
    w
  );
  assert!(w.contains("{type}"), "the value stays recognisable: {}", w);

  // Path 3: the probe's `e.g.` example, which is *built* from the pattern and
  // so carries the payload even when the pattern itself is not echoed raw.
  let w = branch_pattern_warning(
    &format!("~/{{type}}/#{{issue}}-{{desc}}{}", OSC52),
    "gwm-cli",
    &default_branch_types(),
  )
  .expect("a `~`-leading pattern must warn");
  assert!(
    w.contains("match nothing at all") && !w.chars().any(|c| c.is_control()),
    "the formatted example is echoed too: {:?}",
    w
  );
}

/// Issue #415 (Codex review): PR/MR detection goes through
/// `Forge::find_pr_for_branch`, which queries the forge with the *whole*
/// branch name and never parses it — it keeps working whatever the pattern.
/// `gwm pr` does call `parse_branch`, for `[pr_template.by_type]` and its
/// body placeholders, and that consumer has to be named.
#[test]
fn the_consumer_mapping_matches_the_call_sites() {
  let w =
    branch_pattern_warning(UNREADABLE, "gwm-cli", &default_branch_types()).expect("an unreadable pattern must warn");
  assert!(
    w.contains("PR/MR detection is unaffected"),
    "PR detection survives an unreadable pattern — do not claim otherwise: {}",
    w
  );
  assert!(
    w.contains("`gwm pr` template selection and placeholders"),
    "`gwm pr` parses the branch and must be named as broken: {}",
    w
  );
  assert!(
    w.contains("remove/bootstrap hook placeholders") && !w.contains("lifecycle hook placeholders"),
    "`gwm create` passes the original BranchSpec to its hooks — only remove/bootstrap re-parse: {}",
    w
  );
}

// ---------------------------------------------------------------------
// Issue #416 — free-form worktree naming. `WorktreeName` splits the
// structured triple from a name the user simply chose. Free-form names
// are checked for git-ref and filesystem safety only, never against
// `DESC_RE` — the whole point is not having to obey the convention.
// ---------------------------------------------------------------------

#[test]
fn a_freeform_name_is_kept_as_typed_when_it_is_already_safe() {
  let n = WorktreeName::freeform("spike-redis").expect("a plain slug is valid");
  let cfg = WorktreeConfig::default();
  assert_eq!(n.branch_name(&cfg, "gwm-cli").unwrap(), "spike-redis");
  assert_eq!(n.worktree_dirname(&cfg, "gwm-cli").unwrap(), "spike-redis");
}

/// Free-form means free-form: shapes `DESC_RE` rejects are fine here.
#[test]
fn a_freeform_name_is_not_held_to_the_desc_convention() {
  for name in ["Spike_Redis", "2026.07.27", "réécriture", "WIP"] {
    assert!(
      WorktreeName::freeform(name).is_ok(),
      "`{}` is a legal git ref and must be accepted",
      name
    );
  }
}

/// `branch_pattern` / `path_pattern` are defined in terms of `{type}`,
/// `{issue}` and `{desc}`, none of which a free-form name has, so they do
/// not apply. `base` still does, for the placeholders it documents
/// (`{home}` / `{repo}` / `{repo_path}` / `{repo_parent}`).
#[test]
fn patterns_do_not_apply_to_a_freeform_name_but_base_still_does() {
  let cfg = WorktreeConfig {
    base: "/tmp/{repo}".into(),
    path_pattern: "{type}-{issue}-{desc}".into(),
    branch_pattern: "{type}/#{issue}-{desc}".into(),
  };
  let n = WorktreeName::freeform("spike-redis").unwrap();
  assert_eq!(n.branch_name(&cfg, "r").unwrap(), "spike-redis");
  let p = n.worktree_path(&cfg, "r", std::path::Path::new("/repos/r")).unwrap();
  assert_eq!(p, std::path::Path::new("/tmp/r/spike-redis"));
}

/// `base` is only expanded with the placeholders a free-form name can
/// supply. The structured path feeds `{type}` / `{issue}` / `{desc}` into
/// `base` too, so a base written with one of them has nothing to resolve
/// against here — and `expand_placeholders` leaves an unfed placeholder
/// *literal*, which would silently create a directory named `{type}`.
/// Refusing beats creating the wrong path (Codex review on PR #474).
#[test]
fn a_base_written_with_the_structured_placeholders_is_refused_not_left_literal() {
  for base in ["/srv/{type}", "/srv/{repo}-{issue}", "{home}/wt/{desc}"] {
    let cfg = WorktreeConfig {
      base: base.into(),
      ..WorktreeConfig::default()
    };
    let n = WorktreeName::freeform("spike-redis").unwrap();
    let err = n
      .worktree_path(&cfg, "r", std::path::Path::new("/repos/r"))
      .expect_err(&format!("`{}` has no value to resolve for a free-form name", base));
    let msg = format!("{}", err);
    assert!(msg.contains("base"), "the message must point at worktree.base: {}", msg);
  }
}

/// A `/` is legal in a branch name and a common convention, but a worktree
/// directory is a single path component — same split the structured mode
/// already makes between `branch_pattern` and `path_pattern`.
#[test]
fn a_slash_survives_in_the_branch_and_flattens_in_the_directory() {
  let n = WorktreeName::freeform("spike/redis").expect("a slash is a legal ref");
  let cfg = WorktreeConfig::default();
  assert_eq!(n.branch_name(&cfg, "r").unwrap(), "spike/redis");
  assert_eq!(n.worktree_dirname(&cfg, "r").unwrap(), "spike-redis");
}

/// Rejected against libgit2's own `refs/heads/<name>` check rather than a
/// hand-written rule list, plus the path-component rules a ref check does
/// not cover.
#[test]
fn a_freeform_name_that_git_or_the_filesystem_would_refuse_is_rejected() {
  for bad in [
    "",            // empty
    "   ",         // whitespace only
    "..",          // path traversal
    "a..b",        // git: no double dot
    "-leading",    // git: no leading dash on a ref component
    "trailing.",   // git: no trailing dot
    "has space",   // git: no space
    "tilde~1",     // git: no tilde
    "caret^",      // git: no caret
    "colon:",      // git: no colon
    "quest?",      // git: no question mark
    "star*",       // git: no asterisk
    "brack[et",    // git: no open bracket
    "back\\slash", // git: no backslash
    "at@{brace",   // git: no @{
    "ends.lock",   // git: no .lock suffix
    "ctrl\u{7}x",  // control byte
    "/leading",    // empty ref component
    "trailing/",   // empty ref component
    "double//slash",
  ] {
    assert!(
      WorktreeName::freeform(bad).is_err(),
      "`{}` must be rejected as a worktree name",
      bad
    );
  }
}

/// The name becomes the branch verbatim, so it has to be validated
/// verbatim. Trimming would accept `--name " spike"` and quietly create
/// `spike` — a different branch from the one that was asked for. Git
/// already refuses the space; letting it say so is the honest answer
/// (Codex review on PR #474).
#[test]
fn surrounding_whitespace_is_refused_rather_than_silently_stripped() {
  for bad in [" spike", "spike ", "\tspike", "spike\n"] {
    assert!(
      WorktreeName::freeform(bad).is_err(),
      "`{:?}` must be refused, not trimmed into a different branch",
      bad
    );
  }
}

/// The branch and the directory have different length limits: a ref is a
/// path of components (each ≤ 255 bytes), a worktree directory is a single
/// one. `a×130/b×130` is a legal ref and a 261-byte directory name, so the
/// branch gets created and `repo.worktree` then fails — leaving an orphan
/// branch behind. Reproduced against the branch binary before the fix
/// (Codex review on PR #474); the structured path cannot reach it, because
/// the `.lock` suffix makes git's own limit one byte stricter than the
/// directory's.
#[test]
fn a_name_that_would_overflow_a_path_component_is_refused_before_anything_is_created() {
  let long = format!("{}/{}", "a".repeat(130), "b".repeat(130));
  assert!(
    git2::Reference::is_valid_name(&format!("refs/heads/{}", long)),
    "precondition: git accepts this ref, so only our own check can stop it"
  );
  assert!(
    WorktreeName::freeform(&long).is_err(),
    "a 261-byte directory name must be refused up front"
  );
  // Right at the edge: 255 bytes of directory name is legal, 256 is not.
  // Split so the *final* ref component stays clear of the `.lock` rule the
  // next test pins — this one is about the directory, not the ref.
  let edge = format!("{}/{}", "a".repeat(251), "b".repeat(3));
  assert_eq!(edge.len(), 255);
  assert!(WorktreeName::freeform(&edge).is_ok(), "255 bytes is legal");
  assert!(
    WorktreeName::freeform(&format!("{}b", edge)).is_err(),
    "256 bytes is not"
  );
}

/// The other five bytes. Git writes `refs/heads/<name>.lock` *before* the
/// ref itself, so the ref's final path component carries a suffix the
/// directory name never sees — and `Branch::name_is_valid` only checks
/// syntax, never length. Measured against the branch binary: a 250-byte
/// final segment creates, 251 fails, and it fails after `pre_create` hooks
/// have already run (Codex review on PR #474).
///
/// Only the final segment: an earlier one gets no suffix, so it may use the
/// full directory budget. Capping every segment would refuse names git and
/// the filesystem both accept.
#[test]
fn the_final_segment_leaves_room_for_git_s_lock_file() {
  assert!(
    WorktreeName::freeform(&"a".repeat(250)).is_ok(),
    "250 + `.lock` is exactly 255 — legal"
  );
  assert!(
    WorktreeName::freeform(&"a".repeat(251)).is_err(),
    "251 + `.lock` overflows the component git has to create first"
  );
  let front_heavy = format!("{}/{}", "a".repeat(251), "b".repeat(3));
  assert!(
    WorktreeName::freeform(&front_heavy).is_ok(),
    "a 251-byte segment is fine when it is not the one carrying `.lock`"
  );
}

/// A free-form name has to survive three consumers, and the rules are
/// enumerated from those rather than sampled from whatever a reviewer
/// happened to try (Codex review on PR #474 raised three findings of this
/// same class before the invariant got written down):
///
/// 1. it is a **git branch** — checked with the branch-level oracle, not the
///    reference-level one, because they do not agree;
/// 2. it is a **single filesystem path component** — bounded and free of
///    `.` / `..`;
/// 3. it is a **literal value in placeholder expansion** — so it must not
///    itself look like a placeholder.
///
/// This test pins (1): the ref-level oracle accepts `refs/heads/HEAD`, but
/// `git branch HEAD` is refused and the name collides with the HEAD
/// pseudo-ref.
#[test]
fn a_name_git_refuses_as_a_branch_is_refused_even_when_the_ref_syntax_is_legal() {
  assert!(
    git2::Reference::is_valid_name("refs/heads/HEAD"),
    "precondition: the ref-level oracle lets `HEAD` through, so only the branch-level one can stop it"
  );
  assert!(
    !git2::Branch::name_is_valid("HEAD").unwrap(),
    "precondition: the branch-level oracle is the one that refuses it"
  );
  assert!(WorktreeName::freeform("HEAD").is_err(), "`HEAD` is not a branch name");
}

/// (3) `lifecycle::expand_placeholders` substitutes sequentially — `{branch}`
/// first, then `{type}` / `{issue}` / `{desc}` / `{repo}` — so a branch whose
/// own name contains a token gets that token rewritten inside the value that
/// was just substituted: a hook receiving `{branch}` for `spike-{issue}` sees
/// `spike-`. `DESC_RE` made this unreachable for structured names; free-form
/// names reach it, so they are refused at the boundary.
#[test]
fn a_name_that_looks_like_a_placeholder_is_refused() {
  for bad in ["spike-{issue}", "{repo}-spike", "{branch}", "closing}brace"] {
    assert!(
      WorktreeName::freeform(bad).is_err(),
      "`{}` would be re-substituted during hook expansion",
      bad
    );
  }
}

/// The rejection has to say what is wrong, not just that something is.
#[test]
fn the_rejection_names_the_offending_value() {
  let err = WorktreeName::freeform("has space").unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("has space"), "the message must quote the input: {}", msg);
}

// ---------------------------------------------------------------------
// Issue #417 — the parser is compiled from `worktree.branch_pattern`, so
// the shape gwm reads is the shape gwm writes. `BRANCH_RE` is gone.
// ---------------------------------------------------------------------

/// Format a triple through `pattern`, then read it back with the parser
/// compiled from the same pattern. The property every test below asserts.
fn round_trip(pattern: &str, type_: &str, issue: &str, desc: &str) -> (String, Option<(String, String, String)>) {
  let cfg = WorktreeConfig {
    branch_pattern: pattern.into(),
    ..Default::default()
  };
  let spec = BranchSpec::new(type_, issue, desc).expect("the probe triple is valid");
  let branch = spec.branch_name(&cfg, "gwm-cli").expect("the pattern expands");
  let parser = BranchParser::compile(pattern, "gwm-cli", &default_branch_types()).expect("the pattern compiles");
  let back = parser
    .parse(&branch)
    .map(|s| (s.type_.clone(), s.issue.clone(), s.desc.clone()));
  (branch, back)
}

#[test]
fn every_plausible_convention_reads_back_what_it_wrote() {
  // The acceptance criterion for #417 is not "custom patterns stop
  // warning" — it is that the conventions people actually use keep issue
  // auto-linking, gitmoji, hook placeholders and the TUI rename. Slash-less
  // patterns are in scope, not an edge case: `-` and `_` sit in neither the
  // type alternation nor `\d+`, so every split below is forced.
  //
  // The desc carries a `-` on purpose. It is the character most likely to
  // collide with a separator, and `[a-z0-9-]` allows it.
  for pattern in [
    "{type}/#{issue}-{desc}", // today's default
    "{type}-{issue}-{desc}",
    "{type}_{issue}_{desc}",
    "{type}/{issue}-{desc}",
    "{repo}/{type}/#{issue}-{desc}",
  ] {
    let (branch, back) = round_trip(pattern, "feat", "417", "derive-branch-parser");
    assert_eq!(
      back,
      Some(("feat".into(), "417".into(), "derive-branch-parser".into())),
      "pattern `{}` wrote `{}` and could not read it back",
      pattern,
      branch
    );
  }
}

#[test]
fn a_pattern_that_omits_a_token_reports_the_segments_it_does_carry() {
  // `{type}/{desc}` has no issue number to find. Returning `None` for the
  // whole parse would throw away the type and desc that ARE there — and
  // those drive gitmoji, `[pr_template.by_type]` and the hook placeholders.
  // The absent segment comes back empty; callers that need it say so.
  let (branch, back) = round_trip("{type}/{desc}", "fix", "9", "flaky-test");
  assert_eq!(branch, "fix/flaky-test");
  assert_eq!(back, Some(("fix".into(), String::new(), "flaky-test".into())));

  let (branch, back) = round_trip("{issue}-{desc}", "feat", "42", "no-type-here");
  assert_eq!(branch, "42-no-type-here");
  assert_eq!(back, Some((String::new(), "42".into(), "no-type-here".into())));

  // A hardcoded literal prefix is the same situation: `feature/` is text, not
  // a type, so nothing reads a type back.
  let (branch, back) = round_trip("feature/{issue}-{desc}", "feat", "42", "literal-prefix");
  assert_eq!(branch, "feature/42-literal-prefix");
  assert_eq!(back, Some((String::new(), "42".into(), "literal-prefix".into())));
}

#[test]
fn a_separator_the_left_token_could_itself_contain_is_not_an_obstacle() {
  // The rule "the separator must not belong to the charset of the token on
  // its left" would reject `{desc}-{issue}`, and it is wrong to. `{issue}`
  // is `\d+`, so it cannot contain the `-`; the only split where the tail
  // is all digits is the right one, and greedy backtracking finds exactly
  // that one. `user-auth-42` is the counter-example #417's body cites as
  // broken — it is not.
  let (branch, back) = round_trip("{desc}-{issue}", "feat", "42", "user-auth");
  assert_eq!(branch, "user-auth-42");
  assert_eq!(back, Some((String::new(), "42".into(), "user-auth".into())));

  // The same holds when the desc itself ends in digits-then-dash, which is
  // where a naive split would go wrong.
  let (_, back) = round_trip("{desc}-{issue}", "feat", "2", "spike-1");
  assert_eq!(back, Some((String::new(), "2".into(), "spike-1".into())));
}

#[test]
fn two_tokens_with_nothing_between_them_are_refused_at_compile_time() {
  // These are the patterns that genuinely cannot be read back, and they are
  // refused rather than compiled into a parser that silently mis-splits.
  //
  // `{issue}{desc}` is deterministic and wrong: `42` + `123-x` writes
  // `42123-x`, which reads back as `4212` + `3-x`. `{desc}{issue}` is worse
  // than wrong, it is ambiguous — `a` + `12` and `a1` + `2` both write
  // `a12`, so no parser can be correct.
  for pattern in ["{issue}{desc}", "{desc}{issue}", "{type}{desc}", "{type}{issue}-{desc}"] {
    let err = BranchParser::compile(pattern, "gwm-cli", &default_branch_types())
      .expect_err(&format!("`{}` must be refused, not compiled", pattern));
    let msg = format!("{}", err);
    assert!(
      msg.contains("nothing") && msg.contains(pattern),
      "the message must quote the pattern and say what is missing: {}",
      msg
    );
  }
}

#[test]
fn the_same_token_twice_is_refused_rather_than_compiled_into_a_second_group() {
  // The formatter's `str::replace` substitutes every occurrence, so
  // `{desc}-{desc}` writes `foo-foo`. Reading that back needs a
  // backreference; two groups of the same name do not even compile. Refuse
  // with a message about the pattern rather than surfacing a regex error.
  let err = BranchParser::compile("{desc}-{desc}", "gwm-cli", &default_branch_types())
    .expect_err("a repeated token must be refused");
  let msg = format!("{}", err);
  assert!(msg.contains("more than once"), "unexpected message: {}", msg);
}

#[test]
fn a_type_the_repo_has_not_configured_is_not_claimed() {
  // `{type}` compiles to an alternation of the configured types, not to
  // `[a-z]+`. A branch whose type this repo would refuse to create is a
  // branch gwm does not own: `gwm doctor` leaves it alone instead of
  // reporting it as an orphan, and the TUI rename (which already refuses an
  // unconfigured type) stays consistent with the parser feeding it.
  //
  // This is the one behaviour narrowing in #417: `wip/#1-x` used to parse
  // under the hardcoded `[a-z]+`.
  let types = vec![BranchType {
    name: "feat".into(),
    description: "only this one".into(),
  }];
  let parser = BranchParser::compile("{type}/#{issue}-{desc}", "gwm-cli", &types).expect("compiles");
  assert!(parser.parse("feat/#1-x").is_some());
  assert!(
    parser.parse("wip/#1-x").is_none(),
    "an unconfigured type must not be claimed"
  );
}

#[test]
fn a_pattern_that_cannot_be_compiled_reads_nothing_rather_than_the_default_shape() {
  // Falling back to the built-in parser here would reproduce exactly the
  // defect #417 removes: the repo writes one shape, gwm reads another, and
  // the issue number it recovers belongs to a branch that was never created
  // that way. Reading nothing is the honest outcome; `gwm doctor` and
  // `gwm config validate` are where the pattern gets reported.
  let mut config = gwm::config::Config::default();
  config.worktree.branch_pattern = "{desc}{issue}".into();
  let parser = BranchParser::from_config(&config, "gwm-cli");
  assert!(parser.parse("feat/#1-x").is_none());
  assert!(parser.parse("x1").is_none());
}

#[test]
fn the_builtin_parser_still_reads_the_canonical_shape() {
  // `parse_branch` keeps its meaning for the one entry point with no repo
  // to consult (`gwm commit-prefix --branch <name>` outside a checkout).
  let spec = BranchParser::builtin()
    .parse("feat/#417-derive-branch-parser")
    .expect("the canonical shape parses");
  assert_eq!(spec.type_, "feat");
  assert_eq!(spec.issue, "417");
  assert_eq!(spec.desc, "derive-branch-parser");
  assert!(BranchParser::builtin().parse("random").is_none());
}

#[test]
fn the_compiler_handles_every_token_the_formatter_substitutes() {
  // The guard that survives someone adding a placeholder in six months.
  //
  // A token `expand_placeholders` substitutes but the compiler does not know
  // about would be emitted as a literal `{token}` into the regex while the
  // formatter replaced it with a value — the format/parse divergence this
  // issue exists to delete, reintroduced silently. So the list is not
  // hand-maintained here: it is read out of the function itself.
  //
  // `{repo_path}` / `{repo_parent}` are deliberately absent from the handled
  // set. `BranchSpec::branch_name` passes `None` for `repo_path`, so those
  // tokens are NOT substituted on the branch path and survive verbatim —
  // which is exactly what the compiler's literal fallback expects.
  // Line endings are normalised first: the Windows runner checks the source
  // out with CRLF, so the closing-brace anchor below never matched there and
  // the test failed on the read rather than on the compiler.
  let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/config.rs"))
    .expect("read src/config.rs")
    .replace("\r\n", "\n");
  let body = src
    .split_once("pub fn expand_placeholders(")
    .expect("expand_placeholders is still named that")
    .1;
  let body = body.split_once("\n}\n").expect("the function has a body").0;

  let mut found: Vec<String> = Vec::new();
  let mut rest = body;
  while let Some(open) = rest.find("(\"{") {
    rest = &rest[open + 2..];
    let Some(close) = rest.find("}\"") else { break };
    found.push(rest[..close + 1].to_string());
    rest = &rest[close + 1..];
  }
  found.sort();
  found.dedup();
  assert!(
    found.len() >= 5,
    "the token scan found only {:?} — the extraction broke, not the compiler",
    found
  );

  // Substituted on the branch path: the compiler must resolve each one.
  const HANDLED: [&str; 5] = ["{home}", "{repo}", "{type}", "{issue}", "{desc}"];
  // Left literal on the branch path (`repo_path` is `None` there), so the
  // compiler's literal fallback is correct for them by construction.
  const LITERAL_ON_BRANCH_PATH: [&str; 2] = ["{repo_path}", "{repo_parent}"];

  for token in &found {
    assert!(
      HANDLED.contains(&token.as_str()) || LITERAL_ON_BRANCH_PATH.contains(&token.as_str()),
      "`{}` is substituted by expand_placeholders but BranchParser::compile does not know it — \
       it would be matched as a literal while the formatter replaced it. Add it to the compiler \
       (and to this list), or explain why the branch path leaves it literal.",
      token
    );
  }
}
