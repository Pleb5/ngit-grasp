# How to Test GRASP Compliance

**Purpose:** Guide for running compliance tests against ngit-grasp relay  
**Audience:** Developers, contributors, CI/CD maintainers  
**Category:** How-To (task-oriented)

---

## Overview

This guide shows you how to run GRASP protocol compliance tests for the ngit-grasp relay. We have two test suites:

1. **Integration Tests** - Built into ngit-grasp, test core functionality
2. **GRASP Audit Tool** - Standalone compliance checker for any GRASP relay

---

## Quick Start

```bash
# Run all integration tests (automatic relay management)
nix develop -c cargo test --test nip01_compliance --test nip34_announcements

# Run NIP-01 compliance tests only
nix develop -c cargo test --test nip01_compliance

# Run NIP-34 announcement tests only
nix develop -c cargo test --test nip34_announcements

# Run with detailed output
nix develop -c cargo test --test nip01_compliance -- --nocapture
```

**No manual setup needed!** Tests automatically start and stop relay instances.

---

## Integration Tests

### What They Test

**NIP-01 Compliance (`tests/nip01_compliance.rs`)**
- Basic WebSocket connectivity
- Event publishing and subscription
- REQ/EVENT/CLOSE message handling
- Filter-based event queries
- Relay connection lifecycle

**NIP-34 Announcements (`tests/nip34_announcements.rs`)**
- Repository announcement acceptance (kind 30617)
- Repository state event acceptance (kind 30618)
- Clone URL validation
- Relay URL validation
- Domain matching
- Multi-branch state events
- Event queries by kind and tags

### Test Architecture

All integration tests use the **TestRelay fixture pattern**:

```rust
use crate::common::relay::TestRelay;

#[tokio::test]
async fn test_something() {
    // Automatic relay startup on random port
    let relay = TestRelay::start().await;
    
    // Test code here
    // ...
    
    // Automatic cleanup when relay drops
}
```

**Benefits:**
- ✅ Automatic relay lifecycle management
- ✅ Random port allocation (no conflicts)
- ✅ Isolated test environments
- ✅ Automatic cleanup on test completion
- ✅ No manual relay management needed

### Running Specific Tests

```bash
# Run a specific test by name
nix develop -c cargo test --test nip01_compliance test_nip01_smoke

# List all tests without running
nix develop -c cargo test --test nip34_announcements -- --list

# Run tests matching a pattern
nix develop -c cargo test --test nip34_announcements test_accepts
```

### Test Output

```bash
$ nix develop -c cargo test --test nip01_compliance

running 6 tests
test test_nip01_smoke ... ok
test test_subscription ... ok
test test_event_publishing ... ok
test test_filter_queries ... ok
test test_connection_lifecycle ... ok
test test_relay_lifecycle ... ignored

test result: ok. 5 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out
```

---

## GRASP Audit Tool

### What It Does

The `grasp-audit` tool is a standalone compliance checker that can test **any** GRASP relay (local or remote).

**Located:** `grasp-audit/` subdirectory (separate Rust project)

### Running the Audit Tool

```bash
# Enter the grasp-audit directory
cd grasp-audit

# Run against local relay
nix develop -c cargo run -- --url ws://127.0.0.1:7000

# Run against remote relay
nix develop -c cargo run -- --url wss://relay.example.com

# Run with verbose output
nix develop -c cargo run -- --url ws://127.0.0.1:7000 --verbose
```

### What It Tests

- NIP-01 basic relay functionality
- NIP-34 repository announcement handling
- GRASP-01 core service requirements
- Domain validation
- Event acceptance/rejection rules

### Example Output

```bash
$ cd grasp-audit
$ nix develop -c cargo run -- --url ws://127.0.0.1:7000

GRASP Compliance Audit
======================
Relay: ws://127.0.0.1:7000

✅ NIP-01: Basic Connectivity
✅ NIP-01: Event Publishing
✅ NIP-01: Subscriptions
✅ NIP-34: Repository Announcements
✅ NIP-34: State Events
✅ GRASP-01: Domain Validation

Summary: 6/6 tests passed
Status: COMPLIANT
```

---

## Testing Workflow

### For Development

**1. Quick Validation (after code changes)**
```bash
# Run all integration tests
nix develop -c cargo test --test nip01_compliance --test nip34_announcements
```

**2. Deep Compliance Check (before release)**
```bash
# Start your relay
nix develop -c cargo run

# In another terminal, run audit tool
cd grasp-audit
nix develop -c cargo run -- --url ws://127.0.0.1:8080
```

### For CI/CD

**Recommended CI pipeline:**

```yaml
# .github/workflows/test.yml example
test:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v3
    - uses: cachix/install-nix-action@v22
    - name: Run integration tests
      run: nix develop -c cargo test --test nip01_compliance --test nip34_announcements
```

**Why this works:**
- No external relay needed
- Tests manage their own relay instances
- Fast parallel execution
- Clean isolation

---

## Test Configuration

### Environment Variables

Tests use these environment variables (set automatically by TestRelay):

- `NGIT_DOMAIN` - Domain for clone URL validation (auto-set to bind address)
- `NGIT_RELAY_DATA_PATH` - Temporary directory for relay data
- `RUST_LOG` - Logging level (optional, for debugging)

**Example: Enable debug logging**
```bash
RUST_LOG=debug nix develop -c cargo test --test nip01_compliance -- --nocapture
```

### Test Data Locations

Integration tests use temporary directories:

```
/tmp/ngit-test-XXXXXX/     # Relay data (auto-cleaned)
  ├── events/              # Nostr events
  └── git/                 # Git repositories (if tested)
```

**Cleanup:** Automatic when test completes (or on failure).

---

## Troubleshooting

### Test Hangs or Times Out

**Problem:** Test hangs waiting for relay to start

**Solution:**
```bash
# Check if port is already in use
lsof -i :7000

# Kill any stray relay processes
pkill -f ngit-grasp

# Re-run test
nix develop -c cargo test --test nip01_compliance
```

### Connection Refused

**Problem:** `Connection refused` error in tests

**Cause:** Relay failed to start (check for port conflicts)

**Solution:**
```bash
# Tests use random ports, but check for system issues
netstat -tuln | grep LISTEN

# Check relay logs
RUST_LOG=debug nix develop -c cargo test --test nip01_compliance -- --nocapture
```

### Tests Pass Locally but Fail in CI

**Problem:** CI environment differences

**Common causes:**
- Network restrictions (WebSocket blocked)
- Insufficient resources (slow startup)
- Missing dependencies

**Solution:**
```bash
# Ensure Nix is installed in CI
# Use longer timeouts for slow systems
# Check CI logs for specific errors
```

### Audit Tool Can't Connect

**Problem:** `grasp-audit` fails to connect to relay

**Checklist:**
1. Is the relay running? (`ps aux | grep ngit-grasp`)
2. Is the URL correct? (ws:// for local, wss:// for remote)
3. Is the port accessible? (`telnet 127.0.0.1 7000`)
4. Check firewall rules

---

## Writing New Tests

### Integration Test Pattern

**1. Create test file in `tests/` directory**

```rust
// tests/my_new_tests.rs
mod common;

use common::relay::TestRelay;
use tokio_tungstenite::connect_async;

#[tokio::test]
async fn test_my_feature() {
    // Start relay
    let relay = TestRelay::start().await;
    
    // Connect
    let (mut ws, _) = connect_async(relay.ws_url())
        .await
        .expect("Failed to connect");
    
    // Test your feature
    // ...
    
    // Cleanup automatic when relay drops
}
```

**2. Run your test**
```bash
nix develop -c cargo test --test my_new_tests
```

### Adding to Audit Tool

**1. Edit `grasp-audit/src/main.rs`**

Add your test function following existing patterns.

**2. Test it**
```bash
cd grasp-audit
nix develop -c cargo run -- --url ws://127.0.0.1:7000
```

---

## Test Coverage

### Current Coverage

**NIP-01 (Nostr Relay):**
- ✅ WebSocket connectivity
- ✅ Event publishing
- ✅ Subscriptions (REQ/EVENT/EOSE/CLOSE)
- ✅ Filter queries
- ✅ Connection lifecycle

**NIP-34 (Git Stuff):**
- ✅ Repository announcements (kind 30617)
- ✅ Repository state events (kind 30618)
- ✅ Clone URL validation
- ✅ Relay URL validation
- ✅ Domain matching
- ✅ Multi-branch support
- ✅ Event queries

**GRASP-01 (Core Service):**
- ✅ Nostr relay at `/`
- ✅ NIP-34 event acceptance
- ✅ Domain validation
- ⏳ Git HTTP backend (planned)
- ⏳ Push authorization (planned)

### Gaps (TODO)

- Git Smart HTTP protocol tests
- Push authorization validation
- Multi-maintainer scenarios
- PR reference handling (`refs/nostr/<event-id>`)
- CORS headers
- NIP-11 relay info document

---

## Performance Testing

### Load Testing (Future)

```bash
# Planned: Load test with multiple concurrent connections
# TODO: Add load testing tools
```

### Benchmarking (Future)

```bash
# Planned: Benchmark event processing throughput
# TODO: Add criterion benchmarks
```

---

## Related Documentation

- **[Test Strategy](../reference/test-strategy.md)** - Overall testing approach
- **[Architecture](../explanation/architecture.md)** - System design
- **[Getting Started](../tutorials/getting-started.md)** - Initial setup
- **[Nix Flakes](./nix-flakes.md)** - Nix development environment

---

## Summary

**For quick validation:**
```bash
nix develop -c cargo test --test nip01_compliance --test nip34_announcements
```

**For deep compliance check:**
```bash
cd grasp-audit
nix develop -c cargo run -- --url ws://127.0.0.1:8080
```

**Key points:**
- ✅ No manual relay management needed
- ✅ Automatic cleanup and isolation
- ✅ Fast parallel execution
- ✅ Works in CI/CD
- ✅ Tests both local and remote relays

---

**Last Updated:** November 4, 2025  
**Status:** ✅ Complete and current
