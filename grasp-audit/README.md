# GRASP Audit

A reusable audit and compliance testing tool for GRASP protocol implementations.

## Features

- ✅ **Isolated Testing**: Tests run in parallel with unique audit IDs
- ✅ **Production Audit**: Test live services with minimal impact
- ✅ **Clean Audit Events**: Special tags for easy cleanup (no deletion trails)
- ✅ **Spec-Mirrored Tests**: Test structure matches GRASP protocol exactly
- ✅ **Reusable**: Can test any GRASP implementation (Rust, Go, Python, etc.)

## Quick Start

### As a Library

```rust
use grasp_audit::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create audit client for CI testing
    let config = AuditConfig::ci();
    let client = AuditClient::new("ws://localhost:7000", config).await?;
    
    // Run NIP-01 smoke tests
    let results = specs::Nip01SmokeTests::run_all(&client).await;
    results.print_report();
    
    if !results.all_passed() {
        std::process::exit(1);
    }
    
    Ok(())
}
```

### As a CLI Tool

```bash
# Install
cargo install --path .

# Run smoke tests against local relay
grasp-audit audit --relay ws://localhost:7000 --mode ci --spec nip01-smoke

# Audit production server (read-only)
grasp-audit audit --relay wss://relay.example.com --mode production --spec all
```

## Test Specifications

### NIP-01 Smoke Tests (6 tests)

Basic Nostr relay functionality:

1. `websocket_connection` - Can connect to /
2. `send_receive_event` - Can send EVENT, get OK
3. `create_subscription` - Can subscribe with REQ
4. `close_subscription` - Can close subscriptions
5. `reject_invalid_signature` - Rejects bad signatures
6. `reject_invalid_event_id` - Rejects wrong IDs

**Why only smoke tests?** rust-nostr already has 1000+ tests for NIP-01 compliance. We focus on GRASP-specific behavior.

### GRASP-01 Tests (Coming Soon)

- Repository announcement acceptance
- State event handling
- Policy enforcement
- And more...

## Audit Event Strategy

All audit events include special tags:

```json
{
  "tags": [
    ["grasp-audit", "true"],
    ["audit-run-id", "ci-a1b2c3d4-e5f6-7890-abcd-ef1234567890"],
    ["audit-cleanup", "2025-11-03T12:00:00Z"]
  ]
}
```

This allows:
- **Isolation**: Each test run has unique ID
- **Cleanup**: Events marked for cleanup after timestamp
- **No deletion trails**: Direct database cleanup (no NIP-09 deletion events)

## Modes

### CI Mode (Default)

- Tests are isolated by unique run ID
- Tests only see their own events
- Full read/write access
- Cleanup after 1 hour

```rust
let config = AuditConfig::ci();
```

### Production Mode

- Tests see all events (including real ones)
- Read-only by default (minimal impact)
- Cleanup after 5 minutes

```rust
let config = AuditConfig::production();
```

## Examples

See `examples/` directory:

```bash
# Simple audit example
cargo run --example simple_audit
```

## Testing

### Unit Tests

```bash
# Enter dev environment (NixOS)
nix develop

# Run unit tests (no relay required)
cargo test
```

### Integration Tests Against ngit-relay

Test against the reference GRASP implementation to ensure compatibility.

**Note:** ngit-relay is a specialized GRASP relay that only accepts Git-related events (NIP-34). 
Some NIP-01 smoke tests (like `send_receive_event`) will fail because ngit-relay rejects 
non-Git events. This is expected behavior - the validation tests should still pass.

```bash
# 1. Create temporary directory with unique name for clean state
TEST_DIR=$(mktemp -d -t grasp-audit-run-XXXXXXXXXX)
mkdir -p "$TEST_DIR"/{repos,blossom,relay-db,logs}

# 2. Pick a random port to avoid conflicts
PORT=$((20000 + RANDOM % 10000))

# 3. Start ngit-relay with fresh data
CONTAINER_NAME="grasp-audit-run-$"
docker run --rm -d \
  --name "$CONTAINER_NAME" \
  -p "$PORT:8081" \
  -e NGIT_DOMAIN=localhost \
  -e NGIT_RELAY_NAME="ngit-relay test instance" \
  -e NGIT_RELAY_DESCRIPTION="Test instance for grasp-audit" \
  -e NGIT_OWNER_NPUB="npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr" \
  -e NGIT_PROACTIVE_SYNC_GIT=false \
  -e NGIT_PROACTIVE_SYNC_BLOSSOM=false \
  -e NGIT_PROACTIVE_SYNC_NOSTR=false \
  -e NGIT_LOG_LEVEL=INFO \
  -v "$TEST_DIR/repos:/srv/ngit-relay/repos" \
  -v "$TEST_DIR/blossom:/srv/ngit-relay/blossom" \
  -v "$TEST_DIR/relay-db:/srv/ngit-relay/relay-db" \
  -v "$TEST_DIR/logs:/var/log/ngit-relay" \
  ghcr.io/danconwaydev/ngit-relay:latest

# 4. Wait for relay to start
sleep 3

# 5. Run tests against ngit-relay
RELAY_URL="ws://localhost:$PORT" cargo test --ignored

# Expected results when testing against ngit-relay:
# - ✓ websocket_connection (basic connectivity)
# - ✗ send_receive_event (ngit-relay rejects non-Git events - EXPECTED FAILURE)
# - ✗ create_subscription (depends on send_receive_event - EXPECTED FAILURE)
# - ✓ close_subscription (basic protocol)
# - ✓ reject_invalid_signature (validation works! - KEY TEST)
# - ✓ reject_invalid_event_id (validation works! - KEY TEST)
#
# Result: 4/6 passed (66.7%)
#
# This is CORRECT behavior! It shows ngit-relay:
# 1. Implements NIP-01 validation correctly (rejects invalid events)
# 2. Has restrictive acceptance policies (only accepts Git events)
# 3. Is a properly functioning GRASP relay
#
# The test will exit with an error, but the validation tests passing
# is what matters for GRASP compliance.

# 6. Stop and cleanup
docker stop "$CONTAINER_NAME"
docker run --rm -v "$TEST_DIR:/data" alpine sh -c "rm -rf /data/*" 2>/dev/null || true
rm -rf "$TEST_DIR"
```

**Why fresh directories?**
- Ensures clean state for each test run
- Prevents test pollution from previous runs
- Matches CI environment behavior

**Environment variables explained:**
- `NGIT_DOMAIN`: Domain name (localhost for testing)
- `NGIT_RELAY_NAME`: Relay name for NIP-11
- `NGIT_RELAY_DESCRIPTION`: Relay description for NIP-11
- `NGIT_OWNER_NPUB`: Relay owner's npub (uses reference impl owner)
- `NGIT_PROACTIVE_SYNC_*`: Disable proactive sync for testing
- `NGIT_LOG_LEVEL`: Set to INFO for debugging

**Port mapping:**
- ngit-relay serves both WebSocket (relay) and HTTP (git) on port 8081
- WebSocket endpoint: `ws://localhost:8081/`
- Git HTTP endpoint: `http://localhost:8081/<npub>/<identifier>.git`

### Testing Against General-Purpose Relays

For full NIP-01 smoke test coverage (all 6 tests passing), test against a general-purpose relay:

```bash
# Start nostr-rs-relay (accepts all event kinds)
docker run --rm -d --name nostr-test-relay -p 7000:8080 scsibug/nostr-rs-relay

# Run tests (all should pass)
cargo test --lib -- --ignored --nocapture

# Cleanup
docker stop nostr-test-relay
```

Expected: 6/6 tests passed (100%)

### Quick Test Script

Save this as `test-ngit-relay.sh`:

```bash
#!/bin/bash
set -e

# Create temporary directory with random name
TEST_DIR=$(mktemp -d -t grasp-audit-run-XXXXXXXXXX)
# Pick a random port in the range 20000-30000
PORT=$((20000 + RANDOM % 10000))

echo "🧹 Using temporary directory: $TEST_DIR"
echo "🔌 Using port: $PORT"

# Cleanup function
cleanup() {
    echo "🛑 Stopping relay..."
    docker stop "grasp-audit-run-$" 2>/dev/null || true
    
    echo "🧹 Cleaning up temporary directory..."
    docker run --rm -v "$TEST_DIR:/data" alpine sh -c "rm -rf /data/*" 2>/dev/null || true
    rm -rf "$TEST_DIR"
}

# Set trap to cleanup on exit
trap cleanup EXIT

echo "📁 Creating data directories..."
mkdir -p "$TEST_DIR"/{repos,blossom,relay-db,logs}

echo "🚀 Starting ngit-relay..."
# Remove any existing container with this name
CONTAINER_NAME="grasp-audit-run-$"
docker rm -f "$CONTAINER_NAME" 2>/dev/null || true
docker run --rm -d \
  --name "$CONTAINER_NAME" \
  -p "$PORT:8081" \
  -e NGIT_DOMAIN=localhost \
  -e NGIT_RELAY_NAME="ngit-relay test instance" \
  -e NGIT_RELAY_DESCRIPTION="Test instance for grasp-audit" \
  -e NGIT_OWNER_NPUB="npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr" \
  -e NGIT_PROACTIVE_SYNC_GIT=false \
  -e NGIT_PROACTIVE_SYNC_BLOSSOM=false \
  -e NGIT_PROACTIVE_SYNC_NOSTR=false \
  -e NGIT_LOG_LEVEL=INFO \
  -v "$TEST_DIR/repos:/srv/ngit-relay/repos" \
  -v "$TEST_DIR/blossom:/srv/ngit-relay/blossom" \
  -v "$TEST_DIR/relay-db:/srv/ngit-relay/relay-db" \
  -v "$TEST_DIR/logs:/var/log/ngit-relay" \
  ghcr.io/danconwaydev/ngit-relay:latest

echo "⏳ Waiting for relay to start..."
sleep 3

echo "🧪 Running tests..."
echo ""
echo "Note: ngit-relay only accepts Git-related events (NIP-34)."
echo "Some NIP-01 smoke tests will fail (expected behavior)."
echo "Validation tests should pass."
echo ""
RELAY_URL="ws://localhost:$PORT" cargo test --lib -- --ignored --nocapture

echo "✅ Done!"
```

Then run:

```bash
chmod +x test-ngit-relay.sh
./test-ngit-relay.sh
```

## Architecture

```
grasp-audit/
├── src/
│   ├── lib.rs              # Public API
│   ├── audit.rs            # Audit config and event tagging
│   ├── client.rs           # Audit client
│   ├── result.rs           # Test result types
│   ├── isolation.rs        # Test isolation utilities
│   └── specs/
│       ├── mod.rs
│       └── nip01_smoke.rs  # NIP-01 smoke tests
├── examples/
│   └── simple_audit.rs     # Example usage
└── bin/
    └── grasp-audit.rs      # CLI tool
```

## Development Status

- ✅ Audit framework
- ✅ NIP-01 smoke tests (6 tests)
- 🚧 GRASP-01 relay tests (planned)
- 🚧 GRASP-01 git tests (planned)
- 🚧 Cleanup utilities (planned)

## Contributing

This tool is designed to be reusable by any GRASP implementation. Contributions welcome!

## License

MIT
