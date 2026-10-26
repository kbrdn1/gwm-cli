//! Shared template renderer tests for GitHub issue / PR automation.

use std::collections::BTreeMap;

use gwm::templating::{issue_form_metadata, render_form_markdown, render_template, FormDefaults, TemplateContext};

#[test]
fn render_template_replaces_known_placeholders_and_leaves_unknown_verbatim() {
  let ctx = TemplateContext::from_pairs([("type", "feat"), ("issue", "83"), ("desc", "issue-template-defaults")]);

  let rendered = render_template("## {type}\nCloses #{issue}\n{desc}\n{unknown}", &ctx);

  assert_eq!(rendered, "## feat\nCloses #83\nissue-template-defaults\n{unknown}");
}

#[test]
fn render_form_markdown_converts_issue_form_fields_with_defaults() {
  let yaml = r#"
name: Feature
title: "[Feature]: "
labels: ["feature"]
body:
  - type: markdown
    attributes:
      value: |
        Intro for {desc}
  - type: textarea
    id: problem
    attributes:
      label: Problem
      placeholder: "Currently {type} is manual"
  - type: dropdown
    id: surface
    attributes:
      label: Surface
      options:
        - cli
        - tui
  - type: input
    id: owner
    attributes:
      label: Owner
"#;
  let ctx = TemplateContext::from_pairs([("type", "feat"), ("desc", "template-defaults")]);
  let mut defaults = BTreeMap::new();
  defaults.insert("surface".to_string(), "cli".to_string());
  defaults.insert("owner".to_string(), "platform".to_string());

  let rendered = render_form_markdown(yaml, &ctx, &FormDefaults { fields: defaults }).unwrap();

  assert!(rendered.contains("Intro for template-defaults"), "{rendered}");
  assert!(rendered.contains("## Problem"), "{rendered}");
  assert!(rendered.contains("Currently feat is manual"), "{rendered}");
  assert!(rendered.contains("**Surface:** cli"), "{rendered}");
  assert!(rendered.contains("**Owner:** platform"), "{rendered}");
}

/// Issue #688. The three issue forms this repo ships parse through the public
/// path, with the reader that replaced `serde_yaml_ng`.
///
/// `src/templating.rs` is the only `src/` module the parser swap touched, and
/// nothing read the real `.github/ISSUE_TEMPLATE/*.yml` through
/// `issue_form_metadata` / `render_form_markdown`: the suites that cover this
/// surface build synthetic templates in a tempdir, which cannot catch a reader
/// that stops understanding the files actually committed here. `config.yml`
/// is excluded because it is the chooser, not a form.
#[test]
fn every_shipped_issue_form_parses_and_renders() {
  let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/ISSUE_TEMPLATE");
  let ctx = TemplateContext::from_pairs([("type", "feat"), ("issue", "688"), ("desc", "a-description")]);
  let mut parsed = 0usize;
  for entry in std::fs::read_dir(&dir).expect("the issue template directory must exist") {
    let path = entry.expect("a readable entry").path();
    let name = path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or_default()
      .to_string();
    if !(name.ends_with(".yml") || name.ends_with(".yaml")) || name == "config.yml" {
      continue;
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));

    let meta = issue_form_metadata(&raw).unwrap_or_else(|e| panic!("{name} must parse as an issue form: {e}"));
    assert!(
      meta.title.as_deref().is_some_and(|t| !t.trim().is_empty()),
      "{name} must yield its `title:` prefix: a reader that parsed the file but read no title \
       would have `gwm new` open an issue with no prefix and no error, got {:?}",
      meta.title
    );
    assert!(
      !meta.labels.is_empty(),
      "{name} must yield its `labels:`: each shipped form declares at least one, and an empty \
       list is what a reader silently dropping a sequence looks like"
    );

    let rendered =
      render_form_markdown(&raw, &ctx, &FormDefaults::default()).unwrap_or_else(|e| panic!("{name} must render: {e}"));
    assert!(
      !rendered.trim().is_empty(),
      "{name} rendered to nothing: a reader that parsed the file into an empty `body` would \
       leave `gwm new` writing a blank description with no error"
    );
    parsed += 1;
  }
  // Exact, the policy #649 settled for a set of units the repo owns: a red
  // here is a form added or removed, which is an edit to acknowledge. Without
  // it a `read_dir` that stopped matching would report every form fine by
  // reading none.
  assert_eq!(
    parsed, 3,
    "expected the three issue forms besides config.yml, parsed {parsed}"
  );
}

/// Issue #688, the difference the parser swap makes in which forms parse and
/// what they yield.
///
/// `serde_yaml_ng` did not resolve a `<<:` merge key, so a form built on an
/// anchor lost the merged optional fields silently, or was rejected when the
/// merge carried a required one such as `type`, as it does here.
/// `serde-saphyr` resolves merge keys (a YAML 1.1 type), and this is pinned
/// rather than worked around. None of the forms this repo ships uses an
/// anchor today, so nothing changes for them; this is here so the difference
/// is a documented contract instead of a surprise the first time somebody
/// factors a shared field out of two forms.
#[test]
fn a_merge_key_contributes_its_fields() {
  let yaml = r#"
shared: &shared
  type: input
body:
  - <<: *shared
    id: owner
    attributes:
      label: Owner
"#;
  let ctx = TemplateContext::from_pairs([("type", "feat")]);
  let mut defaults = BTreeMap::new();
  defaults.insert("owner".to_string(), "platform".to_string());

  let rendered =
    render_form_markdown(yaml, &ctx, &FormDefaults { fields: defaults }).expect("a form using a merge key must parse");
  assert!(
    rendered.contains("**Owner:** platform"),
    "the merged `type: input` must reach the renderer, or the field is dropped without an \
     error: {rendered}"
  );
}

/// Issue #688. A duplicate key is refused even under a key `gwm` does not read.
///
/// `serde-saphyr` refuses a duplicate mapping key by default, and checks every
/// mapping, where `serde_yaml_ng` let a repeated key through when the struct
/// ignored it. So a form repeating its `name:` now fails `gwm new` instead of
/// rendering. The same form with one `name:` must parse, or the refusal below
/// would hold for a reason that has nothing to do with the duplicate.
#[test]
fn a_duplicate_key_gwm_does_not_read_is_refused() {
  let once = "name: Feature\ntitle: \"[Feature]: \"\nbody: []\n";
  let twice = "name: Feature\nname: Again\ntitle: \"[Feature]: \"\nbody: []\n";
  assert!(
    issue_form_metadata(once).is_ok(),
    "the form with one `name:` must parse"
  );
  assert!(
    issue_form_metadata(twice).is_err(),
    "a repeated `name:`, which no field of the form struct reads, must be refused"
  );
}
