//! GitHub label declarative management (issue #81).
//!
//! `[[labels]]` in `.gwm.toml` declares the desired label set; this
//! module resolves declared entries into concrete `LabelSpec` values
//! (filling in deterministic-pastel colours when omitted), computes
//! the diff against the upstream remote, and exposes the structs that
//! the CLI (`gwm labels list / push`) renders.
//!
//! The `gh`-backed I/O (fetch / create / delete) lives in `github.rs`;
//! this module is intentionally I/O-free so unit tests don't need a
//! network or a `gh` binary.

use crate::config::LabelConfig;
use crate::error::{GwmError, Result};
use std::collections::{HashMap, HashSet};

/// A fully-resolved label declared by the user: same shape as
/// `LabelConfig` but with `color` materialised (deterministic pastel,
/// user-declared, or `--random-colors`) and validated as 6-hex lower.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelSpec {
  pub name: String,
  pub description: Option<String>,
  pub color: String,
}

/// One label as returned by `gh label list --json …` for the upstream
/// remote. Colour is normalised to lowercase 6-hex on parse so the
/// diff doesn't surface a spurious "update" when GitHub renders it
/// uppercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteLabel {
  pub name: String,
  pub description: Option<String>,
  pub color: String,
}

/// Kind of mutation a `LabelUpdate` carries. The variant exists to
/// leave room for future "delete" entries when `--prune` is wired in
/// without reshuffling the `LabelDiff` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelAction {
  Create,
  Update,
}

/// One row in `LabelDiff::to_update`. Carries the previous remote
/// colour / description (when they differed) so `gwm labels list` can
/// render `~ good first issue (color #008672 → #7057ff)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelUpdate {
  pub action: LabelAction,
  pub spec: LabelSpec,
  pub previous_color: Option<String>,
  pub previous_description: Option<String>,
}

/// Result of diffing the declared label set against the remote. Each
/// bucket is rendered separately by `gwm labels list` and consumed by
/// `gwm labels push`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelDiff {
  pub to_create: Vec<LabelSpec>,
  pub to_update: Vec<LabelUpdate>,
  pub matching: Vec<LabelSpec>,
  pub extra_on_remote: Vec<RemoteLabel>,
}

impl LabelDiff {
  /// `(create, update, match, extra_on_remote)` — the four numbers
  /// surfaced in the one-line push summary.
  pub fn counts(&self) -> (usize, usize, usize, usize) {
    (
      self.to_create.len(),
      self.to_update.len(),
      self.matching.len(),
      self.extra_on_remote.len(),
    )
  }
}

// --- Colour helpers ------------------------------------------------------

/// Shape check: `s` is 6 ASCII hex chars (either case, no leading
/// `#`). Returns `Err` on length / non-hex inputs; the OK variant is
/// the input verbatim — case is NOT enforced here. Call
/// `normalize_color` if you also want the canonical lowercase form
/// (`#D73A4A` → `d73a4a`).
pub fn validate_color(s: &str) -> Result<&str> {
  if s.len() != 6 {
    return Err(GwmError::Config(format!(
      "invalid color '{}': expected 6 hex chars, got {}",
      s,
      s.len()
    )));
  }
  if !s.chars().all(|c| c.is_ascii_hexdigit()) {
    return Err(GwmError::Config(format!("invalid color '{}': not a hex string", s)));
  }
  Ok(s)
}

/// Trim a leading `#`, lowercase the hex, then validate. Returns the
/// canonical 6-hex form. Users naturally type `#D73A4A`; we accept it
/// rather than reject the config and lecture them about the spec.
pub fn normalize_color(s: &str) -> Result<String> {
  let trimmed = s.trim().trim_start_matches('#');
  let lower = trimmed.to_ascii_lowercase();
  validate_color(&lower)?;
  Ok(lower)
}

/// Deterministic pastel colour derived from the label name. FNV-1a
/// 64-bit hash → take low 3 bytes as RGB → average with white to push
/// the output into the pastel band (`#7f…` to `#ff…`). The choice of
/// FNV-1a (rather than `DefaultHasher`) is portability: same colour
/// across platforms, compilers, and Rust versions.
pub fn deterministic_color(name: &str) -> String {
  let h = fnv1a_64(name.as_bytes());
  let bytes = h.to_le_bytes();
  // Pastel transform: average each channel with 255 (white).
  let r = ((bytes[0] as u16 + 255) / 2) as u8;
  let g = ((bytes[1] as u16 + 255) / 2) as u8;
  let b = ((bytes[2] as u16 + 255) / 2) as u8;
  format!("{:02x}{:02x}{:02x}", r, g, b)
}

/// Pseudo-random 6-hex colour. Not cryptographic — used only when the
/// user passes `--random-colors`. Source of entropy is monotonic
/// nanoseconds XOR'd with a per-call counter so back-to-back calls
/// inside the same nanosecond don't collide.
pub fn random_color() -> String {
  use std::sync::atomic::{AtomicU64, Ordering};
  static COUNTER: AtomicU64 = AtomicU64::new(0);
  let n = COUNTER.fetch_add(1, Ordering::Relaxed);
  let t = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos() as u64;
  // Mix with the 64-bit golden ratio constant — same trick splitmix64
  // uses to spread sequential inputs across the output space.
  let mixed = (t ^ n).wrapping_mul(0x9E3779B97F4A7C15);
  let bytes = mixed.to_le_bytes();
  format!("{:02x}{:02x}{:02x}", bytes[0], bytes[1], bytes[2])
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
  let mut hash: u64 = 0xcbf29ce484222325;
  for &b in bytes {
    hash ^= b as u64;
    hash = hash.wrapping_mul(0x100000001b3);
  }
  hash
}

// --- Resolve LabelConfig → LabelSpec -------------------------------------

/// Reject label names that would either pass through to `gh label
/// create` as a flag or violate GitHub's own naming rules (issue #100).
///
/// The argv path matters most here: `gh label create <name>` takes the
/// name positionally, so a value like `"--repo"` or `"-h"` is parsed by
/// gh's flag splitter before the create call ever materialises. With
/// `-h` the create call silently no-ops (gh prints its help banner and
/// exits 0); with `--repo other/repo` the call retargets to a
/// different repository entirely. Validation at the source closes the
/// vector without depending on whether gh's parser ever grows a `--`
/// separator.
///
/// Empty strings, leading `-`, embedded `,` (GitHub uses `,` as the
/// list separator in label query strings), and ASCII control characters
/// (newline / tab / etc., which break the `gh label list` JSON round-trip)
/// are rejected. Spaces and unicode are explicitly allowed — GitHub
/// permits them and they are common in real-world label sets.
///
/// The returned error message is unscoped (no `"labels:"` prefix);
/// callers compose their own context — `Config::validate_labels`
/// prepends `labels[<i>]:`, `resolve_labels` prepends `labels:`,
/// the prune path prepends `labels (remote):`. Keeping the scope
/// at the call site avoids the double-prefix the Copilot review on
/// PR #121 flagged ("config error: labels[0]: config error: labels:
/// …").
pub fn validate_label_name(name: &str) -> Result<()> {
  if name.is_empty() {
    return Err(GwmError::Config(
      "entry has empty `name` — GitHub label names must be non-empty".into(),
    ));
  }
  if name.starts_with('-') {
    return Err(GwmError::Config(format!(
      "name {:?} starts with '-' — would be parsed as a flag by `gh label create`; \
       rename or remove the leading dash (issue #100)",
      name
    )));
  }
  if name.contains(',') {
    return Err(GwmError::Config(format!(
      "name {:?} contains ',' — GitHub uses comma as a label-list separator; rename without commas",
      name
    )));
  }
  if let Some(bad) = name.chars().find(|c| c.is_ascii_control()) {
    return Err(GwmError::Config(format!(
      "name {:?} contains ASCII control character {:?} — rename without control characters",
      name, bad
    )));
  }
  Ok(())
}

/// Materialise a list of declared `[[labels]]` entries into concrete
/// `LabelSpec` values. Declared colours win; missing colours fall
/// back to `deterministic_color(name)` unless `random` is set, in
/// which case `random_color()` is used.
///
/// Validates each `name` through [`validate_label_name`] as the
/// in-module defence-in-depth on top of `Config::validate_labels` at
/// load time — keeps the contract enforced even when a `LabelConfig`
/// is constructed in tests or via a future programmatic API that
/// bypasses the loader.
pub fn resolve_labels(declared: &[LabelConfig], random: bool) -> Result<Vec<LabelSpec>> {
  declared
    .iter()
    .map(|l| {
      validate_label_name(&l.name).map_err(|e| {
        let inner = match e {
          GwmError::Config(msg) => msg,
          other => other.to_string(),
        };
        GwmError::Config(format!("labels: {}", inner))
      })?;
      let color = match &l.color {
        Some(c) => {
          normalize_color(c).map_err(|e| GwmError::Config(format!("label '{}' has invalid color: {}", l.name, e)))?
        }
        None => {
          if random {
            random_color()
          } else {
            deterministic_color(&l.name)
          }
        }
      };
      Ok(LabelSpec {
        name: l.name.clone(),
        description: l.description.clone().filter(|s| !s.is_empty()),
        color,
      })
    })
    .collect()
}

// --- Diff declared vs remote --------------------------------------------

/// Compute the diff between the user's declared label set and the
/// labels currently on the upstream remote. Pure function — no I/O,
/// no observable side effects (allocates a transient `HashMap` and
/// `HashSet` to index by name, plus the returned `LabelDiff`).
///
/// Comparison rules:
/// - **Name** is the unique key. Matching is byte-exact (including
///   whitespace), since GitHub treats `bug` and `Bug` as different.
/// - **Color** is lowercased on both sides before comparing — defence
///   in depth on top of [`crate::github::parse_labels_json`], which
///   already lowercases remote colours, so a manually-constructed
///   `RemoteLabel` (e.g. from a test fixture) with uppercase hex
///   doesn't slip through as a spurious "update".
/// - **Description**: `None` and `Some("")` are equivalent (GitHub
///   stores them interchangeably).
pub fn diff_labels(declared: &[LabelSpec], remote: &[RemoteLabel]) -> LabelDiff {
  let remote_by_name: HashMap<&str, &RemoteLabel> = remote.iter().map(|r| (r.name.as_str(), r)).collect();
  let declared_names: HashSet<&str> = declared.iter().map(|s| s.name.as_str()).collect();

  let mut to_create = Vec::new();
  let mut to_update = Vec::new();
  let mut matching = Vec::new();

  for spec in declared {
    match remote_by_name.get(spec.name.as_str()) {
      None => to_create.push(spec.clone()),
      Some(r) => {
        let remote_color = r.color.to_ascii_lowercase();
        let spec_color = spec.color.to_ascii_lowercase();
        let color_match = remote_color == spec_color;
        let desc_match = norm_desc(&spec.description) == norm_desc(&r.description);
        if color_match && desc_match {
          matching.push(spec.clone());
        } else {
          to_update.push(LabelUpdate {
            action: LabelAction::Update,
            spec: spec.clone(),
            previous_color: if color_match { None } else { Some(r.color.clone()) },
            previous_description: if desc_match { None } else { r.description.clone() },
          });
        }
      }
    }
  }

  let extra_on_remote: Vec<RemoteLabel> = remote
    .iter()
    .filter(|r| !declared_names.contains(r.name.as_str()))
    .cloned()
    .collect();

  LabelDiff {
    to_create,
    to_update,
    matching,
    extra_on_remote,
  }
}

fn norm_desc(d: &Option<String>) -> Option<String> {
  d.as_ref().filter(|s| !s.is_empty()).cloned()
}
