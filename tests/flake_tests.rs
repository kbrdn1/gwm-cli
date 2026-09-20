use std::fs;
use std::path::PathBuf;

fn flake_path() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("flake.nix")
}

fn read_flake() -> String {
  let path = flake_path();
  fs::read_to_string(&path).unwrap_or_else(|err| {
    panic!(
      "flake.nix must exist at the repo root for Nix users \
       (`nix run`, `nix profile install`, `nix develop`); read error: {err}"
    )
  })
}

// True iff a line of `s` starts with exactly `indent` spaces, the field
// `name`, and an `=`. Used to pin top-level flake fields without false-
// matching on nested `meta.description = ...` (6+ space indent) or on the
// `description` substring inside comments.
fn has_field_at_indent(s: &str, name: &str, indent: usize) -> bool {
  let prefix = format!("{}{} = ", " ".repeat(indent), name);
  s.lines().any(|line| line.starts_with(&prefix))
}

// `s` with its `#` and `/* */` comments turned into a space. A `#` inside a
// `"…"` or `''…''` string is text, so strings are followed, escapes
// included, and so are the `${…}` inside them, which are code again.
fn strip_comments(s: &str) -> String {
  #[derive(Clone, Copy)]
  enum Ctx {
    Code(usize), // `{` depth, to find the `}` that closes an interpolation
    Str,
    Ind,
  }
  let mut stack = vec![Ctx::Code(0)];
  let mut out = String::with_capacity(s.len());
  let mut it = s.chars().peekable();
  while let Some(c) = it.next() {
    let top = stack.len() - 1;
    match (stack[top], c) {
      (Ctx::Code(_), '#') => {
        while it.next_if(|&n| n != '\n').is_some() {}
        out.push(' ');
      }
      (Ctx::Code(_), '/') if it.next_if_eq(&'*').is_some() => {
        let mut prev = ' ';
        for n in it.by_ref() {
          if prev == '*' && n == '/' {
            break;
          }
          prev = n;
        }
        out.push(' ');
      }
      (Ctx::Code(_), '"') => {
        out.push(c);
        stack.push(Ctx::Str);
      }
      (Ctx::Code(_), '\'') if it.next_if_eq(&'\'').is_some() => {
        out.push_str("''");
        stack.push(Ctx::Ind);
      }
      (Ctx::Code(depth), '{') => {
        out.push(c);
        stack[top] = Ctx::Code(depth + 1);
      }
      (Ctx::Code(depth), '}') => {
        out.push(c);
        if depth > 0 {
          stack[top] = Ctx::Code(depth - 1);
        } else if top > 0 {
          stack.pop();
        }
      }
      (Ctx::Str, '\\') => {
        out.push(c);
        out.extend(it.next());
      }
      (Ctx::Str, '"') => {
        out.push(c);
        stack.pop();
      }
      (Ctx::Ind, '\'') if it.next_if_eq(&'\'').is_some() => {
        out.push_str("''");
        // `'''`, `''$` and `''\` are escapes; any other `''` ends the string.
        match it.next_if(|&n| matches!(n, '\'' | '$' | '\\')) {
          Some(n) => out.push(n),
          None => {
            stack.pop();
          }
        }
      }
      (Ctx::Str | Ctx::Ind, '$') if it.next_if_eq(&'{').is_some() => {
        out.push_str("${");
        stack.push(Ctx::Code(0));
      }
      _ => out.push(c),
    }
  }
  out
}

// `(path, rhs)` for every `path = rhs` in `code`, a Nix source without its
// comments, the path split into its segments, quotes dropped. Each `;` ends
// a statement and each `=` in it binds the attribute path just before it, so
// `pname = "gwm"; version = …;`, `pin = { version = …; };` and
// `{ package.version = …; }` are all read. `rhs` runs to the `;`, over as
// many lines as it takes; a `;` inside a string cuts it short, which fails
// the version guard rather than passing it. A string is read as code, so a
// `version = …` written in one is checked as a binding, which fails closed
// too.
//
// `inherit (src) a b;` is sugar for `a = src.a; b = src.b;` and is read as
// those bindings. A plain `inherit a;` binds `a` to the `a` in scope, whose
// own binding is read where it is written.
//
// Nothing traces which binding the derivation actually receives: a dynamic
// name (`${"version"} = …`) is not a path, and a `pin.version = …` merged
// into the derivation's arguments (`pin // { … }`) is a path the version
// guard does not check. This is a text guard; the `flake` job in `ci.yml`
// asks nix (#672).
fn bindings(code: &str) -> impl Iterator<Item = (Vec<&str>, String)> {
  code.split(';').flat_map(|stmt| {
    let assigned = stmt.match_indices('=').filter_map(move |(i, _)| {
      let (before, rhs) = (&stmt[..i], &stmt[i + 1..]);
      if before.ends_with(['=', '<', '>', '!']) || rhs.starts_with('=') {
        return None;
      }
      let path = before
        .trim_end()
        .rsplit(|c: char| c.is_whitespace() || c == '{' || c == '(')
        .next()?;
      Some((
        path.split('.').map(|seg| seg.trim_matches('"')).collect(),
        rhs.trim().to_string(),
      ))
    });
    // The keyword, not the substring: `inheritedPin` and `inherited` are names.
    let inherited = stmt
      .match_indices("inherit")
      .map(|(i, kw)| (&stmt[..i], &stmt[i + kw.len()..]))
      .find(|(head, tail)| {
        (head.is_empty() || head.ends_with(|c: char| c.is_whitespace() || c == '{'))
          && tail.starts_with(|c: char| c.is_whitespace() || c == '(')
      })
      .and_then(|(_, tail)| tail.trim_start().strip_prefix('('))
      .and_then(|tail| {
        let mut depth = 1;
        let end = tail.find(|c| {
          depth += match c {
            '(' => 1,
            ')' => -1,
            _ => 0,
          };
          depth == 0
        })?;
        Some((tail[..end].trim(), &tail[end + 1..]))
      })
      .into_iter()
      .flat_map(|(src, names)| {
        names.split_whitespace().map(move |name| {
          let name = name.trim_matches('"');
          (vec![name], format!("{src}.{name}"))
        })
      });
    assigned.chain(inherited)
  })
}

const CARGO_TOML_READ: &str = "builtins.fromTOML (builtins.readFile ./Cargo.toml)";

// Exactly the read, whitespace aside: an expression that merely contains it,
// `recursiveUpdate (<read>) { … }` for one, can override what it returns.
fn is_cargo_toml_read(expr: &str) -> bool {
  let expr = expr.split_whitespace().collect::<Vec<_>>().join(" ");
  expr == CARGO_TOML_READ || expr == format!("({CARGO_TOML_READ})")
}

// Every version binding in `s`, a path that is `version` or ends in
// `package.version` (so not `passthru.tests.version`), each of which must be
// the `.package.version` of the Cargo.toml read, bare or interpolated into a
// string: inline,
// `(builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version`, or
// through a name every `name… = …` binding of which is that read,
// `cargoToml.package.version`, or `inherit (cargoToml.package) version;`,
// which is the same binding. One hop only:
// `a = cargoToml; version = a.package.version;` fails.
fn version_derives_from_cargo_toml(s: &str) -> Result<(), String> {
  let code = strip_comments(s);
  let only_reads = |name: &str| {
    let mut rhs = bindings(&code).filter(|(path, _)| path[0] == name).peekable();
    rhs.peek().is_some() && rhs.all(|(_, rhs)| is_cargo_toml_read(&rhs))
  };
  let versions: Vec<_> = bindings(&code)
    .filter(|(path, _)| path == &["version"] || path.ends_with(&["package", "version"]))
    .collect();
  if versions.is_empty() {
    return Err("no `version` binding found".into());
  }
  match versions.iter().find(|(_, rhs)| {
    let rhs = rhs
      .strip_prefix("\"${")
      .and_then(|r| r.strip_suffix("}\""))
      .unwrap_or(rhs);
    !rhs
      .trim()
      .strip_suffix(".package.version")
      .is_some_and(|src| is_cargo_toml_read(src) || only_reads(src))
  }) {
    Some((path, rhs)) => Err(format!(
      "`{} = {rhs};` is not the `package.version` of a Cargo.toml read",
      path.join(".")
    )),
    None => Ok(()),
  }
}

#[test]
fn flake_exists_at_repo_root() {
  assert!(flake_path().exists(), "flake.nix must exist at the repo root");
}

#[test]
fn flake_derives_its_version_from_cargo_toml() {
  // Issue #393: the flake advertised `0.3.0-rc.3` while Cargo.toml was at
  // 1.1.1 — eight releases of drift. The root cause was not the number but a
  // comment ("Bumped in lockstep with Cargo.toml at release time") asserting
  // an invariant that nothing enforced. `nix flake check` never caught it:
  // the version is metadata, so the build stayed green and `gwm --version`
  // stayed correct while the store path, `nix flake show` and
  // `nix profile list` all lied.
  //
  // Deriving the version is what keeps the two in step, so this test pins the
  // mechanism: it fails if someone later "simplifies" the expression back into
  // a literal.
  //
  // Issue #648: the first version of this guard asked whether `fromTOML` and
  // `./Cargo.toml` appeared anywhere in the file, and the MSRV read provides
  // both. `pinnedVersion = "0.3.0-rc.3"; version = pinnedVersion;` passed it.
  // So the guard reads the right-hand side of the `version` binding itself.
  // It checks bindings, not what the derivation receives: the oracle for that
  // is `nix eval` against Cargo.toml, which the `flake` job in ci.yml runs
  // (#672, pinned by `ci_evaluates_the_flake_version_against_cargo_toml` in
  // `release_workflow_tests.rs`).
  // This guard stays because it runs wherever `cargo test` does; a test that
  // skipped when nix is missing would be the vacuous green #648 fixed, and a
  // Nix parser crate for one guard is not worth the dependency.
  let s = read_flake();
  if let Err(why) = version_derives_from_cargo_toml(&s) {
    panic!(
      "flake.nix must derive its version from Cargo.toml: {why}. Write \
       `version = cargoToml.package.version;` with \
       `cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);` (#393). \
       Whether the derivation receives that binding is the `flake` CI job's check (#672)"
    );
  }
  // Only the *version* is derived. Cargo.toml's `name` is `gwm-cli` (the bare
  // `gwm` crate name was taken on crates.io) while the binary — and so the
  // package — is `gwm`. `flake_exposes_gwm_package_via_build_rust_package`
  // pins `pname = "gwm"`, which is what stops a well-meaning "derive
  // everything from Cargo.toml" from renaming the package.
}

#[test]
fn the_version_guard_can_actually_fire() {
  let ok = |s: &str| version_derives_from_cargo_toml(s).is_ok();
  let read = "cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);\n\
              msrv = cargoToml.package.rust-version;\n";

  assert!(
    !ok(&format!(
      "{read}pinnedVersion = \"0.3.0-rc.3\";\nversion = pinnedVersion;\n"
    )),
    "a pin one binding away, which the MSRV read hid from the previous guard (#648)"
  );
  assert!(!ok(&format!("{read}version = \"1.10.0\";\n")), "a literal");
  assert!(
    !ok(&format!("{read}version = cargoToml.package.rust-version;\n")),
    "the wrong field of the right read"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\nversion = pinnedVersion;\n"
    )),
    "a second binding, the derivation's own, overriding the derived one"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\npname = \"gwm\"; version = \"0.3.0-rc.3\";\n"
    )),
    "a second binding sharing a line with another one"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\npin = {{ version = \"0.3.0-rc.3\"; }};\n"
    )),
    "a binding nested in an attribute set on the same line"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\npin = {{ package.version = \"0.3.0-rc.3\"; }};\n"
    )),
    "a binding through an attribute path ending in `version`"
  );
  assert!(
    !ok(&format!("{read}version = cargoToml.package.version\n  + \"-rc.3\";\n")),
    "a right-hand side continued on the next line"
  );
  assert!(
    !ok(
      "cargoToml = lib.recursiveUpdate (builtins.fromTOML (builtins.readFile ./Cargo.toml)) \
       (builtins.fromJSON \"{}\");\nversion = cargoToml.package.version;\n"
    ),
    "a read wrapped in something that can override what it returns"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\ncargoToml = builtins.fromJSON \"{{}}\";\n"
    )),
    "the read's name bound a second time, to something else"
  );
  assert!(
    !ok(&format!("{read}# version = cargoToml.package.version;\n")),
    "a commented-out binding is no binding"
  );
  assert!(
    !ok(&format!("{read}/* version = cargoToml.package.version; */\n")),
    "and neither is one in a block comment"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\nattrs = {{ \"version\" = \"0.3.0-rc.3\"; }};\n"
    )),
    "a quoted name"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\nattrs = {{ version # pinned\n  = \"0.3.0-rc.3\"; }};\n"
    )),
    "a comment between a name and its `=`"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\nattrs = {{ version /* pinned */ = \"0.3.0-rc.3\"; }};\n"
    )),
    "a block comment between a name and its `=`"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       url = \"https://example.org/#top\"; pin = {{ version = \"0.3.0-rc.3\"; }};\n"
    )),
    "a `#` inside a string does not start a comment"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       hook = ''echo #''; pin = {{ version = \"0.3.0-rc.3\"; }};\n"
    )),
    "nor does one inside an indented string"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       say = \"a \\\"#\\\" b\"; pin = {{ version = \"0.3.0-rc.3\"; }};\n"
    )),
    "an escaped quote does not end a string"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       hook = ''it'''s #''; pin = {{ version = \"0.3.0-rc.3\"; }};\n"
    )),
    "nor does an escaped `''` end an indented one"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       say = \"${{f \"a#\"}}\"; pin = {{ version = \"0.3.0-rc.3\"; }};\n"
    )),
    "a string inside an interpolation does not end the one around it"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\ncargoToml.package = builtins.fromJSON \"{{}}\";\n"
    )),
    "the read's name extended through an attribute path"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       pin = builtins.fromJSON \"{{\\\"version\\\": \\\"0.3.0-rc.3\\\"}}\";\n\
       gwm = buildRustPackage {{ pname = \"gwm\"; inherit (pin) version; }};\n"
    )),
    "an `inherit (pin) version;` handing the derivation a version no `version =` spells out (#672)"
  );
  assert!(
    !ok(&format!(
      "{read}pins = builtins.fromJSON (builtins.readFile ./pins.json);\n\
       inherit (pins) cargoToml;\nversion = cargoToml.package.version;\n"
    )),
    "the read's name rebound by an `inherit`"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       inheritedPin = builtins.fromJSON \"{{\\\"version\\\": \\\"0.3.0-rc.3\\\"}}\";\n\
       gwm = buildRustPackage {{ pname = \"gwm\"; inherit (inheritedPin) version; }};\n"
    )),
    "an `inherit` whose source contains the word `inherit`"
  );
  assert!(
    !ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       pin = builtins.fromJSON \"{{\\\"version\\\": \\\"0.3.0-rc.3\\\"}}\";\n\
       gwm = buildRustPackage {{ pname = \"gwm\"; inherit (pin) version inherited; }};\n"
    )),
    "an `inherit` one of whose names contains the word `inherit`"
  );

  assert!(
    ok(&format!("{read}version = cargoToml.package.version;\n")),
    "today's flake"
  );
  assert!(
    ok("version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;\n"),
    "the inline spelling #396 shipped"
  );
  assert!(
    ok(
      "cargoToml =\n  builtins.fromTOML\n    (builtins.readFile ./Cargo.toml);\n\
       version = cargoToml.package.version;\n"
    ),
    "the read broken over several lines"
  );
  assert!(
    ok(&format!(
      "{read}version = cargoToml.package.version; # was: version = \"0.3.0-rc.3\"\n"
    )),
    "a comment after the binding, quoting the old one"
  );
  assert!(
    ok(&format!(
      "{read}version = cargoToml.package.version;\n\
       passthru.tests.version = pkgs.testers.testVersion {{ package = gwm; }};\n"
    )),
    "the nixpkgs version smoke test, which is not a version"
  );
  assert!(
    ok(&format!("{read}version = \"${{cargoToml.package.version}}\";\n")),
    "the version interpolated into a string"
  );
  assert!(
    ok(&format!(
      "{read}gwm = buildRustPackage {{ pname = \"gwm\"; inherit (cargoToml.package) version; }};\n"
    )),
    "`inherit (cargoToml.package) version;` with no `version =` binding at all (#672)"
  );
}

/// Issue #675. `flake-utils.lib.eachDefaultSystem` exposed
/// `packages.x86_64-darwin`, which stopped evaluating at all: the pinned
/// nixpkgs is 26.11, which dropped x86_64-darwin and throws on import, so an
/// Intel Mac running `nix profile install github:kbrdn1/gwm-cli` got nixpkgs'
/// refusal in place of gwm. The systems are listed instead, and the list is
/// pinned here because `eachDefaultSystem` is one word away and puts the
/// broken system back without a word: what it defaults to is flake-utils'
/// business, not this repo's.
///
/// That a listed system *evaluates* is not something text can tell. The
/// `flake` job in ci.yml evaluates every system this file exposes, which is
/// what turns a nixpkgs release dropping the next one into a red PR.
#[test]
fn flake_lists_the_systems_it_serves() {
  let s = strip_comments(&read_flake());
  assert!(
    !s.contains("eachDefaultSystem"),
    "flake.nix must not use `eachDefaultSystem`: flake-utils' defaults include \
     x86_64-darwin, which nixpkgs 26.11 dropped (#675), so the flake would advertise \
     a package that cannot be evaluated, let alone built"
  );
  assert!(
    s.contains("eachSystem"),
    "flake.nix must name the systems it serves with `flake-utils.lib.eachSystem [ … ]`"
  );
  for system in ["x86_64-linux", "aarch64-linux", "aarch64-darwin"] {
    assert!(
      s.contains(&format!("\"{system}\"")),
      "flake.nix must still serve {system}: dropping a platform is a release note, not a \
       side effect of editing this list"
    );
  }
  assert!(
    !s.contains("\"x86_64-darwin\""),
    "flake.nix must not list x86_64-darwin while the pinned nixpkgs refuses it (#675). \
     Intel macOS keeps the prebuilt archive, `cargo install` and `cargo binstall`; \
     serving it from the flake again means pinning a nixpkgs that still supports it, \
     and saying so in the same diff"
  );
}

#[test]
fn flake_declares_top_level_description_inputs_outputs() {
  let s = read_flake();
  assert!(
    has_field_at_indent(&s, "description", 2),
    "flake must declare a top-level `description = ...` (2-space indent — \
     distinct from the derivation's nested `meta.description`)"
  );
  assert!(
    has_field_at_indent(&s, "inputs", 2),
    "flake must declare top-level `inputs = {{ ... }}`"
  );
  assert!(
    has_field_at_indent(&s, "outputs", 2),
    "flake must declare top-level `outputs = ...`"
  );
  assert!(s.contains("nixpkgs.url"), "flake must wire a `nixpkgs.url = ...` input");
}

#[test]
fn flake_exposes_gwm_package_via_build_rust_package() {
  let s = read_flake();
  assert!(
    s.contains("buildRustPackage"),
    "flake must build gwm via `rustPlatform.buildRustPackage` (vendored-libgit2 makes this straightforward)"
  );
  assert!(
    s.contains("pname = \"gwm\""),
    "the derivation must set `pname = \"gwm\";`"
  );
  assert!(
    s.contains("cargoLock") || s.contains("cargoHash") || s.contains("cargoSha256"),
    "flake must pin the Cargo lockfile (`cargoLock = {{ lockFile = ./Cargo.lock; }}`)"
  );
  assert!(
    s.contains("default = gwm;"),
    "flake must alias `packages.<system>.default = gwm;` — \
     a specific pattern that does not collide with `apps.default` / `devShells.default`"
  );
}

#[test]
fn flake_exposes_runnable_app() {
  let s = read_flake();
  assert!(
    s.contains("${gwm}/bin/gwm"),
    "the gwm app must wire `program = \"${{gwm}}/bin/gwm\";` so \
     `nix run github:kbrdn1/gwm-cli` resolves to the built binary"
  );
}

#[test]
fn flake_exposes_dev_shell_with_rust_toolchain() {
  let s = read_flake();
  // Read the binding, not one exact spelling of it: the flake may guard the
  // shell before building it (`= assert msrvOk; pkgs.mkShell { … }`, added
  // when the dev shell started verifying the MSRV floor). What the test owns
  // is that the default devShell IS a `pkgs.mkShell`, not what stands between
  // the `=` and it.
  let binding = s.lines().find(|l| l.contains("devShells.default")).unwrap_or_default();
  assert!(
    binding.contains("pkgs.mkShell"),
    "flake must expose `devShells.<system>.default = … pkgs.mkShell {{ ... }}`, got: {binding:?}"
  );
  assert!(
    s.contains("rust-analyzer"),
    "devShell should bundle `rust-analyzer` for editor integration"
  );
  assert!(
    s.contains("clippy"),
    "devShell should bundle `clippy` (project requires `cargo clippy -- -D warnings`)"
  );
  assert!(
    s.contains("rustfmt"),
    "devShell should bundle `rustfmt` (project enforces `cargo fmt --check`)"
  );
}
