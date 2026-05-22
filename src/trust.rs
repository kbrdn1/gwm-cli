//! TOFU (trust-on-first-use) trust ledger for `.gwm.toml` (issue #95).
//!
//! The ledger persists `(origin URL, sha256 of .gwm.toml)` tuples to a
//! per-user file (default `~/.config/gwm/trust.toml`, overridable via
//! the `GWM_TRUST_LEDGER` env var). On every `gwm create` / `gwm
//! bootstrap` we hash the current `.gwm.toml`, look the tuple up, and
//! either skip silently (already trusted) or prompt the user before
//! handing control to `bootstrap::run` (which is the RCE primitive).
//!
//! Threat model: an attacker who controls a remote repository (a fork,
//! a fresh hostile clone, a co-worker compromise on a shared repo) can
//! drop arbitrary `[[bootstrap.command]]` lines and have them executed
//! the next time anyone runs `gwm create` against that repo. Hashing
//! the raw bytes catches both wholesale rewrites and surgical edits
//! (whitespace included — `rm -rf /tmp/` and `rm -rf /tmp /` are one
//! byte apart and behave catastrophically differently). Storing the
//! origin URL alongside the hash means moving a config from one repo
//! to another forces a fresh trust decision.
//!
//! The ledger format is plain TOML so it can be inspected by hand
//! (`gwm trust show` prints the active path) and version-controlled
//! per-machine if a team wants to share trust decisions explicitly.

use crate::config::{Config, CONFIG_FILE};
use crate::error::{GwmError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::Builder;

/// Trust gating mode resolved at the CLI / TUI entrypoint and threaded
/// through every code path that may invoke `bootstrap::run`. Moved
/// here (from `src/cli.rs`) so the TUI can take the same decision
/// without duplicating the resolution logic — see [`evaluate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustMode {
  /// Default: load the ledger, prompt the user on miss, abort on
  /// non-tty.
  Prompt,
  /// `--allow-bootstrap` or `GWM_ALLOW_BOOTSTRAP=1` — skip the prompt
  /// without touching the ledger. CI bypasses must NOT pollute the
  /// local ledger of whoever ends up running an interactive `gwm`
  /// from the same machine later.
  Allow,
  /// `--deny-bootstrap` — refuse to run bootstrap even if already
  /// trusted. For forensic / first-look inspection of a hostile repo.
  Deny,
}

/// Resolve a [`TrustMode`] from CLI flags + env. Both `--allow-bootstrap`
/// and `--deny-bootstrap` map onto this — `conflicts_with` at the clap
/// level guarantees they aren't both set, the explicit ordering here is
/// defence in depth.
pub fn resolve_mode(allow_flag: bool, deny_flag: bool) -> TrustMode {
  if deny_flag {
    return TrustMode::Deny;
  }
  if allow_flag || env_truthy("GWM_ALLOW_BOOTSTRAP") {
    return TrustMode::Allow;
  }
  TrustMode::Prompt
}

/// Truthy env semantics: any non-empty value other than `0`, `false`,
/// or `no` (case-insensitive) counts as true. Documented here as the
/// source of truth — gwm has no shared env-bool helper yet, so this
/// fn is the canonical reference for `GWM_*` flag-style env vars.
pub fn env_truthy(key: &str) -> bool {
  match std::env::var(key) {
    Ok(v) => {
      let v = v.trim().to_ascii_lowercase();
      !v.is_empty() && v != "0" && v != "false" && v != "no"
    }
    Err(_) => false,
  }
}

/// Resolve the trust-ledger key for the current repo: prefer the
/// `origin` remote URL (the hostile-clone threat axis), fall back to
/// the canonicalised workdir path (purely-local repos with no remote
/// still benefit from the drift-detection half of the feature).
///
/// Takes `origin_url: Option<&str>` rather than a `git2::Repository`
/// so this module stays git2-free — every caller already holds a
/// Repository and extracts the URL on their side in three lines.
pub fn resolve_origin_key(origin_url: Option<&str>, workdir: &Path) -> String {
  if let Some(url) = origin_url {
    if !url.is_empty() {
      return url.to_string();
    }
  }
  workdir
    .canonicalize()
    .unwrap_or_else(|_| workdir.to_path_buf())
    .display()
    .to_string()
}

/// Outcome of a trust gate evaluation. The caller (CLI or TUI) decides
/// what to do with each variant — CLI prompts on `Prompt`, TUI refuses
/// with a helpful message because the alternate-screen mode can't host
/// a stdin read without a full modal view (deferred follow-up).
#[derive(Debug)]
pub enum TrustOutcome {
  /// Cleared to invoke `bootstrap::run`: trusted entry hit, empty
  /// surface, no `.gwm.toml`, or `TrustMode::Allow`.
  Proceed,
  /// Refuse outright. `message` is the user-facing reason; render it
  /// verbatim in stderr (CLI) or status bar (TUI). Used for `Deny`
  /// mode by `evaluate`; the TUI also synthesises a `Refuse` from a
  /// `Prompt` outcome since it can't prompt today.
  Refuse { message: String },
  /// Caller must obtain user approval interactively. On approval the
  /// caller records into `ledger`, then saves to `ledger_path`. The
  /// `body` and `sha` are passed back so the prompt can display a
  /// summary without re-reading the file.
  Prompt {
    cfg_path: PathBuf,
    body: Vec<u8>,
    sha: String,
    origin: String,
    ledger: TrustLedger,
    ledger_path: PathBuf,
  },
}

/// Silent gate evaluation. Reads `.gwm.toml`, hashes it, applies the
/// short-circuits in this order:
///
///   1. No `.gwm.toml` → `Proceed` (nothing to execute).
///   2. `TrustMode::Deny` → `Refuse` (forensic mode).
///   3. Empty bootstrap surface → `Proceed` (UX, see comment in
///      [`trust_or_prompt`]).
///   4. `TrustMode::Allow` → `Proceed` BEFORE touching the ledger
///      (malformed ledger must not break the CI bypass).
///   5. Ledger hit on `(origin, sha)` → `Proceed`.
///   6. Otherwise → `Prompt` (caller decides whether to actually
///      prompt or refuse).
///
/// This is the single source of truth shared by `cli::trust_or_prompt`
/// and the TUI gate — keeps the security policy in one place.
pub fn evaluate(workdir: &Path, origin: &str, mode: TrustMode) -> Result<TrustOutcome> {
  let cfg_path = workdir.join(CONFIG_FILE);
  let bytes = match fs::read(&cfg_path) {
    Ok(b) => b,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TrustOutcome::Proceed),
    Err(e) => return Err(e.into()),
  };
  let sha = hash_config(&bytes);

  if mode == TrustMode::Deny {
    let short_sha: String = sha.chars().take(12).collect();
    return Ok(TrustOutcome::Refuse {
      message: format!(
        "--deny-bootstrap: refusing to run .gwm.toml bootstrap (config hash: {})",
        short_sha
      ),
    });
  }

  // Empty surface short-circuit (defence-in-depth fallthrough on
  // parse error — a broken parser must not open a bypass).
  if let Ok(body_str) = std::str::from_utf8(&bytes) {
    if let Ok(cfg) = toml::from_str::<Config>(body_str) {
      let bs = &cfg.bootstrap;
      if bs.copy.is_empty() && bs.guard.is_empty() && bs.no_symlink.is_empty() && bs.command.is_empty() {
        return Ok(TrustOutcome::Proceed);
      }
    }
  }

  // Allow short-circuit before any ledger I/O so a malformed
  // trust.toml never breaks `--allow-bootstrap` / `GWM_ALLOW_BOOTSTRAP=1`.
  if mode == TrustMode::Allow {
    return Ok(TrustOutcome::Proceed);
  }

  let ledger_path = default_ledger_path()?;
  let ledger = TrustLedger::load(&ledger_path)?;

  if ledger.lookup(origin, &sha) {
    return Ok(TrustOutcome::Proceed);
  }

  Ok(TrustOutcome::Prompt {
    cfg_path,
    body: bytes,
    sha,
    origin: origin.to_string(),
    ledger,
    ledger_path,
  })
}

/// On-disk ledger schema. `serde` defaults make adding new optional
/// fields backward-compatible: older binaries still parse newer files,
/// they just ignore the extra keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustLedger {
  #[serde(default, rename = "entries")]
  pub entries: Vec<TrustEntry>,
}

/// One trust grant. `origin` is the remote URL (kept verbatim so SSH
/// and HTTPS flavours of the same repo are treated as distinct trust
/// boundaries — they ARE distinct: different auth path, different
/// failure modes on intercept). `config_sha` is the lowercase hex
/// sha256 of `.gwm.toml`'s raw bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEntry {
  pub origin: String,
  pub config_sha: String,
  /// RFC3339 timestamp of when the entry was first recorded. Surfaced
  /// by `gwm trust list` so users can audit who/when the trust was
  /// granted before deciding to revoke.
  pub trusted_at: DateTime<Utc>,
  /// Best-effort `user@host` identifier captured at record time. Not
  /// security-relevant on its own (it's local input), purely an audit
  /// hint for multi-machine users sharing a ledger via dotfiles.
  pub trusted_by: String,
}

impl TrustLedger {
  /// Load the ledger from `path`. A missing file is NOT an error — it
  /// is treated as an empty ledger, which is the right default for
  /// the first ever invocation. A malformed file IS an error: silently
  /// treating it as empty would re-prompt every previously trusted
  /// repo and train the user to mash `y` (anti-habituation goal of the
  /// whole feature).
  pub fn load(path: &Path) -> Result<Self> {
    match fs::read_to_string(path) {
      Ok(raw) => {
        let ledger: TrustLedger = toml::from_str(&raw)?;
        Ok(ledger)
      }
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
      Err(e) => Err(e.into()),
    }
  }

  /// Persist the ledger atomically: serialise, write to a uniquely-
  /// named tmp file in the same directory, then rename(2). The
  /// rename is the atomic step on POSIX; on Windows it is also a
  /// single syscall but with slightly different semantics around
  /// open file handles. `tempfile::NamedTempFile::persist` papers
  /// over both.
  ///
  /// The tmp filename is randomised (`gwm-trust-<random>.tmp`) so
  /// two `gwm` processes hitting `save` concurrently don't clobber
  /// each other's intermediate write — pre-fix both raced on the
  /// fixed name `trust.toml.tmp` and could corrupt the final
  /// ledger if one process's rename interleaved with the other's
  /// write.
  ///
  /// Parent directories are created on demand (`mkdir -p`) so the
  /// first ever write on a fresh machine succeeds without the user
  /// having to create `~/.config/gwm/` manually.
  pub fn save(&self, path: &Path) -> Result<()> {
    let parent = match path.parent() {
      Some(p) if !p.as_os_str().is_empty() => {
        fs::create_dir_all(p)?;
        p.to_path_buf()
      }
      // No parent (e.g. relative `trust.toml` in CWD) or empty
      // parent → write the tmp file in `.`.
      _ => PathBuf::from("."),
    };
    let body = toml::to_string_pretty(self)?;
    let mut tmp = Builder::new()
      .prefix("gwm-trust-")
      .suffix(".tmp")
      .tempfile_in(&parent)?;
    tmp.write_all(body.as_bytes())?;
    // `persist` does the atomic rename and consumes the handle so
    // the tempfile crate's drop-cleanup is short-circuited — no
    // sidecar `.tmp` survives a successful save.
    tmp.persist(path).map_err(|e| GwmError::Io(e.error))?;
    Ok(())
  }

  /// Returns true iff there is an entry with both `origin` AND
  /// `config_sha` matching verbatim. Hash drift on a known origin is
  /// a deliberate `false` — that is the re-prompt-on-config-edit
  /// behaviour spec'd by the issue.
  pub fn lookup(&self, origin: &str, config_sha: &str) -> bool {
    self
      .entries
      .iter()
      .any(|e| e.origin == origin && e.config_sha == config_sha)
  }

  /// Record (or refresh) a trust grant. Always produces exactly one
  /// entry per `origin`: any prior entry is dropped first, then a
  /// fresh entry is pushed with `trusted_at = Utc::now()`. So:
  ///
  ///   * Re-recording the same `(origin, config_sha)` keeps a single
  ///     entry but **refreshes the timestamp** — useful when a user
  ///     explicitly re-confirms trust without editing the config.
  ///   * Re-recording the same `origin` with a different
  ///     `config_sha` supersedes the old hash (drift case), keeping
  ///     the ledger bounded over a repo's lifetime — without this,
  ///     every `.gwm.toml` edit would leak a stale tuple that
  ///     `gwm trust list` would surface forever.
  ///
  /// In both cases `entries.len()` after a re-record is the same as
  /// before; `record_is_idempotent_on_exact_match` and
  /// `record_supersedes_drifted_hash_for_same_origin` pin both
  /// halves down.
  pub fn record(&mut self, origin: &str, config_sha: &str, trusted_by: &str) {
    self.entries.retain(|e| e.origin != origin);
    self.entries.push(TrustEntry {
      origin: origin.to_string(),
      config_sha: config_sha.to_string(),
      trusted_at: Utc::now(),
      trusted_by: trusted_by.to_string(),
    });
  }

  /// Remove every entry matching `origin`. Returns the count so
  /// `gwm trust revoke` can print a precise "removed N entries" line
  /// instead of guessing.
  pub fn revoke(&mut self, origin: &str) -> usize {
    let before = self.entries.len();
    self.entries.retain(|e| e.origin != origin);
    before - self.entries.len()
  }
}

/// SHA-256 of the raw bytes of `.gwm.toml`, lowercase hex. Whitespace-
/// sensitive on purpose (see the module-level comment).
pub fn hash_config(bytes: &[u8]) -> String {
  let digest = Sha256::digest(bytes);
  hex_lower(&digest)
}

/// Resolve the active ledger path. Order of precedence:
///   1. `GWM_TRUST_LEDGER` env var (the testability hook + power-user
///      override for users with non-XDG dotfiles).
///   2. `dirs::config_dir()/gwm/trust.toml` (XDG on Linux, the
///      `Application Support` equivalent on macOS, `%APPDATA%` on
///      Windows).
///
/// Returns `Err(GwmError::Other(..))` only on the rare case where
/// `dirs::config_dir()` cannot determine a home — extremely uncommon,
/// but better surfaced than panicked away.
pub fn default_ledger_path() -> Result<PathBuf> {
  if let Ok(p) = std::env::var("GWM_TRUST_LEDGER") {
    if !p.is_empty() {
      return Ok(PathBuf::from(p));
    }
  }
  let base = dirs::config_dir().ok_or_else(|| {
    GwmError::Other("could not resolve user config directory — set GWM_TRUST_LEDGER to override".into())
  })?;
  Ok(base.join("gwm").join("trust.toml"))
}

/// Best-effort `user@host` audit string. Falls back to `"unknown"` on
/// each half independently so we never panic in a CI shell with
/// minimal env, and the resulting string is purely informational
/// (never used for trust decisions).
pub fn current_actor() -> String {
  let user = std::env::var("USER")
    .or_else(|_| std::env::var("USERNAME"))
    .unwrap_or_else(|_| "unknown".into());
  let host = current_hostname().unwrap_or_else(|| "unknown".into());
  format!("{}@{}", user, host)
}

#[cfg(unix)]
fn current_hostname() -> Option<String> {
  // libc is already a pinned Unix dependency (used by bootstrap.rs's
  // O_NOFOLLOW primitives), so reusing it for `gethostname(3)` keeps
  // the dep tree flat — no need for an extra `gethostname`/`whoami`
  // crate just for an audit-log string.
  let mut buf = [0i8; 256];
  let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
  if rc != 0 {
    return None;
  }
  // Find the NUL terminator. POSIX doesn't promise gethostname will
  // null-terminate if the host name is exactly the buffer length, so
  // we cap at buf.len() defensively.
  let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
  // SAFETY: buf[..len] is a valid byte slice; we re-interpret the
  // i8 view as u8 (same layout) to feed it to String.
  let bytes: Vec<u8> = buf[..len].iter().map(|&b| b as u8).collect();
  String::from_utf8(bytes).ok()
}

#[cfg(not(unix))]
fn current_hostname() -> Option<String> {
  std::env::var("COMPUTERNAME")
    .ok()
    .or_else(|| std::env::var("HOSTNAME").ok())
}

fn hex_lower(bytes: &[u8]) -> String {
  let mut s = String::with_capacity(bytes.len() * 2);
  for b in bytes {
    s.push_str(&format!("{:02x}", b));
  }
  s
}
