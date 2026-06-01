//! Shared template renderer tests for GitHub issue / PR automation.

use std::collections::BTreeMap;

use gwm::templating::{render_form_markdown, render_template, FormDefaults, TemplateContext};

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
