# GRASP Audit - Quick Start Guide

## Prerequisites

- Rust 1.75 or later
- C compiler (gcc or clang)
- A Nostr relay for testing (optional for unit tests)

## Setup on NixOS

```bash
# Enter development shell
cd grasp-audit
nix-shell

# Build the project
cargo build

# Run unit tests (no relay needed)
cargo test --lib
```

## Setup on Other Systems

```bash
cd grasp-audit

# Install Rust (if needed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cargo build

# Run unit tests
cargo test --lib
```

## Running Smoke Tests (Requires Relay)

### Option 1: Use a Public Relay

```bash
# Run against a public relay
cargo run --example simple_audit
# Edit the example to use: wss://relay.damus.io or similar
```

### Option 2: Run Local Relay

```bash
# Terminal 1: Start a test relay
# Option A: Using nostr-relay-builder
git clone https://github.com/rust-nostr/nostr
cd nostr/crates/nostr-relay-builder
cargo run --example basic

# Option B: Using docker
docker run -p 7000:7000 scsibug/nostr-rs-relay

# Terminal 2: Run smoke tests
cd grasp-audit
cargo run --example simple_audit
```

### Option 3: Use the CLI

```bash
# Build the CLI
cargo build --release

# Run smoke tests
./target/release/grasp-audit audit \
  --relay ws://localhost:7000 \
  --mode ci \
  --spec nip01-smoke
```

## Running Tests

```bash
# Unit tests only (no relay needed)
cargo test --lib

# Integration tests (needs relay at ws://localhost:7000)
cargo test --ignored

# All tests
cargo test --all

# With output
cargo test -- --nocapture
```

## Using as a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
grasp-audit = { path = "../grasp-audit" }
```

Example code:

```rust
use grasp_audit::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create audit client
    let config = AuditConfig::ci();
    let client = AuditClient::new("ws://localhost:7000", config).await?;
    
    // Run smoke tests
    let results = specs::Nip01SmokeTests::run_all(&client).await;
    
    // Print results
    results.print_report();
    
    // Check if passed
    if !results.all_passed() {
        eprintln!("Some tests failed!");
        std::process::exit(1);
    }
    
    Ok(())
}
```

## Troubleshooting

### Build Errors

**Error:** `linker 'cc' not found`

**Solution (NixOS):**
```bash
nix-shell  # Use the provided shell.nix
```

**Solution (Other Linux):**
```bash
sudo apt-get install build-essential  # Debian/Ubuntu
sudo yum install gcc                   # RedHat/CentOS
```

**Solution (macOS):**
```bash
xcode-select --install
```

### Connection Errors

**Error:** `Failed to connect to relay`

**Solutions:**
1. Make sure a relay is running at the specified URL
2. Check firewall settings
3. Try a different relay URL
4. Use `ws://` for local, `wss://` for remote

### Test Failures

**Error:** Tests fail with timeout

**Solutions:**
1. Increase timeout in test code
2. Check relay is responding (try with `websocat`)
3. Check network connectivity

## Examples

### CI Mode (Isolated Testing)

```bash
# Each run is isolated with unique ID
./target/release/grasp-audit audit \
  --relay ws://localhost:7000 \
  --mode ci \
  --spec nip01-smoke

# Run ID: ci-a1b2c3d4-e5f6-7890-abcd-ef1234567890
# Tests only see events from this run
```

### Production Mode (Audit Live Service)

```bash
# Read-only audit of production relay
./target/release/grasp-audit audit \
  --relay wss://relay.example.com \
  --mode production \
  --spec nip01-smoke

# Run ID: prod-audit-1699027200
# Tests see all events (including real ones)
# Minimal writes (read-only by default)
```

## What's Next?

1. ✅ Run unit tests
2. ✅ Run smoke tests against a relay
3. ✅ Check the report output
4. 🚧 Implement GRASP-01 compliance tests
5. 🚧 Set up CI/CD integration
6. 🚧 Test against ngit-grasp relay

## Resources

- **README.md** - Full documentation
- **SMOKE_TEST_REPORT.md** - Implementation details
- **examples/simple_audit.rs** - Example usage
- **GRASP_AUDIT_PLAN.md** - Original plan

## Support

For issues or questions:
1. Check the documentation in this directory
2. Review the examples
3. Check the test code for usage patterns
4. Open an issue on the project repository
