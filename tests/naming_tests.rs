use gwm::config::{BranchType, WorktreeConfig};
use gwm::naming::{
  branch_pattern_warning, default_branch_types, kebab, parse_branch, BranchSpec, WorktreeName, BRANCH_TYPES,
};

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
  assert_eq!(
    branch_pattern_warning("{type}/#{issue}-{desc}", "gwm-cli", &default_branch_types()),
    None
  );
}

#[test]
fn an_unparseable_pattern_warns_that_everything_is_inactive() {
  let w = branch_pattern_warning("{type}-{issue}-{desc}", "gwm-cli", &default_branch_types())
    .expect("an unparseable pattern must warn");
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
  let w = branch_pattern_warning("{type}/#{issue}-prefix-{desc}", "gwm-cli", &default_branch_types())
    .expect("a lossy pattern must still warn");
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
  let w = branch_pattern_warning("{type}/#1-{desc}", "gwm-cli", &default_branch_types())
    .expect("a pattern with a frozen issue must warn");
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
  let w = branch_pattern_warning("feat/#{issue}-{desc}", "gwm-cli", &default_branch_types())
    .expect("a hardcoded type must warn");
  assert!(
    w.contains("type"),
    "the warning must name `type` as the broken segment: {}",
    w
  );
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

/// Issue #415 (Codex review): `{repo}` is a supported placeholder, so the
/// probe has to expand it with the real repo name. A dummy value makes the
/// verdict repo-dependent: `{repo}/#{issue}-{desc}` parses fine as `repo/…`
/// but produces `gwm-cli/#42-…`, which `BRANCH_RE` rejects outright because
/// `[a-z]+` does not match a name carrying a dash.
#[test]
fn the_probe_expands_repo_with_the_real_repo_name() {
  let w = branch_pattern_warning("{repo}/#{issue}-{desc}", "gwm-cli", &default_branch_types())
    .expect("must warn for this repo");
  assert!(
    w.contains("match nothing at all"),
    "in a repo whose name has a dash this pattern parses back to nothing: {}",
    w
  );
  assert!(
    w.contains("auto-linking"),
    "so every consumer is inactive, not just type: {}",
    w
  );
}

/// Issue #415 (Codex review): a type gwm would refuse to create must not
/// produce a warning about branches that cannot exist. With
/// `[[branch_types]]` narrowed to `feat`, `feat/#{issue}-{desc}` round-trips
/// for every branch gwm accepts, so there is nothing to warn about.
#[test]
fn a_hardcoded_type_is_fine_when_it_is_the_only_configured_type() {
  let only_feat = vec![BranchType {
    name: "feat".into(),
    description: "New feature implementation".into(),
  }];
  assert_eq!(
    branch_pattern_warning("feat/#{issue}-{desc}", "gwm-cli", &only_feat),
    None
  );
  // …and it is still a real problem once a second type can be created.
  assert!(branch_pattern_warning("feat/#{issue}-{desc}", "gwm-cli", &default_branch_types()).is_some());
}

/// Issue #415 (Codex review): parsability is value-dependent. With
/// `{desc}/#{issue}-{type}` a desc carrying a `-` yields
/// `probe-desc/#42-feat`, which `[a-z]+` rejects, while a plain `probe`
/// yields `probe/#42-feat`, which parses and keeps the issue. Reporting
/// "everything is inactive on this pattern" would be false for half the
/// branches it produces.
#[test]
fn a_partially_parseable_pattern_is_not_generalised_to_every_branch() {
  let w = branch_pattern_warning("{desc}/#{issue}-{type}", "gwm-cli", &default_branch_types())
    .expect("a lossy pattern must warn");
  assert!(
    w.contains("match nothing at all") && w.contains("parse but read back"),
    "both outcomes occur here, so both must be reported: {}",
    w
  );
}

/// Issue #415 (Codex review): `DESC_RE` accepts an all-digits desc, and it
/// is the only desc class `BRANCH_RE`'s `\d+` issue group can swallow. With
/// `{type}/#{desc}-{issue}` a word desc never parses while `123` yields
/// `feat/#123-42`, which parses with the segments swapped — a partial
/// round-trip, not a total loss.
#[test]
fn a_digits_only_desc_is_probed_so_the_verdict_stays_partial() {
  let w = branch_pattern_warning("{type}/#{desc}-{issue}", "gwm-cli", &default_branch_types())
    .expect("a swapped pattern must warn");
  assert!(
    w.contains("match nothing at all") && w.contains("parse but read back"),
    "a digits-only desc parses, so the verdict carries both counts, not a total loss: {}",
    w
  );
}

/// Issue #415 (Codex review): the per-segment flags accumulate across probes,
/// so reporting them as if they held for every parsed branch over-claims.
/// `feat/#{issue}-{desc}` round-trips for the `feat` probes and loses the
/// type for every other configured type — the verdict has to be quantified,
/// not universal.
#[test]
fn a_partly_lossy_pattern_quantifies_instead_of_generalising() {
  let w = branch_pattern_warning("feat/#{issue}-{desc}", "gwm-cli", &default_branch_types())
    .expect("a hardcoded type must warn");
  assert!(
    w.contains("of the ") && w.contains("branch shapes probed"),
    "the warning must count the shapes it probed, not claim all branches lose something: {}",
    w
  );
}

/// The verdict is observational in *every* shape. No message may claim
/// anything about branches outside the probe set: which values matter
/// depends on the pattern, so that set cannot be closed without deriving
/// the parser from the pattern (#417). A class the probes miss can only
/// make the counts smaller — never make the statement false.
#[test]
fn every_verdict_is_scoped_to_the_shapes_actually_probed() {
  for pattern in [
    "{type}-{issue}-{desc}",
    "{type}/#{issue}-prefix-{desc}",
    "feat/#{issue}-{desc}",
    "{desc}/#{issue}-{type}",
    "{type}/#{issue}{desc}",
  ] {
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` must warn", pattern));
    assert!(
      w.contains("of the ") && w.contains("branch shapes probed"),
      "`{}` produced an unquantified verdict: {}",
      pattern,
      w
    );
  }
}

/// Guard for the "which patterns actually work today" table in
/// `docs/4.configuration/1.gwm-toml.md` (EN + FR). The docs make concrete
/// promises about specific patterns; this pins them so the table cannot
/// drift away from the code. Every expectation below was read off the real
/// `branch_pattern_warning` output, not assumed.
#[test]
fn the_documented_pattern_table_matches_reality() {
  // Round-trips: the default, for any set of configured types.
  assert_eq!(
    branch_pattern_warning("{type}/#{issue}-{desc}", "gwm-cli", &default_branch_types()),
    None
  );

  // Round-trips: a literal type, but only when it is the *only* configured
  // branch type — `BRANCH_RE` reads the literal back as the type.
  let only_feat = vec![BranchType {
    name: "feat".into(),
    description: "New feature implementation".into(),
  }];
  assert_eq!(
    branch_pattern_warning("feat/#{issue}-{desc}", "gwm-cli", &only_feat),
    None
  );

  // Documented as "nothing parses": any change to the `<type>/#<issue>-<desc>`
  // skeleton the hardcoded parser expects.
  for pattern in [
    "{type}/{issue}-{desc}",         // no `#`
    "{type}-{issue}-{desc}",         // no `/`
    "{type}/#{issue}_{desc}",        // `_` instead of `-`
    "{type}/#{issue}",               // no desc
    "{repo}/{type}/#{issue}-{desc}", // extra leading segment
    "wt/{type}/#{issue}-{desc}",     // extra leading segment
  ] {
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` is documented as fully broken but did not warn", pattern));
    assert!(
      w.contains("match nothing at all"),
      "`{}` is documented as fully unparseable: {}",
      pattern,
      w
    );
  }

  // Documented as "parses, wrong desc": anything glued after `{desc}`, or a
  // literal wedged between the `-` and `{desc}`.
  for pattern in [
    "{type}/#{issue}-{desc}-{repo}",
    "{type}/#{issue}-{desc}{desc}",
    "{type}/#{issue}-prefix-{desc}",
  ] {
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` is documented as lossy but did not warn", pattern));
    assert!(
      w.contains("parse but read back `desc`") && !w.contains("match nothing at all"),
      "`{}` is documented as parseable-but-wrong-desc: {}",
      pattern,
      w
    );
  }
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
/// rewrite). Both the pattern and the formatted example are echoed, so both
/// have to be neutralised.
#[test]
fn control_characters_in_the_pattern_never_reach_the_terminal() {
  // OSC 52 clipboard-write shape: ESC ] 52 ; c ; <payload> BEL
  let hostile = "{type}/#{issue}-{desc}\u{1b}]52;c;cHduZWQ=\u{7}";
  let w = branch_pattern_warning(hostile, "gwm-cli", &default_branch_types())
    .expect("a pattern carrying control bytes must warn");
  assert!(
    !w.chars().any(|c| c.is_control()),
    "no control character may survive into the message: {:?}",
    w
  );
  // The value stays recognisable — neutralised, not silently dropped.
  assert!(w.contains("{type}/#{issue}-{desc}"), "got: {}", w);

  // Same for the formatted example, which is built from the pattern.
  let w = branch_pattern_warning("{type}\u{1b}[2J{issue}", "gwm-cli", &default_branch_types()).expect("must warn");
  assert!(
    !w.chars().any(|c| c.is_control()),
    "the `e.g.` example is echoed too: {:?}",
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
  let w = branch_pattern_warning("{type}-{issue}-{desc}", "gwm-cli", &default_branch_types())
    .expect("an unparseable pattern must warn");
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
  // Right at the edge: 255 bytes is a legal component, 256 is not.
  assert!(WorktreeName::freeform(&"a".repeat(255)).is_ok(), "255 bytes is legal");
  assert!(WorktreeName::freeform(&"a".repeat(256)).is_err(), "256 bytes is not");
}

/// The rejection has to say what is wrong, not just that something is.
#[test]
fn the_rejection_names_the_offending_value() {
  let err = WorktreeName::freeform("has space").unwrap_err();
  let msg = format!("{}", err);
  assert!(msg.contains("has space"), "the message must quote the input: {}", msg);
}
