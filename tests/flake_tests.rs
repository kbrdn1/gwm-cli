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

#[test]
fn flake_exists_at_repo_root() {
  assert!(flake_path().exists(), "flake.nix must exist at the repo root");
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
