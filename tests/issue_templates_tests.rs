//! Issue #617: `gwm new`'s type → (labels, title prefix) map, read backwards.
//!
//! `gwm create --issue <N>` derives the `<type> <issue> <desc>` triple from an
//! issue that already exists on the forge. Two pure steps carry the
//! derivation, and they live here rather than inside `cmd_create` so they are
//! testable without a live forge:
//!
//!   * [`type_from_labels`] inverts `[issue_template.by_type.*].labels`;
//!   * [`desc_from_title`] strips the prefix `gwm new` writes, then produces
//!     the same kebab-case slug a hand-typed `<desc>` would produce.
//!
//! The parity test at the bottom is the one that pins the invariant the issue
//! actually asks for: for the same title, `gwm new` and `gwm create --issue`
//! must agree on the slug.

mod common;

use common::init_repo;
use gwm::config::{Config, IssueTemplateConfig, IssueTemplateTypeConfig};
use gwm::issue_templates::{desc_from_title, title_prefix_for, type_from_labels, DERIVED_DESC_MAX};

fn labelled(pairs: &[(&str, &[&str])]) -> IssueTemplateConfig {
  let mut by_type = std::collections::BTreeMap::new();
  for (name, labels) in pairs {
    by_type.insert(
      (*name).to_string(),
      IssueTemplateTypeConfig {
        labels: labels.iter().map(|l| (*l).to_string()).collect(),
        ..Default::default()
      },
    );
  }
  IssueTemplateConfig { default: None, by_type }
}

fn owned(labels: &[&str]) -> Vec<String> {
  labels.iter().map(|l| (*l).to_string()).collect()
}

// ---- type_from_labels ---------------------------------------------------

#[test]
fn one_matching_label_selects_its_branch_type() {
  let cfg = labelled(&[("feat", &["feature"]), ("fix", &["bug"])]);
  assert_eq!(type_from_labels(&cfg, &owned(&["feature"])).unwrap(), "feat");
}

#[test]
fn labels_are_compared_case_insensitively() {
  // GitHub label names keep the case they were created with, and the same
  // label reads `Feature` on one repo and `feature` on the next. A
  // case-sensitive compare would turn that into "matches no type".
  let cfg = labelled(&[("feat", &["Feature"])]);
  assert_eq!(type_from_labels(&cfg, &owned(&["feature"])).unwrap(), "feat");
}

#[test]
fn a_type_declaring_no_labels_is_never_a_candidate() {
  // The docs' own example has `docs = { template = "task.yml", title_prefix
  // = "[Docs]: " }` with no labels. An empty list says nothing about which
  // issues belong to the type, so reading it as "matches everything" would
  // hand `docs` every issue in the repo.
  let cfg = labelled(&[("feat", &["feature"]), ("docs", &[])]);
  assert_eq!(type_from_labels(&cfg, &owned(&["feature"])).unwrap(), "feat");
  let err = type_from_labels(&cfg, &owned(&["documentation"])).unwrap_err();
  assert!(!err.to_string().contains("docs"), "must not offer docs: {err}");
}

#[test]
fn no_matching_label_refuses_rather_than_guessing_a_default_type() {
  let cfg = labelled(&[("feat", &["feature"])]);
  let err = type_from_labels(&cfg, &owned(&["question", "wontfix"])).unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("question"), "must name the labels it saw: {msg}");
  assert!(msg.contains("wontfix"), "must name the labels it saw: {msg}");
  assert!(msg.contains("--type"), "must name the way out: {msg}");
}

#[test]
fn two_matching_types_refuse_and_name_both() {
  let cfg = labelled(&[("fix", &["bug"]), ("hotfix", &["bug"])]);
  let err = type_from_labels(&cfg, &owned(&["bug"])).unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("fix"), "must name the candidates: {msg}");
  assert!(msg.contains("hotfix"), "must name the candidates: {msg}");
  assert!(msg.contains("--type"), "must name the way out: {msg}");
}

#[test]
fn a_repo_with_no_labelled_types_says_so_instead_of_reporting_no_match() {
  // The two failures read the same to the derivation and completely
  // differently to the user: "your labels don't match" is actionable, and
  // "this repo never configured the map" is a different fix entirely.
  let cfg = labelled(&[("docs", &[])]);
  let err = type_from_labels(&cfg, &owned(&["feature"])).unwrap_err();
  let msg = err.to_string();
  assert!(
    msg.contains("issue_template.by_type"),
    "must point at the unconfigured section: {msg}"
  );
}

// ---- desc_from_title ----------------------------------------------------

#[test]
fn the_configured_prefix_comes_back_off_the_title() {
  assert_eq!(
    desc_from_title("[Feature]: add config types", "[Feature]: ").unwrap(),
    "add-config-types"
  );
}

#[test]
fn a_title_that_does_not_carry_the_prefix_keeps_all_of_itself() {
  // Titles are edited freely on the forge. A prefix that is no longer there
  // must not eat the first characters of the title.
  assert_eq!(
    desc_from_title("add config types", "[Feature]: ").unwrap(),
    "add-config-types"
  );
}

#[test]
fn punctuation_and_case_normalise_the_same_way_a_typed_desc_does() {
  assert_eq!(
    desc_from_title("modals should follow `[tui] layout`", "").unwrap(),
    "modals-should-follow-tui-layout"
  );
}

#[test]
fn a_long_title_truncates_on_a_word_boundary() {
  let title = "expire the fetched statuses on a relist workspace included so the bound holds";
  let desc = desc_from_title(title, "").unwrap();
  assert!(desc.len() <= DERIVED_DESC_MAX, "over the cap: {desc:?}");
  assert!(!desc.ends_with('-'), "must not end mid-separator: {desc:?}");
  // The cut lands between words: every dash-separated piece of the result is
  // a whole word of the source, so no piece is a prefix of a longer one.
  let source: Vec<&str> = "expire the fetched statuses on a relist workspace included so the bound holds"
    .split(' ')
    .collect();
  for word in desc.split('-') {
    assert!(source.contains(&word), "{word:?} is not a whole source word: {desc:?}");
  }
}

#[test]
fn a_first_word_longer_than_the_cap_is_hard_cut_rather_than_lost() {
  // There is no dash to cut back to, and yielding an empty desc would fail
  // the branch validator with a message about the empty string rather than
  // about the title.
  let title = "a".repeat(DERIVED_DESC_MAX + 20);
  let desc = desc_from_title(&title, "").unwrap();
  assert_eq!(desc.len(), DERIVED_DESC_MAX);
}

#[test]
fn a_title_the_slug_cannot_represent_names_the_title_in_the_error() {
  // `kebab` keeps ASCII alphanumerics only, so a title made entirely of
  // punctuation (or of non-Latin script) collapses to the empty string.
  let err = desc_from_title("!!! ??? ***", "").unwrap_err();
  let msg = err.to_string();
  assert!(msg.contains("!!!"), "must quote the title it could not use: {msg}");
}

#[test]
fn the_cap_boundary_is_exact() {
  // N two-letter words slug to 3N-1 characters, so this is the longest
  // sequence that still fits whatever the cap is set to.
  let n = (DERIVED_DESC_MAX + 1) / 3;
  let fits = vec!["ab"; n].join(" ");
  let desc = desc_from_title(&fits, "").unwrap();
  assert!(desc.len() <= DERIVED_DESC_MAX, "fixture must fit the cap: {desc:?}");
  assert_eq!(desc, fits.replace(' ', "-"), "a slug within the cap is untouched");

  let over = format!("{fits} zz");
  assert_eq!(
    desc_from_title(&over, "").unwrap(),
    desc,
    "the word that pushes past the cap is dropped whole"
  );
}

// ---- prefix parity with `gwm new` ---------------------------------------

#[test]
fn the_prefix_is_resolved_from_the_issue_form_when_the_config_declares_none() {
  // `render_issue_draft` falls back to the form YAML's own `title:` when
  // `[issue_template.by_type.<t>].title_prefix` is unset. A derivation that
  // read the config alone would leave `[Feature]-` in the slug on every repo
  // that relies on the YAML — the two halves of the flow would then produce
  // different slugs for the same title, which is exactly what #617 forbids.
  let (dir, repo) = init_repo();
  let tpl = dir.path().join(".github").join("ISSUE_TEMPLATE");
  std::fs::create_dir_all(&tpl).unwrap();
  std::fs::write(
    tpl.join("feature_request.yml"),
    "name: Feature\ndescription: d\ntitle: \"[Feature]: \"\nlabels: [feature]\nbody: []\n",
  )
  .unwrap();

  let config = Config::load_layered_from_bytes(
    Some(b"[issue_template]\ndefault = \"feature_request.yml\"\n\n[issue_template.by_type]\nfeat = { template = \"feature_request.yml\", labels = [\"feature\"] }\n"),
    None,
  )
  .unwrap();

  let prefix = title_prefix_for(&repo, &config, "feat");
  assert_eq!(prefix, "[Feature]: ");
  assert_eq!(
    desc_from_title("[Feature]: add config types", &prefix).unwrap(),
    "add-config-types"
  );
}

#[test]
fn the_configured_prefix_wins_over_the_form_title() {
  let (dir, repo) = init_repo();
  let tpl = dir.path().join(".github").join("ISSUE_TEMPLATE");
  std::fs::create_dir_all(&tpl).unwrap();
  std::fs::write(
    tpl.join("bug_report.yml"),
    "name: Bug\ndescription: d\ntitle: \"[Bug]: \"\nbody: []\n",
  )
  .unwrap();

  let config = Config::load_layered_from_bytes(
    Some(b"[issue_template.by_type]\nhotfix = { template = \"bug_report.yml\", title_prefix = \"[Hotfix]: \", labels = [\"priority: high\"] }\n"),
    None,
  )
  .unwrap();

  assert_eq!(title_prefix_for(&repo, &config, "hotfix"), "[Hotfix]: ");
}

#[test]
fn an_unreadable_template_contributes_no_prefix_instead_of_failing() {
  // Degrading to "keep the whole title" is cosmetically wrong (a `feature-`
  // rider on the slug) and never loses the issue, which is the right trade
  // for a repo whose `[issue_template]` points at a file that has moved.
  let (_dir, repo) = init_repo();
  let config = Config::load_layered_from_bytes(Some(b"[issue_template]\ndefault = \"gone.yml\"\n"), None).unwrap();
  assert_eq!(title_prefix_for(&repo, &config, "feat"), "");
}
