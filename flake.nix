{
  description = "git worktree manager — TUI + CLI, native libgit2, per-repo bootstrap";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      # Bumped in lockstep with `Cargo.toml` `version` at release time.
      version = "0.3.0-rc.3";
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        gwm = pkgs.rustPlatform.buildRustPackage {
          pname = "gwm";
          inherit version;

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          # `git2 = { features = ["vendored-libgit2"] }` builds libgit2 from
          # source, so we only need a C toolchain and cmake — no system
          # libgit2 dep, no pkg-config plumbing for openssl/zlib.
          nativeBuildInputs = with pkgs; [
            cmake
            perl
          ];

          # The integration tests in `tests/worktree_integration.rs` shell
          # out to `git` via libgit2 and `tests/cli_binary.rs` exercises the
          # built binary directly — both work in the sandbox. They also
          # exercise `tempfile`, which needs `/tmp` (provided by Nix build
          # sandbox).
          doCheck = true;

          meta = with pkgs.lib; {
            description = "git worktree manager — TUI + CLI, native libgit2, per-repo bootstrap";
            homepage = "https://github.com/kbrdn1/gwm-cli";
            license = licenses.mit;
            mainProgram = "gwm";
            platforms = platforms.unix;
          };
        };
      in
      {
        packages = {
          inherit gwm;
          default = gwm;
        };

        apps = {
          gwm = {
            type = "app";
            program = "${gwm}/bin/gwm";
          };
          default = self.apps.${system}.gwm;
        };

        devShells.default = pkgs.mkShell {
          name = "gwm-dev";

          # Tools contributors need: the Rust toolchain itself, the
          # editor LSP, the formatter / linter enforced by CI, and the
          # C toolchain for the `git2` vendored build.
          packages = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            cargo-watch
            cargo-edit
            cmake
            perl
            git
          ];

          # `git2`'s vendored-libgit2 build expects a CC; nix-shell wires
          # one in automatically, but exporting RUST_BACKTRACE makes the
          # `cargo test` output friendlier on failure.
          shellHook = ''
            export RUST_BACKTRACE=1
            echo "gwm dev shell — $(rustc --version)"
          '';
        };
      })
    // {
      # System-agnostic overlay so users with their own nixpkgs overlay
      # stack can pull `gwm` in cleanly:
      #
      #   nixpkgs.overlays = [ inputs.gwm.overlays.default ];
      #   environment.systemPackages = [ pkgs.gwm ];
      overlays.default = final: _prev: {
        gwm = self.packages.${final.system}.gwm;
      };
    };
}
