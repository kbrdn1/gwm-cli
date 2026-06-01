use crate::error::{GwmError, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
  values: BTreeMap<String, String>,
}

impl TemplateContext {
  pub fn from_pairs<const N: usize>(pairs: [(&str, &str); N]) -> Self {
    Self {
      values: pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect(),
    }
  }
}

#[derive(Debug, Clone, Default)]
pub struct FormDefaults {
  pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct FormMetadata {
  pub title: Option<String>,
  pub labels: Vec<String>,
}

pub fn render_template(template: &str, ctx: &TemplateContext) -> String {
  let mut rendered = template.to_string();
  for (key, value) in &ctx.values {
    rendered = rendered.replace(&format!("{{{}}}", key), value);
  }
  rendered
}

pub fn issue_form_metadata(raw: &str) -> Result<FormMetadata> {
  let form = parse_issue_form(raw)?;
  Ok(FormMetadata {
    title: form.title,
    labels: form.labels,
  })
}

pub fn render_form_markdown(raw: &str, ctx: &TemplateContext, defaults: &FormDefaults) -> Result<String> {
  let form = parse_issue_form(raw)?;
  let mut out = String::new();
  for item in form.body {
    match item.kind.as_str() {
      "markdown" => {
        if let Some(value) = item.attributes.value {
          push_block(&mut out, &render_template(&value, ctx));
        }
      }
      "textarea" => {
        let label = item
          .attributes
          .label
          .clone()
          .unwrap_or_else(|| item.id.clone().unwrap_or_else(|| "Field".into()));
        let value = field_value(&item, defaults).unwrap_or_default();
        push_block(&mut out, &format!("## {}\n\n{}", label, render_template(&value, ctx)));
      }
      "input" | "dropdown" => {
        let label = item
          .attributes
          .label
          .clone()
          .unwrap_or_else(|| item.id.clone().unwrap_or_else(|| "Field".into()));
        let value = field_value(&item, defaults).unwrap_or_default();
        push_block(&mut out, &format!("**{}:** {}", label, render_template(&value, ctx)));
      }
      _ => {}
    }
  }
  Ok(out.trim_end().to_string())
}

fn parse_issue_form(raw: &str) -> Result<IssueForm> {
  serde_yml::from_str(raw).map_err(|e| GwmError::Config(format!("issue template YAML: {}", e)))
}

fn field_value(item: &IssueFormItem, defaults: &FormDefaults) -> Option<String> {
  if let Some(id) = &item.id {
    if let Some(value) = defaults.fields.get(id) {
      return Some(value.clone());
    }
  }
  item
    .attributes
    .value
    .clone()
    .or_else(|| item.attributes.placeholder.clone())
}

fn push_block(out: &mut String, block: &str) {
  let block = block.trim();
  if block.is_empty() {
    return;
  }
  if !out.is_empty() {
    out.push_str("\n\n");
  }
  out.push_str(block);
}

#[derive(Debug, Deserialize)]
struct IssueForm {
  #[serde(default)]
  title: Option<String>,
  #[serde(default)]
  labels: Vec<String>,
  #[serde(default)]
  body: Vec<IssueFormItem>,
}

#[derive(Debug, Deserialize)]
struct IssueFormItem {
  #[serde(rename = "type")]
  kind: String,
  #[serde(default)]
  id: Option<String>,
  #[serde(default)]
  attributes: IssueFormAttributes,
}

#[derive(Debug, Default, Deserialize)]
struct IssueFormAttributes {
  #[serde(default)]
  label: Option<String>,
  #[serde(default)]
  value: Option<String>,
  #[serde(default)]
  placeholder: Option<String>,
}
