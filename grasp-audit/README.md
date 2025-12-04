# GRASP Audit

A reusable audit and compliance testing tool for GRASP protocol implementations.

## Features

- ✅ **Shared Fixtures**: Fixtures cached and reused across tests (default for CLI) to reduce rate-limitting
- ✅ **Isolated Testing**: Fresh fixtures per test for parallel test isolation
- ✅ **Clean Audit Events**: Special tags for easy cleanup (no deletion trails)
- ✅ **Spec-Mirrored Tests**: Test structure matches GRASP protocol exactly
- ✅ **Reusable**: Can test any GRASP implementation (Rust, Go, Python, etc.)

## Quick Start

Run GRASP compliance tests against any GRASP relay:

```bash
# Install
cd grasp-audit
cargo install --path .

# Audit a production relay
grasp-audit audit --relay wss://relay.ngit.dev

# Or audit a local development relay
grasp-audit audit --relay ws://localhost:7000
```

## Usage Examples

### As a CLI Tool

```bash
# Install
cargo install --path .

# Audit a production GRASP relay (shared fixtures - default)
grasp-audit audit --relay wss://relay.ngit.dev

# Audit local development relay
grasp-audit audit --relay ws://localhost:7000 --spec nip01-smoke

# Run with isolated fixtures (for testing/debugging)
grasp-audit audit --relay ws://localhost:7000 --mode isolated --spec push-auth
```

### As a Library

```rust
use grasp_audit::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Create audit client with isolated fixtures (recommended for library use)
    let config = AuditConfig::isolated();
    // let config = AuditConfig::shared();  // Alternative: shared fixtures
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

## Test Specifications

The audit tool provides **good test coverage of GRASP-01** requirements, with additional smoke tests for basic Nostr relay functionality and git over HTTP.

### GRASP-01 Tests

Test coverage of GRASP-01 specification:

- Repository announcement acceptance
- State event handling
- Push authorization (owner, maintainer, recursive maintainer)
- Event acceptance policy
- Git clone over HTTP
- CORS headers
- NIP-11 relay information document

### NIP-01 Smoke Tests (6 tests)

Basic Nostr relay functionality validation:

1. `websocket_connection` - Can connect to /
2. `send_receive_event` - Can send EVENT, get OK
3. `create_subscription` - Can subscribe with REQ
4. `close_subscription` - Can close subscriptions
5. `reject_invalid_signature` - Rejects bad signatures
6. `reject_invalid_event_id` - Rejects wrong IDs

**Why only smoke tests?** rust-nostr already has 1000+ tests for NIP-01 compliance. We focus on GRASP-specific behavior.

### Git over HTTP Smoke Tests

Basic validation that git clone works over HTTP.

## Fixture Modes

The audit tool supports two fixture caching modes that control how test prerequisites are managed. This is a key feature for controlling test isolation and resource efficiency.

### Shared Mode (Default for CLI)

**Default for CLI usage.** Fixtures are cached and reused across all tests for efficiency.

Use this when:

- Auditing production or development relays

```bash
# CLI uses shared mode by default
grasp-audit audit --relay wss://relay.ngit.dev
```

```rust
let config = AuditConfig::shared();
```

### Isolated Mode (Recommended for Library)

**Recommended for library/test usage.** Each test creates fresh fixtures for complete isolation.

Use this when:

- Using grasp-audit as a library
- Running `cargo test` in parallel
- Tests must not interfere with each other
- Debugging test failures

```bash
# Use isolated mode explicitly
grasp-audit audit --relay ws://localhost:7000 --mode isolated
```

```rust
let config = AuditConfig::isolated();
```

### When to Use Each Mode

| Scenario                      | Recommended Mode |
| ----------------------------- | ---------------- |
| CLI auditing production relay | Shared (default) |
| CLI auditing local relay      | Shared (default) |
| Library usage / `cargo test`  | Isolated         |
| CI/CD pipeline                | Isolated         |
| Debugging a single test       | Isolated         |

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
  - Shared mode: `audit-audit-{uuid}`
  - Isolated mode: `audit-isolated-{uuid}`
- `["t", "audit-cleanup-after-{unix_timestamp}"]` - Cleanup scheduling
  - Default: Current time + 3600 seconds (1 hour)

**Benefits:**

- **Automatic**: Tags added automatically to all events via `AuditEventBuilder`
- **Isolation**: Each test run has unique ID for event filtering
- **Cleanup**: Events marked for cleanup after timestamp (direct database cleanup)
- **No deletion trails**: No NIP-09 deletion events needed
- **Discovery**: Easy to query all audit events via hashtag

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
│       ├── nip01_smoke.rs  # NIP-01 smoke tests
│       └── grasp01/        # GRASP-01 compliance tests
└── bin/
    └── grasp-audit.rs      # CLI tool
```

## Roadmap

Planned features and improvements:

### Near-term

- [ ] **Configurable backoffs for rate limiting** - Allow configuring retry delays when relays rate-limit requests
- [ ] **Delete events per pubkey** - Send NIP-09 deletion events grouped by pubkey for better cleanup on relays that support it
- [ ] **Delete event handling** - Respect NIP-09 support flagged in NIP-11 relay information document

### Future

- [ ] **GRASP-05 support** - Add test coverage for GRASP-05 specification

### Out of Scope

- **GRASP-02 (Proactive Sync)** - Testing proactive synchronization behavior is inherently difficult due to its asynchronous nature and reliance on external state. This specification is out of scope for automated compliance testing.

## Development

This section covers patterns and guidelines for contributing new audit tests.

### Test Design Pattern: Fixture-First

To prevent rate-limiting from production relays during testing, we use a **fixture-first** approach that minimizes relay interactions.

#### Quick Start for New Tests

1. Create TestContext at test start
2. Get prerequisites via `ctx.get_fixture(FixtureKind::...)`
3. Build test-specific events using fixtures as base
4. Verify outcomes via `send_and_verify_accepted/rejected`

#### Pattern Template

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

#### Three-Layer Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Layer 3: Test Functions                       │
│  Create TestContext, get fixtures, build scenarios, verify       │
├─────────────────────────────────────────────────────────────────┤
│           Layer 2: FixtureKind + TestContext                     │
│  ValidRepo, RepoState, OwnerStateDataPushed, etc.                │
│  Mode-aware caching within TestContext                           │
├─────────────────────────────────────────────────────────────────┤
│               Layer 1: AuditClient                               │
│  event_builder, create_repo_announcement, send_event             │
└─────────────────────────────────────────────────────────────────┘
```

#### Available Fixtures

| FixtureKind                          | Provides                                                                                                                                         | Use When                                   |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------ |
| `ValidRepo`                          | Accepted repo announcement (kind 30617). Signed by owner keys, lists maintainer in maintainers tag.                                              | Need a repo as prerequisite                |
| `RepoWithIssue`                      | Repo + accepted issue (kind 1621)                                                                                                                | Testing issue-dependent events             |
| `RepoWithComment`                    | Repo + issue + comment (kind 1111)                                                                                                               | Testing comment-dependent events           |
| `RepoState`                          | Repo + state event (kind 30618). Signed by owner, points to `DETERMINISTIC_COMMIT_HASH`.                                                         | Testing owner state events                 |
| `PREvent`                            | Repo + PR event (kind 1618). Signed by PR author, points to `PR_TEST_COMMIT_HASH`.                                                               | Testing PR-dependent events                |
| `PREventGenerated`                   | PR event built but NOT sent to relay.                                                                                                            | Need PR event ID before publishing         |
| `PRWrongCommitPushedBeforeEvent`     | Wrong commit pushed to `refs/nostr/<pr-event-id>` before PR event sent. Returns unsent PR event.                                                 | Testing pre-event ref cleanup              |
| `PREventSentAfterWrongPush`          | PR event sent after wrong commit was pushed. Tests cleanup behavior.                                                                             | Testing post-event ref cleanup             |
| `OwnerStateDataPushed`               | Full owner push flow: state event + git data pushed. Points to `DETERMINISTIC_COMMIT_HASH`.                                                      | Testing owner push authorization           |
| `MaintainerStateDataPushed`          | Full maintainer push flow: force-pushes over owner's data. Points to `MAINTAINER_DETERMINISTIC_COMMIT_HASH`.                                     | Testing maintainer push authorization      |
| `RecursiveMaintainerStateDataPushed` | Full recursive maintainer push flow: Owner → Maintainer → RecursiveMaintainer chain. Points to `RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH`. | Testing recursive maintainer authorization |
| `HeadSetToDevelopBranch`             | State event with HEAD=refs/heads/develop. Depends on RecursiveMaintainerStateDataPushed.                                                         | Testing HEAD branch switching              |

#### Deterministic Commit Hashes

Fixtures use deterministic commit hashes for reproducible testing:

| Constant                                         | Hash                                       | Used By                                          |
| ------------------------------------------------ | ------------------------------------------ | ------------------------------------------------ |
| `DETERMINISTIC_COMMIT_HASH`                      | `64ea71d79a57a7acb334cd9651f8aec067c0ce5d` | Owner fixtures (RepoState, OwnerStateDataPushed) |
| `MAINTAINER_DETERMINISTIC_COMMIT_HASH`           | `1c2d472c9b71ed51968a66500281a3c4a6840464` | MaintainerStateDataPushed                        |
| `RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH` | `05939b82de66fbdb9c077d0a64fc68522f3cb8e0` | RecursiveMaintainerStateDataPushed               |
| `PR_TEST_COMMIT_HASH`                            | `5d40fb1555a0c28bf4d650515a73aaa54d4d9bfb` | PR fixtures (PREvent, PREventGenerated)          |

#### Fixture Dependencies

Fixtures automatically resolve their dependencies:

```
ValidRepo (base)
├── RepoWithIssue → RepoWithComment
├── RepoState
├── PREventGenerated → PRWrongCommitPushedBeforeEvent → PREventSentAfterWrongPush
├── PREvent
└── OwnerStateDataPushed
    └── MaintainerStateDataPushed
        └── RecursiveMaintainerStateDataPushed
            └── HeadSetToDevelopBranch
```

#### Fixture Lifecycle: Generate → Send → Verify → DataPushed

Every fixture follows a lifecycle (some stop earlier):

1. **GENERATE**: Build event via `AuditClient.event_builder()` (in memory only)
2. **SEND**: `client.send_event(event)` transmits to relay (rate-limited operation)
3. **VERIFY**: Query relay to confirm acceptance/rejection
4. **DATA_PUSHED**: (DataPushed variants only) Clone repo, create commit, push to git server

Caching happens after the fixture completes - same fixture request returns cached Event.

**Note:** Some fixtures handle their own event sending (e.g., `OwnerStateDataPushed`, `MaintainerStateDataPushed`). These are marked with `sends_own_events() -> true`.

#### How TestContext Correlates Events

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

#### When NOT to Use Fixtures

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

#### Anti-Patterns to Avoid

❌ **Creating TestContext inside helper functions** - Tests lose cache control

❌ **Monolithic setup functions** - Mix fixture retrieval with git operations

❌ **Direct event creation when fixture exists** - Misses caching opportunity

✅ **Each test creates own TestContext** - Isolation guaranteed

✅ **Use fixtures for prerequisites** - Caching minimizes relay calls

✅ **Build invalid events directly** - Only for rejection tests

## Contributing

This tool is designed to be reusable by any GRASP implementation. Contributions welcome!

## License

MIT
