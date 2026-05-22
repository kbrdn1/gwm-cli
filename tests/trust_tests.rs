//! Unit tests for the `trust` module (issue #95).
//!
//! The TOFU ledger persists `(origin URL, sha256 of .gwm.toml)` tuples
//! to `~/.config/gwm/trust.toml` so subsequent `gwm bootstrap` / `gwm
//! create` invocations skip the prompt when nothing has drifted. These
//! tests pin down:
//!
//!   * stable byte-level hashing (whitespace matters — see comment on
//!     the `sha2` dep in Cargo.toml)
//!   * empty-ledger semantics on first load
//!   * round-trip serialisation (load → mutate → save → load)
//!   * lookup hits / misses (origin and hash both matter)
//!   * `record` is idempotent and refreshes `trusted_at` on re-record
//!   * `revoke` removes matching entries, returns the count
//!   * `default_path` honours the `GWM_TRUST_LEDGER` override (used by
//!     both these unit tests and the integration tests in
//!     `cli_binary.rs`)
//!
//! The ledger I/O surface is `pub` to keep tests honest — see
//! `tests/cli_binary.rs::trust_*` for the end-to-end coverage.

use gwm::trust::{hash_config, TrustLedger};
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

/// Process-global lock guarding every test that mutates `std::env`.
/// Rust 1.86+ marks `set_var` / `remove_var` as `unsafe` because the
/// underlying libc calls aren't thread-safe; without this lock, two
/// env-mutating tests running in parallel under `cargo test`'s default
/// thread pool can race and trigger UB. All test fns that touch env
/// vars in this file MUST take this lock before any `set_var` /
/// `remove_var`. (Yes, this serialises those specific tests — there's
/// currently one, and `serial_test` would be overkill for one site.)
fn env_lock() -> &'static Mutex<()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn hash_config_is_stable_and_distinguishes_whitespace() {
  let a = hash_config(b"[bootstrap]\n");
  let b = hash_config(b"[bootstrap]\n");
  assert_eq!(a, b, "hash must be deterministic for identical bytes");

  // Whitespace-sensitive on purpose: `rm -rf /tmp/` and `rm -rf /tmp /`
  // are one byte apart and behave catastrophically differently. The
  // ledger has to notice every byte.
  let c = hash_config(b"[bootstrap] \n");
  assert_ne!(a, c, "hash must distinguish payloads that differ only in whitespace");

  // Hex-encoded sha256 is 64 chars.
  assert_eq!(a.len(), 64, "sha256 hex digest is 64 characters");
  assert!(
    a.chars().all(|c| c.is_ascii_hexdigit()),
    "hash must be ascii hex (got {})",
    a
  );
}

#[test]
fn hash_config_known_vector() {
  // sha256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
  let empty = hash_config(b"");
  assert_eq!(
    empty,
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
  );
}

#[test]
fn default_ledger_is_empty() {
  let ledger = TrustLedger::default();
  assert!(ledger.entries.is_empty());
  assert_eq!(ledger.entries.len(), 0);
}

#[test]
fn load_missing_file_yields_empty_ledger() {
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("absent.toml");
  let ledger = TrustLedger::load(&path).expect("missing file must not be an error");
  assert!(ledger.entries.is_empty());
}

#[test]
fn lookup_hit_when_origin_and_hash_match() {
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  assert!(ledger.lookup("git@github.com:kbrdn1/gwm-cli.git", "abc123"));
}

#[test]
fn lookup_miss_when_origin_known_but_hash_drifted() {
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  assert!(
    !ledger.lookup("git@github.com:kbrdn1/gwm-cli.git", "deadbeef"),
    "hash drift on a known origin must NOT be treated as trusted — \
     the whole point of the ledger is to re-prompt on every config edit"
  );
}

#[test]
fn lookup_miss_when_origin_unknown() {
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  assert!(!ledger.lookup("git@gitlab.com:other/repo.git", "abc123"));
}

#[test]
fn record_then_save_then_load_round_trips() {
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("trust.toml");

  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  ledger.record("https://github.com/foo/bar", "deadbeef", "alice@nb");
  ledger.save(&path).unwrap();

  let loaded = TrustLedger::load(&path).unwrap();
  assert_eq!(loaded.entries.len(), 2);
  assert!(loaded.lookup("git@github.com:kbrdn1/gwm-cli.git", "abc123"));
  assert!(loaded.lookup("https://github.com/foo/bar", "deadbeef"));
}

#[test]
fn record_is_idempotent_on_exact_match() {
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  assert_eq!(
    ledger.entries.len(),
    1,
    "re-recording the same (origin, hash) tuple must not duplicate the entry"
  );
}

#[test]
fn record_supersedes_drifted_hash_for_same_origin() {
  // When a `.gwm.toml` edit lands and the user re-trusts, the old hash
  // entry is replaced — keeping the ledger free of stale tuples that
  // would otherwise grow unbounded over a repo's lifetime.
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "old_hash", "kylian@host");
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "new_hash", "kylian@host");
  assert_eq!(ledger.entries.len(), 1);
  assert!(ledger.lookup("git@github.com:kbrdn1/gwm-cli.git", "new_hash"));
  assert!(!ledger.lookup("git@github.com:kbrdn1/gwm-cli.git", "old_hash"));
}

#[test]
fn revoke_removes_all_entries_for_origin() {
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  ledger.record("git@github.com:foo/bar.git", "deadbeef", "alice@nb");
  let removed = ledger.revoke("git@github.com:kbrdn1/gwm-cli.git");
  assert_eq!(removed, 1);
  assert_eq!(ledger.entries.len(), 1);
  assert!(!ledger.lookup("git@github.com:kbrdn1/gwm-cli.git", "abc123"));
  assert!(ledger.lookup("git@github.com:foo/bar.git", "deadbeef"));
}

#[test]
fn revoke_unknown_origin_returns_zero() {
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  let removed = ledger.revoke("git@github.com:does-not-exist/x.git");
  assert_eq!(removed, 0);
  assert_eq!(ledger.entries.len(), 1);
}

#[test]
fn save_creates_parent_directories() {
  // The default path is `~/.config/gwm/trust.toml`; on a fresh machine
  // neither `.config` nor `gwm/` exists yet, so `save` must mkdir -p.
  let dir = TempDir::new().unwrap();
  let nested = dir.path().join("a/b/c/trust.toml");
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  ledger.save(&nested).unwrap();
  assert!(nested.exists());
  assert!(nested.parent().unwrap().is_dir());
}

#[test]
fn save_is_atomic_via_tmp_then_rename() {
  // We can't easily detect the rename itself, but we can verify the
  // sidecar `.tmp` file is gone after a successful save (it would
  // otherwise leak on every write — a portability footgun on Windows).
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("trust.toml");
  let mut ledger = TrustLedger::default();
  ledger.record("git@github.com:kbrdn1/gwm-cli.git", "abc123", "kylian@host");
  ledger.save(&path).unwrap();
  let leftovers: Vec<_> = std::fs::read_dir(dir.path())
    .unwrap()
    .filter_map(|e| e.ok())
    .map(|e| e.file_name().into_string().unwrap_or_default())
    .filter(|n| n.ends_with(".tmp"))
    .collect();
  assert!(
    leftovers.is_empty(),
    "atomic save must not leave a .tmp sidecar (got {:?})",
    leftovers
  );
}

#[test]
fn default_path_honours_gwm_trust_ledger_env_override() {
  // The env override is the testability hook used by `gwm trust *`
  // assert_cmd tests so they don't have to clobber the user's real
  // ~/.config/gwm/trust.toml.
  //
  // Hold the process-wide env lock for the whole test body so a
  // parallel env-mutating test cannot interleave `set_var` /
  // `remove_var` calls with ours — Rust 1.86+ marks both as
  // `unsafe` specifically because libc's env table isn't thread-
  // safe. Poisoning is fine to ignore: a panic in another env test
  // is unrelated to our state, and we're going to overwrite the
  // variable anyway.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let dir = TempDir::new().unwrap();
  let override_path = dir.path().join("custom-trust.toml");

  let prior = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: env mutation is guarded by `env_lock()` above, so no
  // other test in this binary is mutating env concurrently. We
  // restore the prior value (or remove the var) before dropping
  // the lock to keep the harness consistent for any later test.
  unsafe { std::env::set_var("GWM_TRUST_LEDGER", &override_path) };
  let resolved = gwm::trust::default_ledger_path().expect("env override resolves");
  assert_eq!(resolved, override_path);

  // SAFETY: restoration step paired with the set_var above, still
  // under the env_lock guard.
  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn load_malformed_toml_returns_error_not_empty() {
  // A corrupted ledger is suspicious — it could be intentional
  // tampering. We refuse to silently treat it as empty (which would
  // re-prompt every previously trusted repo and habituate the user
  // to mashing `y`).
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("trust.toml");
  std::fs::write(&path, b"this is not valid toml @@@@").unwrap();
  let err = TrustLedger::load(&path).expect_err("malformed TOML must be an error");
  let msg = format!("{}", err);
  assert!(
    msg.contains("toml") || msg.contains("parse") || msg.contains("TOML"),
    "error should mention TOML parsing (got: {})",
    msg
  );
}
