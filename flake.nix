{
  description = "git worktree manager: TUI + CLI, native libgit2, per-repo bootstrap";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      # Read straight out of `Cargo.toml` rather than restated here: a literal
      # silently drifted for eight releases (#393) because the comment claiming
      # it was "bumped in lockstep at release time" was enforced by nothing.
      #
      # `Cargo.toml` names the crate `gwm-cli` (the bare `gwm` name was taken
      # on crates.io) while the binary, and so the package, is `gwm` — see
      # `pname` below.
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      version = cargoToml.package.version;

      # The same lesson, one field over. This flake asks nixpkgs for a bare
      # `rustc`, so the compiler it serves is whatever its pin happens to
      # carry, and that pin moves independently of this repo's floor. It drifted
      # exactly that way: the shell served rustc 1.89 against a declared 1.95,
      # so `nix develop` could not build the project it exists to serve, and
      # nothing noticed because no CI job evaluates this file.
      msrv = cargoToml.package.rust-version;
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # Pinning the toolchain *to* the MSRV was the other option and it is
        # worse: it would hand every contributor the oldest compiler the crate
        # still supports, older than the rustup most of them already run. The
        # floor is a lower bound, not a target. So keep whatever rustc nixpkgs
        # carries and refuse to build the shell when it falls under, which turns
        # a confusing `cargo` refusal deep in a build into a message naming the
        # fix.
        msrvOk =
          assert pkgs.lib.assertMsg
            (builtins.compareVersions pkgs.rustc.version msrv >= 0)
            "flake: nixpkgs ships rustc ${pkgs.rustc.version}, below this repo's declared MSRV ${msrv} (Cargo.toml `rust-version`). Run `nix flake update nixpkgs`.";
          true;

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
            description = "git worktree manager: TUI + CLI, native libgit2, per-repo bootstrap";
            homepage = "https://github.com/kbrdn1/gwm-cli";
            # A list is the nixpkgs idiom for a dual license; `asl20` is the
            # attribute whose `spdxId` is `Apache-2.0`.
            license = [ licenses.asl20 licenses.mit ];
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

        devShells.default = assert msrvOk; pkgs.mkShell {
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
            echo "gwm dev shell: $(rustc --version)"
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
