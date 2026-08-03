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
  println!("{} = {}", resolved_key, crate::config::format_list_value(&rendered));
  Ok(())
}

/// Layer-aware, silent variant of [`set`] for the in-TUI Settings panel
/// (issue #279): set `key = value` in the TOML file at an EXPLICIT `path`
/// (the repo `.gwm.toml` OR the user-global `config.toml`) rather than the
/// discovered repo root, and return without printing so the TUI owns the
/// feedback. Reuses the exact key-path parser, scalar coercion, surgical
/// `toml_edit` write and post-write `Config` validation as `gwm config set`,
/// so a write that round-trips through `gwm config` round-trips here too.
///
/// Creates the parent directory when missing so the user-global file can be
/// written on its first use (the `~/.config/gwm/` dir may not exist yet).
///
/// `raw_value` is coerced with the same scalar heuristic as `gwm config set`
/// (`123` → int, `true` → bool, else string). Use [`set_string_at`] for
/// free-text settings that must stay strings regardless of their content.
pub fn set_value_at(path: &Path, key: &str, raw_value: &str) -> Result<()> {
  set_item_at(path, key, parse_scalar(raw_value))
}

/// String-forced variant of [`set_value_at`] for free-text Settings fields
/// (issue #279 review P2): always writes the value as a TOML string, so a
/// shell/editor command or worktree value like `123` / `true` is preserved
/// as text rather than coerced to a number/bool by `parse_scalar` (which
/// would then fail `Config` validation and, pre-fix, leave the file invalid).
pub fn set_string_at(path: &Path, key: &str, raw_value: &str) -> Result<()> {
  set_item_at(path, key, value(raw_value))
}

/// Array variant of [`set_value_at`] for the in-TUI Keys tab (issue #294):
/// write `key = ["a", "b", …]` as a TOML array of strings at an explicit
/// `path`. Backs the keymap rebind surface — a global action's chord list
/// under `[tui.keys]`, or a modal verb's single-stroke list under
/// `[tui.keys.modal.<context>]`. An empty `items` writes `key = []`, the
/// legitimate "unbind" value. Reuses the same parent-dir creation, surgical
/// `toml_edit` edit and validate-before-write as the scalar writers, so a
/// rebind that produces a conflicting / prefix-colliding keymap is rejected
/// before it can clobber a good file.
pub fn set_array_at(path: &Path, key: &str, items: &[String]) -> Result<()> {
  let mut arr = toml_edit::Array::new();
  for item in items {
    arr.push(item.as_str());
  }
  set_item_at(path, key, value(arr))
}

/// Remove `key` from the TOML file at an explicit `path` (issue #294): the
/// layer-aware sibling of [`unset`], used by the in-TUI Keys tab to strip a
/// pre-#290 alias when the canonical slug is rewritten. Tolerant — an absent
/// file or missing key is a no-op (nothing to remove), so callers can clear a
/// possible alias unconditionally. Validate-before-write like the setters.
pub fn unset_at(path: &Path, key: &str) -> Result<()> {
  if !path.exists() {
    return Ok(());
  }
  let mut doc = load_document(path)?;
  let segments = parse_key(key)?;
  remove_value(doc.as_table_mut(), &segments)?;
  write_and_validate(path, &doc)
}

/// Shared write path: ensure the parent dir exists, set `key` to `item` in
/// the surgically-edited document, and validate-before-write so an invalid
/// edit can never overwrite a good file.
fn set_item_at(path: &Path, key: &str, item: Item) -> Result<()> {
  if let Some(parent) = path.parent() {
    if !parent.as_os_str().is_empty() {
      std::fs::create_dir_all(parent)?;
    }
  }
  let mut doc = load_document(path)?;
  let segments = parse_key(key)?;
  set_value(doc.as_table_mut(), &segments, item)?;
  write_and_validate(path, &doc)
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
  crate::config::flatten_value("", &value, &mut rows);
  for (key, value) in rows {
    if prefix
      .map(|p| key == p || key.starts_with(&format!("{}.", p)) || key.starts_with(&format!("{}[", p)))
      .unwrap_or(true)
    {
      // Issue #473: the *value* is already escaped by `format_list_value`'s
      // `{:?}`, but the key is not — and a key is attacker-controlled wherever
      // the schema is a map rather than fixed fields (`[aliases]`,
      // `[forge_hosts]`, `[exec.profiles]`, `[clean.profiles]`, `[tui.keys]`).
      // Sanitised at the print site rather than inside `flatten_value`, whose
      // other callers compare keys across config layers to attribute a source
      // and must keep seeing them byte-for-byte.
      println!("{} = {}", crate::naming::sanitise_for_terminal(&key), value);
    }
  }
  Ok(())
}

pub fn validate() -> Result<()> {
  // Discover once: the warning below needs the repo *name* too (`{repo}` is
  // a supported `branch_pattern` token and the verdict depends on it), and
  // `repo_root` drops the handle it opened.
  let repo = worktree::discover_repo(None)?;
  let root = repo.workdir().ok_or(GwmError::NotInGitRepo)?.to_path_buf();
  let path = config_path(&root);
  let cfg = validate_file(&path)?;
  println!("{} is valid", path.display());
  // Issue #415: a `branch_pattern` the parser cannot read back is *valid*
  // config — it just silently breaks everything keyed on the re-parsed
  // segments. Stated on stderr so the exit code stays 0 and piped
  // consumers of stdout are unaffected.
  //
  // Read the *effective* pattern, not the repo file's: `branch_pattern`
  // set only in the user-level global config still applies at runtime
  // through `merge_layered`, and validating `path` alone would stay quiet
  // about it while `gwm doctor` (which sees the merged view) warns. A
  // broken global layer is not this command's business — it reports on
  // `path` — so fall back to the repo-only value it just validated.
  let effective = Config::merge_layered(&root, crate::config::global_config_path().as_deref()).unwrap_or(cfg);
  let types = effective.resolved_branch_types().types;
  if let Some(warning) =
    crate::naming::branch_pattern_warning(&effective.worktree.branch_pattern, &worktree::repo_name(&repo), &types)
  {
    // Issue #473: `branch_pattern_warning` already neutralises the pattern it
    // quotes, but it also embeds the repo name, and this `eprintln!` bypasses
    // the sink in `main` (it is a warning, not a returned error). One row, so
    // the row variant.
    eprintln!("warning: {}", crate::naming::sanitise_for_terminal(&warning));
  }
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
  let rendered = doc.to_string();
  match validate_rendered(path, &rendered) {
    // The edit is valid — write it.
    Ok(_) => {
      std::fs::write(path, rendered)?;
      Ok(())
    }
    // The edit would produce an invalid Config. Only refuse the write when
    // the existing on-disk file is VALID (or absent) — i.e. this edit would
    // clobber a good file with a broken one (issue #279 review P2). If the
    // file is ALREADY invalid, keep the historical write-then-error
    // behaviour so `gwm config set` can still edit a broken file toward a
    // fixed state rather than refusing every edit until it is hand-repaired
    // (issue #281 — the validate-before-write chicken-and-egg).
    Err(e) => {
      if validate_file(path).is_ok() {
        return Err(e);
      }
      std::fs::write(path, rendered)?;
      Err(e)
    }
  }
}

/// Returns the validated `Config` so callers can inspect the resolved
/// values without re-parsing the file — an absent config yields the
/// defaults, which is exactly what the loader would have produced.
fn validate_file(path: &Path) -> Result<Config> {
  if !path.exists() {
    return Ok(Config::default());
  }
  let raw = std::fs::read_to_string(path)?;
  validate_rendered(path, &raw)
}

/// Validate `raw` as a complete `Config` (deserialization + the semantic
/// checks `gwm config validate` runs). `path` is only used for error
/// coordinates. Shared by [`validate_file`] (on-disk) and the
/// validate-before-write path in [`write_and_validate`].
fn validate_rendered(path: &Path, raw: &str) -> Result<Config> {
  let cfg = toml::from_str::<Config>(raw).map_err(|e| config_de_error(path, raw, e))?;
  cfg.validate_branch_types()?;
  cfg.validate_bootstrap_paths()?;
  cfg.validate_bootstrap_guards()?;
  cfg.validate_labels()?;
  cfg.validate_aliases()?;
  // `[tui.keys]` / `[theme]` deserialize into raw tables resolved lazily, so a
  // malformed keymap or theme passes `toml::from_str` cleanly. Run the same
  // validators `Config::load_for_repo` does (issue #219 review) — otherwise
  // `gwm config validate` / validate-before-write greenlights a config the
  // loader will later reject.
  cfg.validate_tui_keys()?;
  cfg.validate_theme()?;
  // `[exec.profiles]` / `[clean.profiles]` semantics (non-empty command, a
  // worktree-relative single-name `dirs`) parse cleanly too, so run the same
  // check `load_for_repo` does — otherwise `gwm config validate` greenlights a
  // profile the loader and the new commands reject (issue #324 review).
  cfg.validate_profiles()?;
  Ok(cfg)
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

fn format_get_value(value: &toml::Value) -> String {
  match value {
    // Issue #473: `gwm config get` prints the string bare (that is its
    // contract — the output is meant to be pipeable), so unlike the
    // `format_list_value` path below it gets no incidental protection from
    // `Debug`'s escaping. Neutralise the control bytes here instead.
    toml::Value::String(s) => crate::naming::sanitise_for_terminal(s),
    _ => crate::config::format_list_value(value),
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
    Some(span) => GwmError::ConfigDiagnostic(format!(
      "{}: error at line {}, col {}: {}",
      path.display(),
      line_col(raw, span.start).0,
      line_col(raw, span.start).1,
      msg
    )),
    None => GwmError::ConfigDiagnostic(format!("{}: {}", path.display(), msg)),
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
    Some(span) => GwmError::ConfigDiagnostic(format!(
      "{}: error at line {}, col {}: {}",
      path.display(),
      line_col(raw, span.start).0,
      line_col(raw, span.start).1,
      err
    )),
    None => GwmError::ConfigDiagnostic(format!("{}: {}", path.display(), err)),
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
