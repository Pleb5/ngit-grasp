# Deletion Request Support (NIP-09)

**Status:** 🚧 **PLANNED - NOT YET IMPLEMENTED** 🚧

This document describes the planned architecture for NIP-09 deletion request support in ngit-grasp. Implementation is scheduled for 6-week phased rollout. See `work/active-issues/deletion-request-support.md` for implementation tracking.

---

## Overview

ngit-grasp will implement optional support for NIP-09 deletion requests, allowing repository owners to remove their repositories from the relay while providing safeguards against the "left-pad problem" through configurable archival behavior.

## The Left-Pad Problem

The "left-pad problem" refers to a 2016 incident where a critical npm package was unpublished, breaking thousands of dependent projects. In the context of decentralized Git hosting, this translates to:

**Scenario:** A popular repository with many PRs, issues, and community contributions gets deleted by its owner. All dependent work (forks, patches, discussions) becomes inaccessible, potentially breaking workflows and losing community knowledge.

**Our Solution:** The `deletion-request-disrespector` configuration option allows operators to run **archival relays** that preserve deleted content, ensuring community work survives repository deletion while still respecting deletion requests on standard relays.

## Architecture

### Three-Database Design

The deletion system uses three separate data stores:

```
┌─────────────────────────────────────────────────────────┐
│                    Main Database                        │
│         (Live events - actively served)                 │
│    LMDB/NostrDB/Memory backend                         │
└─────────────────────────────────────────────────────────┘
                         ↓ deletion request
┌─────────────────────────────────────────────────────────┐
│                 Holding Database                        │
│    (Archived events - recovery window)                 │
│    Same backend type as main                           │
│    Retention: configurable (default 90 days)           │
└─────────────────────────────────────────────────────────┘
                         ↓ expiry
┌─────────────────────────────────────────────────────────┐
│              Permanent Deletion                         │
│         (Events removed from holding DB)                │
└─────────────────────────────────────────────────────────┘

                    Git Data Flow
┌─────────────────────────────────────────────────────────┐
│          Git Repository (Live)                          │
│    <git_data_path>/<npub>/<identifier>.git             │
└─────────────────────────────────────────────────────────┘
                         ↓ deletion request
┌─────────────────────────────────────────────────────────┐
│           Archive Filesystem                            │
│    .archive/<npub>/<identifier>-<timestamp>.tar.gz     │
│    + metadata.json                                     │
│    Retention: configurable (default 90 days)           │
└─────────────────────────────────────────────────────────┘
                         ↓ expiry
┌─────────────────────────────────────────────────────────┐
│              Permanent Deletion                         │
│         (Archive files removed)                         │
└─────────────────────────────────────────────────────────┘
```

### Why Three Stores?

1. **Main Database:** Fast queries, clean data model (deleted = gone)
2. **Holding Database:** Recovery mechanism, prevents accidental permanent deletion
3. **Archive Filesystem:** Git data backup, compressed storage

## Deletion Flow

### Standard Mode (Respects Deletions)

```
1. Kind 5 deletion request arrives
   ↓
2. Validate: author matches announcement pubkey
   ↓
3. Query dependent events (PRs, issues, patches, comments)
   ↓
4. Archive git repository to .archive/<npub>/<identifier>-<timestamp>.tar.gz
   ↓
5. Move events to holding database:
   - Announcement
   - All dependent events (cascade delete)
   ↓
6. Delete events from main database
   ↓
7. Events no longer served in queries
   ↓
8. Background task (daily):
   - Check holding database for expired entries
   - Delete events older than retention period
   - Delete corresponding archive files
```

### Archival Mode (Disrespector)

When `deletion_request_disrespector = true`:

```
1. Kind 5 deletion request arrives
   ↓
2. Store deletion request event in main database
   ↓
3. Do NOT process deletion
   ↓
4. Repository and events remain fully accessible
   ↓
Result: Archival relay preserves all content
```

**Implementation Note:** We need to verify that `nostr-relay-builder` doesn't automatically process deletion requests at the relay library level. If it does, we'll need to override or disable this behavior when disrespector mode is enabled. This will be investigated in Phase 6.

## Recovery Mechanism

The holding database enables **accidental deletion recovery**:

```
Scenario: Owner deletes repository, then changes their mind

1. Owner publishes new announcement with same identifier
   ↓
2. System detects matching entry in holding database
   ↓
3. Check: Is entry within retention period?
   ↓
4. If YES:
   - Extract git data from archive tar.gz
   - Restore to <git_data_path>/<npub>/<identifier>.git
   - Move events from holding DB → main DB
   - Re-run acceptance policy (should now pass)
   - Delete archive records
   - Return: "Restored X events"
   ↓
5. If NO (expired):
   - Process as new repository
   - Return: "New repository created"
```

## Cascade Deletion Strategy

When a repository announcement is deleted, we cascade delete **all dependent events**:

### Rationale

**Decision:** Delete all dependent events, not just owner's events.

**Why?**
1. **Deletion Intent:** Owner wants repository gone - includes all associated data
2. **Data Integrity:** Orphaned PRs/issues without context are confusing
3. **Consistency:** Matches user expectation that "delete repo" means "delete everything"
4. **Recovery Available:** Holding database preserves everything for recovery window

**Community Protection:**
- Archival relays (`deletion_request_disrespector = true`) preserve community work
- 90-day default retention allows time for recovery
- Other maintainers can continue repository with different identifier

### Event Cascade Hierarchy

```
Repository Announcement (30617)
    ↓ deleted
├─→ State Events (30618) - same identifier
├─→ Pull Requests (1618) - tag via 'a'
├─→ Issues (1621) - tag via 'a'  
├─→ Patches (1617) - tag via 'a'
    ↓ all above deleted
    └─→ Comments (1111) - tag via 'e'
        ├─→ Reactions (7) - tag via 'e'
        └─→ Text Notes (1) - tag via 'e'
```

**Implementation:** Recursive dependency graph traversal starting from announcement.

## Multi-Maintainer Scenarios

### Challenge

Multiple maintainers can have announcements for the same `identifier`:
- `npub1alice.../my-repo` 
- `npub1bob.../my-repo`

Git data is synced between their repositories. When ONE maintainer deletes, what happens?

### Solution: Graph-Based Retention Algorithm

```
When npub1alice deletes her announcement:

1. Archive HER git directory:
   .archive/npub1alice.../my-repo-<timestamp>.tar.gz

2. Query all events that referenced her announcement

3. Re-evaluate each event through acceptance policy:
   - WITHOUT alice's announcement
   - WITH bob's announcement still present
   
4. Build retention graph:
   Event A kept because:
     - References bob's announcement ✓
   Event B kept because:
     - References Event A ✓
   Event C orphaned because:
     - Only referenced alice's announcement ✗

5. Delete orphaned events, keep retained events

6. Handle circular dependencies:
   - Event X kept because references Event Y
   - Event Y kept because references Event X
   - Neither has external anchor → both deleted
```

### Graph Algorithm Details

**Topological Traversal:**
1. Start from remaining announcements (roots)
2. Traverse dependency edges (a/e/q tags)
3. Mark reachable events as "keep"
4. Mark unreachable events as "delete"

**Max Depth Limit:**
- Configurable maximum traversal depth (prevent infinite loops)
- Default: 100 levels
- Note: Will analyze edge cases where this limit matters

**Complexity:**
- Deletion events are rare (not performance critical)
- Compute on-demand when deletion request arrives
- No pre-computation or caching needed at current scale
- Note: Will analyze large-scale scenarios in future

## Configuration

### deletion_request_disrespector

**Type:** `bool`  
**Default:** `false` (respects deletion requests)  
**CLI:** `--deletion-request-disrespector`  
**Env:** `NGIT_DELETION_REQUEST_DISRESPECTOR`

**Description:**
When `true`, relay ignores deletion requests and acts as an archival server. Critical for preventing left-pad scenarios by ensuring at least some relays preserve deleted content.

**Use Cases:**
- Community archival relays
- Research/historical preservation
- Backup/mirror relays
- GRASP-05 archive mode (future)

### archive_retention_secs

**Type:** `u64`  
**Default:** `7776000` (90 days in seconds)  
**CLI:** `--archive-retention-secs`  
**Env:** `NGIT_ARCHIVE_RETENTION_SECS`

**Description:**
How long to retain archived events and git data before permanent deletion. Provides recovery window for accidental deletions.

**Recommended Values:**
- Development/Testing: `5` seconds (fast test cycles)
- Staging: `300` seconds (5 minutes)
- Production: `7776000` seconds (90 days, default)
- Archival Relay: `31536000` seconds (1 year) or higher

**Notes:**
- Configurable in seconds for testing flexibility
- Background cleanup task runs daily (configuration for testing interval TBD in Phase 6)
- Check occurs on startup to handle offline periods
- **Testing Challenge:** Daily cleanup doesn't work well with 3-5 second retention for tests - alternative timing strategy needed

## NIP-11 Advertisement

Deletion support is **conditionally advertised** in NIP-11 relay information:

- **When `deletion_request_disrespector = false`:** Include `"deletion"` in supported NIPs array
- **When `deletion_request_disrespector = true`:** Do NOT include `"deletion"` (archival mode doesn't honor deletions)

This allows clients to discover whether a relay respects deletion requests.

## Documentation Updates

When implementation is complete, the following documentation will be updated:

**README.md:**
- Add NIP-09 deletion request support to feature list
- Document cascade deletion behavior
- Update "Delete Events" roadmap section (mark as implemented)
- Link to this explanation document

**docs/explanation/architecture.md:**
- Add deletion request system overview
- Document cascade deletion strategy
- Reference this document for detailed information

## Implementation Status

**Phase 1: Core Deletion + Simple Cascade** 🔄 (Planned)
- Config options
- Holding database
- Kind 5 processing
- Simple cascade delete

**Phase 2: Git Archival & Cleanup** 🔄 (Planned)
- Archive tar.gz creation
- Background cleanup task
- Metadata storage

**Phase 3: Multi-Maintainer Graph Algorithm** 🔄 (Planned)
- Dependency graph building
- Re-evaluation through acceptance policy
- Circular dependency detection

**Phase 4: Recovery Mechanism** 🔄 (Planned)
- Re-announcement detection
- Archive restoration
- Event recovery from holding DB

**Phase 5: Extended Cascade Deletion** 🔄 (Planned)
- Patches (1617) cascade
- Issues (1621) cascade  
- PR Updates (1619) cascade
- Full event type coverage

**Phase 6: Analysis & Edge Cases** 🔄 (Planned)
- Background cleanup timing strategy (daily doesn't work with 3-second test retention)
- rust-nostr deletion behavior investigation (does relay builder auto-process deletions?)
- Author validation enforcement and testing
- Max depth edge case analysis
- Large-scale testing
- Race condition investigation
- Lock strategy finalization

## Security Considerations

### Validation

1. **Author Matching:** Deletion request pubkey MUST match announcement pubkey
   - **Critical Requirement:** We ONLY honor deletion requests where the deletion request author is the same as the deleted event author
   - This prevents malicious actors from deleting other people's repositories
   - Enforced at validation layer before any deletion processing
2. **Signature Verification:** Handled by nostr-relay-builder (already implemented)
3. **Timestamp Check:** For addressable events, delete versions up to deletion `created_at`

### Attack Vectors

**DoS via Deletion Spam:**
- Mitigation: Deletion requests only processed if announcement exists
- Mitigation: Idempotent (deleting already-deleted announcement is no-op)

**Archive Disk Exhaustion:**
- Mitigation: Background cleanup enforces retention limits
- Mitigation: Compressed tar.gz archives
- Mitigation: Configurable retention period

**Recovery Abuse:**
- Mitigation: Recovery only within retention window
- Mitigation: Must be original owner (pubkey match)
- Mitigation: Normal announcement validation applies

## Monitoring & Metrics

**Prometheus Metrics (Planned):**
- `ngit_deletion_requests_total` - Count of deletion requests received
- `ngit_deletion_requests_processed` - Count actually processed (disrespector mode = 0)
- `ngit_holding_database_events` - Current event count in holding DB
- `ngit_holding_database_size_bytes` - Holding DB disk usage
- `ngit_archive_files_total` - Count of archive tar.gz files
- `ngit_archive_size_bytes` - Total archive disk usage
- `ngit_recoveries_total` - Count of successful recoveries
- `ngit_permanent_deletions_total` - Count of events permanently deleted (post-retention)

## Testing Strategy

### Unit Tests
- Kind 5 validation and parsing
- Author matching logic
- Cascade dependency query
- Graph traversal algorithm
- Recovery detection

### Integration Tests
- Full deletion workflow (3-5 second retention)
- Multi-maintainer scenarios
- Recovery mechanism
- Disrespector mode behavior
- Background cleanup timing (mocked)

### Audit Tests
- NIP-09 compliance validation
- Event re-submission after deletion (rejected)
- Deletion request event itself (stored)
- Archival mode relay behavior

## Related Documentation

- **NIP-09 Specification:** `/persistent/dcdev/clones/nips/09.md`
- **Architecture Overview:** `docs/explanation/architecture.md`
- **Configuration Reference:** `docs/reference/configuration.md`
- **Roadmap:** `README.md` lines 198-206

## Future Enhancements

### GRASP-05 Archive Mode
Once GRASP-05 is specified, `deletion_request_disrespector` mode can form the foundation for archive relay requirements.

### Selective Disrespect
Allow configuration to disrespect deletions only for specific criteria:
- Popular repositories (e.g., >N PRs)
- Repositories with community contributions
- Specific identifiers (allowlist)

### Distributed Archive Network
Coordinate between archival relays to ensure redundant preservation of deleted content.

### Recovery Notifications
Notify repository owner when content is recovered from holding database, allowing them to confirm or re-delete.

## Conclusion

The deletion request system balances three competing needs:

1. **User Agency:** Owners can delete their repositories
2. **Community Protection:** Archival relays prevent left-pad scenarios  
3. **Recovery Grace Period:** Holding database prevents accidental permanent deletion

By making deletion behavior **configurable** rather than mandatory, we enable a heterogeneous relay network where some relays respect deletions (user privacy) while others preserve content (community resilience).
