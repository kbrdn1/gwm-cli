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

#[test]
fn flake_exists_at_repo_root() {
  assert!(
    flake_path().exists(),
    "flake.nix must exist at the repo root"
  );
}

#[test]
fn flake_declares_inputs_outputs_and_description() {
  let s = read_flake();
  assert!(s.contains("description"), "flake must declare a `description`");
  assert!(s.contains("inputs"), "flake must declare `inputs` (nixpkgs at minimum)");
  assert!(s.contains("outputs"), "flake must declare `outputs`");
  assert!(s.contains("nixpkgs"), "flake must reference the `nixpkgs` input");
}

#[test]
fn flake_exposes_gwm_package_via_build_rust_package() {
  let s = read_flake();
  assert!(
    s.contains("buildRustPackage"),
    "flake must build gwm via `rustPlatform.buildRustPackage` (vendored-libgit2 makes this straightforward)"
  );
  assert!(
    s.contains("packages") && s.contains("gwm"),
    "flake must expose `packages.<system>.gwm`"
  );
  assert!(
    s.contains("cargoLock") || s.contains("cargoHash") || s.contains("cargoSha256"),
    "flake must pin the Cargo lockfile (`cargoLock = {{ lockFile = ./Cargo.lock; }}`)"
  );
  assert!(
    s.contains("default"),
    "flake must alias `packages.<system>.default` to gwm"
  );
}

#[test]
fn flake_exposes_runnable_app() {
  let s = read_flake();
  assert!(
    s.contains("apps"),
    "flake must expose `apps.<system>.gwm` so `nix run github:kbrdn1/gwm-cli` works"
  );
}

#[test]
fn flake_exposes_dev_shell_with_rust_toolchain() {
  let s = read_flake();
  assert!(
    s.contains("devShells"),
    "flake must expose `devShells.<system>.default` so contributors get `nix develop`"
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
