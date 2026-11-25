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

/// The trust-ledger key for `repo` — [`resolve_origin_key`] fed from the
/// `origin` remote.
///
/// One helper rather than the extraction repeated at each call site.
/// That repetition is what broke: the doc below used to argue the module
/// should stay git2-free because "every caller already holds a
/// Repository and extracts the URL on their side in three lines", and
/// three of the four callers wrote the same three lines while the fourth
/// reached for `RemoteRef::web_origin` instead (issue #463).
///
/// The difference is not cosmetic. `web_origin` is scheme + host, so the
/// key stopped identifying a *repo* and started identifying a *host* —
/// collapsing `(origin, sha256)` into a pair shared by every repo on
/// that host with an identically-hashing `.gwm.toml`. Since that file is
/// normally a template copied across a team's repos, identical hashes
/// are the ordinary case, and a hostile repo could inherit a sibling's
/// approval by shipping its config verbatim. The two gates also stopped
/// seeing each other's entries, so `gwm trust add` reported success and
/// `gwm create` in the same repo still refused.
pub fn origin_key_for_repo(repo: &git2::Repository, workdir: &Path) -> String {
  let url = repo
    .find_remote("origin")
    .ok()
    .and_then(|r| r.url().ok().map(String::from));
  resolve_origin_key(url.as_deref(), workdir)
}

/// Resolve the trust-ledger key for the current repo: prefer the
/// `origin` remote URL (the hostile-clone threat axis), fall back to
/// the canonicalised workdir path (purely-local repos with no remote
/// still benefit from the drift-detection half of the feature).
///
/// Prefer [`origin_key_for_repo`] when you hold a `Repository`. This
/// lower-level form stays for callers that genuinely have only a URL.
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

// ---- Trusted origins (issue #751) ----------------------------------------
//
// The ledger approves one `(origin, hash)` pair, so every edit to a repo's
// `.gwm.toml` revokes it. Right for a clone of someone else's repo; a chore
// for your own, where a reviewed PR touching the file stalls the next
// unattended `gwm create`. `[trust] origins` in the user's own config names
// repos trusted whatever the file's hash, as long as origin, asked by a CLI
// run after a ledger miss, publishes that file on its default branch, `dev`
// or `main`. Never read from a repo's `.gwm.toml`: a repo cannot vouch for
// itself.

/// `(host, path segments)` of a remote URL, parsed the way the forge gate
/// parses `origin` (#463): host lower-cased with the documented SSH aliases
/// folded, path without `.git`. So `https`, `ssh` and scp-like spellings of
/// one repo agree, and a workdir key or a malformed URL is `None`.
fn origin_parts(url: &str) -> Option<(String, Vec<String>)> {
  let r = crate::forge::parse_remote_url(url).ok()?;
  Some((r.host, r.path.split('/').map(String::from).collect()))
}

fn pattern_parts(pattern: &str) -> Result<(String, Vec<String>)> {
  let bad = |why: &str| GwmError::Other(format!("origin pattern '{}' {}", pattern, why));
  let (host, segments) = origin_parts(pattern).ok_or_else(|| bad("is not a git remote URL"))?;
  if host.contains('*') {
    return Err(bad(
      "has a wildcard in its host: spell the host out, a wildcard there is how a lookalike gets in",
    ));
  }
  if segments.iter().any(|s| s.is_empty() || s == "." || s == "..") {
    return Err(bad("has an empty or dot path segment"));
  }
  // A pattern matches a whole path, so an owner alone names no repo: the
  // prefix the issue asked for is spelled `owner/*`.
  if segments.len() < 2 {
    return Err(GwmError::Other(format!(
      "origin pattern '{}' names no repo: write '{}/*' to trust the repos under it",
      pattern,
      pattern.trim_end_matches('/')
    )));
  }
  if segments[0].contains('*') {
    return Err(bad(
      "must name its owner: a wildcard owner trusts every repo on the host",
    ));
  }
  Ok((host, segments))
}

/// Refuse a pattern that could never match safely. `gwm trust add --origin`
/// checks before writing; [`matching_origin_pattern`] skips what fails, in
/// case the file was edited by hand.
pub fn validate_origin_pattern(pattern: &str) -> Result<()> {
  pattern_parts(pattern).map(|_| ())
}

/// `*` matches any run of characters inside one path segment, never a `/`.
fn glob_segment(pattern: &str, segment: &str) -> bool {
  let mut parts = pattern.split('*');
  let Some(mut rest) = segment.strip_prefix(parts.next().unwrap_or("")) else {
    return false;
  };
  let parts: Vec<&str> = parts.collect();
  let Some((last, middle)) = parts.split_last() else {
    return rest.is_empty();
  };
  for part in middle {
    match rest.find(part) {
      Some(i) => rest = &rest[i + part.len()..],
      None => return false,
    }
  }
  rest.ends_with(last)
}

/// The first pattern in `patterns` that covers `origin`, if any.
///
/// Host compared whole, so `github.com.evil.io` is not `github.com`; path
/// compared segment by segment, same count, so `owner/*` covers neither a
/// sibling owner nor a deeper path. An origin path with a dot segment
/// matches nothing, and neither does an origin [`plain_origin`] refuses.
pub fn matching_origin_pattern<'a>(patterns: &'a [String], origin: &str) -> Option<&'a str> {
  if !plain_origin(origin) {
    return None;
  }
  let (host, segments) = origin_parts(origin)?;
  if segments.iter().any(|s| s.is_empty() || s == "." || s == "..") {
    return None;
  }
  first_covering(patterns, &host, &segments)
}

/// The first pattern in `patterns` that covers every repo `pattern` covers:
/// another spelling of it or a wider one, the `*` of `pattern` read as a
/// literal. `gwm trust revoke '<pattern>'` must not report success while one
/// remains.
pub fn covering_origin_pattern<'a>(patterns: &'a [String], pattern: &str) -> Option<&'a str> {
  let (host, segments) = pattern_parts(pattern).ok()?;
  first_covering(patterns, &host, &segments)
}

fn first_covering<'a>(patterns: &'a [String], host: &str, segments: &[String]) -> Option<&'a str> {
  patterns
    .iter()
    .find(|p| {
      pattern_parts(p).is_ok_and(|(ph, ps)| {
        ph == host && ps.len() == segments.len() && ps.iter().zip(segments).all(|(p, s)| glob_segment(p, s))
      })
    })
    .map(String::as_str)
}

/// The invariant: an origin is covered only when the host git connects to is
/// the host gwm compared against the rule.
///
/// So only the spellings the forges hand out, `https://`, `ssh://` and the
/// scp-like `[user@]host:path`, in characters every client reads alike, with
/// the host read the way git reads it ([`dialed_host`]) and folded the way
/// gwm folds it. Each gap this closes was a working lookalike:
/// `https://evil.example#@github.com/…` (curl stops at `#`),
/// `evil.example:x@github.com:o/r` (git's scp syntax stops at the first `:`),
/// `/tmp/x@github.com:o/r` (a `/` first makes it a local path). `http` and
/// `git` are unauthenticated, so anyone on the path serves the file.
fn plain_origin(origin: &str) -> bool {
  let charset = origin
    .chars()
    .all(|c| c.is_ascii_alphanumeric() || "-._~:/@".contains(c));
  let host_of = |url: &str| origin_parts(url).map(|(host, _)| host);
  charset
    && origin.matches('@').count() <= 1
    && dialed_host(origin).is_some_and(|host| host_of(&format!("https://{host}/x")) == host_of(origin))
}

/// The host git connects to for `origin`, by git's own rules, for the three
/// shapes a rule covers; `None` for any other. The authority runs to the
/// first `/` after `https://` or `ssh://`, and to the first `:` in scp syntax,
/// which git only reads as such with no `/` before that `:`. A `:` in the
/// user part or a port that is not a number is refused, not interpreted.
fn dialed_host(origin: &str) -> Option<&str> {
  let authority = match origin.split_once("://") {
    Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("ssh") => {
      rest.split_once('/')?.0
    }
    Some(_) => return None,
    None => {
      let (authority, _) = origin.split_once(':')?;
      if authority.contains('/') {
        return None;
      }
      authority
    }
  };
  let (user, host_port) = authority.split_once('@').unwrap_or(("", authority));
  let (host, port) = host_port.split_once(':').unwrap_or((host_port, ""));
  (!host.is_empty() && !user.contains(':') && port.chars().all(|c| c.is_ascii_digit())).then_some(host)
}

/// The user's rule that covers `origin`, if any (issue #751).
pub fn user_origin_rule(origin: &str) -> Option<String> {
  let path = crate::config::global_config_path()?;
  matching_origin_pattern(&Config::trust_in(&path).origins, origin).map(String::from)
}

/// How long origin gets to answer before a rule stops applying (issue #751):
/// past it, the run meets the prompt the hash gives it.
pub const ASK_ORIGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether origin, asked now, publishes `bytes`, up to line endings, as the
/// `.gwm.toml` of its default branch, `dev` or `main` (issue #751).
///
/// The invariant: a rule covers only bytes origin itself announces, through
/// the host it was matched on, or wherever the user's own git config
/// (`insteadOf`) sends that URL, which no clone can change. Nothing local
/// proves that: any fetch writes
/// any `refs/remotes/origin/*` name, from a refspec typed once or from a
/// branch a fork's author named `remotes/origin/dev`. So origin is asked over
/// the network which commits those branches point at, and the bytes must be
/// the `.gwm.toml` of one of them, held locally: an object is named by its
/// content, so holding it is holding what origin published. Named in advance,
/// not the repo's `doctor.trunks`: that list lives in the file being judged.
/// A local edit, a commit not pushed yet or a checkout behind origin falls
/// back to the ledger. CLI only: the TUI does not dial.
pub fn published_on_origin(workdir: &Path, origin: &str, bytes: &[u8]) -> bool {
  if !plain_origin(origin) {
    return false;
  }
  let Ok(repo) = git2::Repository::open(workdir) else {
    return false;
  };
  announced_trunks(&repo, workdir, origin).is_some_and(|oids| publishes(&repo, &oids, bytes))
}

/// The decision alone: whether one of `announced`, the commits origin says
/// its trunks point at, is a commit `repo` holds whose `.gwm.toml` is
/// `bytes`, up to line endings. Refs play no part.
pub fn publishes(repo: &git2::Repository, announced: &[git2::Oid], bytes: &[u8]) -> bool {
  let want = without_crlf(bytes);
  announced
    .iter()
    .any(|oid| config_at(repo, *oid).is_some_and(|blob| without_crlf(&blob) == want))
}

fn config_at(repo: &git2::Repository, oid: git2::Oid) -> Option<Vec<u8>> {
  let tree = repo.find_commit(oid).ok()?.tree().ok()?;
  let entry = tree.get_path(Path::new(CONFIG_FILE)).ok()?;
  Some(repo.find_blob(entry.id()).ok()?.content().to_vec())
}

/// What origin is asked about: its default branch and the built-in trunks.
fn asked_refs() -> Vec<String> {
  std::iter::once("HEAD".to_string())
    .chain(
      crate::config::default_trunks()
        .iter()
        .map(|b| format!("refs/heads/{b}")),
    )
    .collect()
}

/// The commits a `git ls-remote` answer names for exactly the refs asked
/// about. ls-remote matches a pattern on the tail of a name, so
/// `refs/heads/x/refs/heads/dev` comes back too; only an exact name and a
/// full hex id count.
pub fn announced_in(stdout: &str) -> Vec<git2::Oid> {
  let asked = asked_refs();
  stdout
    .lines()
    .filter_map(|line| {
      let (oid, name) = line.split_once('\t')?;
      let full = oid.len() == 40 && oid.bytes().all(|b| b.is_ascii_hexdigit());
      (full && asked.iter().any(|a| a == name))
        .then(|| git2::Oid::from_str(oid).ok())
        .flatten()
    })
    .collect()
}

/// Ask origin, with git so the user's own git config applies, which commits
/// [`asked_refs`] point at. `None` unless git answers, successfully, within
/// [`ASK_ORIGIN_TIMEOUT`].
///
/// Never a prompt: a rule is for runs nobody watches, an agent in a
/// background pane, and a credential or passphrase question left unanswered
/// would hang it. Credential helpers still run, they are how a private repo
/// answers over https.
fn announced_trunks(repo: &git2::Repository, workdir: &Path, origin: &str) -> Option<Vec<git2::Oid>> {
  use std::io::Read;
  use std::process::{Command, Stdio};
  use std::time::{Duration, Instant};

  // The user's ssh command, kept, made unable to ask.
  let ssh = std::env::var("GIT_SSH_COMMAND")
    .ok()
    .or_else(|| repo.config().ok()?.get_string("core.sshCommand").ok())
    .unwrap_or_else(|| "ssh".into());
  let mut cmd = Command::new("git");
  cmd
    .current_dir(workdir)
    .args(["-c", "core.askPass=", "ls-remote", "--"])
    .arg(origin)
    .args(asked_refs())
    .env("GIT_TERMINAL_PROMPT", "0")
    .env_remove("GIT_ASKPASS")
    .env_remove("SSH_ASKPASS")
    .env("SSH_ASKPASS_REQUIRE", "never")
    .env("GCM_INTERACTIVE", "never")
    .env("GIT_SSH_COMMAND", format!("{ssh} -o BatchMode=yes"))
    // A rewrite in the user's git config to `file::`, `ext::` or plain
    // `http` reaches something other than the host compared: fail on it.
    .env("GIT_ALLOW_PROTOCOL", "https:ssh")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
  // Its own group, so a tty read stops it rather than asking, and the kill
  // below reaches `git-remote-https` or `ssh` too.
  #[cfg(unix)]
  std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
  #[cfg(unix)]
  let signals = DialSignals::install();
  let mut child = cmd.spawn().ok()?;
  #[cfg(unix)]
  signals.arm(child.id());

  // Read on a thread: a helper still holding the pipe must not hold gwm.
  let mut stdout = child.stdout.take()?;
  let (tx, rx) = std::sync::mpsc::channel();
  std::thread::spawn(move || {
    let mut out = String::new();
    let _ = stdout.read_to_string(&mut out);
    let _ = tx.send(out);
  });
  let deadline = Instant::now() + ASK_ORIGIN_TIMEOUT;
  let out = rx.recv_timeout(ASK_ORIGIN_TIMEOUT).ok();
  let status = loop {
    match child.try_wait() {
      Ok(Some(status)) => break Some(status),
      Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
      _ => break None,
    }
  };
  match status {
    Some(status) => {
      // Reaped: the group id is no longer ours to kill.
      #[cfg(unix)]
      drop(signals);
      status.success().then_some(out).flatten().map(|out| announced_in(&out))
    }
    // The handlers stay until the group is down, below.
    None => {
      // SAFETY: a plain syscall on the group `process_group(0)` made, whose
      // leader is not reaped yet, so the id is still ours.
      #[cfg(unix)]
      unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL)
      };
      // Killing git alone leaves `git-remote-https` holding gwm's own output
      // handles, inherited, until curl gives up minutes later.
      #[cfg(windows)]
      let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
      let _ = child.kill();
      let _ = child.wait();
      None
    }
  }
}

/// The group [`announced_trunks`] is waiting on when positive; a signal that
/// came before the group was known, negated, when negative; 0 otherwise. One
/// atomic, so the handler and [`DialSignals::arm`] cannot both miss it.
#[cfg(unix)]
static DIAL_GROUP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// A Ctrl-C, a Ctrl-\, a closed pane or a `kill` reaches gwm, not the group git runs
/// in: take that group down, then die of the signal as gwm would have. Before
/// the spawn has returned there is no group yet: the signal is left for
/// [`DialSignals::arm`], which does the same once it knows it.
#[cfg(unix)]
extern "C" fn end_dial_then_die(sig: libc::c_int) {
  use std::sync::atomic::Ordering::SeqCst;
  let group = match DIAL_GROUP.compare_exchange(0, -sig, SeqCst, SeqCst) {
    Ok(_) => return,
    // One already waits for `arm`, which ends gwm with it.
    Err(waiting) if waiting < 0 => return,
    Err(group) => group,
  };
  // SAFETY: `kill` and `raise` are async-signal-safe, and `SA_RESETHAND` put
  // the default action back, so the raise ends gwm once this returns.
  unsafe {
    libc::kill(-group, libc::SIGKILL);
    libc::raise(sig);
  }
}

/// [`end_dial_then_die`] on the signals that end a CLI run, for as long as
/// this lives. A signal already ignored (`nohup`) stays ignored: the dial
/// ends within its timeout anyway.
#[cfg(unix)]
struct DialSignals(Vec<(libc::c_int, libc::sigaction)>);

#[cfg(unix)]
impl DialSignals {
  fn install() -> Self {
    let mut saved = Vec::new();
    for sig in [libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGHUP] {
      // SAFETY: `sigaction` on zeroed structs, read before it is replaced.
      unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(sig, std::ptr::null(), &mut old) != 0 || old.sa_sigaction == libc::SIG_IGN {
          continue;
        }
        let mut new: libc::sigaction = std::mem::zeroed();
        new.sa_sigaction = end_dial_then_die as extern "C" fn(libc::c_int) as libc::sighandler_t;
        new.sa_flags = libc::SA_RESETHAND;
        libc::sigemptyset(&mut new.sa_mask);
        if libc::sigaction(sig, &new, std::ptr::null_mut()) == 0 {
          saved.push((sig, old));
        }
      }
    }
    DialSignals(saved)
  }

  /// Record the group git runs in. A signal the handler left pending, while
  /// the spawn had not returned, is acted on now: the group goes, then gwm.
  fn arm(&self, group: u32) {
    let pending = DIAL_GROUP.swap(group as i32, std::sync::atomic::Ordering::SeqCst);
    if pending < 0 {
      // SAFETY: as in the handler, whose `SA_RESETHAND` already put the
      // default action back for that signal.
      unsafe {
        libc::kill(-(group as libc::pid_t), libc::SIGKILL);
        libc::raise(-pending);
      }
    }
  }
}

#[cfg(unix)]
impl Drop for DialSignals {
  fn drop(&mut self) {
    // Handlers first: cleared before them, the group would let the handler
    // take a signal as pending, and nothing would act on it.
    for (sig, old) in &self.0 {
      // SAFETY: puts back what `install` read.
      unsafe { libc::sigaction(*sig, old, std::ptr::null_mut()) };
    }
    let pending = DIAL_GROUP.swap(0, std::sync::atomic::Ordering::SeqCst);
    if pending < 0 {
      // The spawn failed after a signal came: it meets gwm's own handling.
      // SAFETY: a plain raise of a signal gwm already received.
      unsafe { libc::raise(-pending) };
    }
  }
}

/// `core.autocrlf`, the Git for Windows default, checks a LF blob out as
/// CRLF: the same file.
fn without_crlf(bytes: &[u8]) -> Vec<u8> {
  let mut out = Vec::with_capacity(bytes.len());
  for (i, &b) in bytes.iter().enumerate() {
    if !(b == b'\r' && bytes.get(i + 1) == Some(&b'\n')) {
      out.push(b);
    }
  }
  out
}

/// What the user last saw run under an origin rule, keyed by origin, so the
/// next run can say what changed. Beside the ledger, never in it: a ledger
/// entry would keep the repo trusted after the rule is removed.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SeenConfigs {
  #[serde(default)]
  seen: std::collections::BTreeMap<String, String>,
}

/// The one line `gwm create` prints when an origin rule, not the ledger,
/// cleared `body`: on the first run from `origin`, and whenever the file
/// differs from the last run, naming what changed. `None` when nothing did.
/// Records `body` as seen. Only the CLI calls it, so a change is not marked
/// seen by a caller that has nowhere to show it.
pub fn origin_change_notice(origin: &str, pattern: &str, body: &[u8]) -> Result<Option<String>> {
  let path = default_ledger_path()?.with_file_name("trust-seen.toml");
  // A display cache, not a trust decision: unreadable means start over, and
  // unwritable means the line repeats next run. Either way it is printed.
  let mut seen: SeenConfigs = fs::read_to_string(&path)
    .ok()
    .and_then(|raw| toml::from_str(&raw).ok())
    .unwrap_or_default();
  let now = String::from_utf8_lossy(body).into_owned();
  let notice = match seen.seen.get(origin) {
    Some(before) if *before == now => return Ok(None),
    Some(before) => format!(
      ".gwm.toml changed since last run (trusted by origin {}): {}",
      pattern,
      describe_changes(before, &now)
    ),
    // Nothing to diff against, often a repo the ledger approved until now:
    // name everything the rule lets run.
    None => format!(
      ".gwm.toml trusted by origin {}: first run from {}: {}",
      pattern,
      origin,
      describe_changes("", &now)
    ),
  };
  seen.seen.insert(origin.to_string(), now);
  let _ = write_atomic(&path, &toml::to_string_pretty(&seen)?);
  Ok(Some(notice))
}

/// `bootstrap.command[2] added, hooks.post_create[0] changed`: the settings
/// that differ, one entry per array item rather than per field.
fn describe_changes(before: &str, after: &str) -> String {
  let rows = |s: &str| -> Option<std::collections::BTreeMap<String, String>> {
    let value: toml::Value = toml::from_str(s).ok()?;
    let mut rows = Vec::new();
    crate::config::flatten_value("", &value, &mut rows);
    Some(rows.into_iter().collect())
  };
  let (Some(old), Some(new)) = (rows(before), rows(after)) else {
    return "the file does not parse, no per-setting diff".into();
  };
  let item = |k: &str| k.find(']').map_or(k, |i| &k[..=i]).to_string();
  let has_item = |rows: &std::collections::BTreeMap<String, String>, it: &str| rows.keys().any(|k| item(k) == it);
  let mut changes = std::collections::BTreeMap::new();
  for key in old.keys().chain(new.keys()) {
    if old.get(key) != new.get(key) {
      let it = item(key);
      let what = if !has_item(&old, &it) {
        "added"
      } else if !has_item(&new, &it) {
        "removed"
      } else {
        "changed"
      };
      changes.insert(it, what);
    }
  }
  if changes.is_empty() {
    return "no setting changed, comments or formatting only".into();
  }
  changes
    .iter()
    .map(|(it, what)| format!("{} {}", it, what))
    .collect::<Vec<_>>()
    .join(", ")
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
  ///
  /// `rule` is the `[trust] origins` pattern in the user's config that
  /// covers `origin` (issue #751). It clears the run only once origin, asked
  /// over the network, is seen publishing `body` ([`published_on_origin`]),
  /// which only the CLI does; a caller that cannot dial refuses as for any
  /// other prompt.
  Prompt {
    cfg_path: PathBuf,
    body: Vec<u8>,
    sha: String,
    origin: String,
    rule: Option<String>,
    ledger: TrustLedger,
    ledger_path: PathBuf,
  },
}

/// Is the repo's own `.gwm.toml` approved in the ledger?
///
/// A second, narrower question than [`evaluate`], and it deliberately
/// does **not** reuse it. `evaluate` short-circuits to `Proceed` on an
/// empty bootstrap surface — no commands to run, nothing to gate — and
/// a `.gwm.toml` whose entire content is `forge = "gitlab"` has exactly
/// that shape. It is still a file that ships with the repo telling gwm
/// which host to send an authenticated call to (Codex review #458), so
/// the surface short-circuit is wrong for this question.
///
/// Same ledger, same `(origin, sha256)` key, same `GWM_ALLOW_BOOTSTRAP`
/// escape hatch: approving a repo approves the whole file, and editing
/// the file revokes that approval. Non-interactive by construction —
/// [`crate::forge::resolve`] runs on the TUI's selection path, which
/// cannot host a prompt. `gwm trust add` is how a user answers it. Not the
/// `[trust] origins` rules (issue #751): one clears a file only once origin,
/// asked over the network, publishes it, and that path does not dial.
///
/// No `.gwm.toml` at all is `false`, and the distinction matters.
///
/// This answers "does *the repo's own file* authorise this?", so an
/// absent file is an absent statement, not a blanket yes. It briefly
/// returned `true` on the reasoning that with no repo-controlled file in
/// play the request must have come from the user's own config — which
/// held only while a bare global `forge` key was itself an authority.
/// Once that key stopped authorising hosts, the same `true` became the
/// way around the gate: a repo with no `.gwm.toml` on an unrecognised
/// host inherits `forge` from the global config by merge, reaches this
/// question, and was waved through on the strength of the file it does
/// not have.
pub fn config_is_trusted(workdir: &Path, origin: &str, mode: TrustMode) -> Result<bool> {
  let bytes = match fs::read(workdir.join(CONFIG_FILE)) {
    Ok(b) => b,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
    Err(e) => return Err(e.into()),
  };
  match mode {
    TrustMode::Deny => return Ok(false),
    TrustMode::Allow => return Ok(true),
    TrustMode::Prompt => {}
  }
  let ledger_path = default_ledger_path()?;
  Ok(TrustLedger::load(&ledger_path)?.lookup(origin, &hash_config(&bytes)))
}

/// Record the current repo's `.gwm.toml` as trusted. Backs `gwm trust
/// add`, which exists because the forge gate above is the first thing
/// that can refuse on a config with **no** bootstrap surface — nothing
/// else would ever prompt for it, so without this the refusal would be
/// unclearable.
pub fn record_config(workdir: &Path, origin: &str, actor: &str) -> Result<Option<String>> {
  let path = workdir.join(CONFIG_FILE);
  let bytes = match fs::read(&path) {
    Ok(b) => b,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
    Err(e) => return Err(e.into()),
  };
  let sha = hash_config(&bytes);
  let ledger_path = default_ledger_path()?;
  let mut ledger = TrustLedger::load(&ledger_path)?;
  ledger.record(origin, &sha, actor);
  ledger.save(&ledger_path)?;
  Ok(Some(sha))
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
///      prompt or refuse), naming the `[trust] origins` rule in the user's
///      own config that covers `origin`, if any (issue #751). Only the CLI
///      acts on it, and only after this ledger miss: it asks origin
///      ([`published_on_origin`]), the one network call the gate makes.
///
/// This is the single source of truth shared by `cli::trust_or_prompt`
/// and the TUI gate — keeps the security policy in one place.
pub fn evaluate(workdir: &Path, origin: &str, mode: TrustMode) -> Result<TrustOutcome> {
  let cfg_path = workdir.join(CONFIG_FILE);
  let bytes = match fs::read(&cfg_path) {
    Ok(b) => Some(b),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
    Err(e) => return Err(e.into()),
  };
  evaluate_bytes(&cfg_path, bytes.as_deref(), origin, mode)
}

/// [`evaluate`] on bytes the caller already read (issue #531).
///
/// The decision has to be about the very bytes that get merged into the
/// config whose hooks then run. Re-opening `.gwm.toml` here made the gate and
/// the execution two observations of a file another process can rewrite in
/// between, so a config could be approved as harmless and executed as
/// something else. `None` is "no `.gwm.toml`", which is nothing to execute.
///
/// `cfg_path` is carried only to name the file in the prompt and the refusal;
/// nothing reads it.
pub fn evaluate_bytes(cfg_path: &Path, bytes: Option<&[u8]>, origin: &str, mode: TrustMode) -> Result<TrustOutcome> {
  let Some(bytes) = bytes else {
    return Ok(TrustOutcome::Proceed);
  };
  let cfg_path = cfg_path.to_path_buf();
  let sha = hash_config(bytes);

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
  if let Ok(body_str) = std::str::from_utf8(bytes) {
    if let Ok(cfg) = toml::from_str::<Config>(body_str) {
      let bs = &cfg.bootstrap;
      if bs.copy.is_empty()
        && bs.guard.is_empty()
        && bs.no_symlink.is_empty()
        && bs.command.is_empty()
        && !cfg.hooks.has_any()
      {
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
    body: bytes.to_vec(),
    sha,
    origin: origin.to_string(),
    rule: user_origin_rule(origin),
    ledger,
    ledger_path,
  })
}

/// How a caller that cannot prompt tells the user to grant trust. Both name
/// `gwm trust add` first: it is what clears the TUI next time, while a
/// `gwm bootstrap` that an origin rule clears writes no ledger entry.
pub const APPROVE_VIA_BOOTSTRAP: &str = "run `gwm trust add` from a CLI in another terminal to approve, \
   or `gwm bootstrap` there, or relaunch with GWM_ALLOW_BOOTSTRAP=1 / --allow-bootstrap";
pub const APPROVE_VIA_TRUST_ADD: &str = "run `gwm trust add` from a CLI in another terminal to approve, \
   or relaunch with GWM_ALLOW_BOOTSTRAP=1 / --allow-bootstrap";

/// [`evaluate`] for a caller that owns the terminal and so cannot host the
/// stdin prompt: the TUI. `Ok(None)` clears the caller to execute
/// `.gwm.toml` code, `Ok(Some(message))` is a refusal to show the user.
/// A `Prompt` outcome becomes a refusal here — the alternate screen has
/// nowhere to ask — with `approve_hint` naming the command that grants it.
pub fn evaluate_silent(workdir: &Path, origin: &str, mode: TrustMode, approve_hint: &str) -> Result<Option<String>> {
  Ok(refusal_for(evaluate(workdir, origin, mode)?, approve_hint))
}

/// [`evaluate_silent`] on bytes the caller already read — see
/// [`evaluate_bytes`] for why a delete has to gate the snapshot it merged
/// rather than whatever the file says by now (issue #531).
pub fn evaluate_silent_bytes(
  cfg_path: &Path,
  bytes: Option<&[u8]>,
  origin: &str,
  mode: TrustMode,
  approve_hint: &str,
) -> Result<Option<String>> {
  Ok(refusal_for(
    evaluate_bytes(cfg_path, bytes, origin, mode)?,
    approve_hint,
  ))
}

/// Turn an outcome into what a caller that cannot prompt shows the user.
fn refusal_for(outcome: TrustOutcome, approve_hint: &str) -> Option<String> {
  match outcome {
    TrustOutcome::Proceed => None,
    TrustOutcome::Refuse { message } => Some(message),
    // An origin rule's `rule` too: clearing it means dialing origin.
    TrustOutcome::Prompt { cfg_path, sha, .. } => {
      let short_sha: String = sha.chars().take(12).collect();
      Some(format!(
        ".gwm.toml at {} not in trust ledger (hash {}): {}",
        cfg_path.display(),
        short_sha,
        approve_hint
      ))
    }
  }
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
    write_atomic(path, &toml::to_string_pretty(self)?)
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

/// The write behind [`TrustLedger::save`], shared with the origin-rule
/// snapshots in [`origin_change_notice`].
fn write_atomic(path: &Path, body: &str) -> Result<()> {
  let parent = match path.parent() {
    Some(p) if !p.as_os_str().is_empty() => {
      fs::create_dir_all(p)?;
      p.to_path_buf()
    }
    // No parent (e.g. relative `trust.toml` in CWD) or empty
    // parent → write the tmp file in `.`.
    _ => PathBuf::from("."),
  };
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
    GwmError::Other("could not resolve user config directory: set GWM_TRUST_LEDGER to override".into())
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
