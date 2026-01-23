# Announcements Purgatory Design

## Problem Statement

**Primary problem:** Empty bare git repos mislead clients into thinking we host content.

When an announcement arrives, we must create the bare repo immediately (so git pushes can succeed). But if no git data ever arrives, we serve an empty repo and its announcement indefinitely. Clients see the announcement, try to clone, and get nothing. This is misleading.

**Secondary problem:** Sync downloads refs to deleted repos.

When a repo expires or is cleaned up, sync may still try to download state event refs to it. We need announcements to remain in a holding state until git data proves the repo has content worth serving.

## Solution Overview

New announcements go to **purgatory** instead of being immediately accepted:

1. **Announcement arrives** - Create bare repo immediately, add announcement to purgatory
2. **Git data arrives** - Promote announcement from purgatory to active (now served to clients)
3. **No git data before expiry** - Delete bare repo, discard announcement (never served)

This ensures we only serve announcements for repos that actually have content.

## Key Design Decisions

### 1. Bare Repo Created Immediately

**Decision:** Create the bare git repo when announcement enters purgatory.

**Why:** Git pushes may arrive at any time. Without a repo, pushes fail.

**Consequence:** We allocate disk space for repos that may expire unused. Must delete repos on expiry.

### 2. Git Data Triggers Promotion

**Decision:** Git data arrival promotes the announcement to active status.

**Why:** Git data proves the repository has content. State events alone don't prove content exists - they could reference empty repos.

**Where:** Promotion happens in the git receive path after successful push/fetch with data.

### 3. Replacement Announcements Skip Purgatory

**Decision:** Announcements replacing an existing active announcement are accepted immediately.

**Why:** The repository is already proven active with content.

**How:** Check if active announcement exists for `(pubkey, identifier)` before routing to purgatory.

### 4. Expiry Extension (Two Places)

**Decision:** Extend purgatory announcement expiry in two scenarios:

| Trigger | Location | Why |
|---------|----------|-----|
| State event arrives | `StatePolicy::process_state_event()` | Repo is actively receiving metadata |
| Git auth extends state event | `src/git/auth.rs` | Repo is actively receiving git data |

**Why:** Prevents premature expiry during slow sync operations or multi-step pushes.

### 5. State Events Consider Purgatory Announcements

**Decision:** When validating state events, check purgatory announcements for authorization.

**Why:** State events may arrive before git data promotes the announcement. They still need authorization from the announcement's maintainer set.

## Data Structure

```rust
// Key: (owner pubkey, identifier) - identifier alone is NOT unique
announcement_purgatory: Arc<DashMap<(PublicKey, String), AnnouncementPurgatoryEntry>>

pub struct AnnouncementPurgatoryEntry {
    pub event: Event,
    pub identifier: String,
    pub owner: PublicKey,
    pub repo_path: PathBuf,
    pub created_at: Instant,
    pub expires_at: Instant,
}
```

**Indexed by `(pubkey, identifier)`** because identifier is not unique across different owners.

## Flows

### New Announcement Flow

```
Announcement arrives
    |
    v
Is there an active announcement for (pubkey, identifier)?
    |
    +-- YES --> Accept immediately (replacement)
    |
    +-- NO --> Create bare repo
               Add to purgatory
               Return OK to client (but don't serve)
```

### Git Data Arrival Flow

```
Git push/fetch completes with data
    |
    v
Is there a purgatory announcement for (pubkey, identifier)?
    |
    +-- YES --> Promote to active (move to database)
    |           Now served to clients
    |
    +-- NO --> Normal processing
```

### State Event Arrival Flow

```
State event arrives
    |
    v
Is there an active announcement?
    |
    +-- YES --> Normal validation
    |
    +-- NO --> Check purgatory for announcement
               |
               +-- Found --> Validate against purgatory announcement
               |             Extend purgatory expiry
               |             State event goes to state purgatory
               |
               +-- Not found --> Reject or state purgatory
```

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| Git data before announcement | Push fails (no repo exists) |
| Announcement expires, no git data | Delete bare repo, discard announcement |
| State expires, announcement in purgatory | Announcement keeps its own expiry |
| Multiple owners, same identifier | Each tracked separately by `(pubkey, identifier)` |
| **Newer announcement replaces older (same pubkey)** | Replace purgatory entry, extend expiry |
| **Newer announcement changes services (unacceptable)** | Clear older announcement from purgatory for that `(pubkey, identifier)` |
| Deletion event for purgatory announcement | Remove from purgatory, delete bare repo |

## Purgatory Exit Conditions

An announcement leaves purgatory via:

| Exit | Trigger | Action |
|------|---------|--------|
| **Promotion** | Git data arrives | Move to database, serve to clients |
| **Expiry** | Timeout | Delete bare repo, discard |
| **Deletion** | Kind 5 event | Delete bare repo, discard |
| **Replacement** | Newer announcement (same pubkey, identifier) | Replace entry |
| **Service change** | Newer announcement no longer lists our service | Discard old entry |

## Integration Points

| File | Change |
|------|--------|
| `src/purgatory/mod.rs` | Add `announcement_purgatory` store |
| `src/purgatory/types.rs` | Add `AnnouncementPurgatoryEntry` |
| `src/nostr/policy/announcement.rs` | Route new announcements to purgatory |
| `src/git/receive.rs` | Promote on git data arrival |
| `src/git/auth.rs` | Extend purgatory expiry when extending state event expiry |
| `src/nostr/policy/state.rs` | Check purgatory for authorization |

## Testing

- Announcement to purgatory, git data promotes it
- Announcement expires without git data (repo deleted)
- State event extends purgatory expiry
- Git auth extends purgatory expiry
- Newer announcement replaces older in purgatory
- Service change clears purgatory entry
- `(pubkey, identifier)` indexing with multiple owners

## Risks

| Risk | Mitigation |
|------|------------|
| Disk exhaustion from purgatory repos | Short expiry, monitor purgatory size |
| Race between promotion and expiry | Atomic operations |
| Sync re-fetching expired events | Track expired event IDs |
