use gwm::config::{BranchType, Config, WorktreeConfig};
use gwm::naming::{
  branch_pattern_warning, default_branch_types, kebab, worktree_spec, BranchParser, BranchSpec, WorktreeName,
  BRANCH_TYPES,
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

/// Issue #415 (Codex review): a type gwm would refuse to create must not
/// produce a warning about branches that cannot exist. With `[[branch_types]]`
/// narrowed to `feat`, `feat/#{issue}-{desc}` writes what every branch of this
/// repo *is*, so there is nothing to warn about — and #417's constant recovery
/// reads that frozen `feat` back, so gitmoji and `gwm commit-prefix` work.
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
  let spec = BranchParser::compile("feat/#{issue}-{desc}", "gwm-cli", &only_feat)
    .expect("compiles")
    .parse("feat/#42-x")
    .expect("parses");
  assert_eq!(spec.type_, "feat", "the frozen type is what every branch here is");

  // …and it is still a real loss once a second type can be created, because
  // `gwm create fix 42 x` writes a branch that reads back as `feat`.
  assert!(branch_pattern_warning("feat/#{issue}-{desc}", "gwm-cli", &default_branch_types()).is_some());
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

  // `{type}/{desc}` has no `{issue}` and no digits to freeze one from, so the
  // absence is total and needs no probing to state.
  let w = branch_pattern_warning("{type}/{desc}", "gwm-cli", &default_branch_types()).expect("must warn");
  assert!(
    !w.contains("branch shapes probed") && w.contains("carries no `{issue}`"),
    "a segment the pattern cannot supply at all is not a probe result and must not borrow its hedging: {}",
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
    "{type}/#{desc}-{issue}",
    "{type}{issue}-{desc}",
    "{issue}{type}-{desc}",
    "{type}-{issue}9-{desc}",
  ] {
    assert_eq!(
      branch_pattern_warning(pattern, "gwm-cli", &default_branch_types()),
      None,
      "`{}` is documented as round-tripping",
      pattern
    );
  }

  // Documented as "refused as unreadable".
  for pattern in [
    "{issue}{desc}",
    "{desc}{issue}",
    "{type}{desc}",
    "{type}-{issue}9{desc}",
    "{type}a{desc}",
    "{desc}1{issue}",
    "{desc}-{desc}",
  ] {
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` is documented as refused but did not warn", pattern));
    assert!(
      w.contains("nothing between them") || w.contains("could be read as part of") || w.contains("more than once"),
      "`{}` is documented as refused by the compiler: {}",
      pattern,
      w
    );
  }

  // Documented as "freezes a segment": the literal is read back, so the
  // features keep working, but the pattern ignores what `gwm create` was
  // asked for, and that is the loss the warning names.
  for (pattern, segment, frozen) in [
    ("feat/#{issue}-{desc}", "type", "feat"),
    ("{type}/#1-{desc}", "issue", "1"),
    ("{type}/#{issue}-fixed", "desc", "fixed"),
  ] {
    let parser = BranchParser::compile(pattern, "gwm-cli", &default_branch_types()).expect("compiles");
    assert_eq!(
      parser.constants(),
      &[(segment, frozen.to_string())][..],
      "`{}` is documented as freezing {} to `{}`",
      pattern,
      segment,
      frozen
    );
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` is documented as losing what create asked for", pattern));
    assert!(
      w.contains(&format!("read back `{}`", segment)),
      "`{}` is documented as losing the {} create was given: {}",
      pattern,
      segment,
      w
    );
  }

  // Documented as "carries no such segment at all": nothing in the pattern
  // could freeze one, so the absence needs no probing to state.
  for (pattern, token) in [
    ("{issue}-{desc}", "`{type}`"),
    ("{type}/{desc}", "`{issue}`"),
    ("{type}/#{issue}", "`{desc}`"),
  ] {
    let w = branch_pattern_warning(pattern, "gwm-cli", &default_branch_types())
      .unwrap_or_else(|| panic!("`{}` is documented as carrying no segment but did not warn", pattern));
    assert!(
      w.contains(&format!("carries no {}", token)),
      "`{}` is documented as carrying no {}: {}",
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

/// Issue #482. Reordered layouts and frozen segments are each documented as
/// supported; the two **together** are claimed by no line, and the recovery
/// bounds every literal by the canonical `type, issue, desc` rank, so a
/// reordered pattern can leave a segment's region empty and the literal
/// unread. `{desc}/feat/#{issue}` writes `x/feat/#42` and recovers no type.
///
/// Recovering it is not deliverable without a special case, and this is the
/// measurement that settles it rather than an opinion: `feat/#{issue}-{desc}`
/// and `wt/{type}/#{issue}` both put a literal before every placeholder, and in
/// one it is the branch type while in the other it must stay the namespace it
/// looks like. No rule phrased on position alone separates them, so the
/// canonical order is doing load-bearing work that a reordered pattern removes.
/// Keying on "the literal matches a configured branch type" would separate
/// them, and would be a special case in the one function where three of those
/// have already been wrong.
///
/// So the obligation is not "recover it" but "never be wrong or quiet about
/// it", and that is enumerated here rather than sampled: every ordering of the
/// three segments, each of them frozen in turn, across four separators.
#[test]
fn a_reordered_pattern_with_a_frozen_segment_is_never_wrong_and_never_quiet() {
  let types = default_branch_types();
  let slots = [("{type}", "feat"), ("{issue}", "42"), ("{desc}", "login")];
  let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
  let separators = ["/", "-", "/#", "_"];

  let mut wrong: Vec<String> = Vec::new();
  let mut unstated: Vec<String> = Vec::new();
  let mut checked = 0;

  for order in orders {
    for frozen in 0..3 {
      for separator in separators {
        let pattern = order
          .iter()
          .map(|&i| {
            if i == frozen {
              slots[i].1.to_string()
            } else {
              slots[i].0.to_string()
            }
          })
          .collect::<Vec<_>>()
          .join(separator);
        // A layout the compiler refuses is refused with a reason of its own,
        // which the ambiguity enumeration already covers.
        let Ok(parser) = BranchParser::compile(&pattern, "gwm-cli", &types) else {
          continue;
        };
        checked += 1;
        let segment = ["type", "issue", "desc"][frozen];
        let expected = slots[frozen].1;
        let recovered = parser
          .constants()
          .iter()
          .find(|(s, _)| *s == segment)
          .map(|(_, v)| v.clone());

        match recovered {
          // Never wrong: a value read back must be the one the pattern writes.
          Some(value) if value != expected => wrong.push(format!(
            "`{}` read {} as `{}`, not `{}`",
            pattern, segment, value, expected
          )),
          // Never quiet: a value not read back has to be named by the warning,
          // which is what turns `commit-prefix` returning nothing into
          // something the user can act on.
          None => {
            let warning = branch_pattern_warning(&pattern, "gwm-cli", &types);
            let names_it = warning.as_deref().is_some_and(|w| {
              w.contains(&format!("`{{{}}}`", segment)) || w.contains(&format!("read back `{}`", segment))
            });
            if !names_it {
              unstated.push(format!("`{}` loses {} silently: {:?}", pattern, segment, warning));
            }
          }
          _ => {}
        }
      }
    }
  }

  assert!(checked >= 60, "the family should be large, got {}", checked);
  assert!(
    wrong.is_empty(),
    "a reordered pattern must never read a frozen segment back as the wrong value:\n{}",
    wrong.join("\n")
  );
  assert!(
    unstated.is_empty(),
    "a frozen segment a reordered pattern cannot recover must be named by `branch_pattern_warning`, \
     since that is the whole of what the user gets:\n{}",
    unstated.join("\n")
  );
}

/// Ambiguity is about the *value*, not about how many places it could have
/// come from. `feat/feat/#{issue}-{desc}` offers the type two candidates, and
/// refusing on that count alone reported no type at all for a pattern whose
/// every reading says `feat` — so `commit-prefix`, the templates and the TUI
/// saw a branch with no type where the pattern names one unambiguously.
#[test]
fn two_ways_to_read_the_same_value_are_not_an_ambiguity() {
  for (pattern, segment, frozen) in [
    ("feat/feat/#{issue}-{desc}", "type", "feat"),
    ("{type}/#1/1-{desc}", "issue", "1"),
    // `/` and not `-` between them: a dash is inside `{desc}`'s own charset,
    // so `-fixed-fixed` is one run reading `fixed-fixed`, not two reading
    // `fixed`, and it would test the single-candidate path instead.
    ("{type}/#{issue}-fixed/fixed", "desc", "fixed"),
  ] {
    let parser = BranchParser::compile(pattern, "gwm-cli", &default_branch_types()).expect("compiles");
    assert_eq!(
      parser.constants(),
      &[(segment, frozen.to_string())][..],
      "`{}` reads {} as `{}` whichever candidate is taken, so it is frozen, not ambiguous",
      pattern,
      segment,
      frozen
    );
  }

  // The counterpart the rule still has to refuse: two candidates that would
  // read *different* values really are a coin toss, and inventing one is
  // worse than reporting none.
  let parser = BranchParser::compile("feat/fix/#{issue}-{desc}", "gwm-cli", &default_branch_types()).expect("compiles");
  assert!(
    parser.constants().is_empty(),
    "`feat/fix/#{{issue}}-{{desc}}` names two different configured types, which is a real ambiguity"
  );

  // And the reason the rule is stated per *segment* and not per reading:
  // `feat/#{issue}-fix/done` is read two ways, and they disagree about the
  // description. That says nothing about the type, which both of them read as
  // `feat` — so one genuinely ambiguous segment must not take an unanimous
  // one down with it.
  let parser = BranchParser::compile("feat/#{issue}-fix/done", "gwm-cli", &default_branch_types()).expect("compiles");
  assert_eq!(
    parser.constants(),
    &[("type", "feat".to_string())][..],
    "the type is unanimous across both readings of `feat/#{{issue}}-fix/done`; only the description is ambiguous"
  );
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
  //
  // What they have in common is a shared character, not the adjacency: it is
  // the digit that can end an issue *and* open a description that moves the
  // split. Codex review on PR #476 caught the first version of this rule
  // refusing adjacency outright, which took the whole parser away from
  // patterns that read back perfectly well — see below.
  for pattern in ["{issue}{desc}", "{desc}{issue}", "{type}{desc}"] {
    let err = BranchParser::compile(pattern, "gwm-cli", &default_branch_types())
      .expect_err(&format!("`{}` must be refused, not compiled", pattern));
    let msg = format!("{}", err);
    assert!(
      msg.contains("nothing") && msg.contains(pattern),
      "the message must quote the pattern and say what is missing: {}",
      msg
    );
  }

  // `[a-z]+` stops at the first digit and `\d+` at the first letter, so there
  // is exactly one place the split between them can be — no separator needed,
  // and refusing these would have made `from_config` fall back to the inert
  // parser for a config that works.
  let (branch, back) = round_trip("{type}{issue}-{desc}", "feat", "42", "9-my");
  assert_eq!(branch, "feat42-9-my");
  assert_eq!(back, Some(("feat".into(), "42".into(), "9-my".into())));
  let (branch, back) = round_trip("{issue}{type}-{desc}", "feat", "42", "x9");
  assert_eq!(branch, "42feat-x9");
  assert_eq!(back, Some(("feat".into(), "42".into(), "x9".into())));
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

/// Issue #478. A worktree carries its triple in two places, and neither is
/// complete on its own.
///
/// `branch_pattern` is authoritative for every segment it writes from a
/// placeholder. For one it *freezes* — or does not carry at all — the branch
/// cannot say what `gwm create` was given, and the directory can: under
/// `branch_pattern = "feat/#{issue}-{desc}"` with the default `path_pattern`,
/// `gwm create fix 42 x` writes the branch `feat/#42-x` and the directory
/// `fix-42-x`, and `fix` survives only in the second. Rebuilding the triple
/// from the branch alone dropped it, so a rename that touched only the
/// description also renamed the directory's type component.
#[test]
fn a_worktree_spec_takes_from_the_path_what_the_branch_cannot_carry() {
  let mut config = Config::default();
  config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();

  let spec = worktree_spec(&config, "gwm-cli", "feat/#42-x", Some("fix-42-x")).expect("reads the worktree");
  assert_eq!(
    (spec.type_.as_str(), spec.issue.as_str(), spec.desc.as_str()),
    ("fix", "42", "x"),
    "the type the worktree was created with survives in its directory"
  );

  // Without a directory to read, the frozen literal is still the best answer.
  let spec = worktree_spec(&config, "gwm-cli", "feat/#42-x", None).expect("reads the branch");
  assert_eq!(spec.type_, "feat");
}

#[test]
fn a_segment_the_branch_writes_is_never_overridden_by_the_path() {
  // The branch is the identity. A directory that disagrees — renamed by hand,
  // created under a different `path_pattern` — must not rewrite a segment the
  // branch states outright, or a rename would silently adopt whatever the
  // directory happened to say.
  let config = Config::default(); // `{type}/#{issue}-{desc}` + `{type}-{issue}-{desc}`
  let spec = worktree_spec(&config, "gwm-cli", "feat/#42-x", Some("chore-9-something-else")).expect("reads");
  assert_eq!(
    (spec.type_.as_str(), spec.issue.as_str(), spec.desc.as_str()),
    ("feat", "42", "x")
  );
}

#[test]
fn a_path_pattern_that_says_nothing_leaves_the_branch_reading_alone() {
  let mut config = Config::default();
  config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();

  for dirname in [
    "not-shaped-like-the-pattern-at-all/x", // does not match `path_pattern`
    "",                                     // no directory name at all
  ] {
    let spec = worktree_spec(&config, "gwm-cli", "feat/#42-x", Some(dirname)).expect("reads the branch");
    assert_eq!(spec.type_, "feat", "`{}` must not disturb the reading", dirname);
  }

  // A `path_pattern` that cannot be compiled into a parser is not an error
  // here either — it has its own diagnostic, and the branch still reads.
  config.worktree.path_pattern = "{issue}{desc}".into(); // refused: the split can move
  let spec = worktree_spec(&config, "gwm-cli", "feat/#42-x", Some("fix-42-x")).expect("reads the branch");
  assert_eq!(spec.type_, "feat");
}

/// The complete no-regression obligation for constant recovery, enumerated
/// instead of sampled.
///
/// 1.5.0 read a branch **iff** it matched a single hardcoded regex, so the set
/// of patterns that owe anything to the previous release is exactly the family
/// that regex accepts: `A/#B-C`, where each of the three positions is either a
/// placeholder or a literal fitting the charset of the group that stood there.
/// That is 16 patterns, and the release itself is the oracle — the same regex,
/// run over what each pattern writes.
///
/// The recovery was checked against a hand-picked list before this, and two
/// separate review findings landed in the gap: `feat/#{issue}-fix` (`feat` and
/// `fix` are both configured branch types, so a globally-unique candidate does
/// not exist and *both* segments were dropped) and `-fixed--desc` / `-fixed-`
/// (a doubled dash split the value, a trailing one was trimmed off it). Both
/// live inside this family and both were read correctly by 1.5.0.
#[test]
fn every_pattern_1_5_0_read_is_read_the_same_way() {
  // The 1.5.0 parser, verbatim from `git show v1.5.0:src/naming.rs`.
  let oracle = regex::Regex::new(r"^([a-z]+)/#(\d+)-([a-z0-9-]+)$").expect("the 1.5.0 regex compiles");
  let types = default_branch_types();
  // Deliberately none of the literals below, so a frozen segment and a written
  // one can never be confused for each other.
  const PROBE: (&str, &str, &str) = ("chore", "42", "x");

  let mut checked = 0usize;
  let mut broken: Vec<String> = Vec::new();
  for a in ["{type}", "feat"] {
    for b in ["{issue}", "1"] {
      // 1.5.0's third group is `[a-z0-9-]+`, which is *looser* than `DESC_RE`:
      // it accepts a description opening with `-`. The family is defined by
      // that charset, not by the one gwm validates against today, so those
      // literals are in scope too — leaving them out was what made the first
      // version of this test claim more parity than it had.
      for c in ["{desc}", "fix", "fixed--desc", "fixed-", "--fix", "-"] {
        let pattern = format!("{}/#{}-{}", a, b, c);
        let cfg = WorktreeConfig {
          branch_pattern: pattern.clone(),
          ..WorktreeConfig::default()
        };
        let spec = BranchSpec::new_with_types(PROBE.0, PROBE.1, PROBE.2, &types).expect("a valid triple");
        let name = spec.branch_name(&cfg, "gwm-cli").expect("the formatter writes it");
        // Outside the family 1.5.0 could read, there is nothing to preserve.
        let Some(old) = oracle.captures(&name) else { continue };

        let parser =
          BranchParser::compile(&pattern, "gwm-cli", &types).unwrap_or_else(|e| panic!("`{}`: {}", pattern, e));
        let read = parser
          .parse(&name)
          .unwrap_or_else(|| panic!("`{}` wrote `{}`, which 1.5.0 read and this does not", pattern, name));
        let now = (read.type_.clone(), read.issue.clone(), read.desc.clone());
        let then = (
          old.get(1).unwrap().as_str().to_string(),
          old.get(2).unwrap().as_str().to_string(),
          old.get(3).unwrap().as_str().to_string(),
        );
        if then.2.starts_with('-') {
          // The one deliberate divergence, and it is a bug 1.5.0 had rather
          // than a contract it kept: `--fix` fails `DESC_RE`, so
          // `BranchSpec::validate` rejects the very description that parser
          // handed back — the rename form could not submit it and `gwm create`
          // could never have produced it. A leading `-` is also exactly what
          // #416 banned from a name, since `gwm remove` and `git branch -d`
          // read it as a flag. Hooks, `[pr_template]` and the rename prefill
          // get the value without it.
          assert_eq!(
            now.2,
            then.2.trim_start_matches('-'),
            "`{}` wrote `{}`: a leading dash is dropped from the description, not the description",
            pattern,
            name
          );
          checked += 1;
          continue;
        }
        if now != then {
          broken.push(format!(
            "`{}` wrote `{}`: 1.5.0 read {:?}, this reads {:?}",
            pattern, name, then, now
          ));
        }
        checked += 1;
      }
    }
  }
  assert_eq!(
    checked, 24,
    "every pattern in the family must be inside what 1.5.0 could read"
  );
  assert!(
    broken.is_empty(),
    "patterns 1.5.0 read that this reads differently:\n  {}",
    broken.join("\n  ")
  );
}

/// Codex review on PR #476, fourth and fifth passes — and the reason the
/// mirror is now *checked* rather than argued.
///
/// `expand_placeholders` substitutes in a fixed order (`{home}`, `{repo}`,
/// then the three capturing tokens) and each `str::replace` runs over what the
/// previous ones produced, so an expansion can be substituted again. Both
/// shapes below were reported separately, one review pass apart, because the
/// first fix inspected the expansion in isolation and the second token was
/// formed with the literals around it. Replaying the whole chain for inputs
/// nobody has is not worth it; compiling a parser that recognises none of the
/// branches the pattern creates is worse. So `compile` writes one probe branch
/// with the real formatter and refuses the pattern when it cannot read it back
/// — which closes the class rather than these two instances.
#[test]
fn a_pattern_the_parser_and_the_formatter_disagree_on_is_refused() {
  let types = default_branch_types();
  for (pattern, repo) in [
    // The token sits inside the expansion.
    ("{repo}/#{issue}-{desc}", "{type}"),
    // The token is formed with the braces the pattern wrote around it.
    ("{{repo}}/#{issue}-{desc}", "type"),
  ] {
    // The formatter really does substitute twice — assert that rather than
    // assume it, since the refusal is only correct if it does.
    let written =
      gwm::config::expand_placeholders(pattern, repo, Some("feat"), Some("42"), Some("x"), None).expect("formats");
    assert_eq!(
      written, "feat/#42-x",
      "`{}` in a repo called `{}` is expected to substitute twice",
      pattern, repo
    );

    let err = BranchParser::compile(pattern, repo, &types)
      .map(|_| String::new())
      .unwrap_or_else(|e| e.to_string());
    assert!(
      err.contains(pattern) && err.contains("does not read back"),
      "the message must name the pattern and say the parser cannot read it back: {}",
      err
    );
  }

  // A repo name with no token in it is the ordinary case and must be unaffected.
  let parser = BranchParser::compile("{repo}/#{issue}-{desc}", "gwm-cli", &types).expect("compiles");
  assert_eq!(
    parser.parse("gwm-cli/#42-x").map(|s| (s.issue, s.desc)),
    Some(("42".into(), "x".into()))
  );
}

/// Codex review on PR #476, third pass. Knowing the same *set* of tokens as
/// the formatter is not enough — the compiler has to find them the same way.
///
/// `expand_placeholders` is a chain of `str::replace`, so it sees `{type}` at
/// offset 1 of `{{type}` and writes `{feat`. The compiler scanned for `{`
/// instead, took `{{type}` for one unknown token, and compiled a regex
/// demanding that text literally — so no branch the pattern wrote was ever
/// read back, and auto-linking, the hooks, `[pr_template.by_type]` and the TUI
/// rename all went quiet on a pattern that formats perfectly well.
#[test]
fn the_compiler_finds_placeholders_where_the_formatter_substitutes_them() {
  let types = default_branch_types();
  for (pattern, expected_name) in [
    // A literal brace immediately before a placeholder.
    ("{{type}/#{issue}-{desc}", "{feat/#42-x"),
    // A token that only starts after another one has opened.
    ("{type{issue}}-{desc}", "{type42}-x"),
    // A trailing brace that closes nothing.
    ("{type}/#{issue}-{desc}}", "feat/#42-x}"),
    // An unknown token: literal to the formatter, so literal to the compiler.
    ("{foo}/{type}/#{issue}-{desc}", "{foo}/feat/#42-x"),
  ] {
    let cfg = WorktreeConfig {
      branch_pattern: pattern.into(),
      ..WorktreeConfig::default()
    };
    let spec = BranchSpec::new_with_types("feat", "42", "x", &types).expect("a valid triple");
    let name = spec.branch_name(&cfg, "gwm-cli").expect("the formatter writes it");
    assert_eq!(
      name, expected_name,
      "`{}` does not write what the test assumes",
      pattern
    );

    let parser = BranchParser::compile(pattern, "gwm-cli", &types).unwrap_or_else(|e| panic!("`{}`: {}", pattern, e));
    let read = parser
      .parse(&name)
      .unwrap_or_else(|| panic!("`{}` wrote `{}` and cannot read it back", pattern, name));
    for (token, written, got) in [
      ("{type}", "feat", &read.type_),
      ("{issue}", "42", &read.issue),
      ("{desc}", "x", &read.desc),
    ] {
      if pattern.contains(token) {
        assert_eq!(
          got, written,
          "`{}` wrote `{}` and read {} back as `{}`",
          pattern, name, token, got
        );
      }
    }
  }
}

// ---------------------------------------------------------------------
// Issue #417 — no-regression baseline.
//
// The set of branches gwm 1.5.0 could read is exactly the set matching its
// hardcoded `^([a-z]+)/#(\d+)-([a-z0-9-]+)$`, read verbatim off the `v1.5.0`
// tag. Every pattern below writes names in that set, so every one of them
// worked before this issue and has to keep working after it. The expected
// triples were computed by running that regex, not by reasoning about the
// new parser.
// ---------------------------------------------------------------------

/// Patterns whose output 1.5.0 parsed, with what it read back.
///
/// The last two are the ones it read *wrongly* — `{desc}` swallowed the
/// literal that followed it — and #415's warning existed to say so. Those are
/// listed separately below, because matching 1.5.0 there would mean keeping
/// the bug.
const V1_5_0_PARSED: [(&str, &str, &str, &str); 4] = [
  // pattern, type, issue, desc
  ("{type}/#{issue}-{desc}", "feat", "42", "my-desc"),
  // Freezes the type. 1.5.0's `([a-z]+)` group happened to sit exactly where
  // the literal does, so it read `feat` back and gitmoji worked.
  ("feat/#{issue}-{desc}", "feat", "42", "my-desc"),
  // Freezes the issue number.
  ("{type}/#1-{desc}", "feat", "1", "my-desc"),
  // Freezes the description.
  ("{type}/#{issue}-fixed", "feat", "42", "fixed"),
];

#[test]
fn every_branch_1_5_0_could_read_is_still_read_the_same_way() {
  for (pattern, want_type, want_issue, want_desc) in V1_5_0_PARSED {
    let (branch, back) = round_trip(pattern, "feat", "42", "my-desc");
    assert_eq!(
      back,
      Some((want_type.into(), want_issue.into(), want_desc.into())),
      "`{}` wrote `{}`; gwm 1.5.0 read it as ({}, {}, {}) and this must not regress",
      pattern,
      branch,
      want_type,
      want_issue,
      want_desc
    );
  }
}

#[test]
fn the_two_patterns_1_5_0_read_wrongly_are_read_correctly_now() {
  // 1.5.0's `([a-z0-9-]+)$` desc group ran to the end of the name, so a
  // literal after `{desc}` was swallowed into it. #415 reported it; deriving
  // the parser fixes it. Matching 1.5.0 here would mean keeping the bug.
  let (_, back) = round_trip("{type}/#{issue}-prefix-{desc}", "feat", "42", "my-desc");
  assert_eq!(back, Some(("feat".into(), "42".into(), "my-desc".into())));
  //                                                  ^ 1.5.0 said `prefix-my-desc`

  let (_, back) = round_trip("{type}/#{issue}-{desc}-{repo}", "feat", "42", "my-desc");
  assert_eq!(back, Some(("feat".into(), "42".into(), "my-desc".into())));
  //                                                  ^ 1.5.0 said `my-desc-gwm-cli`
}

#[test]
fn a_branch_type_the_repo_no_longer_declares_is_still_read() {
  // `{type}` is `[a-z]+`, not an alternation of the configured types. A
  // branch created while `wip` was configured keeps being recognised as
  // gwm's after `wip` is dropped from `.gwm.toml`: `doctor` still counts it
  // for the orphan check, `gwm commit-prefix` still renders (with the unknown
  // -type gitmoji), and the TUI rename refuses it with the precise reason
  // rather than a generic "does not match the pattern".
  //
  // Narrowing `{type}` to the configured list would have made all three go
  // quiet on a name the previous release read fine.
  let only_feat = vec![BranchType {
    name: "feat".into(),
    description: "the only configured type".into(),
  }];
  let parser = BranchParser::compile("{type}/#{issue}-{desc}", "gwm-cli", &only_feat).expect("compiles");
  let spec = parser.parse("wip/#1-x").expect("an unconfigured type still parses");
  assert_eq!(spec.type_, "wip");
  assert_eq!(spec.issue, "1");
}

#[test]
fn a_frozen_segment_is_recovered_from_the_literal_that_freezes_it() {
  let types = default_branch_types();
  let c = |p: &str| {
    BranchParser::compile(p, "gwm-cli", &types)
      .expect("compiles")
      .constants()
      .iter()
      .map(|(seg, value)| (*seg, value.clone()))
      .collect::<Vec<_>>()
  };

  assert_eq!(c("feat/#{issue}-{desc}"), vec![("type", "feat".into())]);
  assert_eq!(c("{type}/#1-{desc}"), vec![("issue", "1".into())]);
  assert_eq!(c("{type}/#{issue}-fixed"), vec![("desc", "fixed".into())]);
  // A hardcoded description carrying the `-` `DESC_RE` allows.
  assert_eq!(c("{type}/#{issue}-my-fix"), vec![("desc", "my-fix".into())]);
  // All three at once: each oracle claims its own token and removes it from
  // the pool the next one sees.
  assert_eq!(
    c("feat/#1-fixed"),
    vec![("type", "feat".into()), ("issue", "1".into()), ("desc", "fixed".into())]
  );
  // Nothing frozen when every placeholder is present.
  assert!(c("{type}/#{issue}-{desc}").is_empty());
}

#[test]
fn a_namespace_literal_is_not_mistaken_for_a_branch_type() {
  // The `type` oracle is an exact match against the configured list, so a
  // literal that merely *contains* a type name, or that is a plain namespace,
  // recovers nothing. This is what keeps the recovery from being guesswork.
  let types = default_branch_types();
  for pattern in [
    "feature/{issue}-{desc}", // contains `feat`, is not `feat`
    "wt/{issue}-{desc}",
    "{repo}/{issue}-{desc}", // the repo name is not pattern text the user authored
  ] {
    let parser = BranchParser::compile(pattern, "gwm-cli", &types).expect("compiles");
    assert!(
      !parser.reads_segment("type"),
      "`{}` must not invent a branch type",
      pattern
    );
  }

  // Two configured types in the literal: genuinely ambiguous, so neither is
  // claimed rather than picking one.
  let parser = BranchParser::compile("feat/fix-{issue}-{desc}", "gwm-cli", &types).expect("compiles");
  assert!(!parser.reads_segment("type"));
}

#[test]
fn a_repo_named_after_a_branch_type_does_not_type_its_branches() {
  // `{repo}` is resolved by the formatter, so it is fixed text in the branch
  // name — but it is not text the *pattern author* chose, and a repo called
  // `docs` would otherwise make every branch a docs branch.
  let types = default_branch_types();
  let parser = BranchParser::compile("{repo}/#{issue}-{desc}", "docs", &types).expect("compiles");
  assert!(!parser.reads_segment("type"));
  let spec = parser.parse("docs/#42-x").expect("parses");
  assert_eq!(spec.type_, "");
}

/// Every pattern the compiler is expected to accept.
///
/// Membership is not an opinion: [`every_legal_pattern_round_trips`] writes a
/// branch from each of these with the real formatter and reads it back, so a
/// pattern only belongs here if it survives that.
const LEGAL: [&str; 20] = [
  "{desc}-{issue}",
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
  "{type}/#{desc}-{issue}",
  "feat/#{issue}-{desc}",
  "{type}/#1-{desc}",
  "{type}/#{issue}-fixed",
  // Adjacent, but over disjoint alphabets: `[a-z]+` stops at the first digit
  // and `\d+` stops at the first letter, so the split cannot move.
  "{type}{issue}-{desc}",
  "{issue}{type}-{desc}",
  "{type}{issue}",
  // A multi-character separator the left-hand group can only eat part of.
  "{type}-{issue}9-{desc}",
  "{issue}9-{desc}",
];

/// Every pattern the compiler is expected to refuse, with the phrase its
/// message must carry. Same contract as [`LEGAL`], read from the other side:
/// [`every_refused_pattern_really_is_ambiguous`] proves each one can write a
/// branch name it would then read back wrong.
const REFUSED: [(&str, &str); 8] = [
  // Adjacent over overlapping alphabets.
  ("{issue}{desc}", "nothing between them"),
  ("{desc}{issue}", "nothing between them"),
  ("{type}{desc}", "nothing between them"),
  ("{desc}{type}", "nothing between them"),
  // Separated, but by a character both neighbours can hold.
  ("{type}-{issue}9{desc}", "could be read as part of"),
  ("{type}a{desc}", "could be read as part of"),
  ("{desc}1{issue}", "could be read as part of"),
  // The same value written twice needs a backreference to read back.
  ("{desc}-{desc}", "more than once"),
];

/// The triples the round-trip property is checked over. Small on purpose, but
/// picked to attack the boundary rule rather than to look representative: a
/// description that opens with a digit is what turns `{type}-{issue}9{desc}`
/// into a mis-split, one that opens with the separator's own character is what
/// lets a greedy group slide, and issue numbers of three different lengths are
/// what make a shifted split observable at all.
const TYPES: [&str; 2] = ["feat", "fix"];
const ISSUES: [&str; 4] = ["1", "4", "42", "429"];
const DESCS: [&str; 10] = ["a", "foo", "a-b", "9-my", "19x", "2-a-b", "x9", "fix", "bar", "b9c"];

/// The arbiter for the whole ambiguity rule, and the reason it stopped being
/// argued one hand-picked branch name at a time.
///
/// Three separate Codex findings landed on this rule in two review passes —
/// one false negative, two over-strict refusals — because each was checked
/// against a string chosen to illustrate it. This decides the question by
/// construction instead: for every legal pattern, write a branch with the real
/// formatter for every triple, read it back with the parser, and require the
/// segments the pattern actually writes to come back unchanged. A pattern that
/// fails here does not belong in [`LEGAL`], whatever the rule says.
#[test]
fn every_legal_pattern_round_trips() {
  let types = default_branch_types();
  for pattern in LEGAL {
    let parser = BranchParser::compile(pattern, "gwm-cli", &types)
      .unwrap_or_else(|e| panic!("`{}` is a legal pattern and must compile: {}", pattern, e));
    let cfg = WorktreeConfig {
      branch_pattern: pattern.into(),
      ..WorktreeConfig::default()
    };
    for type_ in TYPES {
      for issue in ISSUES {
        for desc in DESCS {
          let spec = BranchSpec::new_with_types(type_, issue, desc, &types).expect("a valid triple");
          let name = spec.branch_name(&cfg, "gwm-cli").expect("the formatter writes it");
          let read = parser
            .parse(&name)
            .unwrap_or_else(|| panic!("`{}` wrote `{}` and cannot read it back", pattern, name));
          // Only the segments the pattern writes from a placeholder: one it
          // freezes as a literal is not carrying this triple's value, and one
          // it omits is not carrying a value at all.
          for (token, written, got) in [
            ("{type}", &spec.type_, &read.type_),
            ("{issue}", &spec.issue, &read.issue),
            ("{desc}", &spec.desc, &read.desc),
          ] {
            if pattern.contains(token) {
              assert_eq!(
                written,
                got,
                "`{}` wrote `{}` from {:?} and read {} back as `{}`",
                pattern,
                name,
                (type_, issue, desc),
                token,
                got
              );
            }
          }
        }
      }
    }
  }
}

/// The named half of the refusals: each one is refused, and refused for the
/// reason its message states. The *general* claim — that these are exactly the
/// patterns that cannot be read back — is
/// [`the_ambiguity_rule_accepts_exactly_the_patterns_that_round_trip`].
#[test]
fn every_refused_pattern_is_refused_for_the_stated_reason() {
  for (pattern, phrase) in REFUSED {
    let err = BranchParser::compile(pattern, "gwm-cli", &default_branch_types())
      .map(|_| String::new())
      .unwrap_or_else(|e| e.to_string());
    assert!(!err.is_empty(), "`{}` must be refused, not compiled", pattern);
    assert!(
      err.contains(phrase),
      "`{}` must be refused for the stated reason (`{}`): {}",
      pattern,
      phrase,
      err
    );
  }
}

/// The rule itself, decided by enumeration rather than argued from examples.
///
/// Three Codex findings landed on the ambiguity rule across two review passes —
/// one pattern wrongly accepted, two wrongly refused — and every one of them
/// was invisible to the sample branch names the rule was being checked against.
/// So this stops sampling. It generates every pattern over the three
/// placeholders and a set of separators chosen to attack the rule, decides
/// independently whether each one round-trips (with a regex the *test* builds
/// from the same three charsets), and requires `compile` to accept exactly
/// those.
///
/// Accepting a pattern that does not round-trip is the silent mis-split #417
/// exists to remove. Refusing one that does makes `from_config` fall back to
/// the inert parser, which takes auto-linking, `commit-prefix`, the hooks and
/// the TUI rename away from a config that works. This test is the only place
/// that can fail for both.
#[test]
fn the_ambiguity_rule_accepts_exactly_the_patterns_that_round_trip() {
  // The oracle's own copy of the capture groups. Deliberately not imported
  // from `naming.rs`: an oracle that shares the code under test cannot
  // contradict it. `every_legal_pattern_round_trips` is what pins the two
  // together, so drift between them fails there.
  const GROUPS: [(&str, &str); 3] = [
    ("{type}", r"(?P<type>[a-z]+)"),
    ("{issue}", r"(?P<issue>\d+)"),
    ("{desc}", r"(?P<desc>[a-z0-9][a-z0-9-]*)"),
  ];
  // Separators picked for what they do to the rule, not for what a user would
  // write: the empty one makes placeholders adjacent, `9` / `a` belong to a
  // neighbouring charset, `9-` and `-9` are multi-character boundaries only
  // half of which a greedy group can eat.
  const SEPS: [&str; 8] = ["", "-", "/", "#", "_", "9", "a", "9-"];

  let types = default_branch_types();
  let mut orders: Vec<Vec<usize>> = Vec::new();
  for a in 0..3 {
    for b in 0..3 {
      if a == b {
        continue;
      }
      orders.push(vec![a, b]);
      for c in 0..3 {
        if c != a && c != b {
          orders.push(vec![a, b, c]);
        }
      }
    }
  }

  let mut checked = 0usize;
  for order in &orders {
    for seps in separator_tuples(order.len() - 1, &SEPS) {
      let mut pattern = String::new();
      let mut oracle = String::from("^");
      for (position, &segment) in order.iter().enumerate() {
        if position > 0 {
          pattern.push_str(seps[position - 1]);
          oracle.push_str(&regex::escape(seps[position - 1]));
        }
        pattern.push_str(GROUPS[segment].0);
        oracle.push_str(GROUPS[segment].1);
      }
      oracle.push('$');
      let oracle = regex::Regex::new(&oracle).expect("the oracle regex compiles");

      let round_trips = round_trips_every_value(&pattern, &oracle);
      let accepted = BranchParser::compile(&pattern, "gwm-cli", &types).is_ok();
      assert_eq!(
        accepted, round_trips,
        "`{}` round-trips: {}, but the compiler accepts it: {}",
        pattern, round_trips, accepted
      );
      checked += 1;
    }
  }
  // The enumeration is the evidence, so it has to have actually happened.
  assert_eq!(checked, 6 * SEPS.len() + 6 * SEPS.len() * SEPS.len());
}

/// Every tuple of `n` separators drawn from `pool`, in order.
fn separator_tuples(n: usize, pool: &'static [&'static str]) -> Vec<Vec<&'static str>> {
  let mut out: Vec<Vec<&'static str>> = vec![Vec::new()];
  for _ in 0..n {
    out = out
      .into_iter()
      .flat_map(|prefix| {
        pool.iter().map(move |sep| {
          let mut next = prefix.clone();
          next.push(sep);
          next
        })
      })
      .collect();
  }
  out
}

/// Every non-empty string over `alphabet` up to `max` characters long.
///
/// Generated rather than curated on purpose: the two rounds of Codex findings
/// this test exists to close were both invisible to hand-picked values, and a
/// third gap showed up while writing it — the mis-split needs a value that
/// *ends* in the separator's own character, which is precisely the kind of
/// input nobody writes down. The alphabets below therefore include every
/// character the separator pool uses.
fn values(alphabet: &[char], max: usize) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  let mut frontier: Vec<String> = vec![String::new()];
  for _ in 0..max {
    frontier = frontier
      .iter()
      .flat_map(|prefix| {
        alphabet.iter().map(move |c| {
          let mut next = prefix.clone();
          next.push(*c);
          next
        })
      })
      .collect();
    out.extend(frontier.iter().cloned());
  }
  out
}

/// Does `pattern` survive a write-then-read for every value the three
/// placeholders can hold, judged by `oracle` rather than by the parser under
/// test?
///
/// The formatter is called directly rather than through [`BranchSpec`] so the
/// values are not restricted to the repo's configured branch types: the parser
/// reads `{type}` as `[a-z]+`, so that is the space the rule has to be right
/// over.
fn round_trips_every_value(pattern: &str, oracle: &regex::Regex) -> bool {
  for type_ in values(&['a', 'b'], 2) {
    for issue in values(&['9', '1'], 2) {
      for desc in values(&['a', '9', '-'], 3) {
        if !DESC_SHAPE.is_match(&desc) {
          continue;
        }
        let name = gwm::config::expand_placeholders(pattern, "", Some(&type_), Some(&issue), Some(&desc), None)
          .expect("the formatter writes it");
        let Some(cap) = oracle.captures(&name) else {
          return false;
        };
        for (token, written) in [("type", &type_), ("issue", &issue), ("desc", &desc)] {
          if pattern.contains(&format!("{{{}}}", token)) && cap.name(token).map(|m| m.as_str()) != Some(written) {
            return false;
          }
        }
      }
    }
  }
  true
}

/// The oracle's own copy of the description shape, for the same reason its
/// copy of the capture groups is: an oracle that imports the code under test
/// cannot contradict it.
static DESC_SHAPE: std::sync::LazyLock<regex::Regex> =
  std::sync::LazyLock::new(|| regex::Regex::new(r"^[a-z0-9][a-z0-9-]*$").unwrap());

/// Codex review on PR #476, second pass. Literal text either side of a
/// placeholder is not one run: `1{type}2-{desc}` writes `1feat2-foo`, whose
/// only digits are the `1` and the `2` the pattern itself contributes. Fusing
/// them into `12` made the constant recovery freeze an issue number no branch
/// the pattern can write ever contains, and auto-linking then pointed at #12.
///
/// `{repo}` and `{home}` break the run for the same reason: they are real text
/// in the branch name, so the literals they sit between are not adjacent.
///
/// The assertion is the invariant rather than one outcome, and it is *not* the
/// "recovers nothing" this test first pinned. Positional recovery reads the
/// `2` of `1{type}2-{desc}` as the issue number, and it is right to: the `2`
/// sits between the type and the description, which is where an issue number
/// goes, and `{type}/#1-{desc}` freezes `1` from the same position with the
/// same reasoning. What the finding was about is a value assembled *across* a
/// placeholder — `12` is in no branch the pattern writes, and no run of the
/// literals it wrote — so that is what is pinned, once as the general rule and
/// once by name.
#[test]
fn a_placeholder_between_two_literals_does_not_fuse_them() {
  let types = default_branch_types();
  for pattern in ["1{type}2-{desc}", "1{repo}2/{type}-{desc}", "1{home}2/{type}-{desc}"] {
    let parser = BranchParser::compile(pattern, "x", &types).unwrap_or_else(|e| panic!("`{}`: {}", pattern, e));
    let cfg = WorktreeConfig {
      branch_pattern: pattern.into(),
      ..WorktreeConfig::default()
    };
    let name = BranchSpec::new_with_types("feat", "42", "foo", &types)
      .expect("a valid triple")
      .branch_name(&cfg, "x")
      .expect("the formatter writes it");
    for (segment, value) in parser.constants() {
      assert!(
        name.contains(value.as_str()),
        "`{}` froze {} as `{}`, which no branch it writes contains — `{}` does not",
        pattern,
        segment,
        value,
        name
      );
    }
  }

  // The finding itself, by name: `12` is what the two literals fused into.
  let parser = BranchParser::compile("1{type}2-{desc}", "x", &types).expect("compiles");
  assert_eq!(
    parser.constants().iter().find(|(segment, _)| *segment == "issue"),
    Some(&("issue", "2".to_string())),
    "the issue must be the `2` the pattern actually writes, never the fused `12`"
  );
}
