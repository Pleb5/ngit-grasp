# Report: GRASP Compliance Testing Strategy

**Date:** November 3, 2025  
**Subject:** Exportable Test Tool for GRASP-01 First Requirement  
**Status:** Proposal Ready for Review

---

## Executive Summary

I've analyzed the requirements for testing GRASP-01's first requirement: *"MUST serve a NIP-01 compliant nostr relay at / that accepts git repository announcements and their corresponding repo state announcements."*

**Key Finding:** We should NOT extensively test NIP-01 compliance because `rust-nostr` already has 1000+ tests for this. Instead, we should:
- ✅ Write **6 smoke tests** for basic NIP-01 functionality
- ✅ Write **12 GRASP-specific tests** for repository announcements
- ✅ Create a **reusable compliance testing tool** that any GRASP implementation can use

This focused approach saves significant time while ensuring comprehensive GRASP protocol testing.

---

## The NIP-01 Testing Question

### What is NIP-01?

NIP-01 defines the basic Nostr protocol:
- Event structure (id, pubkey, sig, kind, tags, content)
- Event validation (signature verification, ID calculation)
- WebSocket messages (EVENT, REQ, CLOSE, NOTICE, OK, EOSE)
- Subscription filters

### What Does rust-nostr Already Test?

The `nostr-relay-builder` crate we're using includes:
- ✅ Complete event validation
- ✅ Signature verification (Schnorr on secp256k1)
- ✅ Event ID validation (SHA256)
- ✅ WebSocket message handling
- ✅ Subscription management
- ✅ 1000+ unit and integration tests

### Recommendation: Smoke Tests Only

**We should NOT re-test what rust-nostr already tests.**

Instead of writing 50+ tests for NIP-01 compliance, we write:
- **6 smoke tests** to verify the relay works at all
- **12 GRASP-specific tests** for repository announcement logic

This is pragmatic because:
1. We're using a battle-tested library, not implementing NIP-01 from scratch
2. Our value is GRASP protocol logic, not Nostr basics
3. Comprehensive NIP-01 testing would be 80% redundant work
4. Other GRASP implementations (Go, Python) will also use tested Nostr libraries

---

## Proposed Test Structure

### NIP-01 Smoke Tests (6 tests)

**Purpose:** Verify basic relay functionality

1. ✅ `websocket_connection` - Can connect to `/`
2. ✅ `send_receive_event` - Can send EVENT, get OK response
3. ✅ `create_subscription` - Can send REQ, receive EOSE
4. ✅ `close_subscription` - Can close subscriptions
5. ✅ `reject_invalid_event` - Rejects events with bad signatures
6. ✅ `reject_invalid_event_id` - Rejects events with wrong IDs

**Coverage:** Basic relay works, events can be sent/received

### GRASP-01 Specific Tests (12 tests)

**Purpose:** Verify GRASP protocol requirements

7. ✅ `accepts_repository_announcement` - Accepts NIP-34 kind 30617
8. ✅ `accepts_repository_state` - Accepts NIP-34 kind 30618
9. ✅ `rejects_announcement_without_clone_tag` - Enforces clone tag
10. ✅ `rejects_announcement_without_relay_tag` - Enforces relay tag
11. ✅ `accepts_announcement_with_multiple_clones` - Handles multiple URLs
12. ✅ `accepts_events_tagging_announcement` - Accepts related events
13. ✅ `accepts_events_tagged_by_announcement` - Accepts tagged events
14. ✅ `rejects_events_tagging_rejected_announcement` - Rejects orphans
15. ✅ `query_announcements_by_identifier` - Can query repos
16. ✅ `query_state_events` - Can query state
17. ✅ `state_replaces_previous` - Replaceable events work
18. ✅ `concurrent_event_submission` - No race conditions

**Coverage:** GRASP policy enforcement, repository lifecycle

---

## Proposed Implementation

### Structure

```
grasp-compliance-tests/          ← Standalone, reusable crate
├── src/
│   ├── lib.rs                   ← Public API
│   ├── client.rs                ← Test client (HTTP/WS/Git)
│   ├── assertions.rs            ← Spec-based assertions
│   ├── fixtures.rs              ← Event/repo builders
│   └── specs/
│       ├── nip01_smoke.rs       ← 6 smoke tests
│       └── grasp_01.rs          ← 12 GRASP tests
└── examples/
    └── test_server.rs           ← Test any GRASP server
```

### Key Features

1. **Reusable**: Can test ngit-grasp, ngit-relay, or any GRASP implementation
2. **Spec-Mirrored**: Test names and comments cite exact spec lines
3. **Clear Failures**: Failures show requirement + what went wrong
4. **Exportable**: Publish as `grasp-compliance-tests` crate

### Example Usage

```rust
use grasp_compliance_tests::*;

#[tokio::main]
async fn main() {
    let client = GraspTestClient::new("http://localhost:8080");
    
    // Run smoke tests
    let smoke = test_nip01_smoke(&client).await;
    smoke.print_report();
    
    // Run GRASP tests
    let grasp = test_grasp_01_relay(&client).await;
    grasp.print_report();
}
```

### Example Output

```
GRASP-01: Relay Requirements
════════════════════════════════════════════════════════════

✓ accepts_repository_announcement (GRASP-01:9-10)
  Requirement: MUST accept NIP-34 repository announcements
  Duration: 45ms

✗ rejects_announcement_without_clone_tag (GRASP-01:12-13)
  Requirement: MUST reject announcements without clone tag
  Error: Event was accepted but should have been rejected
  Duration: 28ms

Results: 11/12 passed (91.7%)
```

---

## Can We Reuse rust-nostr Tests?

### Direct Reuse: No

- Their tests are internal to their crates
- They test library functions, not running servers
- Not designed for external use

### Indirect Reuse: Yes

We can leverage their patterns:

```rust
// Use their event builders
use nostr_sdk::prelude::*;

let event = EventBuilder::new(Kind::Custom(30617), "", [
    Tag::identifier("my-repo"),
    Tag::custom(TagKind::Custom("clone".into()), vec![domain]),
])
.to_event(&keys)?;

// But test server acceptance, not library validation
assert!(client.send_event(event).await?.ok);
```

**What we leverage:**
- ✅ Event building utilities from `nostr-sdk`
- ✅ Key generation patterns
- ✅ Confidence that underlying validation works

**What we test:**
- 🎯 GRASP policy enforcement (our code)
- 🎯 Repository announcement acceptance (our code)
- 🎯 Integration between relay and Git service (our code)

---

## Timeline & Approach

### Option A: Test-First (Recommended)

**Week 1:**
- Create `grasp-compliance-tests/` crate
- Implement test client (HTTP/WebSocket)
- Write all 18 tests (they will fail)

**Week 2:**
- Create ngit-grasp skeleton
- Wire up nostr-relay-builder
- Implement GRASP policies

**Week 3:**
- Fix failing tests
- Add missing functionality
- Iterate until green

**Week 4:**
- Polish and document
- Extract reusable patterns
- Prepare for next GRASP-01 requirements

### Option B: Parallel Development

Build test tool and implementation simultaneously.

### Option C: Implementation-First

Build ngit-grasp first, then create tests.

**I recommend Option A** because:
- Tests serve as executable specification
- Forces thinking through edge cases early
- Ensures testability from day one
- Tests are immediately reusable by others

---

## Benefits of This Approach

### 1. Focused Testing
- 18 tests vs. 100+ redundant tests
- Test GRASP logic, not generic Nostr
- Fast execution (seconds, not minutes)

### 2. Reusable Tool
- Any GRASP implementation can use it
- Go, Rust, Python, JavaScript
- Publish as standalone crate
- Community contribution opportunity

### 3. Clear Failures
- Cite exact spec requirements
- Show expected vs. actual
- Actionable error messages

### 4. Maintainable
- Tests mirror spec structure
- Easy to add GRASP-02, GRASP-05 tests
- Update tests when spec updates

### 5. Proof of Concept
- Demonstrates architecture viability
- Validates inline authorization approach
- Shows rust-nostr integration works

---

## Questions for Decision

### 1. Scope Confirmation
**Do you agree with smoke tests for NIP-01 rather than comprehensive testing?**

- ✅ Yes: 6 smoke tests + 12 GRASP tests (18 total)
- ❌ No: Write comprehensive NIP-01 tests (50+ tests)

### 2. Implementation Approach
**Which approach should we take?**

- **A**: Test-first (write tests, then implement)
- **B**: Parallel (tests and implementation together)
- **C**: Implementation-first (code first, tests later)

### 3. Crate Structure
**Should the compliance tests be separate from day one?**

- **Separate**: `grasp-compliance-tests/` as standalone crate
- **Integrated**: Start in `ngit-grasp/tests/`, extract later
- **Hybrid**: Some in both places

### 4. Fixture Strategy
**How should we generate test data?**

- **Deterministic**: Same keys/events every run (reproducible)
- **Random**: New keys each run (finds more bugs)
- **Configurable**: Support both modes

---

## Recommended Next Steps

1. ✅ **Review this proposal** - Confirm approach and scope
2. ✅ **Answer decision questions** - Guide implementation direction
3. ✅ **Create test tool skeleton** - Set up project structure
4. ✅ **Implement smoke tests** - Verify basic connectivity
5. ✅ **Implement GRASP tests** - Test repository announcements
6. ✅ **Create minimal ngit-grasp** - Wire up nostr-relay-builder
7. ✅ **Iterate until green** - Fix failing tests
8. ✅ **Document and polish** - Prepare for next requirements

---

## Files Created

1. **COMPLIANCE_TEST_PROPOSAL.md** - Detailed proposal with code examples
2. **REPORT_COMPLIANCE_TESTING.md** - This executive summary

---

## Ready to Proceed?

Please review and advise on:

1. ✅ **Scope**: Agree with smoke tests approach?
2. ✅ **Approach**: Test-first (A), parallel (B), or implementation-first (C)?
3. ✅ **Priority**: Any specific tests to prioritize?
4. ✅ **Changes**: Any modifications to the 18 proposed tests?

Once you confirm the approach, I'll begin implementation immediately.

---

**Status:** ⏸️ Awaiting your decision on approach and scope
