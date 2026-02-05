# Announcements Purgatory Implementation Details

This document provides detailed implementation notes for the [Announcements Purgatory Design](./announcements-purgatory-design.md).

## Sync Integration

### Current Sync Architecture

The sync system uses a two-index approach:

```rust
// What we WANT to sync - source of truth from self-subscription
// Key: repo addressable ref (30617:pubkey:identifier)
pub type RepoSyncIndex = Arc<RwLock<HashMap<String, RepoSyncNeeds>>>;

pub struct RepoSyncNeeds {
    pub relays: HashSet<String>,       // Relay URLs from announcement
    pub root_events: HashSet<EventId>, // 1617/1618/1621 event IDs
}

// What we have CONFIRMED syncing + connection state
// Key: relay URL
pub type RelaySyncIndex = Arc<RwLock<HashMap<String, RelayState>>>;
```

**Three-Layer Sync Strategy:**
1. **Layer 1:** Announcements (kinds 30617, 10317)
2. **Layer 2:** Repo-tagging events (events with `a`/`A`/`q` tags + kind 30618 by identifier)
3. **Layer 3:** Root-event-tagging events (events with `e`/`E`/`q` tags)

### Adding SyncLevel

Add a `sync_level` field to distinguish purgatory from promoted repos:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncLevel {
    #[default]
    Full,       // L2 + L3 (promoted repos)
    StateOnly,  // Only state events (purgatory announcements)
}

pub struct RepoSyncNeeds {
    pub relays: HashSet<String>,
    pub root_events: HashSet<EventId>,
    pub sync_level: SyncLevel,  // NEW
}
```

### Filter Building Changes

In `src/sync/filters.rs`, modify filter building to respect sync level:

```rust
// For StateOnly repos, only build state event filters
pub fn build_layer2_and_layer3_filters(
    repos: &HashMap<String, RepoSyncNeeds>,
    // ...
) -> Vec<Filter> {
    let (full_repos, state_only_repos): (Vec<_>, Vec<_>) = repos
        .iter()
        .partition(|(_, needs)| needs.sync_level == SyncLevel::Full);
    
    let mut filters = Vec::new();
    
    // Full repos get all L2/L3 filters
    if !full_repos.is_empty() {
        filters.extend(tagged_one_of_our_repo_event_filters(&full_repos));
        filters.extend(state_event_filters_for_our_repos(&full_repos));
        filters.extend(tagged_one_of_our_root_event_filters(&full_repos));
    }
    
    // StateOnly repos get only state event filters
    if !state_only_repos.is_empty() {
        filters.extend(state_event_filters_for_our_repos(&state_only_repos));
    }
    
    filters
}
```

The existing `state_event_filters_for_our_repos()` function already builds kind 30618 filters with `#d` tags, which is exactly what we need.

### Self-Subscriber Changes

In `src/sync/self_subscriber.rs`, add purgatory announcements to the sync index:

```rust
// When announcement enters purgatory
fn on_announcement_to_purgatory(
    &self,
    event: &Event,
    identifier: &str,
    relays: HashSet<String>,
) {
    let key = format!("30617:{}:{}", event.pubkey, identifier);
    let mut index = self.repo_sync_index.write().unwrap();
    index.insert(key, RepoSyncNeeds {
        relays,
        root_events: HashSet::new(),
        sync_level: SyncLevel::StateOnly,
    });
}

// When announcement promotes to database
fn on_announcement_promoted(
    &self,
    event: &Event,
    identifier: &str,
) {
    let key = format!("30617:{}:{}", event.pubkey, identifier);
    let mut index = self.repo_sync_index.write().unwrap();
    if let Some(needs) = index.get_mut(&key) {
        needs.sync_level = SyncLevel::Full;
    }
}
```

### Algorithm Changes

In `src/sync/algorithms.rs`, preserve sync level when inverting repo->relay:

```rust
pub fn derive_relay_targets(
    repo_index: &RepoSyncIndex,
) -> HashMap<String, RelaySyncNeeds> {
    // ... existing inversion logic ...
    // Ensure sync_level is preserved/aggregated per relay
    // A relay gets Full if ANY of its repos are Full
}
```

## Authorization Integration

### Current Authorization Flow

Authorization lookups happen in `src/git/authorization.rs`:

| Function | Purpose | Currently Queries |
|----------|---------|-------------------|
| `fetch_repository_data()` | Get announcements + states by identifier | DB only |
| `collect_authorized_maintainers()` | Build maintainer set from announcements | DB only |
| `pubkey_authorised_for_repo_owners()` | Check if pubkey authorized | DB only |

### Required Changes

Modify `fetch_repository_data()` to also query purgatory:

```rust
pub async fn fetch_repository_data(
    db: &Database,
    purgatory: &Purgatory,  // NEW parameter
    identifier: &str,
) -> Result<RepositoryData> {
    // Existing DB query
    let db_events = db.query(/* kind 30617, 30618 by identifier */).await?;
    
    // NEW: Also check purgatory for announcements
    let purgatory_announcements = purgatory
        .get_announcements_by_identifier(identifier);
    
    // Merge results
    let mut announcements = parse_announcements(db_events);
    announcements.extend(purgatory_announcements);
    
    // ... rest of function
}
```

This affects:
- `StatePolicy::process_state_event()` - state event validation
- `get_state_authorization_for_specific_owner_repo()` - git push authorization
- `AnnouncementPolicy::is_maintainer_in_any_announcement()` - maintainer exception

## Purgatory Store Changes

### New Fields

```rust
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

### New Methods

```rust
impl Purgatory {
    /// Get announcements by identifier (for authorization)
    pub fn get_announcements_by_identifier(
        &self,
        identifier: &str,
    ) -> Vec<&AnnouncementPurgatoryEntry> {
        self.announcement_purgatory
            .iter()
            .filter(|entry| entry.identifier == identifier)
            .collect()
    }
    
    /// Transition to soft-expired state
    pub fn soft_expire_announcement(
        &self,
        key: &(PublicKey, String),
    ) -> Option<PathBuf> {
        if let Some(mut entry) = self.announcement_purgatory.get_mut(key) {
            entry.soft_expired = true;
            entry.expires_at = Instant::now() + SOFT_EXPIRY_DURATION; // e.g., 24h
            Some(entry.repo_path.clone()) // Return path for bare repo deletion
        } else {
            None
        }
    }
    
    /// Revive soft-expired announcement (caller must recreate bare repo)
    pub fn revive_announcement(
        &self,
        key: &(PublicKey, String),
    ) -> Option<PathBuf> {
        if let Some(mut entry) = self.announcement_purgatory.get_mut(key) {
            if entry.soft_expired {
                entry.soft_expired = false;
                entry.expires_at = Instant::now() + ACTIVE_EXPIRY_DURATION;
                return Some(entry.repo_path.clone()); // Caller recreates bare repo
            }
        }
        None
    }
}
```

## Expiry Cleanup Task

The existing cleanup task needs to handle the two-phase expiry:

```rust
async fn cleanup_expired_announcements(&self) {
    let now = Instant::now();
    
    for entry in self.announcement_purgatory.iter() {
        if entry.expires_at <= now {
            let key = (entry.owner.clone(), entry.identifier.clone());
            
            if entry.soft_expired {
                // Fully expired - remove entirely
                self.announcement_purgatory.remove(&key);
                self.unregister_from_sync(&key);
            } else {
                // First expiry - transition to soft-expired
                if let Some(repo_path) = self.soft_expire_announcement(&key) {
                    delete_bare_repo(&repo_path).await;
                }
                // Note: stays in sync index with StateOnly level
            }
        }
    }
}
```

## State Event Revival Flow

When a state event arrives for a soft-expired announcement, the state policy must:

1. Check purgatory for a matching announcement (in addition to DB)
2. Validate authorization against the purgatory announcement
3. If soft-expired, call `revive_announcement()` and recreate the bare repo
4. Extend the announcement's expiry
5. Route the state event to state purgatory

The exact integration will depend on the current structure of `StatePolicy::process_state_event()` - see implementation phase for details.

## File Change Summary

| File | Estimated Lines | Changes |
|------|-----------------|---------|
| `src/sync/mod.rs` | ~10 | Add `SyncLevel` enum, field to `RepoSyncNeeds` |
| `src/sync/filters.rs` | ~20 | Partition repos by sync level, build appropriate filters |
| `src/sync/algorithms.rs` | ~15 | Preserve sync level in relay target derivation |
| `src/sync/self_subscriber.rs` | ~40 | Register purgatory announcements, handle promotion |
| `src/purgatory/mod.rs` | ~80 | Add announcement store, soft expiry methods |
| `src/purgatory/types.rs` | ~20 | Add `AnnouncementPurgatoryEntry` |
| `src/git/authorization.rs` | ~30 | Query purgatory in `fetch_repository_data()` |
| `src/nostr/policy/state.rs` | ~40 | Handle soft-expired revival |
| `src/nostr/policy/announcement.rs` | ~30 | Route to purgatory, check for replacements |
| `src/git/receive.rs` | ~20 | Trigger promotion on git data |

**Total: ~305 lines of changes**
