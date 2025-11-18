# Test Implementation Lessons - GRASP-01 Compliance Suite

This document captures key lessons learned during the implementation of GRASP-01 compliance tests. Each entry documents what worked well, what to avoid, and patterns to follow for future tests.

---

## Test #3: test_reject_repo_announcement_missing_relays_tag

**Date:** November 5, 2025  
**Test Duration:** 45.997432ms  
**Status:** ✅ PASSED  
**Port Used:** 24965 (randomly assigned by test-ngit-relay.sh)

### Test Purpose

Validates GRASP-01 line 5 requirement: relays MUST reject repository announcements without a service URL in the relays tag.

### Key Learnings

1. **Pattern Consistency is Key**
   - Following the `test_reject_repo_announcement_missing_clone_tag` pattern significantly simplified implementation
   - When creating similar tests (rejection tests for missing required tags), reuse the proven pattern
   - Only swap out the tag being tested - keep all other structure identical

2. **nostr-sdk 0.43 API Usage**
   - Successfully used direct field access: `event.id` (not `event.id()`)
   - Tag creation pattern: `Tag::custom(TagKind::custom("relays"), vec![...])`
   - EventBuilder chaining: `EventBuilder::new(kind, content).tags(tags)`
   - All work correctly with no compilation issues

3. **Test Automation Workflow**
   - test-ngit-relay.sh handled all relay lifecycle management perfectly
   - Random port assignment (24965) avoided conflicts automatically
   - No manual Docker commands needed - script handles everything
   - Cleanup happens automatically on script exit

### What Worked Well

- **Minimal code changes:** Only needed to modify tag name from "clone" to "relays"
- **Fast test execution:** Sub-50ms duration indicates efficient test design
- **Clear test validation:** Event rejection verified by checking event not present in relay
- **Automated testing:** test-ngit-relay.sh provided seamless relay management

### What to Avoid

- Don't manually start relay containers - let test-ngit-relay.sh handle it
- Don't use `event.id()` method calls - nostr-sdk 0.43 uses fields
- Don't deviate from proven patterns without good reason
- Don't hard-code port numbers - use RELAY_URL env var

### Pattern to Follow

```rust
// Create repo announcement WITHOUT required tag
let tags = vec![
    // Include all other required tags EXCEPT the one being tested
    Tag::custom(
        TagKind::custom("clone"),
        vec!["https://example.com/repo.git"],
    ),
    // Missing: relays tag (the one we're testing)
];

// Build and publish event
let event = client.event_builder()
    .kind(Kind::GitRepoAnnouncement)
    .content("Test repo")
    .tags(tags)
    .build()?;

client.publish_expect_reject(&event).await?;
```

### Test Implementation Time

- Analysis: ~5 minutes (reviewing existing pattern)
- Implementation: ~10 minutes (copying pattern, modifying tag)
- Testing: ~2 minutes (ran via test-ngit-relay.sh)
- Total: ~17 minutes

### Next Test Recommendation

Continue with `test_accept_state_announcement_multiple_refs` - this will test that relays accept repository state announcements with multiple git refs (e.g., multiple branches and tags).

---

## Test #4: test_accept_valid_repo_state_announcement

**Date:** November 5, 2025
**Test Duration:** 148ms
**Status:** ✅ PASSED
**Commit:** ebdf177

### Test Purpose

Validates GRASP-01 lines 6-7 requirement: relays MUST accept valid repository state announcements (kind 30618) with required `d`, `maintainers`, and `r` tags.

### Key Learnings

1. **Kind 30618 Uses Different Tags Than Kind 30617**
   - Repository announcements (30617): `clone`, `relays` tags
   - Repository state announcements (30618): `d`, `maintainers`, `r` tags
   - Don't confuse the two - they serve different purposes
   - State announcements track git refs (branches/tags), repo announcements declare repository metadata

2. **Empty Content is Valid**
   - Repository state announcements use empty content (`""`)
   - All metadata is in the tags, not the content field
   - This is different from repo announcements which may have descriptive content

3. **Test Duration Significantly Longer**
   - Previous tests: ~46ms (rejection tests, publish and query)
   - This test: 148ms (3x longer)
   - Likely due to more complex tag verification (checking d, maintainers, r tags)
   - Additional tag content checks (`contains("refs/heads/main")`)

4. **Tag Structure for State Announcements**
   - `d` tag: Repository identifier (unique per repo)
   - `maintainers` tag: Nostr public key in bech32 format (npub)
   - `r` tag: Git reference like `refs/heads/main` or `refs/tags/v1.0`
   - All three are required for valid state announcement

### What Worked Well

- **Clear tag separation:** Using `Tag::identifier()` for `d` tag vs `Tag::custom()` for others
- **npub conversion:** Converting public key to bech32 format for maintainers tag
- **Comprehensive verification:** Checking all three required tags are present in stored event
- **Specific git ref format:** Using proper git reference format `refs/heads/main`

### What to Avoid

- Don't use content field for state announcements - keep it empty
- Don't confuse kind 30617 tags (`clone`, `relays`) with kind 30618 tags (`d`, `maintainers`, `r`)
- Don't use raw public key hex - convert to npub for maintainers tag
- Don't use shorthand ref names like "main" - use full format `refs/heads/main`

### Pattern to Follow

```rust
// Create kind 30618 repository state announcement
let repo_id = format!("test-repo-state-{}", timestamp);
let npub = client.public_key().to_bech32()?;

let event = client.event_builder(Kind::Custom(30618), "")
    .tag(Tag::identifier(&repo_id))  // d tag for repo identifier
    .tag(Tag::custom(TagKind::custom("maintainers"), vec![npub]))
    .tag(Tag::custom(TagKind::custom("r"), vec!["refs/heads/main".to_string()]))
    .build(client.keys())?;

// Publish and verify acceptance
client.send_event(event.clone()).await?;

// Query using kind, author, and identifier
let filter = Filter::new()
    .kind(Kind::Custom(30618))
    .author(client.public_key())
    .identifier(&repo_id);
    
let events = client.query(filter).await?;
```

### Test Implementation Time

- Analysis: ~8 minutes (understanding kind 30618 vs 30617 differences)
- Implementation: ~12 minutes (new pattern, different tags)
- Testing: ~3 minutes (first run, verification)
- Total: ~23 minutes

### Next Test Recommendation

Continue with `test_accept_state_announcement_multiple_refs` - straightforward extension of this test, just add more `r` tags for different git refs (branches, tags).

---

## Template for Future Entries

```markdown
## Test #N: test_name_here

**Date:** YYYY-MM-DD  
**Test Duration:** XXms  
**Status:** ✅ PASSED / ⚠️ PARTIAL / ❌ FAILED  
**Port Used:** XXXXX

### Test Purpose

Brief description of what this test validates from GRASP-01 spec.

### Key Learnings

1. **Learning Category**
   - Specific insight
   - Why it matters
   - How to apply it

### What Worked Well

- Bullet points of successful approaches

### What to Avoid

- Bullet points of pitfalls encountered

### Pattern to Follow

```rust
// Code example if applicable
```

### Test Implementation Time

Breakdown of time spent on different phases

### Next Test Recommendation

What test should come next and why
```

---

## Summary Statistics

**Tests Completed:** 3 rejection/validation tests  
**Average Test Duration:** ~46ms  
**Success Rate:** 100%  
**Pattern Reuse Rate:** High (tests 2-3 followed same pattern)

**Most Valuable Pattern:** Following existing test structure for similar test types