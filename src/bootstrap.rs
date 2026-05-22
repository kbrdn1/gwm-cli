use crate::config::{BootstrapConfig, CommandStep, Config, CopyStep, Guard, NoSymlink};
use crate::error::{GwmError, Result};
use regex::Regex;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct BootstrapReport {
  pub steps: Vec<StepResult>,
}

#[derive(Debug, Clone)]
pub struct StepResult {
  pub label: String,
  pub status: StepStatus,
  pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
  Ok,
  Skipped,
  Warning,
  Failed,
}

pub struct BootstrapCtx<'a> {
  pub main_repo: &'a Path,
  pub worktree: &'a Path,
  pub config: &'a Config,
}

pub fn run(ctx: &BootstrapCtx<'_>) -> Result<BootstrapReport> {
  let mut report = BootstrapReport { steps: Vec::new() };
  let bs = &ctx.config.bootstrap;

  // Order matters (issue #93): `run_no_symlinks` strips any declared
  // symlinked targets BEFORE `run_copies` opens them for writing.
  // Reversed, an attacker-planted symlink at a copy destination
  // redirects the `fs::copy` write outside the worktree — a write-
  // anywhere primitive triggered by `gwm bootstrap` alone.
  run_no_symlinks(ctx, bs, &mut report);
  run_copies(ctx, bs, &mut report);
  run_commands(ctx, bs, &mut report);

  Ok(report)
}

fn run_copies(ctx: &BootstrapCtx<'_>, bs: &BootstrapConfig, report: &mut BootstrapReport) {
  for step in &bs.copy {
    let label = format!("copy {} -> {}", step.from, step.to);
    let src = ctx.main_repo.join(&step.from);
    let dst = ctx.worktree.join(&step.to);

    // Runtime defence-in-depth (issue #94): `Config::load_for_repo`
    // rejects `..` / absolute paths in `step.to` at load time, but
    // callers can hand `bootstrap::run` a `Config` value built by
    // hand (test harnesses, future programmatic embeds). Re-check
    // here that `dst` resolves under the worktree before any write.
    if let Err(e) = ensure_within(ctx.worktree, &dst) {
      report.steps.push(StepResult {
        label,
        status: StepStatus::Failed,
        detail: format!("destination outside worktree: {}", e),
      });
      continue;
    }

    // Single stat on `dst` (issue #93): `symlink_metadata` does NOT
    // follow symlinks (unlike `Path::exists`), and reusing one result
    // for every branch below avoids the TOCTOU window of a second stat.
    //
    //   Ok(symlink)     → Failed (defence in depth — symlinks at a
    //                     declared copy dst are suspicious enough to
    //                     surface, even when [[bootstrap.no_symlink]]
    //                     didn't list them)
    //   Ok(other)       → Skipped (regular file or directory already
    //                     populated — leave the user's edits alone)
    //   Err(NotFound)   → fall through to the copy / fallback chain
    //   Err(other)      → Failed (permission / IO error masking the
    //                     filesystem state — never silently swallow)
    match std::fs::symlink_metadata(&dst) {
      Ok(meta) if meta.file_type().is_symlink() => {
        report.steps.push(StepResult {
          label,
          status: StepStatus::Failed,
          detail: format!(
            "refusing to copy: destination {} is a symlink — would redirect the write outside the worktree (issue #93)",
            dst.display()
          ),
        });
        continue;
      }
      Ok(_) => {
        report.steps.push(StepResult {
          label,
          status: StepStatus::Skipped,
          detail: "destination already exists, leaving it alone".into(),
        });
        continue;
      }
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
      Err(e) => {
        report.steps.push(StepResult {
          label,
          status: StepStatus::Failed,
          detail: format!(
            "failed to stat destination {}: {} — refusing to proceed with unknown filesystem state",
            dst.display(),
            e
          ),
        });
        continue;
      }
    }

    if !src.exists() {
      match resolve_missing(step, bs, &dst) {
        Some(res) => report.steps.push(StepResult { label, ..res }),
        None => {
          let status = if step.required {
            StepStatus::Failed
          } else {
            StepStatus::Skipped
          };
          let detail = if step.required {
            "required source missing".into()
          } else {
            "optional source missing".into()
          };
          report.steps.push(StepResult { label, status, detail });
        }
      }
      continue;
    }

    // Run guards before copying.
    if let Some(g) = guard_match(step, bs, &src) {
      handle_guard_match(&g, &src, &dst, ctx, report, &label);
      continue;
    }

    match copy_no_follow(&src, &dst) {
      Ok(()) => report.steps.push(StepResult {
        label,
        status: StepStatus::Ok,
        detail: format!("copied from {}", src.display()),
      }),
      Err(e) => report.steps.push(StepResult {
        label,
        status: StepStatus::Failed,
        detail: format!("copy failed: {}", e),
      }),
    }
  }
}

fn resolve_missing(step: &CopyStep, bs: &BootstrapConfig, dst: &Path) -> Option<StepResult> {
  let mode = step.fallback.as_deref().unwrap_or("skip");
  match mode {
    "inline" => {
      // Find a fallback content keyed by the `to` file basename or step.fallback alias.
      let key = key_from_to(&step.to);
      let fb = bs.fallback.get(&key)?;
      match write_no_follow(dst, fb.content.as_bytes()) {
        Ok(()) => Some(StepResult {
          label: String::new(),
          status: StepStatus::Warning,
          detail: format!("source missing — wrote inline fallback to {}", dst.display()),
        }),
        Err(e) => Some(StepResult {
          label: String::new(),
          status: StepStatus::Failed,
          detail: format!("inline fallback write failed: {}", e),
        }),
      }
    }
    "abort" => Some(StepResult {
      label: String::new(),
      status: StepStatus::Failed,
      detail: "source missing and fallback=abort".into(),
    }),
    _ => None,
  }
}

fn key_from_to(to: &str) -> String {
  // ".env.testing" → "env_testing"
  to.trim_start_matches('.').replace(['.', '-'], "_")
}

fn guard_match(step: &CopyStep, bs: &BootstrapConfig, src: &Path) -> Option<Guard> {
  if step.guards.is_empty() {
    return None;
  }
  let content = std::fs::read_to_string(src).ok()?;
  for guard_name in &step.guards {
    let guard = bs.guard.iter().find(|g| &g.name == guard_name)?;
    for pat in &guard.deny_patterns {
      if let Ok(re) = Regex::new(pat) {
        if re.is_match(&content) {
          return Some(guard.clone());
        }
      }
    }
  }
  None
}

fn handle_guard_match(
  guard: &Guard,
  src: &Path,
  dst: &Path,
  ctx: &BootstrapCtx<'_>,
  report: &mut BootstrapReport,
  label: &str,
) {
  match guard.on_match.as_str() {
    "seed-from-example" => {
      let example_rel = guard.example_file.as_deref().unwrap_or(".env.example");
      let example_src = ctx.main_repo.join(example_rel);
      // Runtime defence-in-depth (issue #94): refuse to read an
      // example_file that resolves outside `ctx.main_repo`. Mirrors
      // the dst-side check in `run_copies`; the `Config` loader
      // rejects this at load time, this branch covers hand-built
      // configs.
      if let Err(e) = ensure_within(ctx.main_repo, &example_src) {
        report.steps.push(StepResult {
          label: label.into(),
          status: StepStatus::Failed,
          detail: format!(
            "guard '{}' example_file outside main repo: {} (traversal rejected, issue #94)",
            guard.name, e
          ),
        });
        return;
      }
      if example_src.exists() {
        match copy_no_follow(&example_src, dst) {
          Ok(_) => report.steps.push(StepResult {
            label: label.into(),
            status: StepStatus::Warning,
            detail: format!(
              "guard '{}' tripped on {} — seeded {} from {} (edit before use)",
              guard.name,
              src.display(),
              dst.display(),
              example_src.display()
            ),
          }),
          Err(e) => report.steps.push(StepResult {
            label: label.into(),
            status: StepStatus::Failed,
            detail: format!("guard '{}' seed-from-example failed: {}", guard.name, e),
          }),
        }
      } else {
        report.steps.push(StepResult {
          label: label.into(),
          status: StepStatus::Failed,
          detail: format!(
            "guard '{}' tripped and no example_file {} available",
            guard.name,
            example_src.display()
          ),
        });
      }
    }
    _ => {
      // abort
      report.steps.push(StepResult {
        label: label.into(),
        status: StepStatus::Failed,
        detail: format!("guard '{}' tripped on {} — abort", guard.name, src.display()),
      });
    }
  }
}

fn run_no_symlinks(ctx: &BootstrapCtx<'_>, bs: &BootstrapConfig, report: &mut BootstrapReport) {
  for ns in &bs.no_symlink {
    let label = format!("no-symlink {}", ns.path);
    let target: PathBuf = ctx.worktree.join(&ns.path);
    handle_no_symlink(&label, &target, report);
  }
  // Also enforce common defaults if not declared explicitly.
  for default in ["vendor", "node_modules"] {
    if bs.no_symlink.iter().any(|n: &NoSymlink| n.path == default) {
      continue;
    }
    let target = ctx.worktree.join(default);
    if target.is_symlink() {
      handle_no_symlink(&format!("no-symlink {} (auto)", default), &target, report);
    }
  }
}

fn handle_no_symlink(label: &str, target: &Path, report: &mut BootstrapReport) {
  if !target.exists() && !target.is_symlink() {
    report.steps.push(StepResult {
      label: label.into(),
      status: StepStatus::Skipped,
      detail: "not present".into(),
    });
    return;
  }
  if target.is_symlink() {
    match std::fs::remove_file(target) {
      Ok(_) => report.steps.push(StepResult {
        label: label.into(),
        status: StepStatus::Warning,
        detail: format!("removed symlink {}", target.display()),
      }),
      Err(e) => report.steps.push(StepResult {
        label: label.into(),
        status: StepStatus::Failed,
        detail: format!("failed to remove symlink {}: {}", target.display(), e),
      }),
    }
  } else {
    report.steps.push(StepResult {
      label: label.into(),
      status: StepStatus::Ok,
      detail: "real directory, ok".to_string(),
    });
  }
}

fn run_commands(ctx: &BootstrapCtx<'_>, bs: &BootstrapConfig, report: &mut BootstrapReport) {
  for step in &bs.command {
    let label = format!("run {}", step.name);
    if let Some(ref guard) = step.when {
      if !evaluate_when(guard, ctx.worktree) {
        report.steps.push(StepResult {
          label,
          status: StepStatus::Skipped,
          detail: format!("when condition '{}' false", guard),
        });
        continue;
      }
    }
    match exec_shell(step, ctx.worktree) {
      Ok(output) => report.steps.push(StepResult {
        label,
        status: StepStatus::Ok,
        detail: trailing_lines(&output, 3),
      }),
      Err(e) => report.steps.push(StepResult {
        label,
        status: StepStatus::Failed,
        detail: e.to_string(),
      }),
    }
  }
}

/// Evaluate a `[[bootstrap.command]].when` expression against the given
/// worktree. Supports the keyword predicates `file_exists:`, `cmd_exists:`,
/// `env_set:`, `env_eq:`, `glob_exists:`, plus the boolean operators `!`,
/// `&&`, `||` with conventional precedence (`!` > `&&` > `||`). Unknown
/// keyword predicates default to `true` so older configs keep running while
/// the doctor surfaces them as warnings.
pub fn evaluate_when(expr: &str, cwd: &Path) -> bool {
  let tokens = tokenize_when(expr);
  let mut parser = WhenParser {
    tokens: &tokens,
    pos: 0,
    cwd,
  };
  parser.parse_or()
}

/// Return every atom string contained in a `when` expression, dropping
/// the boolean operators. Callers (e.g. `doctor::check_when_predicates`)
/// can then validate each atom independently — `w.starts_with(prefix)`
/// on the raw expression misses negated atoms (`!env_set:CI`) and
/// unsupported keywords sitting on the RHS of `&&` / `||`.
pub fn when_atoms(expr: &str) -> Vec<String> {
  tokenize_when(expr)
    .into_iter()
    .filter_map(|t| match t {
      WhenToken::Atom(s) => Some(s),
      _ => None,
    })
    .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum WhenToken {
  Atom(String),
  Not,
  And,
  Or,
}

fn tokenize_when(expr: &str) -> Vec<WhenToken> {
  let bytes = expr.as_bytes();
  let mut tokens = Vec::new();
  let mut i = 0;
  while i < bytes.len() {
    let c = bytes[i];
    if c.is_ascii_whitespace() {
      i += 1;
      continue;
    }
    if c == b'!' {
      tokens.push(WhenToken::Not);
      i += 1;
      continue;
    }
    if c == b'&' && bytes.get(i + 1) == Some(&b'&') {
      tokens.push(WhenToken::And);
      i += 2;
      continue;
    }
    if c == b'|' && bytes.get(i + 1) == Some(&b'|') {
      tokens.push(WhenToken::Or);
      i += 2;
      continue;
    }
    let start = i;
    while i < bytes.len() {
      let b = bytes[i];
      if b.is_ascii_whitespace() {
        break;
      }
      if b == b'&' && bytes.get(i + 1) == Some(&b'&') {
        break;
      }
      if b == b'|' && bytes.get(i + 1) == Some(&b'|') {
        break;
      }
      i += 1;
    }
    tokens.push(WhenToken::Atom(expr[start..i].to_string()));
  }
  tokens
}

struct WhenParser<'a> {
  tokens: &'a [WhenToken],
  pos: usize,
  cwd: &'a Path,
}

impl<'a> WhenParser<'a> {
  fn peek(&self) -> Option<&WhenToken> {
    self.tokens.get(self.pos)
  }

  fn parse_or(&mut self) -> bool {
    let mut acc = self.parse_and();
    while let Some(WhenToken::Or) = self.peek() {
      self.pos += 1;
      let rhs = self.parse_and();
      acc = acc || rhs;
    }
    acc
  }

  fn parse_and(&mut self) -> bool {
    let mut acc = self.parse_not();
    while let Some(WhenToken::And) = self.peek() {
      self.pos += 1;
      let rhs = self.parse_not();
      acc = acc && rhs;
    }
    acc
  }

  fn parse_not(&mut self) -> bool {
    if let Some(WhenToken::Not) = self.peek() {
      self.pos += 1;
      return !self.parse_not();
    }
    self.parse_atom()
  }

  fn parse_atom(&mut self) -> bool {
    match self.tokens.get(self.pos) {
      Some(WhenToken::Atom(s)) => {
        self.pos += 1;
        eval_when_atom(s, self.cwd)
      }
      // Empty expression or a dangling operator: fall back to true to
      // match the "unknown predicate" contract — a config we can't
      // understand should not silently skip every command.
      _ => true,
    }
  }
}

fn eval_when_atom(atom: &str, cwd: &Path) -> bool {
  // Each atom-argument is trimmed to absorb any Unicode whitespace that
  // the ASCII-only tokenizer left glued to the value. Preserves the
  // legacy `file_exists:` tolerance from the pre-tokenizer evaluator.
  if let Some(rest) = atom.strip_prefix("file_exists:") {
    return cwd.join(rest.trim()).exists();
  }
  if let Some(rest) = atom.strip_prefix("cmd_exists:") {
    return which::which(rest.trim()).is_ok();
  }
  if let Some(rest) = atom.strip_prefix("env_set:") {
    return std::env::var(rest.trim()).is_ok();
  }
  if let Some(rest) = atom.strip_prefix("env_eq:") {
    let Some((name, value)) = rest.split_once('=') else {
      return false;
    };
    return std::env::var(name.trim()).ok().as_deref() == Some(value);
  }
  if let Some(pattern) = atom.strip_prefix("glob_exists:") {
    return glob_exists(pattern.trim(), cwd);
  }
  // Unknown keyword: default to true so we don't silently neutralise a
  // command the user clearly wanted to run.
  true
}

fn glob_exists(pattern: &str, cwd: &Path) -> bool {
  let full = cwd.join(pattern);
  let Some(full_str) = full.to_str() else {
    return false;
  };
  match glob::glob(full_str) {
    Ok(mut iter) => iter.any(|r| r.is_ok()),
    Err(_) => false,
  }
}

fn exec_shell(step: &CommandStep, cwd: &Path) -> Result<String> {
  let mut cmd = Command::new("sh");
  cmd.arg("-c").arg(&step.run).current_dir(cwd);
  for (k, v) in &step.env {
    cmd.env(k, v);
  }
  // The `bootstrap step '…'` prefix used to live in the variant's
  // Display impl; it moved into the data string when the variant was
  // generalised in #65 so other subcommands (gwm tmux / gwm zellij)
  // don't inherit a misleading "bootstrap" prefix on their own
  // spawn failures.
  let out = cmd
    .output()
    .map_err(|e| GwmError::CommandFailed(format!("bootstrap step '{}': {}", step.name, e)))?;
  let stdout = String::from_utf8_lossy(&out.stdout).to_string();
  let stderr = String::from_utf8_lossy(&out.stderr).to_string();
  if !out.status.success() {
    return Err(GwmError::CommandFailed(format!(
      "bootstrap step '{}' exited with {}\n{}",
      step.name,
      out.status,
      if stderr.is_empty() { stdout } else { stderr }
    )));
  }
  Ok(if stdout.is_empty() { stderr } else { stdout })
}

fn trailing_lines(s: &str, n: usize) -> String {
  let lines: Vec<&str> = s.lines().collect();
  let start = lines.len().saturating_sub(n);
  lines[start..].join("\n")
}

// --------------------------------------------------------------------------
// TOCTOU-safe write primitives (issue #93 follow-up)
// --------------------------------------------------------------------------
//
// The `symlink_metadata` guard at the top of `run_copies` closes the
// "symlink already exists at copy time" attack vector, but a small
// race window remained between the stat and the subsequent `fs::copy`
// (or `fs::write` in the inline-fallback path): an attacker with
// concurrent write access to the worktree could plant a symlink in
// the µs after the stat, redirecting the write through `O_CREAT |
// O_TRUNC` (both of which follow symlinks). These helpers close that
// window by opening `dst` with `O_NOFOLLOW | O_CREAT | O_EXCL` so
// that:
//
//   - A symlink at `dst` causes `open()` to fail with `ELOOP`.
//   - Any other entry (regular file, dir, FIFO) causes `EEXIST` —
//     `create_new(true)` maps to `O_EXCL` on unix and `CREATE_NEW`
//     on Windows.
//   - On a fresh `dst`, the file is created and truncated atomically
//     under the same fd handed to `write_all`.
//
// On non-unix platforms `O_NOFOLLOW` is unavailable in `std`; the
// `create_new(true)` half still holds, and the bug class flagged on
// #93 is unix-only anyway (Windows symlinks require admin and aren't
// the realistic attack surface for `gwm bootstrap`).

/// Copy the contents of `src` into `dst` as a fresh regular file,
/// refusing to follow any symlink at `dst`. `src` permissions are
/// preserved on unix.
///
/// Returns the standard `io::Result` so callers can format the errno
/// into their step report without losing the error kind. The `dst`
/// is opened with `O_NOFOLLOW | O_CREAT | O_EXCL` on unix; a symlink
/// (broken or live) at `dst` triggers `ELOOP`, anything else
/// pre-existing triggers `EEXIST`.
pub fn copy_no_follow(src: &Path, dst: &Path) -> std::io::Result<()> {
  let mut buf = Vec::new();
  std::fs::File::open(src)?.read_to_end(&mut buf)?;
  #[cfg(unix)]
  let src_perms = std::fs::metadata(src)?.permissions();
  write_no_follow(dst, &buf)?;
  #[cfg(unix)]
  std::fs::set_permissions(dst, src_perms)?;
  Ok(())
}

/// Verify that `path` resolves to a location inside `base` (issue
/// #94). Both `path` and `base` may contain symlinks; both are
/// canonicalized so the check operates on real on-disk identities.
///
/// `path` typically does NOT exist yet (it's a freshly-computed copy
/// destination), so we canonicalize the deepest existing ancestor
/// and check that the canonical ancestor still falls under
/// `base.canonicalize()`. This catches `..` traversal, absolute
/// paths, and symlinks in intermediate components that redirect
/// outside `base`.
///
/// Returns `Err` with `ErrorKind::InvalidInput` when the path
/// escapes `base`; surrounding code surfaces the error verbatim
/// in the step report so the user knows which field went wrong.
fn ensure_within(base: &Path, path: &Path) -> std::io::Result<()> {
  let base_canon = base.canonicalize()?;
  let mut anc: &Path = path;
  let canon_anc = loop {
    if let Ok(c) = anc.canonicalize() {
      break c;
    }
    match anc.parent() {
      Some(p) if !p.as_os_str().is_empty() => anc = p,
      _ => {
        return Err(std::io::Error::new(
          std::io::ErrorKind::InvalidInput,
          format!("cannot resolve any ancestor of {:?}", path),
        ));
      }
    }
  };
  if !canon_anc.starts_with(&base_canon) {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidInput,
      format!(
        "{:?} resolves outside {:?} — '..' traversal or absolute path rejected (issue #94)",
        path, base_canon
      ),
    ));
  }
  Ok(())
}

/// Companion to [`copy_no_follow`] for callers that already hold the
/// payload in memory (e.g. the inline-fallback branch in
/// [`resolve_missing`]). Same TOCTOU-closing semantics: `dst` must
/// not exist and must not be a symlink, or `open()` fails.
pub fn write_no_follow(dst: &Path, bytes: &[u8]) -> std::io::Result<()> {
  let mut opts = std::fs::OpenOptions::new();
  opts.write(true).create_new(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    opts.custom_flags(libc::O_NOFOLLOW);
  }
  let mut f = opts.open(dst)?;
  f.write_all(bytes)?;
  Ok(())
}
