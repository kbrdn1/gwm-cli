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

// True iff a line assigns a *literal* version, e.g. `version = "0.3.0-rc.3";`.
// A derived version (`version = (builtins.fromTOML ...).package.version;`) has
// no opening quote, and an interpolated one does not start with a digit, so
// neither trips this.
fn has_hardcoded_version_literal(s: &str) -> bool {
  s.lines().any(|line| {
    line
      .trim()
      .strip_prefix("version = \"")
      .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
  })
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
  let s = read_flake();
  assert!(
    !has_hardcoded_version_literal(&s),
    "flake.nix must not hardcode a version literal — derive it with \
     `version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;` \
     so it cannot drift from Cargo.toml again (#393)"
  );
  assert!(
    s.contains("fromTOML") && s.contains("./Cargo.toml"),
    "flake.nix must read its version out of Cargo.toml"
  );
  // Only the *version* is derived. Cargo.toml's `name` is `gwm-cli` (the bare
  // `gwm` crate name was taken on crates.io) while the binary — and so the
  // package — is `gwm`. `flake_exposes_gwm_package_via_build_rust_package`
  // pins `pname = "gwm"`, which is what stops a well-meaning "derive
  // everything from Cargo.toml" from renaming the package.
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
  assert!(
    s.contains("devShells.default = pkgs.mkShell"),
    "flake must expose `devShells.<system>.default = pkgs.mkShell {{ ... }}`"
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
