# GRASP-01 Event Relationship Smoke Tests Design

**Version:** 1.0  
**Date:** 2025-11-05  
**Status:** Ready for Implementation

## Overview

This document specifies a focused suite of **smoke tests** for GRASP-01 event reference validation (lines 7-9). These tests validate the basic acceptance/rejection behavior based on event tagging relationships, separate from the comprehensive test suite.

**Key Principle:** Events are accepted if they tag OR are tagged by accepted repositories.

---

## File Location

**Proposed Path:** `grasp-audit/src/specs/grasp01/event-acceptance-policy.rs`

**Rationale:**
- Separate from comprehensive suite
- Clear naming indicates purpose (smoke tests for event acceptance policy)
- Lives in `grasp01/` subdirectory for organization
- Can be run independently or as part of full suite

---

## Test Scenarios

### Scenario Group 1: Accept Events Tagging Accepted Repositories

Events that reference an already-accepted repo should be accepted.

#### Test 1.1: `test_accept_issue_via_a_tag`
**Tags Issue → Repo via `a` tag**

```rust
Setup:
1. Create and send repo announcement (kind 30617)
2. Create issue (kind 1621) with:
   - ["a", "30617:{pubkey}:{repo-id}"]
3. Send issue

Expected: Issue SHOULD be stored (query returns it)
```

---

#### Test 1.2: `test_accept_comment_via_A_tag`
**Tags Comment → Repo via `A` tag (NIP-22 root)**

```rust
Setup:
1. Create and send repo announcement
2. Create comment (kind 1111) with:
   - ["A", "30617:{pubkey}:{repo-id}"]  // Root
   - ["K", "30617"]
   - ["P", "{repo-pubkey}"]
3. Send comment

Expected: Comment SHOULD be stored
```

---

#### Test 1.3: `test_accept_kind1_via_q_tag`
**Tags Kind 1 → Repo via `q` tag (quote)**

```rust
Setup:
1. Create and send repo announcement
2. Create kind 1 text note with:
   - ["q", "30617:{pubkey}:{repo-id}"]
   - content: "Check out this repo!"
3. Send kind 1

Expected: Kind 1 SHOULD be stored
```

---

### Scenario Group 2: Accept Events Tagging Accepted Events

Events that reference other accepted events should be accepted (transitive acceptance).

#### Test 2.1: `test_accept_issue_quoting_issue_via_q`
**Issue referencing unaccepted repo but quoting accepted issue**

```rust
Setup:
1. Create and send repo A announcement
2. Create and send issue A (for repo A)
3. Create repo B announcement (DO NOT send - not accepted)
4. Create issue B (for repo B) with:
   - ["a", "30617:{pubkey}:{repo-b-id}"]  // References unaccepted repo B
   - ["q", "{issue-a-id}"]  // Quote accepted issue A
5. Send issue B

Expected: Issue B SHOULD be stored (related via quote to accepted issue A,
          even though its own repo reference is not accepted)
```

---

#### Test 2.2: `test_accept_comment_via_E_tag`
**Comment on issue via `E` tag (NIP-22)**

```rust
Setup:
1. Create and send repo announcement
2. Create and send issue (kind 1621)
3. Create comment (kind 1111) with:
   - ["E", "{issue-id}"]  // Root
   - ["K", "1621"]
   - ["P", "{issue-author}"]
   - ["e", "{issue-id}"]  // Parent (same as root for top-level)
   - ["k", "1621"]
   - ["p", "{issue-author}"]
4. Send comment

Expected: Comment SHOULD be stored (related to accepted issue)
```

---

#### Test 2.3: `test_accept_kind1_via_e_tag`
**Kind 1 referencing another kind 1 via `e` tag**

```rust
Setup:
1. Create and send repo announcement
2. Create kind 1 note A with ["q", "30617:{pubkey}:{repo-id}"]
3. Send kind 1 A
4. Create kind 1 note B with:
   - ["e", "{kind1-a-id}", "", "reply"]
   - content: "Great point!"
5. Send kind 1 B

Expected: Kind 1 B SHOULD be stored (related via e tag to accepted kind 1 A)
```

---

### Scenario Group 3: Accept Events Tagged by Accepted Events

Events that are referenced BY accepted events should be accepted (forward references).

#### Test 3.1: `test_accept_kind1_referenced_in_issue`
**Kind 1 referenced in issue via `q` tag**

```rust
Setup:
1. Create kind 1 note (NOT sent yet)
2. Create and send repo announcement
3. Create issue with:
   - ["a", "30617:{pubkey}:{repo-id}"]
   - ["q", "{kind1-id}"]  // Reference the not-yet-sent kind 1
4. Send issue
5. Send kind 1 note

Expected: Kind 1 SHOULD be stored (referenced by accepted issue)
```

---

#### Test 3.2: `test_accept_comment_referenced_in_comment`
**Comment referenced in another comment via `q` tag**

```rust
Setup:
1. Create and send repo announcement
2. Create and send issue
3. Create comment A (NOT sent yet)
4. Create comment B with:
   - ["E", "{issue-id}"]  // Root
   - ["e", "{issue-id}"]  // Parent
   - ["q", "{comment-a-id}"]  // Quote comment A
5. Send comment B
6. Send comment A

Expected: Comment A SHOULD be stored (referenced by accepted comment B)
```

---

#### Test 3.3: `test_accept_kind1_referenced_in_kind1`
**Kind 1 referenced in accepted kind 1 via `e` tag**

```rust
Setup:
1. Create and send repo announcement
2. Create kind 1 A (NOT sent yet)
3. Create kind 1 B with:
   - ["q", "30617:{pubkey}:{repo-id}"]
   - ["e", "{kind1-a-id}", "", "mention"]
4. Send kind 1 B
5. Send kind 1 A

Expected: Kind 1 A SHOULD be stored (referenced by accepted kind 1 B)
```

---

### Scenario Group 4: Reject Unrelated Events

Events with no relationship to accepted repositories should be rejected.

#### Test 4.1: `test_reject_orphan_issue`
**Issue from unrelated repository**

```rust
Setup:
1. Create issue (kind 1621) with:
   - ["a", "30617:{other-pubkey}:{other-repo-id}"]  // Different repo
2. Send issue

Expected: Issue SHOULD NOT be stored (no accepted repo)
```

---

#### Test 4.2: `test_reject_orphan_kind1`
**Kind 1 from unrelated context**

```rust
Setup:
1. Create kind 1 note with generic content (no tags)
2. Send kind 1

Expected: Kind 1 SHOULD NOT be stored (no relationship to any repo)
```

---

#### Test 4.3: `test_reject_comment_quoting_other_repo`
**Comment quoting announcement from different repository**

```rust
Setup:
1. Create repo A announcement (sent)
2. Create repo B announcement (NOT sent - different owner)
3. Create comment with:
   - ["A", "30617:{other-pubkey}:{repo-b-id}"]  // Root
   - ["q", "30617:{other-pubkey}:{repo-b-id}"]  // Quote unaccepted repo
4. Send comment

Expected: Comment SHOULD NOT be stored (references unaccepted repo)
```

---

## Helper Functions

Keep helpers minimal and focused on smoke test needs.

**Implementation Note:** Reference [`nostr-sdk`](https://docs.rs/nostr-sdk) (rust-nostr) for event generation patterns. The SDK provides robust helpers for creating events with proper signatures and tags. Use these patterns rather than building everything from scratch.

### `create_test_repo(client, repo_id) -> Event`
Creates a basic repo announcement with required tags.

```rust
async fn create_test_repo(client: &AuditClient, repo_id: &str) -> Result<Event> {
    client.create_repo_announcement(repo_id).await
}
```

---

### `create_issue_for_repo(client, repo_event, subject) -> Event`
Creates issue referencing repo via `a` tag.

```rust
async fn create_issue_for_repo(
    client: &AuditClient,
    repo_event: &Event,
    subject: &str,
) -> Result<Event> {
    let repo_id = extract_d_tag(repo_event)?;
    let a_tag = Tag::parse(&["a", &format!("30617:{}:{}", repo_event.pubkey, repo_id)])?;
    
    client.event_builder()
        .kind(Kind::Custom(1621))
        .content(format!("Issue: {}", subject))
        .tag(a_tag)
        .build()
        .await
}
```

---

### `create_comment_for_event(client, root_event, content) -> Event`
Creates NIP-22 comment for an event.

```rust
async fn create_comment_for_event(
    client: &AuditClient,
    root_event: &Event,
    content: &str,
) -> Result<Event> {
    client.event_builder()
        .kind(Kind::Custom(1111))
        .content(content)
        .tag(Tag::parse(&["E", &root_event.id.to_string()])?)
        .tag(Tag::parse(&["K", &root_event.kind.to_string()])?)
        .tag(Tag::parse(&["P", &root_event.pubkey.to_string()])?)
        .tag(Tag::parse(&["e", &root_event.id.to_string()])?)
        .tag(Tag::parse(&["k", &root_event.kind.to_string()])?)
        .tag(Tag::parse(&["p", &root_event.pubkey.to_string()])?)
        .build()
        .await
}
```

---

### `send_and_verify_accepted(client, event) -> Result<()>`
Sends event and verifies it was stored.

```rust
async fn send_and_verify_accepted(client: &AuditClient, event: Event) -> Result<()> {
    let event_id = client.send_event(event.clone()).await?;
    
    // Small delay for propagation
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let filter = Filter::new()
        .id(event_id)
        .limit(1);
    
    let results = client.query(filter).await?;
    
    if results.is_empty() {
        return Err("Event was not stored".into());
    }
    
    Ok(())
}
```

---

### `send_and_verify_rejected(client, event) -> Result<()>`
Sends event and verifies it was NOT stored.

```rust
async fn send_and_verify_rejected(client: &AuditClient, event: Event) -> Result<()> {
    let event_id = event.id;
    
    // Attempt to send
    let _ = client.send_event(event).await;
    
    // Small delay for propagation
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let filter = Filter::new()
        .id(event_id)
        .limit(1);
    
    let results = client.query(filter).await?;
    
    if !results.is_empty() {
        return Err("Event was stored but should have been rejected".into());
    }
    
    Ok(())
}
```

---

### `extract_d_tag(event) -> Result<String>`
Extracts `d` tag value from event.

```rust
fn extract_d_tag(event: &Event) -> Result<String> {
    event.tags
        .iter()
        .find(|t| t.kind() == TagKind::d())
        .and_then(|t| t.content())
        .ok_or("Missing d tag")?
        .to_string()
}
```

---

## Module Structure

```rust
//! GRASP-01 Event Relationship Smoke Tests
//!
//! Focused smoke tests validating basic event acceptance/rejection
//! based on tagging relationships with accepted repositories.

use crate::{AuditClient, AuditResult, TestResult};
use nostr_sdk::prelude::*;
use std::time::Duration;

pub struct EventAcceptancePolicyTests;

impl EventAcceptancePolicyTests {
    pub async fn run_all(client: &AuditClient) -> AuditResult {
        let mut results = AuditResult::new("GRASP-01 Event Acceptance Policy Tests");
        
        // Group 1: Events tagging repos
        results.add(Self::test_accept_issue_via_a_tag(client).await);
        results.add(Self::test_accept_comment_via_A_tag(client).await);
        results.add(Self::test_accept_kind1_via_q_tag(client).await);
        
        // Group 2: Events tagging accepted events
        results.add(Self::test_accept_issue_quoting_issue_via_q(client).await);
        results.add(Self::test_accept_comment_via_E_tag(client).await);
        results.add(Self::test_accept_kind1_via_e_tag(client).await);
        
        // Group 3: Events tagged by accepted events
        results.add(Self::test_accept_kind1_referenced_in_issue(client).await);
        results.add(Self::test_accept_comment_referenced_in_comment(client).await);
        results.add(Self::test_accept_kind1_referenced_in_kind1(client).await);
        
        // Group 4: Reject unrelated events
        results.add(Self::test_reject_orphan_issue(client).await);
        results.add(Self::test_reject_orphan_kind1(client).await);
        results.add(Self::test_reject_comment_quoting_other_repo(client).await);
        
        results
    }
    
    // Test implementations follow...
}

// Helper functions follow...
```

---

## Integration with Test Suite

Add to `grasp-audit/src/specs/grasp01/mod.rs`:

```rust
pub mod event_acceptance_policy;

pub use event_acceptance_policy::EventAcceptancePolicyTests;
```

Add to main test runner if desired, or run independently:

```rust
// In grasp01_nostr_relay.rs or separate test file
#[tokio::test]
#[ignore]
async fn test_event_acceptance_policy_suite() {
    let client = AuditClient::new_for_relay(&relay_url()).await.unwrap();
    let results = EventAcceptancePolicyTests::run_all(&client).await;
    
    // Assert all tests passed
    assert!(results.all_passed(), "Some tests failed:\n{}", results);
}
```

---

## Implementation Notes

1. **Simplicity First:** Keep test logic straightforward - setup, send, verify
2. **Independent Tests:** Each test should be runnable standalone
3. **Clear Failures:** Use descriptive error messages for debugging
4. **Minimal Helpers:** Only create helpers that reduce significant duplication
5. **Fast Execution:** Smoke tests should run quickly (use minimal delays)

---

## Expected Outcomes

When implemented, this suite should:

- ✅ Run in under 5 seconds total
- ✅ Clearly show which relationship types work/fail
- ✅ Provide quick validation during development
- ✅ Act as regression tests for basic GRASP-01 compliance
- ✅ Be easy to understand and modify

---

## Next Steps

1. Create `grasp-audit/src/specs/grasp01/event-acceptance-policy.rs`
2. Implement helper functions (referencing nostr-sdk patterns)
3. Implement each test function following the specifications above
4. Add module declaration to `grasp01/mod.rs`
5. Run tests: `cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test`
6. Verify all tests pass or show expected "Not implemented yet" status

---

## Success Criteria

- [ ] All 12 tests compile without errors
- [ ] Tests run independently and as a suite
- [ ] Accept tests verify events ARE stored
- [ ] Reject tests verify events are NOT stored
- [ ] Helper functions eliminate code duplication
- [ ] Test output clearly indicates pass/fail/not-implemented