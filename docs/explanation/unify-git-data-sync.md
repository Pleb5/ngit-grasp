# Unified Git Data Sync

## Status

**Proposed** - January 2026

## Context

Currently, two separate code paths handle "git data is now available" scenarios:

1. **`handle_receive_pack`** (src/git/handlers.rs) - After a successful `git push`
2. **`sync_state_git_data`** (src/purgatory/mod.rs) - After purgatory sync fetches OIDs from remote servers

Both paths perform essentially the same post-processing:

| Step | `handle_receive_pack` | `sync_state_git_data` |
|------|----------------------|----------------------|
| Set HEAD | ✅ `try_set_head_if_available()` | ✅ (via `align_repository_with_state`) |
| Save events to DB | ✅ `database.save_event()` | ✅ `database.save_event()` |
| Remove from purgatory | ✅ `remove_state_event()` / `remove_pr()` | ✅ `remove_state_event()` |
| Notify WebSocket | ✅ `relay.notify_event()` | ✅ `relay.notify_event()` |
| Sync state to owner repos | ✅ `sync_to_owner_repos()` | ✅ `sync_to_owner_repos()` |
| Sync PR refs to owner repos | ✅ `sync_pr_refs_to_tagged_owner_repos()` | ❌ Not implemented |

This duplication creates maintenance burden and inconsistent behavior (e.g., PR sync missing from purgatory path).

## Decision

Create a single unified function that handles all post-git-data-available processing:

```rust
pub async fn process_newly_available_git_data(
    source_repo_path: &Path,
    new_oids: &HashSet<String>,
    database: &SharedDatabase,
    local_relay: Option<&nostr_relay_builder::LocalRelay>,
    purgatory: &Purgatory,
    git_data_path: &Path,
) -> ProcessResult
```

### Key Design Principles

**1. Always discover events from purgatory**

Rather than accepting pre-authorized events (which may have changed since authorization), the function always scans purgatory to find satisfiable events. This ensures consistency and handles race conditions where events change between authorization and processing.

**2. Minimal input, maximal output**

Callers only need to provide:
- `source_repo_path` - Where the git data landed
- `new_oids` - Which OIDs are now available (for efficient filtering)

The function handles everything else: finding events, syncing across repos, aligning refs, setting HEAD, saving to database, notifying subscribers, and cleaning up purgatory.

**3. Process all event types uniformly**

Both state events (kind 30618) and PR events (kind 1617/1618) are processed in the same flow, ensuring consistent behavior.

## Architecture

### Flow Overview

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                         Git Data Becomes Available                               │
│                                                                                  │
│   ┌─────────────────────┐              ┌─────────────────────┐                   │
│   │  handle_receive_pack │              │  purgatory sync     │                   │
│   │  (push received)     │              │  (fetch completed)  │                   │
│   └──────────┬──────────┘              └──────────┬──────────┘                   │
│              │                                    │                              │
│              │  source_repo_path                  │  source_repo_path            │
│              │  new_oids                          │  new_oids                    │
│              │                                    │                              │
│              └────────────────┬───────────────────┘                              │
│                               │                                                  │
│                               ▼                                                  │
│              ┌────────────────────────────────────────┐                          │
│              │  process_newly_available_git_data()    │                          │
│              │                                        │                          │
│              │  1. Extract identifier from path       │                          │
│              │  2. Fetch repository data from DB      │                          │
│              │  3. Find satisfiable state events      │                          │
│              │  4. Find satisfiable PR events         │                          │
│              │  5. For each event:                    │                          │
│              │     - Sync OIDs to owner repos         │                          │
│              │     - Align refs (+ set HEAD)          │                          │
│              │     - Save to database                 │                          │
│              │     - Notify WebSocket                 │                          │
│              │     - Remove from purgatory            │                          │
│              └────────────────────────────────────────┘                          │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### Event Discovery

The function discovers satisfiable events by scanning purgatory:

**For State Events:**
1. Get all state entries for the identifier from purgatory
2. For each entry, check if ALL required OIDs exist in source repo
3. Quick optimization: skip if none of `new_oids` are in the state's OID set

**For PR Events:**
1. Get all PR entries for the identifier from purgatory (via secondary index)
2. For each entry with an event, check if the commit OID exists in source repo
3. Quick optimization: skip if commit not in `new_oids`

### Sync to Owner Repos

**For State Events:**

For each owner whose maintainer set authorizes the state author:
1. Skip if a newer state already exists for that owner
2. Copy missing OIDs from source repo to target repo
3. Align refs (create/update/delete branches and tags)
4. Set HEAD per state announcement

**For PR Events:**

For each owner whose maintainer set includes any tagged owner (from `a` tags):
1. Copy commit from source repo to target repo (if missing)
2. Create `refs/nostr/<event-id>` pointing to the commit

## Data Structure Changes

### PrPurgatoryEntry

Add `identifier` field for secondary index lookup:

```rust
#[derive(Debug, Clone)]
pub struct PrPurgatoryEntry {
    /// The nostr PR event, if received (None = git data arrived first)
    pub event: Option<Event>,

    /// The expected commit SHA from 'c' tag or actual commit pushed
    pub commit: String,

    /// Repository identifier extracted from 'a' tag (30617:<owner>:<identifier>)
    /// Used for lookup when git data arrives
    pub identifier: Option<String>,

    /// When this entry was added to purgatory
    pub created_at: Instant,

    /// Expiry deadline
    pub expires_at: Instant,
}
```

### Purgatory Secondary Index

Add index for finding PR events by identifier:

```rust
pub struct Purgatory {
    /// State events indexed by repository identifier
    state_events: Arc<DashMap<String, Vec<StatePurgatoryEntry>>>,

    /// PR events indexed by event ID (hex string)
    pr_events: Arc<DashMap<String, PrPurgatoryEntry>>,

    /// Secondary index: identifier -> event_ids for PR events
    pr_events_by_identifier: Arc<DashMap<String, HashSet<String>>>,

    git_data_path: PathBuf,
}
```

### New Purgatory Methods

```rust
impl Purgatory {
    /// Find all PR events for an identifier
    pub fn find_prs_for_identifier(&self, identifier: &str) -> Vec<PrPurgatoryEntry>;
    
    /// Add PR with automatic identifier extraction and indexing
    pub fn add_pr(&self, event: Event, event_id: String, commit: String);
    
    /// Add placeholder with optional identifier
    pub fn add_pr_placeholder(&self, event_id: String, commit: String, identifier: Option<String>);
    
    /// Remove PR (also cleans up secondary index)
    pub fn remove_pr(&self, event_id: &str);
}
```

## Implementation

### Core Function

```rust
/// Unified processing of newly available git data.
///
/// Called whenever git data becomes available, whether from:
/// - A successful `git push` (handle_receive_pack)
/// - Purgatory sync fetching OIDs from remote servers
///
/// # What it does
///
/// 1. **Discover satisfiable events**: Scans purgatory for state and PR events
///    whose required OIDs are now available in `source_repo_path`
///
/// 2. **For each satisfiable STATE event**:
///    - Find all owner repos that authorize this state's author
///    - Copy OIDs from source repo to each authorized owner repo
///    - Align refs (create/update/delete) to match state
///    - Set HEAD per state announcement
///    - Save event to database
///    - Notify WebSocket subscribers
///    - Remove from purgatory
///
/// 3. **For each satisfiable PR event**:
///    - Find all owner repos that list tagged owners as maintainers
///    - Copy commit from source repo to each relevant owner repo
///    - Create refs/nostr/<event-id> in each repo
///    - Save event to database
///    - Notify WebSocket subscribers
///    - Remove from purgatory
pub async fn process_newly_available_git_data(
    source_repo_path: &Path,
    new_oids: &HashSet<String>,
    database: &SharedDatabase,
    local_relay: Option<&nostr_relay_builder::LocalRelay>,
    purgatory: &Purgatory,
    git_data_path: &Path,
) -> ProcessResult {
    let mut result = ProcessResult::default();

    // Extract identifier from repo path
    let identifier = match extract_identifier_from_repo_path(source_repo_path, git_data_path) {
        Some(id) => id,
        None => return result,
    };

    // Fetch repository data once for all operations
    let db_repo_data = match fetch_repository_data(database, &identifier).await {
        Ok(data) => data,
        Err(e) => {
            result.errors.push(format!("Failed to fetch repo data: {}", e));
            return result;
        }
    };

    // Process satisfiable state events
    let state_result = process_satisfiable_state_events(
        source_repo_path,
        &identifier,
        new_oids,
        &db_repo_data,
        database,
        local_relay,
        purgatory,
        git_data_path,
    ).await;
    
    result.merge_state_result(state_result);

    // Process satisfiable PR events
    let pr_result = process_satisfiable_pr_events(
        source_repo_path,
        &identifier,
        new_oids,
        &db_repo_data,
        database,
        local_relay,
        purgatory,
        git_data_path,
    ).await;
    
    result.merge_pr_result(pr_result);

    result
}
```

### Result Type

```rust
/// Result of processing newly available git data
#[derive(Debug, Default)]
pub struct ProcessResult {
    /// Number of state events released from purgatory
    pub states_released: usize,
    /// Number of PR events released from purgatory
    pub prs_released: usize,
    /// Number of owner repositories synced
    pub repos_synced: usize,
    /// Number of refs created across all repos
    pub refs_created: usize,
    /// Number of refs updated across all repos
    pub refs_updated: usize,
    /// Number of refs deleted across all repos
    pub refs_deleted: usize,
    /// Errors encountered (non-fatal)
    pub errors: Vec<String>,
}
```

### Helper: Extract Identifier from PR Event

```rust
/// Extract identifier from PR event's `a` tag.
/// Format: 30617:<owner_pubkey>:<identifier>
fn extract_identifier_from_pr_event(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let tag_vec = tag.clone().to_vec();
        if tag_vec.len() >= 2 && tag_vec[0] == "a" && tag_vec[1].starts_with("30617:") {
            let parts: Vec<&str> = tag_vec[1].split(':').collect();
            if parts.len() >= 3 {
                Some(parts[2].to_string())
            } else {
                None
            }
        } else {
            None
        }
    })
}
```

### Helper: Extract Identifier from Repo Path

```rust
/// Extract identifier from repository path.
/// Path format: {git_data_path}/{npub}/{identifier}.git
fn extract_identifier_from_repo_path(repo_path: &Path, git_data_path: &Path) -> Option<String> {
    let relative = repo_path.strip_prefix(git_data_path).ok()?;
    let components: Vec<_> = relative.components().collect();

    if components.len() >= 2 {
        let identifier_with_git = components[1].as_os_str().to_str()?;
        Some(identifier_with_git.trim_end_matches(".git").to_string())
    } else {
        None
    }
}
```

## Integration

### handle_receive_pack (Simplified)

```rust
// After git receive-pack succeeds:

// Collect new OIDs from the push
let new_oids: HashSet<String> = pushed_refs
    .iter()
    .filter(|(_, new_oid, _)| new_oid != "0000000000000000000000000000000000000000")
    .map(|(_, new_oid, _)| new_oid.clone())
    .collect();

// Single unified call handles everything
let result = process_newly_available_git_data(
    &repo_path,
    &new_oids,
    &database,
    Some(&relay),
    &purgatory,
    Path::new(git_data_path),
).await;

info!(
    "Processed push: {} states, {} PRs released, {} repos synced",
    result.states_released,
    result.prs_released,
    result.repos_synced
);
```

### Purgatory Sync (Simplified)

```rust
// After fetching OIDs from remote:

let new_oids: HashSet<String> = fetched_oids.into_iter().collect();

let result = process_newly_available_git_data(
    &source_repo_path,
    &new_oids,
    &database,
    local_relay.as_ref(),
    &purgatory,
    &git_data_path,
).await;
```

### Integration with Purgatory Sync Redesign

The purgatory sync redesign (see `purgatory-sync-redesign.md`) uses this unified function in its `sync_identifier_from_url` implementation:

```rust
pub async fn sync_identifier_from_url<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    url: &str,
    throttle_manager: &Arc<ThrottleManager>,
) -> usize {
    // ... fetch OIDs from URL ...
    
    let fetched_oids = ctx.fetch_oids(&target_repo, url, &needed_oids).await?;
    
    if !fetched_oids.is_empty() {
        // Use unified processing
        let new_oids: HashSet<String> = fetched_oids.into_iter().collect();
        
        let result = process_newly_available_git_data(
            &target_repo,
            &new_oids,
            ctx.database(),
            ctx.local_relay(),
            ctx.purgatory(),
            ctx.git_data_path(),
        ).await;
        
        // Result already handled purgatory removal, DB saves, etc.
    }
    
    fetched_oids.len()
}
```

The `SyncContext` trait wraps this function in its `process_newly_available_git_data` method for testability.

## Benefits

1. **Single source of truth** - One function handles all post-git-data processing
2. **Always fresh discovery** - Events discovered from purgatory at processing time
3. **Consistent behavior** - Push and sync paths behave identically
4. **Simpler callers** - Just pass repo_path + new_oids
5. **Complete processing** - Handles all event types, all repo syncing, HEAD, DB, WebSocket, purgatory
6. **PR sync parity** - PR events now synced in purgatory path (was missing)

## Code to Remove/Simplify

After implementing the unified function:

1. **Remove**: Most of `sync_state_git_data` in `src/purgatory/mod.rs`
2. **Simplify**: Event handling in `handle_receive_pack` (replace ~100 lines with single call)
3. **Internalize**: `sync_to_owner_repos` and `sync_pr_refs_to_tagged_owner_repos` become internal helpers

## Testing Strategy

### Unit Tests

1. `extract_identifier_from_repo_path` - Various path formats
2. `extract_identifier_from_pr_event` - Various tag formats
3. Event discovery logic with mock purgatory

### Integration Tests

1. Push triggers processing and releases state event
2. Push triggers processing and releases PR event
3. Purgatory sync triggers processing
4. Multiple events for same identifier processed correctly
5. Cross-repo sync works for both state and PR events

## Future Considerations

### Batch Processing

Currently processes events one at a time. Could batch database saves and WebSocket notifications for efficiency with many events.

### Partial Failures

Currently continues on errors and collects them in result. Could add retry logic or transaction semantics if needed.

### Metrics

Add Prometheus metrics for:
- Events processed by type (state/PR)
- Repos synced per processing call
- Processing duration
- Errors by type

## Related Documents

- [Purgatory Sync Redesign](purgatory-sync-redesign.md) - Uses this unified function for purgatory sync operations
