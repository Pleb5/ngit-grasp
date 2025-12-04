# Test Strategy for ngit-grasp

## Overview

This document describes the testing strategy for ngit-grasp, including the **grasp-audit** reusable compliance testing tool and the **integration tests** in the main repository.

## Testing Philosophy

1. **Specification-Driven**: Tests mirror GRASP-01 protocol structure exactly
2. **Compliance-First**: Every requirement in the spec has a corresponding test
3. **Reusable**: The grasp-audit tool can validate any GRASP implementation
4. **Isolated**: Each test runs with its own relay instance via [`TestRelay`](tests/common/relay.rs:14)
5. **Clear Failures**: Test failures cite exact spec requirements

## Test Pyramid

```
                    ╱╲
                   ╱  ╲
                  ╱ E2E╲              ~ 10%  End-to-end with real Git
                 ╱──────╲
                ╱        ╲
               ╱Compliance╲           ~ 30%  GRASP-01 spec validation
              ╱────────────╲                  (grasp-audit)
             ╱              ╲
            ╱  Integration   ╲        ~ 30%  Component interaction
           ╱──────────────────╲              (tests/)
          ╱                    ╲
         ╱   Unit Tests         ╲     ~ 30%  Individual functions
        ╱────────────────────────╲           (src/**/tests)
```

## Project Structure

### Actual Test Layout

```
ngit-grasp/
├── tests/                          # Integration tests for ngit-grasp
│   ├── common/
│   │   ├── mod.rs                  # Test utilities module
│   │   └── relay.rs                # TestRelay fixture
│   ├── nip01_compliance.rs         # NIP-01 relay compliance
│   ├── nip11_document.rs           # NIP-11 document tests
│   ├── nip34_announcements.rs      # Repository announcement tests
│   ├── repository_creation.rs      # Git repo creation tests
│   ├── push_authorization.rs       # Push validation tests
│   ├── cors.rs                     # CORS header tests
│   └── git_clone.rs                # Git clone tests
│
└── grasp-audit/                    # Reusable GRASP compliance tool
    ├── Cargo.toml
    ├── flake.nix
    └── src/
        ├── lib.rs                  # Public API
        ├── client.rs               # AuditClient
        ├── audit.rs                # AuditConfig, cleanup tags
        ├── fixtures.rs             # Test fixtures
        └── specs/
            └── grasp01/            # GRASP-01 specification tests
                ├── mod.rs          # Module exports
                ├── nip01_smoke.rs  # NIP-01 smoke tests
                ├── nip11_document.rs
                ├── event_acceptance_policy.rs
                ├── cors.rs
                ├── git_clone.rs
                ├── push_authorization.rs
                ├── repository_creation.rs
                └── spec_requirements.rs  # Requirement definitions
```

## Integration Tests (tests/)

### TestRelay Fixture

The [`TestRelay`](tests/common/relay.rs:14) fixture provides automatic relay lifecycle management:

```rust
// From tests/common/relay.rs

/// Test relay fixture that manages relay lifecycle
///
/// Automatically starts and stops the ngit-grasp relay for testing.
/// Uses a random port to avoid conflicts and cleans up created repositories.
pub struct TestRelay {
    process: Child,
    url: String,
    port: u16,
}

impl TestRelay {
    /// Start a test relay instance
    pub async fn start() -> Self { ... }
    
    /// Get the relay WebSocket URL
    pub fn url(&self) -> &str { ... }
    
    /// Get the relay domain (host:port)
    pub fn domain(&self) -> String { ... }
    
    /// Stop the relay
    pub async fn stop(mut self) { ... }
}
```

### Using TestRelay in Integration Tests

From [`tests/nip01_compliance.rs`](tests/nip01_compliance.rs):

```rust
use common::TestRelay;
use grasp_audit::*;

/// Macro to generate isolated integration tests
macro_rules! isolated_test {
    ($test_name:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start().await;
            let config = AuditConfig::isolated();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = specs::Nip01SmokeTests::$test_name(&client).await;

            relay.stop().await;

            assert!(
                result.passed,
                "{} failed: {}",
                stringify!($test_name),
                result.error.as_deref().unwrap_or("unknown error")
            );
        }
    };
}

// Generate isolated tests for all NIP-01 smoke tests
isolated_test!(test_websocket_connection);
isolated_test!(test_send_receive_event);
isolated_test!(test_create_subscription);
```

### Running Integration Tests

```bash
# Run all integration tests
cargo test --test '*'

# Run specific test file
cargo test --test nip01_compliance

# Run with output
cargo test --test nip01_compliance -- --nocapture
```

## GRASP Audit Tool (grasp-audit/)

### Purpose

The grasp-audit tool is a **reusable GRASP compliance testing library** that can:

- Test ngit-grasp for self-validation
- Test any other GRASP implementation (like ngit-relay)
- Run in CI/CD for continuous compliance verification
- Generate compliance reports

### Test Suites

From [`grasp-audit/src/specs/grasp01/mod.rs`](grasp-audit/src/specs/grasp01/mod.rs):

| Suite | Description | Requirements |
|-------|-------------|--------------|
| [`Nip01SmokeTests`](grasp-audit/src/specs/grasp01/nip01_smoke.rs) | Basic NIP-01 relay functionality | WebSocket only |
| [`Nip11DocumentTests`](grasp-audit/src/specs/grasp01/nip11_document.rs) | NIP-11 relay information document | WebSocket only |
| [`EventAcceptancePolicyTests`](grasp-audit/src/specs/grasp01/event_acceptance_policy.rs) | Event acceptance rules | WebSocket only |
| [`CorsTests`](grasp-audit/src/specs/grasp01/cors.rs) | CORS headers on Git HTTP endpoints | git-data-dir |
| [`GitCloneTests`](grasp-audit/src/specs/grasp01/git_clone.rs) | Git clone operations | git-data-dir |
| [`PushAuthorizationTests`](grasp-audit/src/specs/grasp01/push_authorization.rs) | Push authorization | git-data-dir |
| [`RepositoryCreationTests`](grasp-audit/src/specs/grasp01/repository_creation.rs) | Repository creation | git-data-dir |

### Spec Requirements Database

From [`grasp-audit/src/specs/grasp01/spec_requirements.rs`](grasp-audit/src/specs/grasp01/spec_requirements.rs):

```rust
pub struct SpecRequirement {
    pub id: &'static str,           // e.g., "GRASP-01:L9"
    pub section: &'static str,      // e.g., "Nostr Relay"
    pub level: RequirementLevel,    // MUST, SHOULD, MAY
    pub text: &'static str,         // Exact text from spec
    pub line: u32,                  // Line number in spec
}

pub enum RequirementLevel {
    Must,
    Should,
    May,
}
```

### Automatic Cleanup Tags

All audit events include cleanup tags for production safety (from [`grasp-audit/src/audit.rs`](grasp-audit/src/audit.rs)):

```rust
// Automatically added to EVERY audit event:
["t", "grasp-audit-test-event"]              // Marker
["t", "audit-{run_id}"]                      // Run isolation  
["t", "audit-cleanup-after-{unix_timestamp}"] // Cleanup time
```

### Running grasp-audit

**Testing the reference implementation (ngit-relay):**

```bash
# Use test-ngit-relay.sh for automated relay management
cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test

# Or manually:
docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest
cd grasp-audit
RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib -- --ignored --nocapture
```

**Testing ngit-grasp (the main project):**

```bash
# Integration tests use TestRelay fixture - just run:
cargo test --test '*'
```

## Test Patterns

### Isolated Test Pattern

Each test runs with its own fresh relay instance:

```rust
#[tokio::test]
async fn test_something() {
    // Start fresh relay
    let relay = TestRelay::start().await;
    
    // Run test
    let client = AuditClient::new(relay.url(), AuditConfig::isolated()).await?;
    // ... test logic ...
    
    // Cleanup
    relay.stop().await;
}
```

### Macro-Based Test Generation

For test suites that follow the same pattern, use macros:

```rust
macro_rules! isolated_test {
    ($test_name:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start().await;
            // ... standard setup and teardown ...
        }
    };
}

isolated_test!(test_websocket_connection);
isolated_test!(test_send_receive_event);
```

## Coverage Targets

| Test Type | Coverage Target |
|-----------|-----------------|
| Unit Tests | >80% line coverage of `src/` |
| Integration Tests | All critical user paths |
| GRASP-01 Compliance | 100% of MUST requirements |

## CI/CD Integration

### Running All Tests

```bash
# Unit tests (fast, no external dependencies)
cargo test --lib

# Integration tests (requires relay binary built)
cargo build --release
cargo test --test '*'

# Compliance tests against ngit-relay reference
cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test
```

## Summary

| What | Where | Purpose |
|------|-------|---------|
| Unit tests | `src/**/tests` modules | Test individual functions |
| Integration tests | `tests/*.rs` | Test ngit-grasp as a whole |
| TestRelay fixture | [`tests/common/relay.rs`](tests/common/relay.rs) | Manage relay lifecycle |
| GRASP audit library | `grasp-audit/` | Reusable compliance testing |
| GRASP-01 specs | [`grasp-audit/src/specs/grasp01/`](grasp-audit/src/specs/grasp01/) | Spec requirement tests |

## Related Documentation

- [Architecture](../explanation/architecture.md) - System design
- [GRASP-01 Implementation Learnings](../learnings/grasp-01-implementation.md) - Patterns and lessons
- [GRASP Audit Learnings](../learnings/grasp-audit.md) - Audit tool patterns