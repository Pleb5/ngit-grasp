# GRASP-01 Test Plan

**Date:** November 5, 2025  
**Status:** Planning Phase  
**Scope:** Complete test coverage for GRASP-01 Core Service Requirements

---

## Overview

This document outlines all tests needed to validate GRASP-01 compliance. Each test maps directly to requirements in `../grasp/01.md`.

**Test Strategy:**
1. Build tests against ngit-relay reference implementation FIRST
2. Each requirement = one or more test functions
3. All tests reference specific spec line numbers
4. Tests organized by spec sections

---

## Test Organization

```
grasp-audit/src/specs/
├── mod.rs                          # Export all test modules
├── nip01_smoke.rs                  # ✅ DONE - Basic relay functionality
├── grasp01_nostr_relay.rs          # NEW - Nostr relay requirements
├── grasp01_git_http.rs             # NEW - Git Smart HTTP requirements
└── grasp01_cors.rs                 # NEW - CORS requirements
```

---

## 1. NIP-01 Smoke Tests (✅ COMPLETE)

**File:** `grasp-audit/src/specs/nip01_smoke.rs`

**Status:** Already implemented and working

**Coverage:**
- ✅ WebSocket connection
- ✅ Send/receive events
- ✅ Subscriptions (REQ/CLOSE)
- ✅ Event validation (signatures, IDs)

**Note:** These are smoke tests only. We don't comprehensively test NIP-01 since rust-nostr already has 1000+ tests.

---

## 2. GRASP-01 Nostr Relay Tests (🔜 TO DO)

**File:** `grasp-audit/src/specs/grasp01_nostr_relay.rs`

**Spec Reference:** Lines 1-14 of `../grasp/01.md`

### Test Functions to Implement:

#### 2.1 Repository Announcement Acceptance

```rust
/// Test: Accept valid repository announcements
/// Spec: Lines 3-5
/// Requirement: MUST accept repo announcements listing service in clone & relays tags
async fn test_accept_valid_repo_announcement()
```

**Test Details:**
- Create kind 30617 event with valid tags
- Include service URL in both `clone` and `relays` tags
- Send to relay
- Verify acceptance (OK response)
- Query back to confirm stored

```rust
/// Test: Reject repo announcements not listing service (unless GRASP-05)
/// Spec: Line 5
/// Requirement: MUST reject announcements not listing service
async fn test_reject_repo_announcement_missing_clone_tag()
```

**Test Details:**
- Create kind 30617 event WITHOUT service in `clone` tag
- Send to relay
- Verify rejection (error response)
- Confirm not stored in relay

```rust
/// Test: Reject repo announcements not listing service in relays tag
/// Spec: Line 5
/// Requirement: MUST reject announcements not listing service in relays
async fn test_reject_repo_announcement_missing_relays_tag()
```

**Test Details:**
- Create kind 30617 event WITHOUT service in `relays` tag
- Send to relay
- Verify rejection
- Confirm not stored

#### 2.2 Repository State Announcement Acceptance

```rust
/// Test: Accept valid repository state announcements
/// Spec: Line 3
/// Requirement: MUST accept repo state announcements
async fn test_accept_valid_repo_state_announcement()
```

**Test Details:**
- First send valid kind 30617 (repo announcement)
- Then send kind 30618 (state announcement) with matching `d` tag
- Include `refs/heads/main` and `HEAD` tags
- Verify acceptance
- Query back to confirm

```rust
/// Test: Accept state announcement with multiple refs
/// Spec: Line 3
/// Requirement: MUST accept state announcements with multiple refs
async fn test_accept_state_announcement_multiple_refs()
```

**Test Details:**
- Send kind 30618 with multiple `refs/heads/*` tags
- Include `refs/tags/*` tags
- Verify all refs are stored

```rust
/// Test: Accept state announcement with no refs (stop tracking)
/// Spec: NIP-34 spec
/// Requirement: Support stopping state tracking
async fn test_accept_state_announcement_no_refs()
```

**Test Details:**
- Send kind 30618 with only `d` tag (no refs)
- Verify acceptance (allows author to stop tracking)

#### 2.3 Related Event Acceptance

```rust
/// Test: Accept events tagging accepted repo announcements
/// Spec: Lines 7-9
/// Requirement: MUST accept events that tag accepted repo announcements
async fn test_accept_event_tagging_repo_announcement()
```

**Test Details:**
- Create and accept kind 30617 (repo announcement)
- Create kind 1621 (issue) with `a` tag pointing to repo
- Verify issue is accepted

```rust
/// Test: Accept events tagged by repo announcements
/// Spec: Lines 7-9
/// Requirement: MUST accept events tagged by accepted announcements
async fn test_accept_event_tagged_by_repo()
```

**Test Details:**
- Create event (e.g., kind 1 note)
- Create kind 30617 that tags the note
- Verify note is accepted/retained

```rust
/// Test: Accept patches (kind 1617) for accepted repos
/// Spec: Lines 8-9
/// Requirement: MUST accept patches for accepted repos
async fn test_accept_patch_for_repo()
```

**Test Details:**
- Create kind 30617 repo announcement
- Create kind 1617 patch with `a` tag to repo
- Verify patch acceptance

```rust
/// Test: Accept pull requests (kind 1618) for accepted repos
/// Spec: Lines 8-9
/// Requirement: MUST accept PRs for accepted repos
async fn test_accept_pull_request_for_repo()
```

**Test Details:**
- Create kind 30617 repo announcement
- Create kind 1618 PR with `a` tag to repo
- Include required tags: `c` (commit), `clone`, etc.
- Verify PR acceptance

```rust
/// Test: Accept issues (kind 1621) for accepted repos
/// Spec: Lines 8-9
/// Requirement: MUST accept issues for accepted repos
async fn test_accept_issue_for_repo()
```

**Test Details:**
- Create kind 30617 repo announcement
- Create kind 1621 issue with `a` tag to repo
- Verify issue acceptance

```rust
/// Test: Accept replies to accepted patches/PRs/issues
/// Spec: Lines 8-9
/// Requirement: MUST accept replies to accepted events
async fn test_accept_reply_to_issue()
```

**Test Details:**
- Create kind 1621 issue
- Create NIP-22 comment (kind 1111) replying to issue
- Verify reply acceptance

#### 2.4 NIP-11 Relay Information

```rust
/// Test: Serve NIP-11 document at /.well-known/nostr.json
/// Spec: Line 11
/// Requirement: MUST serve NIP-11 document
async fn test_nip11_document_exists()
```

**Test Details:**
- HTTP GET to `/.well-known/nostr.json` or `https://domain/` with `Accept: application/nostr+json`
- Verify 200 response
- Verify valid JSON

```rust
/// Test: NIP-11 includes supported_grasps field
/// Spec: Line 12
/// Requirement: MUST list supported GRASPs as string array
async fn test_nip11_supported_grasps_field()
```

**Test Details:**
- Fetch NIP-11 document
- Verify `supported_grasps` field exists
- Verify it's a string array
- Verify includes "GRASP-01"
- Format check: each entry matches `GRASP-XX` pattern

```rust
/// Test: NIP-11 includes repo_acceptance_criteria field
/// Spec: Line 13
/// Requirement: MUST list repository acceptance criteria
async fn test_nip11_repo_acceptance_criteria_field()
```

**Test Details:**
- Fetch NIP-11 document
- Verify `repo_acceptance_criteria` field exists
- Verify it's a human-readable string
- Verify non-empty

```rust
/// Test: NIP-11 curation field handling
/// Spec: Line 14
/// Requirement: MUST include curation if curated, omit otherwise
async fn test_nip11_curation_field()
```

**Test Details:**
- Fetch NIP-11 document
- If `curation` field exists, verify it's a non-empty string
- Document behavior (present or absent is both valid)

#### 2.5 Event Rejection Policies

```rust
/// Test: MAY reject based on custom criteria
/// Spec: Line 6
/// Requirement: Document that custom rejection is allowed
async fn test_custom_rejection_allowed()
```

**Test Details:**
- This is a policy test, not a functional test
- Verify relay can reject for reasons like:
  - Pre-payment required
  - Quota exceeded
  - WoT filtering
  - Whitelist
  - SPAM prevention
- Document in test that this is implementation-specific

```rust
/// Test: MAY reject/delete for SPAM prevention
/// Spec: Line 10
/// Requirement: Generic SPAM prevention allowed
async fn test_spam_prevention_allowed()
```

**Test Details:**
- Document that relay may reject/delete for SPAM
- This is permissive, not mandatory
- Test should document the policy, not enforce specific behavior

---

## 3. GRASP-01 Git Smart HTTP Tests (🔜 TO DO)

**File:** `grasp-audit/src/specs/grasp01_git_http.rs`

**Spec Reference:** Lines 15-31 of `../grasp/01.md`

### Test Functions to Implement:

#### 3.1 Repository Serving

```rust
/// Test: Serve git repo at /<npub>/<identifier>.git
/// Spec: Line 17
/// Requirement: MUST serve git repo at correct path
async fn test_serve_git_repo_at_correct_path()
```

**Test Details:**
- Create kind 30617 announcement with `d` tag = "test-repo"
- Push git data to repository
- HTTP GET to `/<npub>/test-repo.git/info/refs?service=git-upload-pack`
- Verify 200 response
- Verify git smart HTTP response format

```rust
/// Test: Unauthenticated git-upload-pack (clone/fetch)
/// Spec: Line 17
/// Requirement: MUST allow unauthenticated clone/fetch
async fn test_unauthenticated_clone()
```

**Test Details:**
- Create and push repository
- Perform git clone without authentication
- Verify clone succeeds
- Verify repository contents match

```rust
/// Test: Repository only served for accepted announcements
/// Spec: Line 17
/// Requirement: Only serve repos with accepted announcements
async fn test_no_git_repo_without_announcement()
```

**Test Details:**
- Try to access `/<npub>/nonexistent.git/info/refs`
- Verify 404 response
- Verify no git data served

#### 3.2 Push Authorization

```rust
/// Test: Accept push matching latest state announcement
/// Spec: Line 19
/// Requirement: MUST accept pushes matching state announcement
async fn test_accept_push_matching_state()
```

**Test Details:**
- Create kind 30617 repo announcement
- Create kind 30618 state with `refs/heads/main` = commit A
- Attempt git push updating main to commit B (child of A)
- Verify push accepted
- Verify repository updated

```rust
/// Test: Reject push not matching state announcement
/// Spec: Line 19
/// Requirement: Implicit - only accept matching pushes
async fn test_reject_push_not_matching_state()
```

**Test Details:**
- Create kind 30618 state with `refs/heads/main` = commit A
- Attempt git push updating main to commit X (unrelated)
- Verify push rejected
- Verify repository unchanged

```rust
/// Test: Respect recursive maintainer set
/// Spec: Line 19
/// Requirement: MUST respect recursive maintainer set
async fn test_push_authorization_maintainer_set()
```

**Test Details:**
- Create repo announcement by user A
- Add user B to `maintainers` tag
- User B creates state announcement
- User B pushes matching state
- Verify push accepted
- Test recursion: B lists C as maintainer, C can push

```rust
/// Test: Reject push from non-maintainer
/// Spec: Line 19 (implicit)
/// Requirement: Only maintainers can push
async fn test_reject_push_from_non_maintainer()
```

**Test Details:**
- Create repo announcement by user A
- User B (not in maintainers) creates state announcement
- User B attempts push
- Verify push rejected

#### 3.3 HEAD Management

```rust
/// Test: Set HEAD per state announcement
/// Spec: Line 21
/// Requirement: MUST set HEAD when git data received
async fn test_set_head_from_state_announcement()
```

**Test Details:**
- Create kind 30618 with `HEAD = ref: refs/heads/develop`
- Push git data for develop branch
- Clone repository
- Verify HEAD points to develop (not main)

```rust
/// Test: Update HEAD when state changes
/// Spec: Line 21
/// Requirement: Update HEAD as soon as git data available
async fn test_update_head_when_state_changes()
```

**Test Details:**
- Initial state: HEAD = main
- Push new state: HEAD = develop
- Push git data for develop
- Verify HEAD updates to develop

#### 3.4 Pull Request Refs

```rust
/// Test: Accept push to refs/nostr/<event-id>
/// Spec: Line 23
/// Requirement: MUST accept pushes to PR refs
async fn test_accept_push_to_pr_ref()
```

**Test Details:**
- Create kind 1618 PR event
- Push to `refs/nostr/<pr-event-id>`
- Verify push accepted
- Verify ref exists in repository

```rust
/// Test: Reject PR ref if event has different tip
/// Spec: Line 23
/// Requirement: SHOULD reject if tip mismatch
async fn test_reject_pr_ref_tip_mismatch()
```

**Test Details:**
- Create kind 1618 PR with `c` tag = commit A
- Push to `refs/nostr/<pr-event-id>` with commit B
- Verify push rejected (or document if accepted)

```rust
/// Test: Delete PR ref if no event within 20 minutes
/// Spec: Line 23
/// Requirement: SHOULD delete orphaned PR refs
async fn test_delete_orphaned_pr_ref()
```

**Test Details:**
- Push to `refs/nostr/<event-id>`
- Wait 20+ minutes without sending kind 1618/1619 event
- Check if ref is deleted
- Note: This is SHOULD, not MUST - document behavior

```rust
/// Test: Keep PR ref if event exists
/// Spec: Line 23 (implicit)
/// Requirement: Keep ref if valid PR/update event exists
async fn test_keep_pr_ref_with_event()
```

**Test Details:**
- Push to `refs/nostr/<event-id>`
- Send kind 1618 PR event with matching `c` tag
- Wait 20+ minutes
- Verify ref still exists

#### 3.5 Git Protocol Features

```rust
/// Test: Advertise allow-reachable-sha1-in-want
/// Spec: Line 25
/// Requirement: MUST advertise and serve capability
async fn test_advertise_reachable_sha1_in_want()
```

**Test Details:**
- GET `/repo.git/info/refs?service=git-upload-pack`
- Parse git protocol response
- Verify `allow-reachable-sha1-in-want` in capabilities

```rust
/// Test: Advertise allow-tip-sha1-in-want
/// Spec: Line 25
/// Requirement: MUST advertise and serve capability
async fn test_advertise_tip_sha1_in_want()
```

**Test Details:**
- GET `/repo.git/info/refs?service=git-upload-pack`
- Parse git protocol response
- Verify `allow-tip-sha1-in-want` in capabilities

```rust
/// Test: Serve available OIDs by SHA1
/// Spec: Line 25
/// Requirement: MUST serve available OIDs
async fn test_serve_oids_by_sha1()
```

**Test Details:**
- Push repository with known commits
- Perform git fetch with specific SHA1 want
- Verify server provides the object

#### 3.6 Web Interface

```rust
/// Test: Serve webpage at repo endpoint
/// Spec: Line 27
/// Requirement: SHOULD serve webpage with links
async fn test_serve_webpage_at_repo_endpoint()
```

**Test Details:**
- HTTP GET to `/<npub>/<identifier>.git` with `Accept: text/html`
- Verify HTML response (not git protocol)
- Verify links to git nostr clients (optional check)

```rust
/// Test: Serve 404 for non-existent repos
/// Spec: Line 27
/// Requirement: SHOULD serve 404 for missing repos
async fn test_serve_404_for_missing_repo()
```

**Test Details:**
- HTTP GET to `/<npub>/nonexistent.git` with `Accept: text/html`
- Verify 404 response
- Verify helpful error message

---

## 4. GRASP-01 CORS Tests (🔜 TO DO)

**File:** `grasp-audit/src/specs/grasp01_cors.rs`

**Spec Reference:** Lines 32-40 of `../grasp/01.md`

### Test Functions to Implement:

```rust
/// Test: Access-Control-Allow-Origin on all responses
/// Spec: Line 35
/// Requirement: MUST set ACAO: * on ALL responses
async fn test_cors_allow_origin_on_all_responses()
```

**Test Details:**
- Test multiple endpoints:
  - WebSocket upgrade (Nostr relay)
  - Git HTTP endpoints (info/refs, upload-pack, receive-pack)
  - NIP-11 endpoint
  - Web interface
- Verify ALL include `Access-Control-Allow-Origin: *`

```rust
/// Test: Access-Control-Allow-Methods on all responses
/// Spec: Line 36
/// Requirement: MUST set ACAM: GET, POST on ALL responses
async fn test_cors_allow_methods_on_all_responses()
```

**Test Details:**
- Test same endpoints as above
- Verify ALL include `Access-Control-Allow-Methods: GET, POST`

```rust
/// Test: Access-Control-Allow-Headers on all responses
/// Spec: Line 37
/// Requirement: MUST set ACAH: Content-Type on ALL responses
async fn test_cors_allow_headers_on_all_responses()
```

**Test Details:**
- Test same endpoints as above
- Verify ALL include `Access-Control-Allow-Headers: Content-Type`

```rust
/// Test: OPTIONS requests return 204 No Content
/// Spec: Line 38
/// Requirement: MUST respond to OPTIONS with 204
async fn test_cors_options_request()
```

**Test Details:**
- Send OPTIONS request to various endpoints
- Verify 204 No Content response
- Verify CORS headers present on OPTIONS response

```rust
/// Test: CORS headers on error responses
/// Spec: Line 35 (ALL responses)
/// Requirement: CORS headers even on errors
async fn test_cors_headers_on_error_responses()
```

**Test Details:**
- Trigger various error conditions:
  - 404 not found
  - 403 forbidden (unauthorized push)
  - 400 bad request
- Verify CORS headers present on all error responses

```rust
/// Test: Preflight request handling
/// Spec: Lines 35-38
/// Requirement: Full preflight support for web clients
async fn test_cors_preflight_request()
```

**Test Details:**
- Send OPTIONS with Origin and Access-Control-Request-Method headers
- Verify proper preflight response
- Verify subsequent actual request succeeds

---

## Implementation Priority

### Phase 1: Core Nostr Relay Tests (Complete these first)
1. ✅ NIP-01 smoke tests (DONE)
2. Repository announcement acceptance/rejection
3. Repository state announcement acceptance
4. NIP-11 relay information document
5. Related event acceptance (issues, patches, PRs)

### Phase 2: Git Smart HTTP Tests
1. Repository serving at correct paths
2. Unauthenticated clone/fetch
3. Push authorization and maintainer sets
4. HEAD management
5. Git protocol features (SHA1 capabilities)

### Phase 3: Advanced Git Features
1. Pull request refs (refs/nostr/<event-id>)
2. PR ref lifecycle (creation, validation, deletion)
3. Web interface (optional)

### Phase 4: CORS Tests
1. CORS headers on all endpoints
2. OPTIONS request handling
3. Preflight requests
4. Error response CORS

---

## Test Execution Plan

### Against ngit-relay Reference Implementation

```bash
# 1. Start ngit-relay
cd ../ngit-relay
docker-compose up -d

# 2. Run tests
cd ../ngit-grasp/grasp-audit
cargo test --lib  # Unit tests

# Run integration tests by category
cargo test --test grasp01_nostr_relay
cargo test --test grasp01_git_http
cargo test --test grasp01_cors

# 3. Run full audit
cargo run -- --url ws://localhost:8081
```

### Test Data Requirements

For comprehensive testing, we need:
- Multiple test keypairs (maintainers, contributors, non-maintainers)
- Sample git repositories with known commit history
- Valid NIP-34 event templates
- Test data for edge cases

---

## Success Criteria

- [ ] All GRASP-01 requirements have corresponding tests
- [ ] All tests reference specific spec line numbers
- [ ] All tests pass against ngit-relay reference implementation
- [ ] Tests are organized logically by spec sections
- [ ] Clear test output shows what requirement is being tested
- [ ] Tests can be run individually or as full suite
- [ ] Documentation explains what each test validates

---

## Notes

### Spec Line Number References

When implementing tests, use this format:

```rust
/// Test: <Short description>
/// Spec: Lines X-Y of ../grasp/01.md
/// Requirement: <Exact quote or paraphrase from spec>
async fn test_name() {
    // Implementation
}
```

### Test Naming Convention

- `test_accept_*` - Tests that verify acceptance of valid input
- `test_reject_*` - Tests that verify rejection of invalid input
- `test_serve_*` - Tests that verify correct serving of data
- `test_cors_*` - Tests for CORS functionality
- `test_nip11_*` - Tests for NIP-11 relay information

### Edge Cases to Consider

1. **Concurrent updates** - Multiple maintainers pushing simultaneously
2. **Large repositories** - Performance with large git data
3. **Invalid git data** - Corrupted pack files, invalid refs
4. **Event ordering** - State announcement before repo announcement
5. **Deleted events** - What happens when announcement is deleted?
6. **Network failures** - Partial push, interrupted clone
7. **Recursive maintainers** - Deep maintainer chains, circular references

---

**Next Steps:**
1. Implement Phase 1 tests (Nostr relay)
2. Run against ngit-relay to validate
3. Fix any failing tests
4. Move to Phase 2 (Git HTTP)
5. Iterate until all tests pass

