# GRASP Audit Tool - Patterns and Learnings

**Purpose:** Document grasp-audit architecture, patterns, and lessons learned  
**Last Updated:** November 4, 2025

---

## Overview

`grasp-audit` is a compliance testing tool for GRASP (Git Relays Authorized via Signed-Nostr Proofs) protocol implementations. It tests both Nostr relay compliance (NIP-01) and GRASP-specific functionality.

---

## Architecture Decisions

### Separate Crate Strategy

**Decision:** Build `grasp-audit` as a separate crate from `ngit-grasp`

**Why:**
1. **Parallel Development**: Can build tests before implementation
2. **Isolated Testing**: Tests run in isolation (CI/CD safe)
3. **Production Auditing**: Can audit live production services
4. **Reusability**: Other GRASP implementations can use it

**Location:** `grasp-audit/` subdirectory with own `Cargo.toml` and `flake.nix`

---

### Audit Event Tagging Strategy

**Problem:** Test events pollute the relay and need cleanup without deletion events.

**Solution:** Use special tags to mark audit events:

```rust
// Every audit event includes these tags
[
    ["t", "grasp-audit-test-event"],           // Marker
    ["t", "audit-{run-id}"],                   // Run isolation
    ["t", "audit-cleanup-after-{timestamp}"]   // Cleanup time
]
```

**Benefits:**
- ✅ **Queryable**: Can find all audit events via tag filter
- ✅ **Isolated**: Each test run has unique run ID
- ✅ **Self-cleaning**: Cleanup timestamp indicates when to delete
- ✅ **No deletion events**: Direct database cleanup, no KIND 5 events
- ✅ **Production safe**: Won't interfere with real events

**Reference:** See `docs/archive/2025-11-04-tag-migration.md`

---

### Standard "t" Tags vs Custom Tags

**Evolution:**
1. **Original**: Custom single-letter tags (`g`, `r`, `c`)
2. **Current**: Standard NIP-01 "t" tags with prefixed values

**Why we changed:**
- ❌ Custom tags could conflict with other systems
- ✅ "t" tag is standard for categorization/topics
- ✅ Multiple "t" tags are expected and supported
- ✅ Self-documenting values (`audit-{run-id}` vs just `{run-id}`)
- ✅ Better namespacing with prefixes

**Migration:** Completed November 4, 2025

---

## Code Patterns

### Audit Configuration

```rust
use grasp_audit::audit::AuditConfig;

// CI mode - isolated test runs
let config = AuditConfig::ci();
// Generates UUID run ID: "ci-{uuid}"
// Cleanup after 1 hour

// Production mode - persistent run ID
let config = AuditConfig::production("prod-server-1");
// Uses provided run ID
// Cleanup after 24 hours
```

**When to use:**
- **CI mode**: Automated testing, parallel runs, temporary
- **Production mode**: Manual audits, monitoring, persistent

---

### Creating Audit Events

```rust
use grasp_audit::audit::{AuditConfig, AuditEventBuilder};
use nostr_sdk::prelude::*;

let config = AuditConfig::ci();
let keys = Keys::generate();

// Create audit event
let event = AuditEventBuilder::new(&config, Kind::TextNote, "test content")
    .build(&keys)?;

// Event automatically includes:
// - Audit marker tag
// - Run ID tag
// - Cleanup timestamp tag
```

---

### Querying Audit Events

```rust
use grasp_audit::client::AuditClient;
use grasp_audit::audit::AuditConfig;

let config = AuditConfig::ci();
let client = AuditClient::new(config, keys);

// Connect to relay
client.add_relay("ws://localhost:7000").await?;
client.connect().await;

// Query audit events for this run
let events = client.query().await?;

// Events are filtered by:
// - "grasp-audit-test-event" marker
// - Current run ID
```

---

### Test Isolation

**Each test run is isolated by unique run ID:**

```rust
// CI mode generates unique UUID per run
let config1 = AuditConfig::ci();
let config2 = AuditConfig::ci();

// config1.run_id != config2.run_id
// Tests won't interfere with each other
```

**Benefits:**
- ✅ Parallel CI/CD runs don't conflict
- ✅ Can run multiple test suites simultaneously
- ✅ Easy to identify which run created which events
- ✅ Cleanup can target specific runs

---

### Cleanup Strategy

**Two-phase cleanup:**

1. **Automatic expiry** via cleanup timestamp tag
2. **Manual cleanup** by querying and deleting

```rust
// Events include cleanup timestamp
["t", "audit-cleanup-after-1730707200"]

// Cleanup process:
// 1. Query events with expired cleanup timestamp
// 2. Delete from database directly (no KIND 5)
// 3. Avoid deletion event pollution
```

**Implementation:** To be built in relay (not in audit tool)

---

## Testing Strategy

### Test Organization

```
grasp-audit/src/specs/
├── nip01_smoke.rs      # NIP-01 basic functionality
├── grasp_01_relay.rs   # GRASP-01 relay requirements (planned)
└── mod.rs              # Test suite registry
```

### Unit vs Integration Tests

**Unit Tests** (no relay required):
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_audit_config() {
        let config = AuditConfig::ci();
        assert!(config.run_id.starts_with("ci-"));
    }
}
```

**Integration Tests** (relay required):
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore]  // Requires relay
    async fn test_smoke_tests_against_relay() {
        // Test against real relay
    }
}
```

**Running tests:**
```bash
# Unit tests (fast, no dependencies)
cargo test --lib

# Integration tests (requires relay)
docker run --rm -p 7000:7000 scsibug/nostr-rs-relay
cargo test -- --ignored
```

---

### Test Result Reporting

```rust
use grasp_audit::result::AuditResult;

// Run tests
let results = vec![
    AuditResult::pass("websocket_connection", "Connected successfully"),
    AuditResult::fail("invalid_event", "Expected rejection, got acceptance"),
];

// Report
for result in &results {
    println!("{}", result);
}

// Summary
let passed = results.iter().filter(|r| r.is_pass()).count();
let total = results.len();
println!("Results: {}/{} passed ({:.1}%)", 
    passed, total, (passed as f64 / total as f64) * 100.0);
```

---

## CLI Design

### Command Structure

```bash
grasp-audit audit [OPTIONS]

Options:
  --relay <URL>        Relay to test (required)
  --mode <MODE>        ci or production (default: ci)
  --run-id <ID>        Custom run ID (production mode only)
  --spec <SPEC>        Test spec to run (default: all)
  --verbose            Detailed output
```

### Usage Examples

```bash
# CI mode - quick smoke test
grasp-audit audit \
  --relay ws://localhost:7000 \
  --mode ci \
  --spec nip01-smoke

# Production mode - full compliance audit
grasp-audit audit \
  --relay wss://relay.example.com \
  --mode production \
  --run-id "audit-2025-11-04" \
  --verbose

# Test all specs
grasp-audit audit --relay ws://localhost:7000
```

---

## Lessons Learned

### 1. Tag Migration is Breaking

**Lesson:** Changing tag structure breaks event queries.

**Impact:** Events created with old tags won't be found by new queries.

**Mitigation:**
- ✅ Accept breaking changes in alpha stage
- ✅ Document migration clearly
- ✅ Old events auto-expire via cleanup
- ✅ No production deployments affected

**Reference:** `docs/archive/2025-11-04-tag-migration.md`

---

### 2. Test Data Lifecycle Matters

**Lesson:** Test events accumulate and pollute relay.

**Solution:** Built-in cleanup strategy from day one.

**Implementation:**
- Every event has cleanup timestamp
- Relay can cleanup expired events
- No deletion event pollution (direct DB cleanup)

---

### 3. Isolation Enables Parallel Testing

**Lesson:** Unique run IDs enable parallel test execution.

**Benefit:** CI/CD can run multiple test suites simultaneously.

**Pattern:**
```rust
// Each CI run gets unique ID
let config = AuditConfig::ci();
// run_id = "ci-{uuid}"

// Tests isolated by run ID
let events = client.query().await?;
// Only returns events for this run
```

---

### 4. Standards Compliance Reduces Friction

**Lesson:** Using standard NIP-01 "t" tags instead of custom tags.

**Benefits:**
- ✅ No conflicts with other systems
- ✅ Standard relay filtering works
- ✅ Better interoperability
- ✅ Self-documenting

---

## Future Enhancements

### Planned Features

- [ ] **GRASP-01 Test Suite**: Repository announcement and state event tests
- [ ] **Test Report Generation**: JSON/HTML output for CI/CD
- [ ] **Performance Benchmarks**: Measure relay performance
- [ ] **Relay Comparison**: Side-by-side compliance comparison
- [ ] **Continuous Monitoring**: Periodic production audits

---

### Possible Improvements

- [ ] **Parallel Test Execution**: Run specs in parallel
- [ ] **Retry Logic**: Handle transient failures
- [ ] **Custom Assertions**: Domain-specific test helpers
- [ ] **Event Diff Tool**: Compare expected vs actual events
- [ ] **Cleanup Automation**: Auto-cleanup after tests

---

## Common Issues

### Issue: Integration Tests Fail

**Symptoms:** Tests timeout or fail to connect

**Causes:**
1. No relay running
2. Wrong relay URL
3. Firewall blocking connection

**Solution:**
```bash
# Start relay
docker run --rm -p 7000:7000 scsibug/nostr-rs-relay

# Verify relay is running
curl http://localhost:7000

# Run tests
cargo test -- --ignored
```

---

### Issue: Events Not Found in Query

**Symptoms:** Query returns empty even though events were sent

**Causes:**
1. Wrong run ID (querying different run)
2. Connection timing (query before event propagated)
3. Tag mismatch (uppercase vs lowercase)

**Solution:**
```rust
// Use same config for send and query
let config = AuditConfig::ci();

// Wait for event to propagate
tokio::time::sleep(Duration::from_millis(500)).await;

// Verify tags match exactly
let t_tag = SingleLetterTag::lowercase(Alphabet::T);  // Lowercase!
```

---

### Issue: Build Fails in CI

**Symptoms:** `cargo build` fails with dependency errors

**Cause:** Not in Nix dev environment

**Solution:**
```bash
# Enter Nix environment first
cd grasp-audit
nix develop

# Then build
cargo build
```

---

## Quick Reference

### Configuration

```rust
// CI mode
let config = AuditConfig::ci();

// Production mode
let config = AuditConfig::production("run-id");
```

### Event Creation

```rust
let event = AuditEventBuilder::new(&config, kind, content)
    .build(&keys)?;
```

### Client Usage

```rust
let client = AuditClient::new(config, keys);
client.add_relay("ws://localhost:7000").await?;
client.connect().await;
let events = client.query().await?;
```

### Running Tests

```bash
# Unit tests
cargo test --lib

# Integration tests
cargo test -- --ignored

# CLI
cargo run -- audit --relay ws://localhost:7000
```

---

## References

- **GRASP Protocol**: https://gitworkshop.dev/danconwaydev.com/grasp
- **NIP-01**: https://github.com/nostr-protocol/nips/blob/master/01.md
- **NIP-34**: https://github.com/nostr-protocol/nips/blob/master/34.md
- **grasp-audit README**: `grasp-audit/README.md`
- **Tag Migration**: `docs/archive/2025-11-04-tag-migration.md`

---

*Last updated: November 4, 2025*  
*Status: Living document - update as grasp-audit evolves*
