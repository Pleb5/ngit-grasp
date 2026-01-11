{
  description = "ngit-grasp - A GRASP implementation in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    (flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [ rustToolchain pkg-config openssl git ];

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "🚀 ngit-grasp development environment"
            echo "Rust version: $(rustc --version)"
            echo ""
            echo "Quick commands:"
            echo "  cargo build          - Build the project"
            echo "  cargo test           - Run unit tests"
            echo "  cargo run            - Run the relay"
            echo ""
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "ngit-grasp";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "nostr-0.44.1" =
                "sha256-DwcWmwxNUQRR32E3hqbm7PNkGdK8LB3sGtH1Zfrkigk=";
            };
          };

          nativeBuildInputs = with pkgs; [ pkg-config ];

          buildInputs = with pkgs; [ openssl ];

          # Skip tests that require git in PATH (sandboxing issue)
          # These tests run fine in dev environment and CI
          checkFlags = [
            # Unit tests that spawn git subprocesses
            "--skip=git::subprocess::tests::"
            "--skip=git::tests::"
            "--skip=purgatory::helpers::tests::"
            # Integration tests that create git repos
            "--skip=common::git_server::"
            "--skip=common::purgatory_helpers::"
          ];
        };
      })) // {
        # NixOS module for deployment
        nixosModules.default = import ./nix/module.nix;
        nixosModules.ngit-grasp = self.nixosModules.default;
      };
}
