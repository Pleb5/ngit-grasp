# GRASP-01 Event Reference Validation Test Design

**Version:** 1.0  
**Date:** 2025-11-05  
**Status:** Design Phase - Ready for Review

## Executive Summary

This document provides a comprehensive test design for GRASP-01 lines 7-9 compliance, covering event reference validation. The design reshapes existing test stubs to implement proper event relationship testing across all NIP-34 event types (issues, patches, PRs, comments, status updates, and text notes).

## 1. Analysis Section

### 1.1 NIP-34 Event Structures

From `/persistent/dcdev/clones/nips/34.md`, we have these git-related event types:

#### Repository Announcements (kind 30617)
```json
{
  "kind": 30617,
  "tags": [
    ["d", "<repo-id>"],
    ["a", "30617:<pubkey>:<repo-id>"],
    ["clone", "<url>", ...],
    ["relays", "<relay-url>", ...],
    ["maintainers", "<pubkey>", ...]
  ]
}
```

#### Patches (kind 1617)
```json
{
  "kind": 1617,
  "tags": [
    ["a", "30617:<base-repo-owner-pubkey>:<base-repo-id>"],
    ["e", "<parent-patch-id>", "", "reply"],  // NIP-10 threading
    ["p", "<repository-owner>"],
    ["r", "<earliest-unique-commit-id>"]
  ]
}
```

#### Pull Requests (kind 1618)
```json
{
  "kind": 1618,
  "tags": [
    ["a", "30617:<base-repo-owner-pubkey>:<base-repo-id>"],
    ["e", "<root-patch-event-id>"],  // Optional revision reference
    ["p", "<repository-owner>"],
    ["c", "<current-commit-id>"]
  ]
}
```

#### Issues (kind 1621)
```json
{
  "kind": 1621,
  "tags": [
    ["a", "30617:<base-repo-owner-pubkey>:<base-repo-id>"],
    ["p", "<repository-owner>"]
  ]
}
```

#### Comments (kind 1111 - NIP-22)
```json
{
  "kind": 1111,
  "tags": [
    ["E", "<root-event-id>"],  // Root scope (uppercase)
    ["K", "<root-kind>"],
    ["P", "<root-pubkey>"],
    ["e", "<parent-event-id>"],  // Parent (lowercase)
    ["k", "<parent-kind>"],
    ["p", "<parent-pubkey>"]
  ]
}
```

### 1.2 GRASP-01 Lines 7-9 Requirements

Based on test stub comments in [`grasp01_nostr_relay.rs:29-36`](grasp-audit/src/specs/grasp01_nostr_relay.rs:29-36):

**Line 7-9 (inferred):** Events that **tag** OR **are tagged by** accepted repository announcements SHOULD be stored.

This breaks down into three scenarios:

1. **Events NOT referenced** by or referencing other events → SHOULD NOT be stored (orphans)
2. **Events referenced BY** an existing stored event → SHOULD be stored (forward reference)
3. **Events referencing** an existing stored event → SHOULD be stored (backward reference)

### 1.3 Reference Tag Types and Semantics

#### Standard Nostr Reference Tags

| Tag | Purpose | Format | NIP |
|-----|---------|--------|-----|
| `e` | Event ID reference | `["e", "<event-id>", "<relay>", "<marker>", "<pubkey>"]` | NIP-10 |
| `a` | Addressable event reference | `["a", "<kind>:<pubkey>:<d-tag>", "<relay>"]` | NIP-01 |
| `p` | Pubkey reference | `["p", "<pubkey>", "<relay>"]` | NIP-01 |
| `q` | Quote reference | `["q", "<event-id or address>", "<relay>", "<pubkey>"]` | NIP-10 |

#### NIP-22 Comment Tags (Uppercase = Root, Lowercase = Parent)

| Tag | Purpose | Format |
|-----|---------|--------|
| `E` | Root event ID | `["E", "<event-id>", "<relay>", "<pubkey>"]` |
| `A` | Root addressable event | `["A", "<kind>:<pubkey>:<d-tag>", "<relay>"]` |
| `K` | Root event kind | `["K", "<kind>"]` |
| `P` | Root author pubkey | `["P", "<pubkey>", "<relay>"]` |
| `e` | Parent event ID | `["e", "<event-id>", "<relay>", "<pubkey>"]` |
| `k` | Parent event kind | `["k", "<kind>"]` |
| `p` | Parent author pubkey | `["p", "<pubkey>", "<relay>"]` |

#### NIP-10 Threading Tags

| Marker | Purpose |
|--------|---------|
| `root` | First event in thread |
| `reply` | Direct reply to parent |

### 1.4 Event Type Coverage Requirements

Tests must cover:

- ✅ **Issues** (kind 1621) - referencing repos via `a` tag
- ✅ **Patches** (kind 1617) - referencing repos via `a` tag, threading via `e` tags
- ✅ **Pull Requests** (kind 1618) - referencing repos via `a` tag
- ✅ **Comments** (kind 1111) - replying via NIP-22 structure
- ✅ **Status updates** (kinds 1630-1633) - referencing issues/PRs via `e` tag (may also use `E` tag for root references)
- ✅ **Text notes** (kind 1) - may reference announcements/issues/patches/comments OR be referenced by them

## 2. Test Architecture Design

### 2.1 Overall Test Suite Structure

To manage the growing number of tests, we'll organize them into separate test module files:

```
grasp-audit/src/specs/
├── mod.rs (module declarations)
├── grasp01_nostr_relay.rs (main entry point, existing tests)
└── grasp01/
    ├── mod.rs (test suite registration)
    ├── helpers.rs (shared helper functions)
    ├── issues.rs (issue reference tests)
    ├── patches.rs (patch reference tests)
    ├── pull_requests.rs (PR reference tests)
    ├── comments.rs (NIP-22 comment tests)
    ├── status_updates.rs (status change tests)
    └── text_notes.rs (kind 1 reference tests)
```

**Benefits:**
- Better code organization and navigation
- Isolated test contexts
- Easier to maintain and extend
- Clear separation of concerns

### 2.2 Test Organization Strategy

**Group by relationship type:**

1. **Forward References** - Event A exists, send Event B that references A
2. **Backward References** - Send Event A that references B, then send B
3. **Bidirectional** - Events that both reference each other
4. **Orphans** - Events with no references (should be rejected)
5. **Transitive** - Multi-hop references (A → B → C)

**Group by event type:**

1. Issues referencing repos
2. Patches referencing repos (with threading)
3. PRs referencing repos
4. Comments replying to issues/patches/PRs
5. Status updates for issues/PRs
6. Text notes being tagged by repos

## 3. Helper Function Specifications

### 3.1 Core Event Creation Helpers

```rust
/// Create a NIP-34 issue event
async fn create_issue(
    client: &AuditClient,
    repo_announcement: &Event,
    subject: &str,
    content: &str,
) -> Result<Event>
```

**Purpose:** Create properly formatted issue (kind 1621) with `a` tag to repo  
**Returns:** Signed event ready to send  
**Usage:**
```rust
let issue = create_issue(&client, &repo_event, "Bug: Test", "Description").await?;
```

---

```rust
/// Create a NIP-34 patch event
async fn create_patch(
    client: &AuditClient,
    repo_announcement: &Event,
    parent_patch: Option<&Event>,
    patch_content: &str,
) -> Result<Event>
```

**Purpose:** Create patch (kind 1617) with optional NIP-10 threading  
**Returns:** Signed event with proper `a` tag and optional `e` reply tag  
**Usage:**
```rust
// First patch in series
let patch1 = create_patch(&client, &repo, None, "diff...").await?;

// Reply patch
let patch2 = create_patch(&client, &repo, Some(&patch1), "diff...").await?;
```

---

```rust
/// Create a NIP-34 pull request event
async fn create_pull_request(
    client: &AuditClient,
    repo_announcement: &Event,
    branch_name: &str,
    commit_id: &str,
) -> Result<Event>
```

**Purpose:** Create PR (kind 1618) with proper repo reference  
**Returns:** Signed event with `a` tag  
**Usage:**
```rust
let pr = create_pull_request(&client, &repo, "feature-x", "abc123").await?;
```

---

```rust
/// Create a NIP-22 comment event
async fn create_comment(
    client: &AuditClient,
    root_event: &Event,         // The root (issue, patch, or PR)
    parent_event: Option<&Event>, // None for top-level, Some for replies
    content: &str,
) -> Result<Event>
```

**Purpose:** Create comment (kind 1111) with proper NIP-22 tags  
**Returns:** Signed event with E/K/P (root) and e/k/p (parent) tags  
**Usage:**
```rust
// Top-level comment
let comment1 = create_comment(&client, &issue, None, "Great idea!").await?;

// Reply to comment  
let comment2 = create_comment(&client, &issue, Some(&comment1), "Thanks!").await?;
```

---

```rust
/// Create a status event
async fn create_status(
    client: &AuditClient,
    target_event: &Event,  // Issue, patch, or PR
    status_kind: Kind,     // 1630 (Open), 1631 (Resolved), 1632 (Closed), 1633 (Draft)
    reason: &str,
) -> Result<Event>
```

**Purpose:** Create status change event  
**Returns:** Signed event with `e` tag to target  
**Usage:**
```rust
let status = create_status(&client, &issue, Kind::Custom(1631), "Fixed in v1.0").await?;
```

### 3.2 Test Orchestration Helpers

```rust
/// Send event and verify acceptance by querying back
async fn send_and_verify_stored(
    client: &AuditClient,
    event: Event,
) -> Result<()>
```

**Purpose:** Send event, wait for propagation, query to confirm storage  
**Reduces:** Duplication of send → wait → query → verify pattern  
**Usage:**
```rust
send_and_verify_stored(&client, issue_event).await?;
```

---

```rust
/// Send event and verify it was NOT stored (rejection test)
async fn send_and_verify_rejected(
    client: &AuditClient,
    event: Event,
) -> Result<()>
```

**Purpose:** Send event, verify it's not in relay storage  
**Reduces:** Duplication in negative tests  
**Usage:**
```rust
send_and_verify_rejected(&client, orphan_event).await?;
```

---

```rust
/// Extract repo identifier from announcement event
fn extract_repo_id(repo_announcement: &Event) -> Result<String>
```

**Purpose:** Get `d` tag value from repo announcement  
**Reduces:** Tag parsing duplication  
**Usage:**
```rust
let repo_id = extract_repo_id(&repo_event)?;
```

---

```rust
/// Build addressable event tag (a tag) for repo
fn build_repo_atag(repo_announcement: &Event) -> Result<Tag>
```

**Purpose:** Create properly formatted `a` tag for repo reference  
**Reduces:** Tag construction errors  
**Usage:**
```rust
let a_tag = build_repo_atag(&repo_announcement)?;
```

## 4. Test Case Specifications

### 4.1 Issues Referencing Repositories

#### Test: `test_accept_issue_for_repo`
**Validates:** GRASP-01 lines 8-9 - Accept issues referencing accepted repos
**Reference Tags:** `a` tag (repo)
**Expected:** Issue event SHOULD be stored

**Setup:**
1. Create and send kind 30617 repo announcement
2. Verify repo is stored
3. Create kind 1621 issue with:
   - `["a", "30617:{pubkey}:{d-tag}"]`
   - `["subject", "Bug: Something broken"]`
4. Send issue event

**Verification:**
- Query for kind 1621 with author filter
- Verify issue event was stored
- Verify `a` tag correctly references repo

---

#### Test: `test_reject_issue_for_nonexistent_repo`
**Validates:** GRASP-01 line 7 - Reject orphaned issues  
**Reference Tags:** `a` tag (nonexistent repo)  
**Expected:** Issue event SHOULD NOT be stored  

**Setup:**
1. Create kind 1621 issue with `a` tag referencing non-existent repo
2. Send issue event

**Verification:**
- Query for issue event
- Verify it was NOT stored (empty result)

### 4.2 Patches Referencing Repositories

#### Test: `test_accept_patch_for_repo`
**Validates:** GRASP-01 lines 8-9 - Accept patches for accepted repos  
**Reference Tags:** `a` tag (repo), `p` tag, `r` tag  
**Expected:** Patch event SHOULD be stored  

**Setup:**
1. Create and send repo announcement
2. Create kind 1617 patch with:
   - `["a", "30617:{pubkey}:{d-tag}"]`
   - `["p", "{repo-owner}"]`
   - `["r", "{commit-id}"]`
   - `["t", "root"]` (first patch marker)
3. Send patch

**Verification:**
- Query for kind 1617
- Verify patch stored
-Verify proper repo reference

---

#### Test: `test_accept_patch_series_threading`
**Validates:** NIP-10 threading in patches  
**Reference Tags:** `e` reply tag for threading  
**Expected:** All patches in series SHOULD be stored  

**Setup:**
1. Send repo announcement
2. Create and send patch 1 with `["t", "root"]`
3. Create patch 2 with `["e", "{patch1-id}", "", "reply"]`
4. Create patch 3 with `["e", "{patch2-id}", "", "reply"]`
5. Send patches 2 and 3

**Verification:**
- Query all 3 patches
- Verify threading structure via `e` tags
- Verify all stored

### 4.3 Pull Requests Referencing Repositories

#### Test: `test_accept_pull_request_for_repo`
**Validates:** GRASP-01 lines 8-9 - Accept PRs for accepted repos  
**Reference Tags:** `a` tag, `c` tag (commit)  
**Expected:** PR event SHOULD be stored  

**Setup:**
1. Send repo announcement  
2. Create kind 1618 PR with:
   - `["a", "30617:{pubkey}:{d-tag}"]`
   - `["c", "{commit-id}"]`
   - `["subject", "Add feature X"]`
3. Send PR

**Verification:**
- Query kind 1618
- Verify PR stored with correct repo reference

---

#### Test: `test_accept_pr_update`
**Validates:** PR updates (kind 1619) reference original PR  
**Reference Tags:** `E` tag (NIP-22 root), `P` tag  
**Expected:** PR update SHOULD be stored  

**Setup:**
1. Create and send repo + original PR
2. Create kind 1619 update with:
   - `["E", "{pr-event-id}"]`
   - `["P", "{pr-author}"]`
   - `["c", "{new-commit-id}"]`
3. Send update

**Verification:**
- Query kind 1619
- Verify update references original PR

### 4.4 Comments (NIP-22)

#### Test: `test_accept_reply_to_issue`
**Validates:** Comments on issues using NIP-22  
**Reference Tags:** `E`, `K`, `P` (root), `e`, `k`, `p` (parent)  
**Expected:** Comment SHOULD be stored  

**Setup:**
1. Send repo + issue
2. Create kind 1111 comment with:
   - `["E", "{issue-id}"]` (root)
   - `["K", "1621"]` (issue kind)
   - `["P", "{issue-author}"]`
   - `["e", "{issue-id}"]` (parent, same as root for top-level)
   - `["k", "1621"]`
   - `["p", "{issue-author}"]`
3. Send comment

**Verification:**
- Query kind 1111
- Verify proper NIP-22 tag structure

---

#### Test: `test_accept_nested_comment_thread`
**Validates:** Multi-level comment threading  
**Reference Tags:** E/K/P (constant root), e/k/p (changing parent)  
**Expected:** All comments SHOULD be stored  

**Setup:**
1. Send repo + issue
2. Send comment 1 (to issue)
3. Send comment 2 (reply to comment 1):
   - Root tags point to issue
   - Parent tags point to comment 1
4. Send comment 3 (reply to comment 2):
   - Root tags still point to issue
   - Parent tags point to comment 2

**Verification:**
- Query all 3 comments
- Verify root tags always reference issue
- Verify parent tags form chain

---

#### Test: `test_accept_comment_on_patch`
**Validates:** Comments work on patches  
**Reference Tags:** NIP-22 tags for kind 1617  
**Expected:** Comment on patch SHOULD be stored  

**Setup:**
1. Send repo + patch
2. Send kind 1111 comment referencing patch
3. Verify stored

---

#### Test: `test_accept_comment_on_pr`
**Validates:** Comments work on PRs  
**Reference Tags:** NIP-22 tags for kind 1618  
**Expected:** Comment on PR SHOULD be stored  

### 4.5 Status Updates

#### Test: `test_accept_status_for_issue`
**Validates:** Status changes for issues
**Reference Tags:** `e` tag, `p` tag
**Expected:** Status event SHOULD be stored

**Setup:**
1. Send repo + issue
2. Create kind 1631 (Resolved) status with:
   - `["e", "{issue-id}", "", "root"]`
   - `["p", "{issue-author}"]`
   - `["a", "30617:{pubkey}:{repo-id}"]` (optional)
3. Send status

**Verification:**
- Query kind 1631
- Verify references issue

### 4.6 Text Notes and Cross-References

#### Test: `test_accept_kind1_quoted_by_issue`
**Validates:** Kind 1 text notes referenced by issues using `q` tag
**Reference Tags:** Issue's `q` tag pointing to kind 1 note
**Expected:** Kind 1 note SHOULD be accepted when issue quotes it

**Setup:**
1. Create kind 1 text note about project
2. Send text note (may initially be rejected)
3. Send repo announcement
4. Create kind 1621 issue with:
   - `["a", "30617:{pubkey}:{d-tag}"]` (repo reference)
   - `["q", "{note-id}"]` (quote reference to kind 1)
   - `["subject", "Discussion: Feature Request"]`
5. Send issue
6. Re-query for text note

**Verification:**
- Text note should now be stored
- Verifies kind 1 being referenced by issue scenario

## 5. Implementation Phases

### Phase 1: Module Structure Setup (Priority: HIGH)
**Goal:** Create new test suite file structure
**Duration:** 0.5 days

**Tasks:**
1. Create `grasp-audit/src/specs/grasp01/` directory
2. Set up module files:
   - `mod.rs` (test registration)
   - `helpers.rs` (shared functions)
   - `issues.rs`
   - `patches.rs`
   - `pull_requests.rs`
   - `comments.rs`
   - `status_updates.rs`
   - `text_notes.rs`
3. Update `grasp-audit/src/specs/mod.rs` to include new module

**Acceptance Criteria:**
- Module structure compiles
- Tests can be run from new location
- No duplicate code

### Phase 2: Helper Functions (Priority: HIGH)
**Goal:** Core helper functions in `helpers.rs`
**Duration:** 1 day

**Tasks:**
1. Implement core event creation helpers:
   - `create_issue()`
   - `create_patch()`
   - `create_pull_request()`
   - `create_comment()`
   - `create_status()`

2. Implement test orchestration helpers:
   - `send_and_verify_stored()`
   - `send_and_verify_rejected()`
   - `extract_repo_id()`
   - `build_repo_atag()`

**Acceptance Criteria:**
- All helper functions documented
- Unit tests for helpers
- Functions follow nostr-sdk 0.43 API

### Phase 3: Core Event Type Tests (Priority: HIGH)
**Goal:** Implement tests for issues, patches, PRs
**Duration:** 1.5 days

**Tasks:**
1. Implement in `issues.rs`:
   - `test_accept_issue_for_repo`
   - `test_reject_issue_for_nonexistent_repo`

2. Implement in `patches.rs`:
   - `test_accept_patch_for_repo`
   - `test_accept_patch_series_threading`

3. Implement in `pull_requests.rs`:
   - `test_accept_pull_request_for_repo`
   - `test_accept_pr_update`

**Acceptance Criteria:**
- All tests pass against ngit-relay
- Proper event tagging
- Clear test documentation

### Phase 4: Comment Threading (Priority: HIGH)
**Goal:** NIP-22 comment support in `comments.rs`
**Duration:** 1 day

**Tasks:**
1. Implement comment tests:
   - `test_accept_reply_to_issue`
   - `test_accept_nested_comment_thread`
   - `test_accept_comment_on_patch`
   - `test_accept_comment_on_pr`

**Acceptance Criteria:**
- Multi-level threading works
- Uppercase/lowercase tag handling correct
- All comment tests pass

### Phase 5: Status Updates and Text Notes (Priority: MEDIUM)
**Goal:** Complete remaining event types
**Duration:** 1 day

**Tasks:**
1. Implement in `status_updates.rs`:
   - `test_accept_status_for_issue`

2. Implement in `text_notes.rs`:
   - `test_accept_kind1_quoted_by_issue`

**Acceptance Criteria:**
- Status updates work correctly
- Kind 1 quote references validated
- All tests documented

### Phase 6: Documentation and Finalization (Priority: HIGH)
**Goal:** Complete documentation and code review
**Duration:** 0.5 days

**Tasks:**
1. Add comprehensive doc comments to all modules
2. Create migration guide from old structure
3. Update main README with new structure
4. Code review and refactoring
5. Run full test suite verification

**Acceptance Criteria:**
- All modules documented
- Clear organization
- No compiler warnings
- All tests pass

## 6. Edge Cases and Considerations

### 6.1 Potential Edge Cases

1. **Event Arrival Order:**
   - Issue arrives before repo announcement
   - Comment arrives before target event
   - **Mitigation:** Test both orders, document relay behavior

2. **Reference Ambiguity:**
   - Multiple `a` tags to different repos
   - Conflicting `e` tags
   - **Mitigation:** Document which reference takes precedence

3. **Deleted Events:**
   - Event references something that gets deleted
   - **Mitigation:** Test and document behavior

4. **Malformed Tags:**
   - Invalid `a` tag format
   - Missing required tag components
   - **Mitigation:** Test rejection with clear errors

5. **Threading Depth:**
   - Very deep reply chains (100+ levels)
   - **Mitigation:** Set reasonable limits, test performance

6. **Circular References:**
   - A references B, B references A
   - **Mitigation:** Prevent infinite loops, document handling

### 6.2 Performance Considerations

1. **Query Efficiency:**
   - Use specific filters (kind + author)
   - Avoid full relay scans
   - Timeout after 5 seconds

2. **Event Batching:**
   - Send multiple events efficiently
   - Wait between sends (100ms) for propagation

3. **Cleanup:**
   - All events have audit tags for cleanup
   - Use `run_id` for isolation

### 6.3 Test Isolation Requirements

1. **Unique Identifiers:**
   - Use UUIDs for repo IDs
   - Avoid collisions between test runs

2. **Audit Tags:**
   - Automatic via `AuditClient::event_builder()`
   - Enable production cleanup

3. **Relay State:**
   - Assume shared relay (ngit-relay)
   - Don't depend on empty state

## 7. Implementation Guidelines

### 7.1 Code Style

Follow existing patterns in [`grasp01_nostr_relay.rs`](grasp-audit/src/specs/grasp01_nostr_relay.rs):

```rust
/// Test: <description>
///
/// Spec: Line X of ../grasp/01.md
/// Requirement: <exact or paraphrased requirement>
async fn test_name(client: &AuditClient) -> TestResult {
    TestResult::new(
        "test_name",
        "GRASP-01:nostr-relay:X",
        "Human-readable requirement description",
    )
    .run(|| async {
        // Test implementation
        Ok(())
    })
    .await
}
```

### 7.2 nostr-sdk 0.43 API Usage

**Field Access (NOT method calls):**
```rust
event.id          // ✅ Correct
event.tags        // ✅ Correct  
event.tags.iter() // ✅ Correct

event.id()        // ❌ Wrong (0.35 API)
```

**Tag Construction:**
```rust
Tag::custom(TagKind::custom("a"), vec!["30617:pubkey:repo-id"])  // ✅
Tag::identifier("repo-id")                                       // ✅
Tag::from_standardized(TagStandard::PublicKey { ... })          // ✅
```

**Event Building:**
```rust
client.event_builder(kind, content)
    .tag(tag1)
    .tag(tag2)
    .build(client.keys())?
```

### 7.3 Test Naming Convention

Pattern: `test_{action}_{subject}_{condition}`

Examples:
- `test_accept_issue_for_repo` (positive)
- `test_reject_orphan_issue` (negative)
- `test_accept_nested_comment_thread` (complex)

### 7.4 Error Handling

```rust
.run(|| async {
    // Create events
    let repo = client.create_repo_announcement("test").await
        .map_err(|e| format!("Failed to create repo: {}", e))?;
    
    // Send events
    client.send_event(repo.clone()).await
        .map_err(|e| format!("Failed to send to relay: {}", e))?;
    
    // Verify results
    let events = client.query(filter).await
        .map_err(|e| format!("Failed to query: {}", e))?;
    
    if events.is_empty() {
        return Err("Event not stored".to_string());
    }
    
    Ok(())
})
```

## 8. Test Data Patterns

### 8.1 Sample Event IDs
Use realistic hex event IDs:
```rust
"abc123def456789012345678901234567890abcd"  // 40 hex characters
```

### 8.2 Sample Pubkeys
Use proper npub format:
```rust
client.public_key().to_bech32()?  // Real key from client
```

### 8.3 Sample Repo IDs
Use test name + UUID:
```rust
format!("test-{}-{}", test_name, Timestamp::now().as_u64())
```

## 9. Acceptance Criteria

### 9.1 Code Quality

- ✅ All functions have doc comments
- ✅ No compiler warnings
- ✅ Follows existing code patterns
- ✅ Uses nostr-sdk 0.43 API correctly
- ✅ Proper error messages

### 9.2 Test Coverage

- ✅ All 7 test stubs implemented
- ✅ All NIP-34 event types covered
- ✅ All reference tag types tested
- ✅ Both positive and negative cases
- ✅ Edge cases documented

### 9.3 Passing Tests

- ✅ All tests pass against ngit-relay
- ✅ Tests properly isolated
- ✅ No flaky tests
- ✅ Clear failure messages

## 10. References

- **NIP-34:** `/persistent/dcdev/clones/nips/34.md` (Git Stuff)
- **NIP-10:** `/persistent/dcdev/clones/nips/10.md` (Threading)
- **NIP-22:** `/persistent/dcdev/clones/nips/22.md` (Comments)
- **Current Implementation:** [`grasp01_nostr_relay.rs:29-36`](grasp-audit/src/specs/grasp01_nostr_relay.rs:29-36)
- **Client Helpers:** [`client.rs:193-235`](grasp-audit/src/client.rs:193-235)
- **AGENTS.md:** Code patterns and testing guidelines

## 11. Next Steps

1. **Review this design document with user**
2. **Get approval or iterate on design**
3. **Switch to Code mode for implementation**
4. **Implement Phase 1 (Foundation)**
5. **Test against ngit-relay**
6. **Iterate through remaining phases**

## Appendix A: Test Flow Diagram

```
Event Reference Testing Flow
============================

┌─────────────────────────────────────────────────┐
│ Setup: Create Repo Announcement                 │
│ - Send kind 30617 with clone/relays tags       │
│ - Verify acceptance and storage                 │
└────────────────┬────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────┐
│ Test 1: Issues (kind 1621)                     │
│ ┌─────────────────────────────────────────────┐│
│ │ → Create issue with 'a' tag to repo         ││
│ │ → Send to relay                              ││
│ │ → Query back                                 ││
│ │ → Verify stored                              ││
│ └─────────────────────────────────────────────┘│
└────────────────┬────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────┐
│ Test 2: Patches (kind 1617)                    │
│ ┌─────────────────────────────────────────────┐│
│ │ → Create patch with 'a' tag to repo          ││
│ │ → Optionally thread with 'e' tag             ││
│ │ → Send and verify                            ││
│ └─────────────────────────────────────────────┘│
└────────────────┬────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────┐
│ Test 3: Pull Requests (kind 1618)              │
│ ┌─────────────────────────────────────────────┐│
│ │ → Create PR with 'a' tag and 'c' commit     ││
│ │ → Send and verify                            ││
│ └─────────────────────────────────────────────┘│
└────────────────┬────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────┐
│ Test 4: Comments (kind 1111 - NIP-22)          │
│ ┌─────────────────────────────────────────────┐│
│ │ Top-level:                                   ││
│ │   E/K/P → Issue                              ││
│ │   e/k/p → Issue (same as root)               ││
│ │ Nested:                                      ││
│ │   E/K/P → Issue (unchanged)                  ││
│ │   e/k/p → Parent Comment                     ││
│ └─────────────────────────────────────────────┘│
└────────────────┬────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────┐
│ Test 5: Status Updates (kinds 1630-1633)       │
│ ┌─────────────────────────────────────────────┐│
│ │ → Create status with 'e' tag to issue/PR    ││
│ │ → Test state transitions                     ││
│ └─────────────────────────────────────────────┘│
└────────────────┬────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────┐
│ Test 6: Negative Cases                         │
│ ┌─────────────────────────────────────────────┐│
│ │ → Orphan events (no references)              ││
│ │ → Invalid references                         ││
│ │ → Verify rejection                           ││
│ └──────────────────────────── ────────────────┘│
└─────────────────────────────────────────────────┘
```

## Appendix B: Helper Function Dependency Graph

```
Helper Functions
================

create_repo_announcement()  (exists in AuditClient)
         │
         ├─→ extract_repo_id()
         └─→ build_repo_atag()
                 │
                 ├─→ create_issue()
                 ├─→ create_patch()
                 ├─→ create_pull_request()
                 ├─→ create_comment()
                 └─→ create_status()
                         │
                         ├─→ send_and_verify_stored()
                         └─→ send_and_verify_rejected()
```
