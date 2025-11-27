# GRASP Audit

A reusable audit and compliance testing tool for GRASP protocol implementations.

## Features

- ✅ **Isolated Testing**: Tests run in parallel with unique audit IDs
- ✅ **Production Audit**: Test live services with minimal impact
- ✅ **Clean Audit Events**: Special tags for easy cleanup (no deletion trails)
- ✅ **Spec-Mirrored Tests**: Test structure matches GRASP protocol exactly
- ✅ **Reusable**: Can test any GRASP implementation (Rust, Go, Python, etc.)

## Quick Start

The fastest way to run GRASP-01 compliance tests:

```bash
# Run the test suite against ngit-relay
cd grasp-audit
nix develop -c bash test-ngit-relay.sh --mode test
```

This automatically:
- ✅ Starts ngit-relay in an isolated Docker container
- ✅ Runs all GRASP-01 compliance tests
- ✅ Cleans up resources when finished

**Currently Passing:** 4/18 tests (14 tests stubbed with "Not implemented yet")

For more options:
```bash
./test-ngit-relay.sh --help
```

## Usage Examples

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

# Audit production server
grasp-audit audit --relay wss://grasp.example.com --mode production --spec all
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

All audit events automatically include special tags for isolation and cleanup:

```json
{
  "tags": [
    ["t", "grasp-audit-test-event"],
    ["t", "audit-ci-a1b2c3d4-e5f6-7890-abcd-ef1234567890"],
    ["t", "audit-cleanup-after-1730822334"]
  ]
}
```

**Tag Format:**

- `["t", "grasp-audit-test-event"]` - Identifies all audit-related events
- `["t", "audit-{run_id}"]` - Unique identifier for each audit run
  - CI mode: `audit-ci-{uuid}`
  - Production mode: `audit-prod-audit-{timestamp}`
- `["t", "audit-cleanup-after-{unix_timestamp}"]` - Cleanup scheduling
  - CI mode: Current time + 3600 seconds (1 hour)
  - Production mode: Current time + 300 seconds (5 minutes)

**Benefits:**

- **Automatic**: Tags added automatically to all events via `AuditEventBuilder`
- **Isolation**: Each test run has unique ID for event filtering
- **Cleanup**: Events marked for cleanup after timestamp (direct database cleanup)
- **No deletion trails**: No NIP-09 deletion events needed
- **Discovery**: Easy to query all audit events via hashtag

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

> **TL;DR:** See the [Quick Start](#quick-start) section for the fastest way to run tests.

### Unit Tests

```bash
# Enter dev environment (NixOS)
nix develop

# Run unit tests (no relay required)
cargo test
```

### Integration Tests

The recommended approach is [`test-ngit-relay.sh`](test-ngit-relay.sh), which handles all relay lifecycle management automatically.

See the [Quick Start](#quick-start) section for common usage patterns.

**Advanced: Manual Relay Setup**

<details>
<summary>Click to expand manual testing instructions</summary>

For advanced use cases where you need direct control over the relay:

```bash
# Start relay on a specific port (example uses 18081)
docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest

# In another terminal, run tests with RELAY_URL
grasp-audit audit --relay ws://localhost:18081 --mode ci

# or run all ignored tests via cargo
RELAY_URL="ws://localhost:18081" cargo test --lib -- --ignored --nocapture

# or run specific test via cargo
RELAY_URL="ws://localhost:18081" cargo test --lib test_grasp01_nostr_relay_against_relay -- --ignored --nocapture
```

</details>

## Test Design Pattern: Fixture-First

To prevent rate-limiting from production relays during testing, we use a **fixture-first** approach that minimizes relay interactions.

### Quick Start for New Tests

1. Create TestContext at test start
2. Get prerequisites via `ctx.get_fixture(FixtureKind::...)`
3. Build test-specific events using fixtures as base
4. Verify outcomes via `send_and_verify_accepted/rejected`

### Pattern Template

```rust
pub async fn test_something(client: &AuditClient) -> TestResult {
    TestResult::new(...)
        .run(|| async {
            // 1. Context
            let ctx = TestContext::new(client);
            
            // 2. Prerequisites (cached per-TestContext)
            let repo = ctx.get_fixture(FixtureKind::ValidRepo).await?;
            
            // 3. Test-specific event
            let my_event = client.create_issue(&repo, "Title", "Content", vec![])?;
            
            // 4. Verify
            send_and_verify_accepted(client, my_event, "description").await?;
            
            Ok(())
        })
        .await
}
```

### Three-Layer Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Layer 3: Test Functions                       │
│  Create TestContext, get fixtures, build scenarios, verify       │
├─────────────────────────────────────────────────────────────────┤
│           Layer 2: FixtureKind + TestContext                     │
│  ValidRepo, RepoState, MaintainerState, etc.                     │
│  Mode-aware caching within TestContext                           │
├─────────────────────────────────────────────────────────────────┤
│               Layer 1: AuditClient                               │
│  event_builder, create_repo_announcement, send_event             │
└─────────────────────────────────────────────────────────────────┘
```

### Available Fixtures

| FixtureKind | Provides | Use When |
|-------------|----------|----------|
| `ValidRepo` | Accepted repo announcement (kind 30617) | Need a repo as prerequisite |
| `RepoState` | Repo + state event (kind 30618) | Testing owner push authorization |
| `MaintainerAnnouncement` | Maintainer's repo announcement | Testing maintainer chain setup |
| `MaintainerState` | Maintainer's state event | Testing maintainer push authorization |
| `RepoWithIssue` | Repo + accepted issue (kind 1621) | Testing issue-dependent events |
| `RepoWithComment` | Repo + issue + comment | Testing comment-dependent events |

### Fixture Lifecycle: Generate → Send → Verify

Every fixture follows a 3-step lifecycle:

1. **GENERATE**: Build event via `AuditClient.event_builder()` (in memory only)
2. **SEND**: `client.send_event(event)` transmits to relay (rate-limited operation)
3. **VERIFY**: Query relay to confirm acceptance/rejection

Caching happens after SEND succeeds - same fixture request returns cached Event.

### How TestContext Correlates Events

Each TestContext shares a `run_id` with all events:

```rust
// All events in a TestContext get these tags automatically:
["t", "grasp-audit-test-event"]     // Identifies test events
["t", "audit-{run_id}"]             // Unique ID for this run
["t", "audit-cleanup-after-{ts}"]   // Cleanup timestamp
```

This enables:
- Event correlation within a test run
- Production relay cleanup scripts
- Test isolation between runs

### When NOT to Use Fixtures

Use direct event building (NOT fixtures) when:

- **Testing event REJECTION** - Build invalid events directly
- **Testing signature/ID validation** - Need malformed events
- **One-off connectivity tests** - No prerequisites needed

```rust
// Example: Testing rejection (build invalid event directly)
let invalid_event = client.event_builder(Kind::GitRepoAnnouncement, "")
    .tag(Tag::identifier("test"))
    // Missing required 'clone' tag - should be rejected
    .build(client.keys())?;

send_and_verify_rejected(client, invalid_event, "missing clone tag").await?;
```

### Anti-Patterns to Avoid

❌ **Creating TestContext inside helper functions** - Tests lose cache control

❌ **Monolithic setup functions** - Mix fixture retrieval with git operations

❌ **Direct event creation when fixture exists** - Misses caching opportunity

✅ **Each test creates own TestContext** - Isolation guaranteed

✅ **Use fixtures for prerequisites** - Caching minimizes relay calls

✅ **Build invalid events directly** - Only for rejection tests

## Architecture

```
grasp-audit/
├── src/
│   ├── lib.rs              # Public API
│   ├── audit.rs            # Audit config and event tagging
│   ├── client.rs           # Audit client
│   ├── fixtures.rs         # TestContext and FixtureKind
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
