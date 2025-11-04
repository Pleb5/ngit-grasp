# GRASP Compliance Test Tool - Implementation Proposal

## Executive Summary

This document proposes the implementation of a **reusable GRASP compliance testing tool** as a standalone Rust crate. The first phase focuses on testing GRASP-01's requirement: "MUST serve a NIP-01 compliant nostr relay at / that accepts git repository announcements and their corresponding repo state announcements."

## Key Question: How Much NIP-01 Testing Do We Need?

### Analysis

**NIP-01** specifies the basic Nostr protocol including:
1. Event structure and validation (id, pubkey, created_at, kind, tags, content, sig)
2. Event ID calculation (SHA256 of serialized event)
3. Signature verification (Schnorr signatures on secp256k1)
4. WebSocket message types (EVENT, REQ, CLOSE, NOTICE, OK, EOSE, CLOSED, AUTH)
5. Subscription filters
6. Message format and serialization rules

**rust-nostr's `nostr-relay-builder`** already provides:
- ✅ Full NIP-01 event validation
- ✅ WebSocket message handling
- ✅ Signature verification
- ✅ Event ID validation
- ✅ Subscription management
- ✅ Comprehensive test suite for all of the above

### Recommendation: Smoke Tests Only for NIP-01 Core

**We should NOT re-test what rust-nostr already tests extensively.**

Instead, we should focus on:

1. **Smoke Tests** (10-15 tests):
   - WebSocket connection works
   - Can send/receive basic EVENT messages
   - Can create subscriptions with REQ
   - Receive EOSE for subscriptions
   - Basic event validation works (reject invalid events)
   - Can close subscriptions with CLOSE

2. **GRASP-Specific Tests** (majority of effort):
   - Accepts NIP-34 repository announcements (kind 30617)
   - Accepts NIP-34 repository state events (kind 30618)
   - Rejects announcements without required clone/relay tags
   - Accepts events that tag accepted announcements
   - NIP-11 document has GRASP-specific fields
   - Repository creation triggered by announcements
   - State events update repository HEAD

**Rationale:**
- rust-nostr has 1000+ tests for NIP-01 compliance
- We're using their relay builder, not implementing NIP-01 from scratch
- Our value-add is GRASP protocol logic, not Nostr basics
- Testing what's already tested wastes time and creates maintenance burden
- Focus on integration points and GRASP-specific behavior

## Proposed Test Structure

### Phase 1: Exportable Test Tool Foundation

Create `grasp-compliance-tests/` as a standalone crate that can be:
- Used by ngit-grasp
- Published for other GRASP implementations
- Run against any GRASP service (Go, Rust, Python, etc.)

### Directory Structure

```
grasp-compliance-tests/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                    # Public API
│   ├── client.rs                 # HTTP/WebSocket/Git test clients
│   ├── assertions.rs             # Spec-based assertions
│   ├── fixtures.rs               # Event/repo builders
│   └── specs/
│       ├── mod.rs                # Spec registry
│       ├── nip01_smoke.rs        # Minimal NIP-01 smoke tests
│       └── grasp_01.rs           # GRASP-01 compliance tests
├── fixtures/
│   ├── repos/                    # Test git repositories
│   ├── events/                   # Nostr event JSON fixtures
│   └── keys/                     # Test keypairs (deterministic)
└── examples/
    └── test_server.rs            # Example: test any GRASP server
```

## Test Breakdown: GRASP-01 First Requirement

**Requirement:** "MUST serve a NIP-01 compliant nostr relay at / that accepts git repository announcements and their corresponding repo state announcements."

### Proposed Tests (18 total)

#### NIP-01 Smoke Tests (6 tests)

These verify basic Nostr relay functionality:

1. **websocket_connection**
   - Spec: NIP-01 basic requirement
   - Test: Can establish WebSocket connection to `/`
   - Assertion: Upgrade successful, connection stays open

2. **send_receive_event**
   - Spec: NIP-01 EVENT message
   - Test: Send valid EVENT, receive OK response
   - Assertion: OK response with event ID

3. **create_subscription**
   - Spec: NIP-01 REQ message
   - Test: Send REQ with filters, receive EOSE
   - Assertion: EOSE received for subscription ID

4. **close_subscription**
   - Spec: NIP-01 CLOSE message
   - Test: Send CLOSE, verify subscription closed
   - Assertion: No more events for closed subscription

5. **reject_invalid_event**
   - Spec: NIP-01 event validation
   - Test: Send event with invalid signature
   - Assertion: OK response with ok=false

6. **reject_invalid_event_id**
   - Spec: NIP-01 event ID validation
   - Test: Send event with wrong ID
   - Assertion: OK response with ok=false, error message

#### GRASP-01 Specific Tests (12 tests)

These verify GRASP protocol requirements:

7. **accepts_repository_announcement**
   - Spec: GRASP-01:9-10
   - Test: Send NIP-34 kind 30617 with clone/relay tags
   - Assertion: Event accepted (OK with ok=true)

8. **accepts_repository_state**
   - Spec: GRASP-01:9-10
   - Test: Send NIP-34 kind 30618 state event
   - Assertion: Event accepted

9. **rejects_announcement_without_clone_tag**
   - Spec: GRASP-01:12-13
   - Test: Send announcement missing clone tag for this service
   - Assertion: Event rejected with descriptive error

10. **rejects_announcement_without_relay_tag**
    - Spec: GRASP-01:12-13
    - Test: Send announcement missing relay tag for this service
    - Assertion: Event rejected with descriptive error

11. **accepts_announcement_with_multiple_clones**
    - Spec: GRASP-01:12-13 (inverse - should accept if listed)
    - Test: Announcement with multiple clone URLs including ours
    - Assertion: Event accepted

12. **accepts_events_tagging_announcement**
    - Spec: GRASP-01:17-20
    - Test: Send issue (kind 1621) tagging accepted announcement
    - Assertion: Event accepted

13. **accepts_events_tagged_by_announcement**
    - Spec: GRASP-01:17-20
    - Test: Send event that announcement tags
    - Assertion: Event accepted

14. **rejects_events_tagging_rejected_announcement**
    - Spec: GRASP-01:17-20 (inverse)
    - Test: Send issue tagging announcement we rejected
    - Assertion: Event rejected

15. **query_announcements_by_identifier**
    - Spec: GRASP-01 (implied - must be queryable)
    - Test: REQ filter for kind 30617, specific identifier
    - Assertion: Can retrieve accepted announcements

16. **query_state_events**
    - Spec: GRASP-01 (implied - must be queryable)
    - Test: REQ filter for kind 30618
    - Assertion: Can retrieve state events

17. **state_replaces_previous**
    - Spec: NIP-01 replaceable events
    - Test: Send two state events with same d-tag
    - Assertion: Only latest state returned in queries

18. **concurrent_event_submission**
    - Spec: General reliability
    - Test: Send 100 events concurrently
    - Assertion: All valid events accepted, no race conditions

## Can We Reuse rust-nostr Tests?

### Direct Reuse: No

We cannot directly import rust-nostr's test suite because:
1. Their tests are internal to their crates
2. They test library functions, not running servers
3. They don't test GRASP-specific behavior

### Indirect Reuse: Yes

We can learn from their test patterns:

1. **Event Building Patterns**: Use similar builder patterns from `nostr-sdk`
   ```rust
   use nostr_sdk::prelude::*;
   
   let event = EventBuilder::new(Kind::Custom(30617), "", [
       Tag::identifier("my-repo"),
       Tag::custom(TagKind::Custom("clone".into()), vec![domain]),
   ])
   .to_event(&keys)?;
   ```

2. **Assertion Helpers**: Adapt their validation logic
   ```rust
   // They test event.verify() - we test server accepts it
   assert!(event.verify().is_ok()); // Their test
   assert!(server.send_event(event).await?.ok); // Our test
   ```

3. **Test Fixtures**: Use their event generation utilities
   ```rust
   use nostr_sdk::Keys;
   
   // Generate deterministic test keys (same as they do)
   let keys = Keys::from_mnemonic("test seed phrase", None)?;
   ```

### What We Leverage from rust-nostr

Since we're using `nostr-relay-builder`, we get:
- ✅ Event validation (don't need to test)
- ✅ Signature verification (don't need to test)
- ✅ WebSocket handling (smoke test only)
- ✅ Subscription management (smoke test only)

We focus on testing:
- 🎯 GRASP policy enforcement (our code)
- 🎯 Repository announcement acceptance (our code)
- 🎯 Integration between Nostr relay and Git service (our code)

## Implementation Plan

### Step 1: Create Standalone Crate (Week 1)

```bash
# Create the compliance test crate
cargo new --lib grasp-compliance-tests
cd grasp-compliance-tests
```

**Dependencies:**
```toml
[dependencies]
nostr-sdk = "0.43"
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = "0.21"  # WebSocket client
reqwest = { version = "0.11", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "1"

[dev-dependencies]
tokio-test = "0.4"
```

### Step 2: Implement Test Client (Week 1)

```rust
// src/client.rs

pub struct GraspTestClient {
    http_client: reqwest::Client,
    base_url: String,
    ws_url: String,
}

impl GraspTestClient {
    pub fn new(base_url: &str) -> Self { /* ... */ }
    
    pub async fn websocket_connect(&self) -> Result<WebSocketClient> { /* ... */ }
    
    pub async fn send_event(&self, event: Event) -> Result<OkResponse> { /* ... */ }
    
    pub async fn subscribe(&self, filters: Vec<Filter>) -> Result<Subscription> { /* ... */ }
    
    pub async fn fetch_nip11(&self) -> Result<RelayInformationDocument> { /* ... */ }
}
```

### Step 3: Implement NIP-01 Smoke Tests (Week 1)

```rust
// src/specs/nip01_smoke.rs

pub async fn test_nip01_smoke(client: &GraspTestClient) -> ComplianceResult {
    let mut results = ComplianceResult::new("NIP-01 Smoke Tests");
    
    results.add(test_websocket_connection(client).await);
    results.add(test_send_receive_event(client).await);
    results.add(test_create_subscription(client).await);
    results.add(test_close_subscription(client).await);
    results.add(test_reject_invalid_event(client).await);
    results.add(test_reject_invalid_event_id(client).await);
    
    results
}
```

### Step 4: Implement GRASP-01 Tests (Week 2)

```rust
// src/specs/grasp_01.rs

pub async fn test_grasp_01_relay_requirements(
    client: &GraspTestClient
) -> ComplianceResult {
    let mut results = ComplianceResult::new("GRASP-01: Relay Requirements");
    
    results.add(test_accepts_repository_announcement(client).await);
    results.add(test_accepts_repository_state(client).await);
    results.add(test_rejects_announcement_without_clone_tag(client).await);
    // ... etc
    
    results
}
```

### Step 5: Create Fixtures and Builders (Week 2)

```rust
// src/fixtures.rs

pub struct AnnouncementBuilder {
    keys: Keys,
    identifier: String,
    clone_urls: Vec<String>,
    relay_urls: Vec<String>,
    maintainers: Vec<String>,
}

impl AnnouncementBuilder {
    pub fn new(identifier: &str) -> Self { /* ... */ }
    
    pub fn with_clone(mut self, url: &str) -> Self {
        self.clone_urls.push(url.to_string());
        self
    }
    
    pub fn with_relay(mut self, url: &str) -> Self {
        self.relay_urls.push(url.to_string());
        self
    }
    
    pub async fn build(self) -> Result<Event> {
        EventBuilder::new(Kind::Custom(30617), "", [
            Tag::identifier(&self.identifier),
            // Add clone tags
            // Add relay tags
            // Add maintainer tags
        ])
        .to_event(&self.keys)
    }
}
```

## Example Usage

```rust
// examples/test_server.rs

use grasp_compliance_tests::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Test any GRASP implementation
    let client = GraspTestClient::new("http://localhost:8080");
    
    // Run NIP-01 smoke tests
    println!("Running NIP-01 smoke tests...");
    let nip01_results = test_nip01_smoke(&client).await;
    nip01_results.print_report();
    
    // Run GRASP-01 relay tests
    println!("\nRunning GRASP-01 relay tests...");
    let grasp01_results = test_grasp_01_relay_requirements(&client).await;
    grasp01_results.print_report();
    
    // Exit with error if any failed
    if !nip01_results.all_passed() || !grasp01_results.all_passed() {
        std::process::exit(1);
    }
    
    Ok(())
}
```

## Test Output Format

```
GRASP-01: Relay Requirements
════════════════════════════════════════════════════════════

✓ accepts_repository_announcement (GRASP-01:9-10)
  Requirement: MUST accept NIP-34 repository announcements
  Duration: 45ms

✓ accepts_repository_state (GRASP-01:9-10)
  Requirement: MUST accept NIP-34 repository state events
  Duration: 32ms

✗ rejects_announcement_without_clone_tag (GRASP-01:12-13)
  Requirement: MUST reject announcements without clone tag
  Error: Event was accepted but should have been rejected
  Expected: OK response with ok=false
  Got: OK response with ok=true
  Duration: 28ms

Results: 2/3 passed (66.7%)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Overall: 17/18 tests passed (94.4%)
```

## Benefits of This Approach

1. **Focused Testing**: Test GRASP-specific behavior, not generic Nostr
2. **Reusable Tool**: Any GRASP implementation can use this
3. **Clear Failures**: Failures cite exact spec requirements
4. **Maintainable**: Only 18 tests instead of 100+ redundant tests
5. **Fast**: Smoke tests run in seconds, not minutes
6. **Exportable**: Can be published as `grasp-compliance-tests` crate

## Questions for You

1. **Scope Confirmation**: Do you agree we should do smoke tests for NIP-01 rather than comprehensive testing?

2. **Test Count**: Are 18 tests (6 smoke + 12 GRASP-specific) sufficient for the first requirement?

3. **Implementation Order**: Should we:
   - a) Build the test tool first, then implement ngit-grasp to pass it?
   - b) Build them in parallel?
   - c) Start with minimal ngit-grasp, then add tests?

4. **Fixture Strategy**: Should we use:
   - a) Deterministic test keys (same keys every run)?
   - b) Random keys (new keys each run)?
   - c) Configurable (support both)?

5. **Integration**: Should the compliance tests:
   - a) Be a separate crate from day one?
   - b) Start in ngit-grasp, extract later?
   - c) Hybrid (some tests in both places)?

## Recommended Next Steps

**Option A: Test-First Approach (Recommended)**
1. Create `grasp-compliance-tests/` crate
2. Implement all 18 tests (they will all fail)
3. Implement ngit-grasp to pass tests
4. Iterate until all tests pass

**Option B: Parallel Development**
1. Create minimal ngit-grasp skeleton
2. Create test tool in parallel
3. Wire them together
4. Fix failing tests

**Option C: Implementation-First**
1. Build ngit-grasp based on architecture docs
2. Create tests to verify it works
3. Extract tests to standalone crate

I recommend **Option A** because:
- Tests serve as executable specification
- Forces us to think through edge cases
- Tests are reusable immediately
- TDD approach ensures testability

## Timeline Estimate

- **Week 1**: Test tool foundation + NIP-01 smoke tests
- **Week 2**: GRASP-01 relay tests + fixtures
- **Week 3**: Integration with ngit-grasp skeleton
- **Week 4**: Iterate until all tests pass

Total: **4 weeks** to prove the concept with working tests and passing implementation.

---

**Ready to proceed?** Please advise on:
1. Approach (A, B, or C)
2. Any changes to test scope
3. Priority of specific tests
4. Any additional tests you want included
