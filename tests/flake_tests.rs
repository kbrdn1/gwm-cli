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

// `(name, rhs)` for every `name = rhs` in `code`, a Nix source stripped of
// its `#` comment lines. Each `;` ends a statement and each `=` in it binds
// the last segment of the attribute path just before it, so
// `pname = "gwm"; version = …;`, `pin = { version = …; };` and
// `{ package.version = …; }` all yield a `version`. `rhs` runs to the `;`,
// over as many lines as it takes; a `;` inside a string cuts it short, which
// fails the version guard rather than passing it. A string and a comment
// after code are read as code, so a `version = …` in either is checked as a
// binding, which fails closed too.
//
// Only the `name = rhs` form of a binding is read. `inherit`, a quoted name
// (`"version" = …`) and a comment between a name and its `=` are not, and
// neither is anything else Nix binds without spelling `name =` (#672): this
// is a text guard, and the oracle for Nix is `nix eval`.
fn bindings(code: &str) -> impl Iterator<Item = (&str, &str)> {
  code.split(';').flat_map(|stmt| {
    stmt.match_indices('=').filter_map(move |(i, _)| {
      let (before, rhs) = (&stmt[..i], &stmt[i + 1..]);
      if before.ends_with(['=', '<', '>', '!']) || rhs.starts_with('=') {
        return None;
      }
      let path = before
        .trim_end()
        .rsplit(|c: char| c.is_whitespace() || c == '{' || c == '(')
        .next()?;
      Some((path.rsplit('.').next()?, rhs.trim()))
    })
  })
}

const CARGO_TOML_READ: &str = "builtins.fromTOML (builtins.readFile ./Cargo.toml)";

// Exactly the read, whitespace aside: an expression that merely contains it,
// `recursiveUpdate (<read>) { … }` for one, can override what it returns.
fn is_cargo_toml_read(expr: &str) -> bool {
  let expr = expr.split_whitespace().collect::<Vec<_>>().join(" ");
  expr == CARGO_TOML_READ || expr == format!("({CARGO_TOML_READ})")
}

// Every `version` binding in `s`, each of which must be the `.package.version`
// of the Cargo.toml read: inline,
// `(builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version`, or
// through a name every `name = …` binding of which is that read,
// `cargoToml.package.version`. One hop only:
// `a = cargoToml; version = a.package.version;` fails.
fn version_derives_from_cargo_toml(s: &str) -> Result<(), String> {
  let code: String = s
    .lines()
    .filter(|l| !l.trim_start().starts_with('#'))
    .collect::<Vec<_>>()
    .join("\n");
  let only_reads = |name: &str| {
    let mut rhs = bindings(&code).filter(|(n, _)| *n == name).peekable();
    rhs.peek().is_some() && rhs.all(|(_, rhs)| is_cargo_toml_read(rhs))
  };
  let versions: Vec<&str> = bindings(&code)
    .filter(|(name, _)| *name == "version")
    .map(|(_, rhs)| rhs)
    .collect();
  if versions.is_empty() {
    return Err("no `version = …;` binding found".into());
  }
  match versions.iter().find(|rhs| {
    !rhs
      .strip_suffix(".package.version")
      .is_some_and(|src| is_cargo_toml_read(src) || only_reads(src))
  }) {
    Some(rhs) => Err(format!(
      "`version = {rhs};` is not the `package.version` of a Cargo.toml read"
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
  // Deriving the version makes the drift structurally impossible, so this
  // test pins the mechanism rather than comparing two numbers (which would be
  // tautological once derived). It fails if someone later "simplifies" the
  // expression back into a literal.
  //
  // Issue #648: the first version of this guard asked whether `fromTOML` and
  // `./Cargo.toml` appeared anywhere in the file, and the MSRV read provides
  // both. `pinnedVersion = "0.3.0-rc.3"; version = pinnedVersion;` passed it.
  // So the guard reads the right-hand side of the `version` binding itself.
  // The oracle would be `nix eval .#gwm.version` against Cargo.toml, but no CI
  // runner has nix, and a test that skips when its tool is missing is the
  // vacuous green this fixes; a Nix parser crate for one guard is not worth
  // the dependency.
  let s = read_flake();
  if let Err(why) = version_derives_from_cargo_toml(&s) {
    panic!(
      "flake.nix must derive its version from Cargo.toml: {why}. Write \
       `version = cargoToml.package.version;` with \
       `cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);` so it \
       cannot drift again (#393)"
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
