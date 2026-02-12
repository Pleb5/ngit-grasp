# Announcements Purgatory Design

## Problem Statement

**Primary problem:** Serving announcement events alongside empty bare git repos misleads clients into thinking we host content.

When an announcement arrives, we must create the bare repo immediately (so git pushes can succeed). But if no git data ever arrives, we serve an empty repo and its announcement indefinitely. Clients see the announcement, try to clone, and get nothing. This is misleading.

**Secondary problem:** Sync downloads events for repos that may never have content.

Without purgatory, sync would fetch all L2/L3 events (patches, issues, etc.) for announcements that may never receive git data. This wastes bandwidth and creates orphaned events.

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

**Decision:** Extend purgatory announcement expiry (reset the 30-minute protocol timer) in two scenarios:

| Trigger                      | Location                             | Why                                 |
| ---------------------------- | ------------------------------------ | ----------------------------------- |
| State event arrives          | `StatePolicy::process_state_event()` | Repo is actively receiving metadata |
| Git auth extends state event | `src/git/auth.rs`                    | Repo is actively receiving git data |

**Why:** Prevents premature expiry during slow sync operations or multi-step pushes. The protocol's 30-minute expiry is intended for abandoned repositories, not active ones receiving data.

### 5. Authorization Must Check Purgatory Announcements

**Decision:** When validating state events or git operations, check purgatory announcements in addition to the database.

**Why:** State events and git pushes may arrive before git data promotes the announcement. They still need authorization from the announcement's maintainer set.

**Where:** `fetch_repository_data()` and related authorization functions must query both DB and purgatory.

### 6. Sync Only State Events for Purgatory Announcements

**Decision:** Purgatory announcements trigger sync for state events only, not other L2/L3 events (patches, issues, PRs, etc.).

**Why:** Other L2/L3 events would be rejected anyway (no promoted announcement in DB). Syncing them wastes bandwidth and creates work for announcements that may never promote.

**How:** Sync uses a `SyncLevel` concept - `Full` for promoted repos, `StateOnly` for purgatory. On promotion, upgrade to `Full`.

### 7. Soft Expiry Preserves Event Without Bare Repo

**Decision:** When a purgatory announcement expires (30 minutes per protocol spec), delete the bare repo but retain the announcement event for an extended period (e.g., 24h).

**Why the protocol specifies 30 minutes:** The grasp protocol defines a 30-minute expiry for announcement events to ensure clients don't indefinitely cache stale repository information.

**Why we implement soft expiry:** The protocol's 30-minute expiry creates a sync/storage problem. Without soft expiry, we'd either:

- Add expired announcements to `failed_events` and permanently reject future state events (losing potential revival when state events arrive late)
- Re-fetch the announcement event repeatedly on every sync cycle (wasting bandwidth and creating unnecessary sync traffic)

**Behavior during soft expiry:**

- Bare repo is deleted (saves disk space, respects protocol expiry)
- Announcement event retained in purgatory with `soft_expired` flag
- Sync continues requesting state events (same as active purgatory)
- If state event arrives: recreate bare repo, clear `soft_expired`, extend expiry
- If announcement republished directly to us: treat as fresh arrival
- After extended expiry: fully remove from purgatory

**In summary:** Soft expiry is an implementation optimization that prevents us from constantly re-syncing announcement events or permanently blocking repositories that receive delayed state events.

## Data Structure

```rust
// Key: (owner pubkey, identifier) - identifier alone is NOT unique
announcement_purgatory: Arc<DashMap<(PublicKey, String), AnnouncementPurgatoryEntry>>

pub struct AnnouncementPurgatoryEntry {
    pub event: Event,
    pub identifier: String,
    pub owner: PublicKey,
    pub repo_path: PathBuf,
    pub relays: HashSet<String>,  // For sync registration
    pub created_at: Instant,
    pub expires_at: Instant,
    pub soft_expired: bool,       // Bare repo deleted, event retained
}
```

**Indexed by `(pubkey, identifier)`** because identifier is not unique across different owners. Lookups are primarily from nostr events which have pubkey and identifier readily available.

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

| Scenario                                               | Behavior                                                                                                |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| Git data before announcement                           | Push fails (no repo exists)                                                                             |
| Announcement expires, no git data                      | Delete bare repo, set `soft_expired` flag, retain event for extended period                             |
| Soft-expired announcement fully expires                | Remove from purgatory entirely                                                                          |
| State event arrives for soft-expired announcement      | Recreate bare repo, clear `soft_expired`, extend expiry                                                 |
| State expires, announcement in purgatory               | Announcement keeps its own expiry                                                                       |
| Multiple owners, same identifier                       | Each tracked separately by `(pubkey, identifier)`                                                       |
| **Newer announcement replaces older (same pubkey)**    | Replace purgatory entry, extend expiry, and state event expiry                                          |
| **Newer announcement changes services (unacceptable)** | Clear older announcement from purgatory, delete bare repo, remove state events from purgatory if exists |
| Deletion event for purgatory announcement              | Remove from purgatory, delete bare repo                                                                 |

## Purgatory Lifecycle

An announcement progresses through purgatory states:

```
                    ┌─────────────────────────────────────┐
                    │                                     │
                    v                                     │
Announcement ──> ACTIVE ──────────────────────────────────┤
  arrives        (bare repo exists)                       │
                    │                                     │
                    ├── Git data ──> PROMOTED (exit)      │
                    │                                     │
                    ├── Deletion ──> REMOVED (exit)       │
                    │                                     │
                    v                                     │
               SOFT_EXPIRED ──────────────────────────────┘
               (bare repo deleted,        ^
                event retained)           │
                    │                     │
                    ├── State event arrives (revival)
                    │
                    └── Extended expiry ──> REMOVED (exit)
```

| Exit               | Trigger                                      | Action                                        |
| ------------------ | -------------------------------------------- | --------------------------------------------- |
| **Promotion**      | Git data arrives                             | Move to database, upgrade sync to Full        |
| **Soft expiry**    | Initial timeout                              | Delete bare repo, retain event, continue sync |
| **Full expiry**    | Extended timeout (soft-expired)              | Remove from purgatory entirely                |
| **Deletion**       | Kind 5 event                                 | Delete bare repo, remove from purgatory       |
| **Replacement**    | Newer announcement (same pubkey, identifier) | Replace entry                                 |
| **Service change** | Newer announcement removes our service       | Remove from purgatory                         |

## Integration Points

| File                               | Change                                                     |
| ---------------------------------- | ---------------------------------------------------------- |
| `src/purgatory/mod.rs`             | Add `announcement_purgatory` store                         |
| `src/purgatory/types.rs`           | Add `AnnouncementPurgatoryEntry`                           |
| `src/nostr/policy/announcement.rs` | Route new announcements to purgatory                       |
| `src/git/receive.rs`               | Promote on git data arrival                                |
| `src/git/auth.rs`                  | Extend purgatory expiry when extending state event expiry  |
| `src/git/authorization.rs`         | Check purgatory announcements for maintainer authorization |
| `src/nostr/policy/state.rs`        | Check purgatory for authorization                          |
| `src/sync/mod.rs`                  | Add `SyncLevel` to `RepoSyncNeeds`                         |
| `src/sync/filters.rs`              | Respect sync level when building filters                   |
| `src/sync/self_subscriber.rs`      | Register purgatory announcements with `StateOnly` level    |

See [announcements-purgatory-implementation.md](./announcements-purgatory-implementation.md) for detailed implementation notes.

## Testing

- Announcement to purgatory, git data promotes it
- Announcement soft-expires without git data (repo deleted, event retained)
- State event revives soft-expired announcement (repo recreated)
- Soft-expired announcement fully expires after extended period
- State event extends purgatory expiry
- Git auth extends purgatory expiry
- Newer announcement replaces older in purgatory
- Service change clears purgatory entry
- `(pubkey, identifier)` indexing with multiple owners
- Sync requests only state events for purgatory announcements
- Sync upgrades to full on promotion

## Risks

| Risk                                 | Mitigation                                             |
| ------------------------------------ | ------------------------------------------------------ |
| Disk exhaustion from purgatory repos | Short expiry, soft expiry deletes repo early           |
| Race between promotion and expiry    | Atomic operations                                      |
| Sync re-fetching expired events      | Soft expiry retains event; no need for `failed_events` |
| Filter explosion from many purgatory | Existing consolidation handles this (threshold at 70)  |
