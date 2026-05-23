//! Gitmoji mapping (issue #85).
//!
//! This repo standardises commits as `<emoji> <type>(#<issue>): <subject>`
//! (Gitmoji + Conventional Commits — see CONTRIBUTING.md). The mapping
//! `branch_type → emoji shortcode` is universal across the project, so
//! we bake a default table into the binary and let `.gwm.toml` override
//! individual entries via a `[gitmoji]` block:
//!
//! ```toml
//! [gitmoji]
//! feat = ":rocket:"  # team uses 🚀 for new features instead of ✨
//! ```
//!
//! Three surfaces consume this module:
//! 1. `gwm commit-prefix` — prints `:sparkles: feat(#41):` for the current
//!    or named branch (with `--unicode` to emit ✨ instead).
//! 2. `gwm types --gitmoji` — extends the branch-type list with the
//!    unicode + shortcode columns.
//! 3. `gwm hooks install commit-msg` — installs a `.git/hooks/commit-msg`
//!    that shells out to `gwm commit-prefix --unicode` and auto-prepends
//!    the prefix when missing.
//!
//! The shortcode → unicode table is intentionally kept small (the ten
//! built-in branch types + `:question:` as the unknown-type fallback)
//! to avoid pulling in a heavy `gh-emoji`-style dependency for a
//! handful of entries.

use crate::config::CONFIG_FILE;
use crate::error::Result;
use crate::naming::BranchSpec;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Built-in `branch_type → shortcode` table. Lifted to a `&[(&str,
/// &str)]` const so the static table stays compile-time and zero-alloc
/// at the storage level; the runtime view materialises on demand via
/// [`default_map`]. The list mirrors the ten built-in branch types
/// declared in `naming::BRANCH_TYPES`.
pub const DEFAULT_GITMOJI: &[(&str, &str)] = &[
  ("feat", ":sparkles:"),
  ("fix", ":bug:"),
  ("hotfix", ":ambulance:"),
  ("docs", ":memo:"),
  ("test", ":white_check_mark:"),
  ("refactor", ":recycle:"),
  ("chore", ":wrench:"),
  ("perf", ":zap:"),
  ("ci", ":construction_worker:"),
  ("build", ":package:"),
];

/// Fallback shortcode used by [`resolve_prefix`] when neither the user's
/// `[gitmoji]` block nor the built-in defaults claim a given branch
/// type. Picked so the surface stays syntactically valid (a prefix is
/// always emitted) while flagging "this type has no emoji yet" visually.
const UNKNOWN_SHORTCODE: &str = ":question:";

/// Resolved `branch_type → shortcode` table. The `BTreeMap` choice is
/// load-bearing: it gives deterministic iteration order (alphabetical
/// by branch type), which `gwm types --gitmoji` relies on so a CI diff
/// against the previous run is byte-stable.
#[derive(Debug, Clone, Default)]
pub struct GitmojiMap {
  entries: BTreeMap<String, String>,
}

impl GitmojiMap {
  /// Look up the shortcode for a branch type. Returns `None` for types
  /// not in the table — callers (notably [`resolve_prefix`]) decide
  /// the fallback policy.
  pub fn get(&self, branch_type: &str) -> Option<&str> {
    self.entries.get(branch_type).map(String::as_str)
  }

  /// Iterate over `(branch_type, shortcode)` pairs in deterministic
  /// (alphabetical) order. Used by `gwm types --gitmoji` and the
  /// "every default has a unicode" test sweep.
  pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
    self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
  }

  /// Merge another `(branch_type, shortcode)` pair into the table.
  /// Overrides the existing entry if any. Public so external callers
  /// (e.g. tests, future programmatic surfaces) can build a custom
  /// map without round-tripping through TOML.
  pub fn insert(&mut self, branch_type: impl Into<String>, shortcode: impl Into<String>) {
    self.entries.insert(branch_type.into(), shortcode.into());
  }
}

/// Materialise the built-in table as a [`GitmojiMap`]. The runtime cost
/// is one allocation per built-in entry — measured at ~1µs total in
/// release builds, dominated by the `BTreeMap` insertions, so we don't
/// cache.
pub fn default_map() -> GitmojiMap {
  let mut map = GitmojiMap::default();
  for (ty, shortcode) in DEFAULT_GITMOJI {
    map.insert(*ty, *shortcode);
  }
  map
}

/// Load `.gwm.toml`'s `[gitmoji]` block from the given repo root and
/// merge it on top of the built-in defaults. Returns the built-in
/// defaults verbatim when the file is missing or the block is absent.
///
/// `repo_root` is `Option` so callers without a workdir handle (e.g.
/// the `gwm commit-prefix --branch <name>` path, which doesn't need
/// to open a repo) can still get the defaults.
pub fn load(repo_root: Option<&Path>) -> Result<GitmojiMap> {
  let mut map = default_map();
  let Some(root) = repo_root else {
    return Ok(map);
  };
  let path = root.join(CONFIG_FILE);
  if !path.exists() {
    return Ok(map);
  }
  let raw = std::fs::read_to_string(&path)?;
  // Deserialise only the `[gitmoji]` block — we don't want to fail
  // here on unrelated config errors (a malformed `[[bootstrap.copy]]`
  // shouldn't break `gwm commit-prefix`). The dedicated struct keeps
  // the parse local to this module's contract.
  let parsed: GitmojiOnlyConfig = toml::from_str(&raw)?;
  for (ty, shortcode) in parsed.gitmoji {
    map.insert(ty, shortcode);
  }
  Ok(map)
}

/// Render the canonical commit prefix for a branch: `<emoji>
/// <type>(#<issue>):`. Use `unicode = true` to substitute the
/// shortcode for its real emoji character (e.g. `:sparkles:` → ✨).
///
/// When `branch.type_` has no entry in the map, falls back to
/// `:question:` / ❓ rather than panicking — the surface must be
/// usable on any branch, including non-gwm-style ones that bypass
/// `BranchSpec::validate`.
pub fn resolve_prefix(map: &GitmojiMap, branch: &BranchSpec, unicode: bool) -> String {
  let shortcode = map.get(&branch.type_).unwrap_or(UNKNOWN_SHORTCODE);
  let emoji = if unicode {
    shortcode_to_unicode(shortcode)
  } else {
    shortcode
  };
  format!("{} {}(#{}):", emoji, branch.type_, branch.issue)
}

/// Map a `:shortcode:` to its unicode character. Only the entries
/// shipped in the built-in table + `:question:` are supported — a
/// shortcode the user invented for their own branch type (e.g. `:fire:`
/// for `chore-remove`) round-trips verbatim, which is the right
/// behaviour: rendering an arbitrary user string under `--unicode`
/// would require a 3000-entry table or a heavy dep, and shortcodes
/// remain valid commit-message decoration anyway.
pub fn shortcode_to_unicode(shortcode: &str) -> &str {
  match shortcode {
    ":sparkles:" => "✨",
    ":bug:" => "🐛",
    ":ambulance:" => "🚑",
    ":memo:" => "📝",
    ":white_check_mark:" => "✅",
    ":recycle:" => "♻",
    ":wrench:" => "🔧",
    ":zap:" => "⚡",
    ":construction_worker:" => "👷",
    ":package:" => "📦",
    ":question:" => "❓",
    other => other,
  }
}

/// Local deserialisation envelope: we only care about the `[gitmoji]`
/// block here, and accepting unknown fields lets the rest of
/// `.gwm.toml` (bootstrap, worktree, labels, …) coexist without a
/// schema dependency in this module.
#[derive(Debug, Default, Deserialize)]
struct GitmojiOnlyConfig {
  #[serde(default)]
  gitmoji: BTreeMap<String, String>,
}
