//! Unit tests for the `aliases` module (issue #86).
//!
//! TDD canary for the alias resolution chain. The matrix verifies:
//!   - Parsing the `[aliases]` block from `.gwm.toml` and from
//!     `~/.config/gwm/aliases.toml` (user-level fallback).
//!   - Resolution order: built-in subcommands always win, then repo
//!     `.gwm.toml`, then user `aliases.toml` (last fallback).
//!   - Argv expansion is string-only — `gwm wip` becomes a token
//!     sequence, not a shell command.
//!   - Shell pipelines (`&&`, `|`, `;`, backticks) in alias values are
//!     rejected at load time so `wip = "create … && lazygit"` never
//!     silently passes through to the dispatcher.
//!   - A repo alias whose name collides with a built-in subcommand is
//!     a hard config error surfaced by `Config::load_for_repo`.

use gwm::aliases::{self, BUILT_IN_ALIASES};
use gwm::config::{Config, CONFIG_FILE};
use gwm::error::GwmError;
use tempfile::TempDir;

// ---- Parsing ------------------------------------------------------------

#[test]
fn config_aliases_default_is_empty() {
  // Absent `[aliases]` block must resolve to an empty map — never `None`
  // or a placeholder set. This is the "aliasing disabled" no-op contract
  // from the issue: zero churn for repos that never opt in.
  let cfg = Config::default();
  assert!(cfg.aliases.is_empty());
}

#[test]
fn config_aliases_round_trip_through_toml() {
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[aliases]
wip = "create feat 0 wip"
ll = "list --format names"
"#,
  )
  .unwrap();

  let cfg = Config::load_for_repo(dir.path()).unwrap();
  assert_eq!(cfg.aliases.len(), 2);
  assert_eq!(cfg.aliases.get("wip").map(String::as_str), Some("create feat 0 wip"));
  assert_eq!(cfg.aliases.get("ll").map(String::as_str), Some("list --format names"));
}

#[test]
fn config_aliases_rejects_shadow_of_built_in_subcommand() {
  // `list` is a built-in subcommand; aliasing it would silently shadow
  // the built-in (or, worse, infinite-loop the expansion). Refuse at
  // load time so the user finds out before reaching for the alias.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[aliases]
list = "create feat 0 wip"
"#,
  )
  .unwrap();

  let err = Config::load_for_repo(dir.path()).unwrap_err();
  match err {
    GwmError::Config(msg) => {
      assert!(msg.contains("list"), "error must name the offending alias: {msg}");
      assert!(msg.contains("built-in"), "error must mention built-in shadowing: {msg}");
    }
    other => panic!("expected GwmError::Config, got {other:?}"),
  }
}

#[test]
fn config_aliases_rejects_shadow_of_visible_built_in_alias() {
  // `s` is a visible alias of `switch` (issue #43); `cd` is a visible
  // alias of `path`. Both are reachable as `gwm s` / `gwm cd`, so an
  // alias declaration that would shadow them must fail at load time —
  // otherwise `gwm s` resolves to the user's alias and the built-in
  // becomes unreachable without anyone noticing.
  for shadow in ["s", "cd"] {
    let dir = TempDir::new().unwrap();
    std::fs::write(
      dir.path().join(CONFIG_FILE),
      format!(
        r#"
[aliases]
{shadow} = "create feat 0 wip"
"#
      ),
    )
    .unwrap();
    let err = Config::load_for_repo(dir.path()).unwrap_err();
    match err {
      GwmError::Config(msg) => {
        assert!(msg.contains(shadow), "error must name '{shadow}': {msg}");
      }
      other => panic!("expected GwmError::Config for '{shadow}', got {other:?}"),
    }
  }
}

#[test]
fn config_aliases_rejects_shell_pipeline_in_value() {
  // `&&`, `||`, `|`, `;` and backticks are shell metachars — they
  // cannot be honoured by a string-substitution expansion that hands
  // argv to clap. Refuse at load so users notice the limit (and reach
  // for a shell alias instead) rather than silently dropping the
  // suffix.
  for bad_value in [
    "create feat 0 wip && lazygit",
    "path | pbcopy",
    "list ; remove x",
    "echo `whoami`",
  ] {
    let dir = TempDir::new().unwrap();
    std::fs::write(
      dir.path().join(CONFIG_FILE),
      format!(
        r#"
[aliases]
copy = "{bad_value}"
"#
      ),
    )
    .unwrap();
    let err = Config::load_for_repo(dir.path()).unwrap_err();
    match err {
      GwmError::Config(msg) => {
        assert!(
          msg.contains("copy"),
          "error must name the offending alias 'copy': {msg}"
        );
      }
      other => panic!("expected GwmError::Config for {bad_value:?}, got {other:?}"),
    }
  }
}

#[test]
fn config_aliases_rejects_empty_value() {
  // `wip = ""` is meaningless — there's no command to expand to.
  // Refuse at load time so the user notices the typo.
  let dir = TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(CONFIG_FILE),
    r#"
[aliases]
wip = ""
"#,
  )
  .unwrap();
  let err = Config::load_for_repo(dir.path()).unwrap_err();
  match err {
    GwmError::Config(msg) => assert!(msg.contains("wip"), "{msg}"),
    other => panic!("expected GwmError::Config, got {other:?}"),
  }
}

// ---- ResolvedAliases -----------------------------------------------------

#[test]
fn resolved_aliases_contains_built_in_set() {
  // The built-in row must be observable from `gwm aliases list` so
  // users can answer "why does `gwm s` work without being declared?".
  // Empty repo + no user config still surfaces every visible-alias
  // entry from clap.
  let dir = TempDir::new().unwrap();
  let resolved = aliases::load(Some(dir.path()), None).unwrap();
  assert!(!resolved.built_in.is_empty());
  // Sanity: `s → switch` is the canonical example from issue #43;
  // it MUST live in the built-in list for the chain to be honest.
  let s_entry = resolved
    .built_in
    .iter()
    .find(|e| e.name == "s")
    .expect("built-in 's' alias must be present");
  assert_eq!(s_entry.expansion, "switch");
}

#[test]
fn resolved_aliases_repo_overrides_user_for_same_name() {
  // Repo-level beats user-level: a user with `copy = "path foo"` in
  // `~/.config/gwm/aliases.toml` and a repo `.gwm.toml` declaring
  // `copy = "path bar"` must see the repo expansion when `gwm copy`
  // is invoked inside that repo. The order matters because repo
  // aliases follow the repo across machines while user aliases don't.
  let repo_dir = TempDir::new().unwrap();
  std::fs::write(
    repo_dir.path().join(CONFIG_FILE),
    r#"
[aliases]
copy = "path bar"
"#,
  )
  .unwrap();

  let user_dir = TempDir::new().unwrap();
  std::fs::write(
    user_dir.path().join("aliases.toml"),
    r#"
[aliases]
copy = "path foo"
ll = "list --format names"
"#,
  )
  .unwrap();
  let user_path = user_dir.path().join("aliases.toml");

  let resolved = aliases::load(Some(repo_dir.path()), Some(&user_path)).unwrap();

  // Repo entry present and wins for "copy".
  assert_eq!(resolved.repo.get("copy").map(String::as_str), Some("path bar"));
  assert_eq!(resolved.user.get("copy").map(String::as_str), Some("path foo"));

  // The effective lookup must return the repo expansion.
  let argv = aliases::expand_argv(vec!["gwm".into(), "copy".into()], &resolved);
  assert_eq!(argv, vec!["gwm", "path", "bar"]);
}

#[test]
fn resolved_aliases_user_only_when_no_repo_block() {
  // Without a repo `[aliases]` block, the user-level file still
  // surfaces — that's the whole point of the user fallback.
  let repo_dir = TempDir::new().unwrap();
  let user_dir = TempDir::new().unwrap();
  std::fs::write(
    user_dir.path().join("aliases.toml"),
    r#"
[aliases]
ll = "list --format names"
"#,
  )
  .unwrap();
  let user_path = user_dir.path().join("aliases.toml");

  let resolved = aliases::load(Some(repo_dir.path()), Some(&user_path)).unwrap();
  assert!(resolved.repo.is_empty());
  assert_eq!(resolved.user.get("ll").map(String::as_str), Some("list --format names"));
}

#[test]
fn resolved_aliases_user_missing_file_is_no_op() {
  // An absent user file is the common case (fresh install). It must
  // not fail load, it must resolve to an empty user map.
  let repo_dir = TempDir::new().unwrap();
  let resolved = aliases::load(Some(repo_dir.path()), Some(std::path::Path::new("/nope/aliases.toml"))).unwrap();
  assert!(resolved.repo.is_empty());
  assert!(resolved.user.is_empty());
}

#[test]
fn resolved_aliases_user_rejects_shell_pipeline() {
  // Same validation as repo-level — a user-level alias with `&&` is
  // rejected at load time. The error names the file and the alias so
  // the user knows which file to fix.
  let user_dir = TempDir::new().unwrap();
  std::fs::write(
    user_dir.path().join("aliases.toml"),
    r#"
[aliases]
copy = "path | pbcopy"
"#,
  )
  .unwrap();
  let user_path = user_dir.path().join("aliases.toml");
  let err = aliases::load(None, Some(&user_path)).unwrap_err();
  match err {
    GwmError::Config(msg) => {
      assert!(msg.contains("copy"), "{msg}");
    }
    other => panic!("expected GwmError::Config, got {other:?}"),
  }
}

#[test]
fn resolved_aliases_user_rejects_shadow_of_built_in() {
  // The shadow rule applies symmetrically to the user file.
  let user_dir = TempDir::new().unwrap();
  std::fs::write(
    user_dir.path().join("aliases.toml"),
    r#"
[aliases]
list = "list --format names"
"#,
  )
  .unwrap();
  let user_path = user_dir.path().join("aliases.toml");
  let err = aliases::load(None, Some(&user_path)).unwrap_err();
  match err {
    GwmError::Config(msg) => assert!(msg.contains("list"), "{msg}"),
    other => panic!("expected GwmError::Config, got {other:?}"),
  }
}

// ---- expand_argv --------------------------------------------------------

#[test]
fn expand_argv_no_match_passes_through_unchanged() {
  // Argv that doesn't start with an alias is returned verbatim. The
  // dispatcher (clap) still sees what the user typed.
  let resolved = aliases::load(None, None).unwrap();
  let argv = vec!["gwm".into(), "list".into(), "--format".into(), "names".into()];
  assert_eq!(aliases::expand_argv(argv.clone(), &resolved), argv);
}

#[test]
fn expand_argv_expands_repo_alias() {
  let repo_dir = TempDir::new().unwrap();
  std::fs::write(
    repo_dir.path().join(CONFIG_FILE),
    r#"
[aliases]
wip = "create feat 0 wip"
"#,
  )
  .unwrap();
  let resolved = aliases::load(Some(repo_dir.path()), None).unwrap();
  let argv = vec!["gwm".into(), "wip".into()];
  assert_eq!(
    aliases::expand_argv(argv, &resolved),
    vec!["gwm", "create", "feat", "0", "wip"]
  );
}

#[test]
fn expand_argv_built_in_subcommand_always_wins() {
  // Even if the user-level file somehow declares `list = "create
  // feat 0 wip"` (which `load` should reject — but bypass via direct
  // ResolvedAliases construction is possible for tests), the
  // expansion path itself MUST treat `list` as a built-in token and
  // pass it through unchanged. This is the second line of defence on
  // top of the load-time shadow check.
  use gwm::aliases::{AliasEntry, ResolvedAliases};
  use std::collections::BTreeMap;

  let mut user = BTreeMap::new();
  user.insert("list".to_string(), "create feat 0 wip".to_string());
  let resolved = ResolvedAliases {
    built_in: vec![AliasEntry {
      name: "s",
      expansion: "switch",
    }],
    repo: BTreeMap::new(),
    user,
  };
  let argv = vec!["gwm".into(), "list".into()];
  assert_eq!(aliases::expand_argv(argv.clone(), &resolved), argv);
}

#[test]
fn expand_argv_appends_user_trailing_arguments() {
  // `gwm wip --foo bar` with `wip = "create feat 0 wip"` expands to
  // `gwm create feat 0 wip --foo bar` — the user-supplied trailing
  // args are appended after the substitution, matching git's
  // `[alias]` behaviour.
  let repo_dir = TempDir::new().unwrap();
  std::fs::write(
    repo_dir.path().join(CONFIG_FILE),
    r#"
[aliases]
wip = "create feat 0 wip"
"#,
  )
  .unwrap();
  let resolved = aliases::load(Some(repo_dir.path()), None).unwrap();
  let argv = vec!["gwm".into(), "wip".into(), "--no-bootstrap".into()];
  assert_eq!(
    aliases::expand_argv(argv, &resolved),
    vec!["gwm", "create", "feat", "0", "wip", "--no-bootstrap"]
  );
}

#[test]
fn expand_argv_only_first_position_is_substituted() {
  // `gwm create feat 86 wip` must NOT expand the `wip` token (it's a
  // positional, not the subcommand slot). Only argv[1] is a
  // candidate.
  let user_dir = TempDir::new().unwrap();
  std::fs::write(
    user_dir.path().join("aliases.toml"),
    r#"
[aliases]
wip = "create feat 0 wip"
"#,
  )
  .unwrap();
  let user_path = user_dir.path().join("aliases.toml");
  let resolved = aliases::load(None, Some(&user_path)).unwrap();
  let argv = vec!["gwm".into(), "create".into(), "feat".into(), "86".into(), "wip".into()];
  assert_eq!(aliases::expand_argv(argv.clone(), &resolved), argv);
}

#[test]
fn expand_argv_no_recursion_alias_pointing_at_alias() {
  // `wip = "ll"` followed by `ll = "list --format names"` must NOT
  // double-expand — `gwm wip` becomes `gwm ll`, which then errors at
  // clap parse time (ll isn't a subcommand) UNLESS we also expand a
  // second time. We pick "no recursion" to mirror git's behaviour
  // (one substitution, then dispatch). This is the simplest contract
  // and the easiest to reason about.
  let repo_dir = TempDir::new().unwrap();
  std::fs::write(
    repo_dir.path().join(CONFIG_FILE),
    r#"
[aliases]
wip = "ll"
ll = "list --format names"
"#,
  )
  .unwrap();
  let resolved = aliases::load(Some(repo_dir.path()), None).unwrap();
  let argv = vec!["gwm".into(), "wip".into()];
  // After ONE expansion, argv[1] is `ll` — recursion would substitute
  // it again, but we explicitly don't.
  assert_eq!(aliases::expand_argv(argv, &resolved), vec!["gwm", "ll"]);
}

#[test]
fn expand_argv_empty_argv_passes_through() {
  // `gwm` (no args, opens the TUI) must not be touched.
  let resolved = aliases::load(None, None).unwrap();
  let argv = vec!["gwm".into()];
  assert_eq!(aliases::expand_argv(argv.clone(), &resolved), argv);
}

#[test]
fn expand_argv_global_flags_before_alias_are_preserved() {
  // `gwm --allow-bootstrap wip` (global flag BEFORE the alias) is the
  // tricky case. clap parses global flags anywhere, so the alias slot
  // is technically argv[1] OR argv[2] depending on the user. We
  // pick the simple rule: expansion looks at the first non-flag
  // token in argv[1..]. Anything starting with `-` (short or long
  // flag) is skipped over.
  let repo_dir = TempDir::new().unwrap();
  std::fs::write(
    repo_dir.path().join(CONFIG_FILE),
    r#"
[aliases]
wip = "create feat 0 wip"
"#,
  )
  .unwrap();
  let resolved = aliases::load(Some(repo_dir.path()), None).unwrap();
  let argv = vec!["gwm".into(), "--allow-bootstrap".into(), "wip".into()];
  assert_eq!(
    aliases::expand_argv(argv, &resolved),
    vec!["gwm", "--allow-bootstrap", "create", "feat", "0", "wip"]
  );
}

// ---- Built-in alias snapshot --------------------------------------------

#[test]
fn built_in_aliases_constant_matches_clap_visible_aliases() {
  // BUILT_IN_ALIASES is the snapshot used by `aliases list` and the
  // shadow check. It MUST stay in lockstep with the `visible_alias`
  // attributes on the clap subcommands — otherwise a `gwm cd` that
  // works through clap would not appear under `aliases list` and
  // would not be protected from being shadowed by user config.
  //
  // Current set: `cd → path` (issue #67), `s → switch` (issue #43).
  let names: Vec<&str> = BUILT_IN_ALIASES.iter().map(|e| e.name).collect();
  assert!(names.contains(&"cd"), "expected 'cd' in BUILT_IN_ALIASES: {names:?}");
  assert!(names.contains(&"s"), "expected 's' in BUILT_IN_ALIASES: {names:?}");
}
