{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        manifest = pkgs.lib.importTOML ./Cargo.toml;
      in with pkgs; {
        devShells.default = mkShell {
          nativeBuildInputs = [
            # Rust toolchain
            rust-bin.stable.latest.default
            
            # Build tools
            pkg-config
            
            # Development tools
            gitlint
          ];

          buildInputs = [
            # Required dependencies
            openssl
          ];
          
          shellHook = ''
            echo "🦀 GRASP Audit development environment loaded"
            echo ""
            echo "Available commands:"
            echo "  cargo build          - Build the project"
            echo "  cargo test           - Run unit tests"
            echo "  cargo test --ignored - Run integration tests (needs relay)"
            echo "  cargo run --example simple_audit - Run example"
            echo ""
            echo "Rust version: $(rustc --version)"
            echo "Cargo version: $(cargo --version)"
            echo ""
            echo "For RUST_SRC_PATH (rust-analyzer):"
            export RUST_SRC_PATH=${pkgs.rustPlatform.rustLibSrc}
          '';
        };
        
        # Create package for the CLI binary
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.package.name;
          version = manifest.package.version;
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          buildInputs = [
            openssl
          ];
          nativeBuildInputs = [
            pkg-config
          ];
          doCheck = false; # Tests require a running Nostr relay
        };
      });
}
