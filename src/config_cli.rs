use crate::config::{Config, CONFIG_FILE};
use crate::error::{GwmError, Result};
use crate::worktree;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Index {
  Number(usize),
  Append,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
  name: String,
  index: Option<Index>,
}

pub fn get(key: &str) -> Result<()> {
  let root = repo_root()?;
  let cfg = Config::load_for_repo(&root)?;
  let value = resolved_value(&cfg, key)?;
  println!("{}", format_get_value(&value));
  Ok(())
}

pub fn set(key: &str, raw_value: Option<&str>) -> Result<()> {
  let (key, raw_value) = split_set_args(key, raw_value)?;
  let root = repo_root()?;
  let path = config_path(&root);
  let mut doc = load_document(&path)?;
  let segments = parse_key(&key)?;
  let value = parse_scalar(&raw_value);
  let resolved_key = set_value(doc.as_table_mut(), &segments, value)?;
  write_and_validate(&path, &doc)?;
  let rendered = resolved_value(&Config::load_for_repo(&root)?, &resolved_key)?;
  println!("{} = {}", resolved_key, format_list_value(&rendered));
  Ok(())
}

fn split_set_args(key: &str, raw_value: Option<&str>) -> Result<(String, String)> {
  match (key.split_once('='), raw_value) {
    (Some((key, value)), None) if !key.is_empty() => Ok((key.to_string(), value.to_string())),
    (None, Some(value)) => Ok((key.to_string(), value.to_string())),
    (Some(_), Some(_)) => Err(GwmError::Config(
      "`gwm config set` accepts either `<key> <value>` or `<key=value>`, not both".into(),
    )),
    _ => Err(GwmError::Config(
      "`gwm config set` requires a value (`<key> <value>` or `<key=value>`)".into(),
    )),
  }
}

pub fn unset(key: &str) -> Result<()> {
  let root = repo_root()?;
  let path = config_path(&root);
  let mut doc = load_document(&path)?;
  let segments = parse_key(key)?;
  remove_value(doc.as_table_mut(), &segments)?;
  write_and_validate(&path, &doc)?;
  println!("unset {}", key);
  Ok(())
}

pub fn list(prefix: Option<&str>) -> Result<()> {
  let root = repo_root()?;
  let cfg = Config::load_for_repo(&root)?;
  let value = toml::Value::try_from(cfg).map_err(|e| GwmError::Config(e.to_string()))?;
  let mut rows = Vec::new();
  flatten_value("", &value, &mut rows);
  for (key, value) in rows {
    if prefix
      .map(|p| key == p || key.starts_with(&format!("{}.", p)) || key.starts_with(&format!("{}[", p)))
      .unwrap_or(true)
    {
      println!("{} = {}", key, value);
    }
  }
  Ok(())
}

pub fn validate() -> Result<()> {
  let root = repo_root()?;
  let path = config_path(&root);
  validate_file(&path)?;
  println!("{} is valid", path.display());
  Ok(())
}

pub fn path() -> Result<()> {
  let root = repo_root()?;
  println!("{}", config_path(&root).display());
  Ok(())
}

pub fn edit() -> Result<()> {
  let root = repo_root()?;
  let path = config_path(&root);
  if !path.exists() {
    std::fs::write(&path, "")?;
  }
  let editor = std::env::var("EDITOR")
    .map_err(|_| GwmError::Config("EDITOR is not set; set EDITOR or open `gwm config path` manually".into()))?;
  let status = Command::new(&editor)
    .arg(&path)
    .status()
    .map_err(|e| GwmError::CommandFailed(format!("{}: failed to spawn editor ({})", editor, e)))?;
  if !status.success() {
    return Err(GwmError::CommandFailed(format!("{} exited with {}", editor, status)));
  }
  validate_file(&path)?;
  Ok(())
}

fn repo_root() -> Result<PathBuf> {
  let repo = worktree::discover_repo(None)?;
  let workdir = repo.workdir().ok_or(GwmError::NotInGitRepo)?;
  Ok(workdir.to_path_buf())
}

fn config_path(root: &Path) -> PathBuf {
  root.join(CONFIG_FILE)
}

fn load_document(path: &Path) -> Result<DocumentMut> {
  if !path.exists() {
    return Ok(DocumentMut::new());
  }
  let raw = std::fs::read_to_string(path)?;
  raw
    .parse::<DocumentMut>()
    .map_err(|e| config_parse_error(path, &raw, e))
}

fn write_and_validate(path: &Path, doc: &DocumentMut) -> Result<()> {
  std::fs::write(path, doc.to_string())?;
  validate_file(path)
}

fn validate_file(path: &Path) -> Result<()> {
  if !path.exists() {
    return Ok(());
  }
  let raw = std::fs::read_to_string(path)?;
  let cfg = toml::from_str::<Config>(&raw).map_err(|e| config_de_error(path, &raw, e))?;
  cfg.validate_branch_types()?;
  cfg.validate_bootstrap_paths()?;
  cfg.validate_bootstrap_guards()?;
  cfg.validate_labels()?;
  cfg.validate_aliases()?;
  Ok(())
}

fn resolved_value(cfg: &Config, key: &str) -> Result<toml::Value> {
  let value = toml::Value::try_from(cfg.clone()).map_err(|e| GwmError::Config(e.to_string()))?;
  Ok(lookup_value(&value, &parse_key(key)?)?.clone())
}

fn lookup_value<'a>(value: &'a toml::Value, segments: &[Segment]) -> Result<&'a toml::Value> {
  let mut current = value;
  for segment in segments {
    current = current
      .get(&segment.name)
      .ok_or_else(|| GwmError::Config(format!("unknown config key '{}'", render_segments(segments))))?;
    if let Some(index) = &segment.index {
      let array = current
        .as_array()
        .ok_or_else(|| GwmError::Config(format!("'{}' is not an array", segment.name)))?;
      let Index::Number(i) = index else {
        return Err(GwmError::Config("[+] is only valid for `config set`".into()));
      };
      current = array
        .get(*i)
        .ok_or_else(|| GwmError::Config(format!("array index out of bounds: {}[{}]", segment.name, i)))?;
    }
  }
  Ok(current)
}

fn parse_key(key: &str) -> Result<Vec<Segment>> {
  let mut segments = Vec::new();
  for raw in key.split('.') {
    if raw.is_empty() {
      return Err(GwmError::Config(format!(
        "invalid empty config key segment in '{}'",
        key
      )));
    }
    let (name, index) = if let Some(open) = raw.find('[') {
      let close = raw
        .strip_suffix(']')
        .ok_or_else(|| GwmError::Config(format!("invalid array segment '{}'", raw)))?;
      let name = &raw[..open];
      let idx = &close[open + 1..];
      let index = if idx == "+" {
        Index::Append
      } else {
        Index::Number(
          idx
            .parse()
            .map_err(|_| GwmError::Config(format!("invalid array index '{}'", idx)))?,
        )
      };
      (name, Some(index))
    } else {
      (raw, None)
    };
    if name.is_empty() || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
      return Err(GwmError::Config(format!("invalid config key segment '{}'", raw)));
    }
    segments.push(Segment {
      name: name.to_string(),
      index,
    });
  }
  Ok(segments)
}

fn parse_scalar(raw: &str) -> Item {
  if let Ok(parsed) = raw.parse::<i64>() {
    return value(parsed);
  }
  if let Ok(parsed) = raw.parse::<f64>() {
    return value(parsed);
  }
  match raw {
    "true" => value(true),
    "false" => value(false),
    _ => value(raw),
  }
}

fn set_value(table: &mut Table, segments: &[Segment], new_value: Item) -> Result<String> {
  let Some((head, tail)) = segments.split_first() else {
    return Err(GwmError::Config("empty config key".into()));
  };
  if tail.is_empty() {
    if head.index.is_some() {
      return Err(GwmError::Config(
        "array-table keys must name a field after the index".into(),
      ));
    }
    table.insert(&head.name, new_value);
    return Ok(render_segments(segments));
  }

  match &head.index {
    None => {
      let item = table.entry(&head.name).or_insert_with(|| Item::Table(Table::new()));
      if item.is_none() {
        *item = Item::Table(Table::new());
      }
      let child = item
        .as_table_mut()
        .ok_or_else(|| GwmError::Config(format!("'{}' is not a table", head.name)))?;
      let tail_key = set_value(child, tail, new_value)?;
      Ok(format!("{}.{}", head.name, tail_key))
    }
    Some(index) => {
      let item = table
        .entry(&head.name)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
      if item.is_none() {
        *item = Item::ArrayOfTables(ArrayOfTables::new());
      }
      let array = item
        .as_array_of_tables_mut()
        .ok_or_else(|| GwmError::Config(format!("'{}' is not an array of tables", head.name)))?;
      let actual = match index {
        Index::Number(i) => {
          while array.len() <= *i {
            array.push(Table::new());
          }
          *i
        }
        Index::Append => {
          array.push(Table::new());
          array.len() - 1
        }
      };
      let child = array
        .get_mut(actual)
        .ok_or_else(|| GwmError::Config(format!("array index out of bounds: {}[{}]", head.name, actual)))?;
      let mut resolved = segments.to_vec();
      resolved[0].index = Some(Index::Number(actual));
      let tail_key = set_value(child, tail, new_value)?;
      Ok(format!("{}.{}", render_segment(&resolved[0]), tail_key))
    }
  }
}

fn remove_value(table: &mut Table, segments: &[Segment]) -> Result<()> {
  let Some((head, tail)) = segments.split_first() else {
    return Err(GwmError::Config("empty config key".into()));
  };
  if tail.is_empty() {
    if head.index.is_some() {
      return Err(GwmError::Config(
        "array-table keys must name a field after the index".into(),
      ));
    }
    table.remove(&head.name);
    return Ok(());
  }
  match &head.index {
    None => {
      let Some(item) = table.get_mut(&head.name) else {
        return Ok(());
      };
      let Some(child) = item.as_table_mut() else {
        return Ok(());
      };
      remove_value(child, tail)
    }
    Some(Index::Number(i)) => {
      let Some(item) = table.get_mut(&head.name) else {
        return Ok(());
      };
      let Some(array) = item.as_array_of_tables_mut() else {
        return Ok(());
      };
      let Some(child) = array.get_mut(*i) else {
        return Ok(());
      };
      remove_value(child, tail)
    }
    Some(Index::Append) => Err(GwmError::Config("[+] is only valid for `config set`".into())),
  }
}

fn flatten_value(prefix: &str, value: &toml::Value, rows: &mut Vec<(String, String)>) {
  match value {
    toml::Value::Table(table) => {
      for (key, value) in table {
        let next = if prefix.is_empty() {
          key.to_string()
        } else {
          format!("{}.{}", prefix, key)
        };
        flatten_value(&next, value, rows);
      }
    }
    toml::Value::Array(values) if values.iter().all(toml::Value::is_table) => {
      for (i, value) in values.iter().enumerate() {
        flatten_value(&format!("{}[{}]", prefix, i), value, rows);
      }
    }
    _ => rows.push((prefix.to_string(), format_list_value(value))),
  }
}

fn format_get_value(value: &toml::Value) -> String {
  match value {
    toml::Value::String(s) => s.clone(),
    _ => format_list_value(value),
  }
}

fn format_list_value(value: &toml::Value) -> String {
  match value {
    toml::Value::String(s) => format!("{:?}", s),
    toml::Value::Integer(i) => i.to_string(),
    toml::Value::Float(f) => f.to_string(),
    toml::Value::Boolean(b) => b.to_string(),
    toml::Value::Datetime(d) => d.to_string(),
    toml::Value::Array(_) | toml::Value::Table(_) => value.to_string(),
  }
}

fn render_segments(segments: &[Segment]) -> String {
  segments.iter().map(render_segment).collect::<Vec<_>>().join(".")
}

fn render_segment(segment: &Segment) -> String {
  match &segment.index {
    Some(Index::Number(i)) => format!("{}[{}]", segment.name, i),
    Some(Index::Append) => format!("{}[+]", segment.name),
    None => segment.name.clone(),
  }
}

fn config_de_error(path: &Path, raw: &str, err: toml::de::Error) -> GwmError {
  let msg = enrich_schema_hint(err.to_string());
  match err.span() {
    Some(span) => GwmError::Config(format!(
      "{}: error at line {}, col {}: {}",
      path.display(),
      line_col(raw, span.start).0,
      line_col(raw, span.start).1,
      msg
    )),
    None => GwmError::Config(format!("{}: {}", path.display(), msg)),
  }
}

fn enrich_schema_hint(message: String) -> String {
  if message.contains("fullscreem") {
    format!("{} (did you mean 'fullscreen'?)", message)
  } else {
    message
  }
}

fn config_parse_error(path: &Path, raw: &str, err: toml_edit::TomlError) -> GwmError {
  match err.span() {
    Some(span) => GwmError::Config(format!(
      "{}: error at line {}, col {}: {}",
      path.display(),
      line_col(raw, span.start).0,
      line_col(raw, span.start).1,
      err
    )),
    None => GwmError::Config(format!("{}: {}", path.display(), err)),
  }
}

fn line_col(raw: &str, offset: usize) -> (usize, usize) {
  let mut line = 1;
  let mut col = 1;
  for (idx, ch) in raw.char_indices() {
    if idx >= offset {
      break;
    }
    if ch == '\n' {
      line += 1;
      col = 1;
    } else {
      col += 1;
    }
  }
  (line, col)
}
