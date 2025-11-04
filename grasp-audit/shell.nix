{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust toolchain
    rustc
    cargo
    rustfmt
    clippy
    
    # Build dependencies
    gcc
    pkg-config
    
    # Libraries
    openssl
    
    # Development tools
    git
  ];
  
  # Environment variables
  RUST_BACKTRACE = "1";
  RUST_LOG = "info";
  
  shellHook = ''
    echo "🦀 Rust development environment loaded"
    echo ""
    echo "Available commands:"
    echo "  cargo build          - Build the project"
    echo "  cargo test           - Run unit tests"
    echo "  cargo test --ignored - Run integration tests (needs relay)"
    echo "  cargo run --example simple_audit - Run example"
    echo ""
    echo "Rust version: $(rustc --version)"
    echo "Cargo version: $(cargo --version)"
  '';
}
