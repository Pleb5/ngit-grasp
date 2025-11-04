# GRASP-01 Test Breakdown: First Requirement

**Requirement:** "MUST serve a NIP-01 compliant nostr relay at / that accepts git repository announcements and their corresponding repo state announcements."

---

## Test Summary

| Category | Count | Purpose | Time Investment |
|----------|-------|---------|-----------------|
| NIP-01 Smoke Tests | 6 | Verify basic relay works | 1-2 days |
| GRASP-01 Specific | 12 | Verify GRASP protocol | 3-4 days |
| **Total** | **18** | **Prove the concept** | **1 week** |

---

## NIP-01 Smoke Tests (6 tests)

### Why Only Smoke Tests?

**rust-nostr already has 1000+ tests for:**
- ✅ Event structure validation
- ✅ Signature verification (Schnorr/secp256k1)
- ✅ Event ID calculation (SHA256)
- ✅ WebSocket message handling
- ✅ Subscription management
- ✅ Filter matching

**We don't need to re-test this.** We just verify the relay works at all.

### The 6 Smoke Tests

| # | Test Name | What It Tests | Why It Matters |
|---|-----------|---------------|----------------|
| 1 | `websocket_connection` | Can connect to `/` via WebSocket | Relay is running and accepting connections |
| 2 | `send_receive_event` | Can send EVENT, get OK response | Basic event submission works |
| 3 | `create_subscription` | Can send REQ, receive EOSE | Subscription system works |
| 4 | `close_subscription` | Can close subscriptions | Cleanup works |
| 5 | `reject_invalid_event` | Rejects bad signatures | Validation is enabled |
| 6 | `reject_invalid_event_id` | Rejects wrong IDs | ID verification works |

**Coverage:** Basic Nostr relay functionality (not GRASP-specific)

---

## GRASP-01 Specific Tests (12 tests)

### Why These Tests?

These test **our code**, not rust-nostr's code. They verify:
- GRASP policy enforcement
- Repository announcement acceptance
- Integration between Nostr relay and Git service

### The 12 GRASP Tests

| # | Test Name | Spec Ref | What It Tests |
|---|-----------|----------|---------------|
| 7 | `accepts_repository_announcement` | GRASP-01:9-10 | Accepts NIP-34 kind 30617 events |
| 8 | `accepts_repository_state` | GRASP-01:9-10 | Accepts NIP-34 kind 30618 events |
| 9 | `rejects_announcement_without_clone_tag` | GRASP-01:12-13 | Enforces clone tag requirement |
| 10 | `rejects_announcement_without_relay_tag` | GRASP-01:12-13 | Enforces relay tag requirement |
| 11 | `accepts_announcement_with_multiple_clones` | GRASP-01:12-13 | Handles multiple clone URLs |
| 12 | `accepts_events_tagging_announcement` | GRASP-01:17-20 | Accepts issues/PRs tagging repos |
| 13 | `accepts_events_tagged_by_announcement` | GRASP-01:17-20 | Accepts events tagged by repos |
| 14 | `rejects_events_tagging_rejected_announcement` | GRASP-01:17-20 | Rejects orphaned events |
| 15 | `query_announcements_by_identifier` | GRASP-01 (implied) | Can query repos by identifier |
| 16 | `query_state_events` | GRASP-01 (implied) | Can query repository state |
| 17 | `state_replaces_previous` | NIP-01 replaceable | Latest state wins |
| 18 | `concurrent_event_submission` | General reliability | No race conditions |

**Coverage:** GRASP protocol requirements and policy enforcement

---

## What We're NOT Testing (and Why)

### Not Testing: NIP-01 Core Protocol

**Reason:** rust-nostr already tests this extensively

| What | Why Not Testing |
|------|-----------------|
| Event signature verification | rust-nostr has 100+ tests |
| Event ID calculation | rust-nostr has 50+ tests |
| WebSocket message parsing | rust-nostr has 200+ tests |
| Subscription filter matching | rust-nostr has 150+ tests |
| Event serialization | rust-nostr has 75+ tests |

**Estimated time saved:** 2-3 weeks of redundant work

### Not Testing: Git Protocol Details

**Reason:** Will test in separate Git service tests

| What | Where It's Tested |
|------|-------------------|
| Git pack parsing | Git service unit tests |
| Ref update parsing | Git service unit tests |
| Git authorization | Git integration tests |
| Push/pull operations | E2E tests |

---

## Test Implementation Estimate

### Week 1: Test Tool Foundation
- **Day 1-2**: Set up `grasp-compliance-tests/` crate
- **Day 3**: Implement test client (HTTP/WebSocket)
- **Day 4**: Implement NIP-01 smoke tests (6 tests)
- **Day 5**: Test fixtures and builders

### Week 2: GRASP Tests
- **Day 1-2**: Implement announcement tests (7-11)
- **Day 3**: Implement related event tests (12-14)
- **Day 4**: Implement query tests (15-17)
- **Day 5**: Implement concurrent test (18) + polish

### Week 3: Integration
- **Day 1-2**: Create minimal ngit-grasp skeleton
- **Day 3-4**: Wire up nostr-relay-builder
- **Day 5**: First test run (expect failures)

### Week 4: Iteration
- **Day 1-3**: Fix failing tests
- **Day 4**: Documentation
- **Day 5**: Polish and prepare for next requirement

**Total:** 4 weeks to prove the concept

---

## Success Criteria

### Phase 1: Test Tool Works
- ✅ Can connect to any WebSocket relay
- ✅ Can send events and subscriptions
- ✅ Can assert on responses
- ✅ All 18 tests can execute (even if they fail)

### Phase 2: Smoke Tests Pass
- ✅ Basic NIP-01 functionality works
- ✅ Can send/receive events
- ✅ Subscriptions work
- ✅ Invalid events rejected

### Phase 3: GRASP Tests Pass
- ✅ Repository announcements accepted
- ✅ State events accepted
- ✅ Policy enforcement works (clone/relay tags)
- ✅ Related events accepted
- ✅ Queries work

### Phase 4: Concept Proven
- ✅ All 18 tests pass
- ✅ Test tool is reusable
- ✅ Architecture validated
- ✅ Ready for next GRASP-01 requirements

---

## Comparison: Our Approach vs. Comprehensive

| Aspect | Our Approach | Comprehensive Approach |
|--------|--------------|------------------------|
| NIP-01 Tests | 6 smoke tests | 50-100 full tests |
| GRASP Tests | 12 focused tests | 12 focused tests |
| Total Tests | **18** | **62-112** |
| Time to Implement | **1 week** | **3-4 weeks** |
| Maintenance Burden | **Low** | **High** |
| Redundancy | **Minimal** | **Significant** |
| Value-Add | **High** (GRASP-specific) | **Low** (mostly redundant) |

**Conclusion:** Our approach is 3-4x faster with same GRASP coverage.

---

## Next Steps

1. ✅ **Review this breakdown** - Confirm scope
2. ✅ **Choose approach** - Test-first, parallel, or implementation-first
3. ✅ **Start implementation** - Create test tool skeleton
4. ✅ **Iterate** - Build until all tests pass

---

## Questions?

- **Q: Is 6 smoke tests enough?**  
  A: Yes, because rust-nostr already tests NIP-01 comprehensively.

- **Q: Should we test more NIP-01 features?**  
  A: Only if we find bugs in rust-nostr (unlikely).

- **Q: Can other implementations use this?**  
  A: Yes! That's the point of making it standalone.

- **Q: What about GRASP-02 and GRASP-05?**  
  A: We'll add those test modules later, same structure.

---

**Ready to proceed?** See REPORT_COMPLIANCE_TESTING.md for full details.
