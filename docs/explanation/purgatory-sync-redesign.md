# Purgatory Sync Redesign

## Status

**Proposed** - January 2026

## Context

The current purgatory sync implementation (`start_state_sync` at `src/purgatory/mod.rs:510`) has several limitations:

1. **Per-event syncing**: Each state event triggers its own independent sync operation
2. **No PR event syncing**: PR events enter purgatory but don't trigger git data fetching
3. **No batching**: Multiple events for the same repository cause redundant fetch requests
4. **No rate limiting**: Can overwhelm remote git servers or get rate-limited
5. **No coordination**: Multiple concurrent syncs may fetch the same OIDs

When syncing a new repository, we often receive multiple state and PR events in a burst. The current approach creates unnecessary load on remote servers and doesn't handle this common case efficiently.

## Decision

Redesign purgatory sync to be **identifier-based** rather than **event-based**, with:

1. A background sync loop that processes identifiers, not individual events
2. Batched OID fetching across all purgatory events for an identifier
3. Domain-based throttling (30 requests/minute per domain)
4. Exponential backoff per identifier (20s → 2m, then 2m intervals)
5. Debouncing for burst event arrivals (500ms for sync-triggered, 3min default)
6. **Clean separation of concerns**: Domain throttle handles rate limiting only; sync logic tracks its own tried URLs

### Key Design Decision: Where Does OID Copying Happen?

**Answer: In `process_newly_available_git_data`, NOT after the entire sync completes.**

The current implementation (`sync_state_git_data`) fetches all OIDs first, then at the end:
1. Copies OIDs to all authorized owner repos
2. Aligns refs with state
3. Saves to database
4. Notifies subscribers
5. Removes from purgatory

The redesign moves all of this into `process_newly_available_git_data`, which is called after **each successful URL fetch**. This enables:

| Aspect | Current (end-of-sync) | Redesign (per-fetch) |
|--------|----------------------|---------------------|
| **When events release** | Only after all URLs tried | As soon as OIDs available |
| **Partial success** | All or nothing per event | Events release independently |
| **Multiple state events** | All wait for slowest | Each releases when ready |
| **Authorization check** | Once at start | At release time (handles changes) |

**Why this matters:**

Consider syncing an identifier with 3 state events from different maintainers:
- State A needs OIDs from `server1.com` (fast)
- State B needs OIDs from `server2.com` (slow)  
- State C needs OIDs from `server3.com` (down)

With the redesign:
1. Fetch from `server1.com` succeeds → `process_newly_available_git_data` releases State A immediately
2. Fetch from `server2.com` succeeds → `process_newly_available_git_data` releases State B
3. Fetch from `server3.com` fails → State C stays in purgatory, retries with backoff

The current implementation would wait for all servers before releasing any events.

### Unified Processing with Git Push Handler

**Key insight**: The post-git-data-available processing is identical whether data arrives via:
- A successful `git push` (handle_receive_pack)
- Purgatory sync fetching OIDs from remote servers

Both paths need to:
1. Discover satisfiable events from purgatory
2. Sync OIDs to authorized owner repos
3. Align refs (+ set HEAD)
4. Save events to database
5. Notify WebSocket subscribers
6. Remove from purgatory

Rather than duplicate this logic, we use a single unified function `process_newly_available_git_data` that handles all post-git-data-available processing. See [Unified Git Data Sync](unify-git-data-sync.md) for the complete design.

This means:
- **`handle_receive_pack`** calls `process_newly_available_git_data` after git push succeeds
- **`sync_identifier_from_url`** calls `process_newly_available_git_data` after OID fetch succeeds
- **Same behavior** regardless of how git data arrived

## Architecture

### Overview

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                                  Purgatory                                        │
│                                                                                   │
│  ┌─────────────────┐  ┌─────────────────┐                                         │
│  │  State Events   │  │   PR Events     │                                         │
│  │  (by identifier)│  │  (by event_id)  │                                         │
│  └────────┬────────┘  └────────┬────────┘                                         │
│           │                    │                                                  │
│           └──────────┬─────────┘                                                  │
│                      │ add_state() / add_pr() / trigger_immediate_sync()          │
│                      ▼                                                            │
│           ┌──────────────────────────┐                                            │
│           │      Sync Queue          │                                            │
│           │  DashMap<id, Entry>      │                                            │
│           │                          │                                            │
│           │  Entry {                 │                                            │
│           │    next_attempt,         │  ← delay/backoff timer                     │
│           │    attempt_count,        │  ← for backoff calculation                 │
│           │    in_progress,          │  ← prevents concurrent runs                │
│           │  }                       │                                            │
│           └────────────┬─────────────┘                                            │
│                        │                                                          │
│  ┌─────────────────────┼──────────────────────────────────────────────────────┐   │
│  │                     ▼                                                      │   │
│  │          ┌─────────────────────┐                                           │   │
│  │          │   Main Sync Loop    │  (every 1s)                               │   │
│  │          │                     │                                           │   │
│  │          │  1. Find ALL ready  │                                           │   │
│  │          │     identifiers     │                                           │   │
│  │          │  2. Spawn parallel  │───────┐                                   │   │
│  │          │     tasks for each  │       │  (parallel tasks)                 │   │
│  │          │  3. Apply backoff   │       │                                   │   │
│  │          │     when done       │       │                                   │   │
│  │          └─────────────────────┘       │                                   │   │
│  │                                        ▼                                   │   │
│  │                             ┌──────────────────────────────────────────┐   │   │
│  │                             │       sync_identifier()                  │   │   │
│  │                             │                                          │   │   │
│  │                             │  Owns its own tried_urls: HashSet        │   │   │
│  │                             │                                          │   │   │
│  │                             │  loop:                                   │   │   │
│  │                             │    url = sync_identifier_next_url(       │   │   │
│  │                             │            domain=None)                  │   │   │
│  │                             │    if url is Some:                       │   │   │
│  │                             │      sync_identifier_from_url(url)       │   │   │
│  │                             │      tried_urls.insert(url)              │   │   │
│  │                             │    else:                                 │   │   │
│  │                             │      break (no non-throttled URLs left)  │   │   │
│  │                             │                                          │   │   │
│  │                             │  Enqueue throttled domains then return   │   │   │
│  │                             └──────────────────────────────────────────┘   │   │
│  │                                        │                                   │   │
│  │                                        │ enqueue_identifier()              │   │
│  │                                        ▼                                   │   │
│  │  ┌─────────────────────────────────────────────────────────────────────┐   │   │
│  │  │                        ThrottleManager                              │   │   │
│  │  │                                                                     │   │   │
│  │  │   DashMap<domain, DomainThrottle>                                   │   │   │
│  │  │                                                                     │   │   │
│  │  │   ┌─────────────────────────────────────────────────────────────┐   │   │   │
│  │  │   │  DomainThrottle (per domain)                                │   │   │   │
│  │  │   │                                                             │   │   │   │
│  │  │   │  Rate limiting:           │  Queue (IndexMap for ordering): │   │   │   │
│  │  │   │    - in_flight: u32       │    - queue: IndexMap<id, State> │   │   │   │
│  │  │   │    - request_times        │    - State: tried_urls,         │   │   │   │
│  │  │   │    - round_robin_index    │             in_progress         │   │   │   │
│  │  │   └─────────────────────────────────────────────────────────────┘   │   │   │
│  │  │                                                                     │   │   │
│  │  │   Trigger-based processing (no polling loop):                       │   │   │
│  │  │     - enqueue_identifier() triggers if capacity available           │   │   │
│  │  │     - complete_request() triggers next item if capacity available   │   │   │
│  │  │                                                                     │   │   │
│  │  │   process_queued_identifier():                                      │   │   │
│  │  │     1. Pick next identifier (round-robin, not in_progress)          │   │   │
│  │  │     2. url = sync_identifier_next_url(domain=Some(this_domain))     │   │   │
│  │  │     3. If url: sync_identifier_from_url(url), mark tried            │   │   │
│  │  │        Else: remove identifier from queue, try next                 │   │   │
│  │  └─────────────────────────────────────────────────────────────────────┘   │   │
│  │                                                                            │   │
│  └────────────────────────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────────────────────┘
```

### Key Design Principles

**1. Two Independent Execution Paths**

The main sync loop and DomainThrottle loops run independently:
- **Main sync**: Tries non-throttled URLs, completes quickly, applies backoff, retries later
- **DomainThrottle**: Processes queued identifiers when capacity frees, doesn't block main sync

**2. Two Separate tried_urls Tracking**

Each path tracks its own tried URLs:
- **sync_identifier**: Local `HashSet<String>` for current attempt (all domains)
- **DomainThrottle**: Per-identifier `HashSet<String>` for URLs tried via throttle (this domain only)

These don't need to merge because:
- Main sync skips throttled domains anyway
- DomainThrottle only processes its own domain's URLs

**3. Shared Functions**

Both paths use the same core functions:
- **`sync_identifier_next_url`**: Pure URL selection logic
- **`sync_identifier_from_url`**: Pure fetch logic

The `domain` parameter determines behavior:
- `None`: Return any non-throttled URL
- `Some(domain)`: Return URL from that specific domain only

### Flow Summary

1. **Event arrives** → added to state_events/pr_events + sync_queue with delay
   - User-submitted: 3 minute delay (expect git push to follow)
   - Sync-triggered: 500ms delay (batch burst arrivals)
   - `enqueue_sync()` resets `attempt_count` to 0 and updates `next_attempt` if needed

2. **Main sync loop** (every 1s):
   - Finds ALL ready identifiers (where `!in_progress && next_attempt <= now`)
   - Spawns parallel tasks for each (marks `in_progress = true`)
   - Each `sync_identifier()` task:
     - Creates fresh `tried_urls: HashSet<String>`
     - Loops calling `sync_identifier_next_url(domain=None)` + `sync_identifier_from_url`
     - When no non-throttled URLs remain: enqueue with throttled domains, return
   - When task completes: apply backoff or remove from queue

3. **ThrottleManager / DomainThrottle** (trigger-based, no polling):
   - Processing triggered by `enqueue_identifier()` or `complete_request()`
   - When triggered and capacity available: pick next queued identifier (round-robin, not in_progress)
   - Call `sync_identifier_next_url(domain=Some(this_domain))`
   - If URL returned: call `sync_identifier_from_url`, mark URL tried, mark not in_progress
   - If no URL: remove identifier from queue, try next identifier

## Data Structures

### SyncQueueEntry

Tracks sync state for each identifier in the main sync queue:

```rust
/// Entry in the sync queue tracking when/how to sync an identifier
#[derive(Debug, Clone)]
pub struct SyncQueueEntry {
    /// Don't attempt sync before this time
    pub next_attempt: Instant,
    
    /// Number of sync attempts (for backoff calculation)
    /// Reset to 0 when new event arrives for this identifier
    pub attempt_count: u32,
    
    /// Whether a sync is currently in progress for this identifier
    pub in_progress: bool,
}

impl SyncQueueEntry {
    pub fn new(delay: Duration) -> Self {
        Self {
            next_attempt: Instant::now() + delay,
            attempt_count: 0,
            in_progress: false,
        }
    }
    
    /// Calculate backoff: 20s, 40s, 80s, 120s (capped at 2min)
    pub fn backoff(&self) -> Duration {
        let base = Duration::from_secs(20);
        let multiplier = 2u32.saturating_pow(self.attempt_count.saturating_sub(1).min(3));
        (base * multiplier).min(Duration::from_secs(120))
    }
    
    pub fn is_ready(&self) -> bool {
        !self.in_progress && Instant::now() >= self.next_attempt
    }
    
    /// Called when new event arrives - resets attempt_count
    pub fn on_new_event(&mut self, delay: Duration) {
        self.attempt_count = 0;
        let new_attempt = Instant::now() + delay;
        if new_attempt < self.next_attempt {
            self.next_attempt = new_attempt;
        }
    }
    
    /// Called when sync attempt completes
    pub fn on_sync_complete(&mut self) {
        self.in_progress = false;
        if self.next_attempt <= Instant::now() {
            self.attempt_count += 1;
            self.next_attempt = Instant::now() + self.backoff();
        }
    }
}
```

### ThrottleManager

Manages all per-domain throttles and provides the interface for checking throttle status:

```rust
/// Manages rate limiting across all domains.
/// 
/// Owns a collection of DomainThrottle instances and provides:
/// - Throttle status checking for sync_identifier_next_url
/// - Identifier queue management
/// - Trigger-based processing when capacity frees up
pub struct ThrottleManager {
    /// Per-domain throttle state
    throttles: DashMap<String, DomainThrottle>,
    
    /// Sync context for processing queued identifiers
    /// Set once at startup via set_context()
    ctx: OnceLock<Arc<dyn SyncContext>>,
    
    /// Configuration
    max_concurrent_per_domain: u32,
    max_per_minute_per_domain: u32,
}

impl ThrottleManager {
    pub fn new(max_concurrent: u32, max_per_minute: u32) -> Self {
        Self {
            throttles: DashMap::new(),
            ctx: OnceLock::new(),
            max_concurrent_per_domain: max_concurrent,
            max_per_minute_per_domain: max_per_minute,
        }
    }
    
    /// Set the sync context (called once at startup)
    pub fn set_context(&self, ctx: Arc<dyn SyncContext>) {
        let _ = self.ctx.set(ctx);
    }
    
    /// Check if a domain is currently throttled (at capacity)
    pub fn is_throttled(&self, domain: &str) -> bool {
        self.throttles
            .get(domain)
            .map_or(false, |t| !t.has_capacity())
    }
    
    /// Get or create throttle for a domain
    fn get_or_create(&self, domain: &str) -> dashmap::mapref::one::RefMut<String, DomainThrottle> {
        self.throttles
            .entry(domain.to_string())
            .or_insert_with(|| DomainThrottle::new(
                domain.to_string(),
                self.max_concurrent_per_domain,
                self.max_per_minute_per_domain,
            ))
    }
    
    /// Record that a request is starting for a domain
    pub fn start_request(&self, domain: &str) {
        self.get_or_create(domain).start_request();
    }
    
    /// Record that a request completed for a domain.
    /// Triggers processing of next queued identifier if capacity available.
    pub fn complete_request(self: &Arc<Self>, domain: &str) {
        let should_trigger = {
            if let Some(mut throttle) = self.throttles.get_mut(domain) {
                throttle.complete_request();
                throttle.has_capacity() && throttle.has_queued_work()
            } else {
                false
            }
        };
        
        if should_trigger {
            self.try_process_next(domain);
        }
    }
    
    /// Add an identifier to a domain's waiting queue.
    /// Triggers processing if capacity is available.
    pub fn enqueue_identifier(
        self: &Arc<Self>,
        domain: &str,
        identifier: String,
        tried_urls_for_domain: HashSet<String>,
    ) {
        let should_trigger = {
            let mut throttle = self.get_or_create(domain);
            throttle.enqueue_identifier(identifier, tried_urls_for_domain);
            throttle.has_capacity()
        };
        
        if should_trigger {
            self.try_process_next(domain);
        }
    }
    
    /// Try to process the next queued identifier for a domain
    fn try_process_next(self: &Arc<Self>, domain: &str) {
        let identifier = {
            if let Some(mut throttle) = self.throttles.get_mut(domain) {
                throttle.next_ready_identifier()
            } else {
                None
            }
        };
        
        if let Some(identifier) = identifier {
            let manager = self.clone();
            let domain = domain.to_string();
            
            tokio::spawn(async move {
                manager.process_queued_identifier(&domain, &identifier).await;
            });
        }
    }
    
    /// Process a single identifier from a domain's queue
    async fn process_queued_identifier(self: &Arc<Self>, domain: &str, identifier: &str) {
        let ctx = match self.ctx.get() {
            Some(ctx) => ctx,
            None => return,
        };
        
        // Get next URL for this identifier on this domain
        let url = {
            let throttle = match self.throttles.get(domain) {
                Some(t) => t,
                None => return,
            };
            let tried_urls = throttle.get_tried_urls(identifier);
            
            sync_identifier_next_url(
                ctx.as_ref(),
                identifier,
                Some(domain),
                &tried_urls,
                self,
            ).await
        };
        
        match url {
            Some(url) => {
                // Fetch from this URL (this calls start_request/complete_request internally)
                sync_identifier_from_url(ctx.as_ref(), identifier, &url, self).await;
                
                // Record URL as tried and mark not in_progress
                // complete_request() will trigger next item if capacity available
                if let Some(mut throttle) = self.throttles.get_mut(domain) {
                    throttle.mark_url_tried(identifier, url);
                    throttle.mark_identifier_not_in_progress(identifier);
                }
            }
            None => {
                // No more URLs for this identifier on this domain - remove from queue
                if let Some(mut throttle) = self.throttles.get_mut(domain) {
                    throttle.remove_identifier(identifier);
                }
                // Try next identifier since we didn't use any capacity
                self.try_process_next(domain);
            }
        }
    }
}
```

### DomainThrottle

Per-domain rate limiting and waiting queue:

```rust
/// Per-domain rate limiting and identifier queue.
/// 
/// Handles:
/// - Rate limiting (concurrent requests, requests per minute)
/// - Queue of identifiers waiting for capacity (using IndexMap for round-robin order)
/// - Tracking tried URLs per identifier (for this domain only)
/// - In-progress flag per identifier (prevents concurrent fetches for same identifier
///   on this domain, important when queue is small and we have multiple concurrent slots)
pub struct DomainThrottle {
    /// Domain this throttle manages
    domain: String,
    
    /// Current in-flight request count
    in_flight: u32,
    
    /// Request timestamps (sliding window for rate limiting)
    request_times: VecDeque<Instant>,
    
    /// Queued identifiers with their state.
    /// IndexMap preserves insertion order for round-robin processing.
    queue: IndexMap<String, IdentifierQueueState>,
    
    /// Round-robin index for fair processing across identifiers
    round_robin_index: usize,
    
    /// Configuration
    max_concurrent: u32,
    max_per_minute: u32,
}

/// State for an identifier waiting in a domain's queue
#[derive(Debug, Clone)]
struct IdentifierQueueState {
    /// URLs from this domain that have been tried
    tried_urls: HashSet<String>,
    
    /// Whether a fetch is currently in progress for this identifier on this domain.
    /// Prevents starting multiple concurrent fetches for the same identifier,
    /// which is important when the queue is small (e.g., 2 identifiers with 5 
    /// concurrent slots would otherwise try to process the same identifier multiple times).
    in_progress: bool,
}

impl DomainThrottle {
    pub fn new(domain: String, max_concurrent: u32, max_per_minute: u32) -> Self {
        Self {
            domain,
            in_flight: 0,
            request_times: VecDeque::new(),
            queue: IndexMap::new(),
            round_robin_index: 0,
            max_concurrent,
            max_per_minute,
        }
    }
    
    /// Check if domain has capacity for another request
    pub fn has_capacity(&self) -> bool {
        if self.in_flight >= self.max_concurrent {
            return false;
        }
        
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let recent_count = self.request_times
            .iter()
            .filter(|t| now.duration_since(**t) < window)
            .count();
        
        recent_count < self.max_per_minute as usize
    }
    
    /// Check if there are any identifiers in the queue
    pub fn has_queued_work(&self) -> bool {
        !self.queue.is_empty()
    }
    
    /// Record that a request is starting
    pub fn start_request(&mut self) {
        self.in_flight += 1;
        self.request_times.push_back(Instant::now());
    }
    
    /// Record that a request completed
    pub fn complete_request(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
        
        // Clean old timestamps
        let now = Instant::now();
        let window = Duration::from_secs(60);
        while self.request_times.front().map_or(false, |t| now.duration_since(*t) >= window) {
            self.request_times.pop_front();
        }
    }
    
    /// Add an identifier to the queue
    pub fn enqueue_identifier(&mut self, identifier: String, tried_urls: HashSet<String>) {
        self.queue
            .entry(identifier)
            .and_modify(|state| {
                // Merge tried_urls if already exists
                state.tried_urls.extend(tried_urls.iter().cloned());
            })
            .or_insert(IdentifierQueueState {
                tried_urls,
                in_progress: false,
            });
    }
    
    /// Get next identifier ready for processing (round-robin, not in_progress).
    /// 
    /// Iterates through the queue starting from round_robin_index, skipping
    /// any identifiers that are already in_progress. This ensures fair
    /// distribution even when some identifiers have active fetches.
    pub fn next_ready_identifier(&mut self) -> Option<String> {
        let len = self.queue.len();
        if len == 0 {
            return None;
        }
        
        // Try each identifier starting from round_robin_index
        for i in 0..len {
            let index = (self.round_robin_index + i) % len;
            if let Some((identifier, state)) = self.queue.get_index_mut(index) {
                if !state.in_progress {
                    state.in_progress = true;
                    self.round_robin_index = (index + 1) % len;
                    return Some(identifier.clone());
                }
            }
        }
        
        None // All identifiers are in_progress
    }
    
    /// Get tried URLs for an identifier
    pub fn get_tried_urls(&self, identifier: &str) -> HashSet<String> {
        self.queue
            .get(identifier)
            .map(|s| s.tried_urls.clone())
            .unwrap_or_default()
    }
    
    /// Mark a URL as tried for an identifier
    pub fn mark_url_tried(&mut self, identifier: &str, url: String) {
        if let Some(state) = self.queue.get_mut(identifier) {
            state.tried_urls.insert(url);
        }
    }
    
    /// Mark identifier as not in progress (fetch completed)
    pub fn mark_identifier_not_in_progress(&mut self, identifier: &str) {
        if let Some(state) = self.queue.get_mut(identifier) {
            state.in_progress = false;
        }
    }
    
    /// Remove an identifier from the queue entirely
    pub fn remove_identifier(&mut self, identifier: &str) {
        if let Some((index, _, _)) = self.queue.shift_remove_full(identifier) {
            // Adjust round_robin_index if we removed an entry before it
            if index < self.round_robin_index && self.round_robin_index > 0 {
                self.round_robin_index -= 1;
            }
            // Clamp to valid range
            if !self.queue.is_empty() {
                self.round_robin_index = self.round_robin_index % self.queue.len();
            } else {
                self.round_robin_index = 0;
            }
        }
    }
}
```

### SyncContext Trait (For Testability)

Abstract the external dependencies to enable unit testing:

```rust
/// Abstraction over external dependencies for sync operations.
/// 
/// This trait allows unit testing of sync logic by mocking:
/// - Repository data fetching
/// - OID existence checks
/// - Git fetch operations
/// - Event processing
#[async_trait]
pub trait SyncContext: Send + Sync {
    /// Get repository data (announcements, clone URLs, etc.)
    async fn fetch_repository_data(&self, identifier: &str) -> Result<RepositoryData>;
    
    /// Get all OIDs needed for purgatory events with this identifier
    fn collect_needed_oids(&self, identifier: &str) -> HashSet<String>;
    
    /// Check if an OID exists locally
    fn oid_exists(&self, repo_path: &Path, oid: &str) -> bool;
    
    /// Fetch OIDs from a remote server
    async fn fetch_oids(&self, repo_path: &Path, url: &str, oids: &[String]) -> Result<Vec<String>>;
    
    /// Process newly available git data.
    /// 
    /// This is a thin wrapper around the unified `process_newly_available_git_data` function.
    /// Called after each successful OID fetch to check if any purgatory events can now be satisfied.
    /// 
    /// See [Unified Git Data Sync](unify-git-data-sync.md) for the complete design.
    async fn process_newly_available_git_data(
        &self,
        source_repo_path: &Path,
        new_oids: &HashSet<String>,
    ) -> Result<ProcessResult>;
    
    /// Check if there are still pending events for this identifier
    fn has_pending_events(&self, identifier: &str) -> bool;
    
    /// Find the best local repo to fetch into
    fn find_target_repo(&self, db_repo_data: &RepositoryData) -> Option<PathBuf>;
    
    /// Our domain (to exclude from clone URLs)
    fn our_domain(&self) -> Option<&str>;
}

/// Real implementation of SyncContext with all dependencies
pub struct RealSyncContext {
    purgatory: Purgatory,
    database: SharedDatabase,
    git_data_path: PathBuf,
    our_domain: Option<String>,
    local_relay: Option<nostr_relay_builder::LocalRelay>,
}

impl RealSyncContext {
    pub fn new(
        purgatory: Purgatory,
        database: SharedDatabase,
        git_data_path: PathBuf,
        our_domain: Option<String>,
        local_relay: Option<nostr_relay_builder::LocalRelay>,
    ) -> Self {
        Self {
            purgatory,
            database,
            git_data_path,
            our_domain,
            local_relay,
        }
    }
}

#[async_trait]
impl SyncContext for RealSyncContext {
    // ... other methods ...
    
    async fn process_newly_available_git_data(
        &self,
        source_repo_path: &Path,
        new_oids: &HashSet<String>,
    ) -> Result<ProcessResult> {
        // Call the unified function that handles all post-git-data-available processing
        // This is the same function called by handle_receive_pack after a git push
        crate::git::process_newly_available_git_data(
            source_repo_path,
            new_oids,
            &self.database,
            self.local_relay.as_ref(),
            &self.purgatory,
            &self.git_data_path,
        ).await
    }
    
    // ... other methods ...
}
```

**Note**: The `SyncContext` trait abstracts away the dependencies for testability. The real implementation (`RealSyncContext`) holds references to purgatory, database, etc., and the `process_newly_available_git_data` method delegates to the unified function. This keeps the sync logic functions (`sync_identifier_next_url`, `sync_identifier_from_url`) clean and testable with mocks.

## Core Sync Logic

### Two-Function Design

The sync logic is split into two functions that can be called by either the main sync loop or by DomainThrottle:

1. **`sync_identifier_next_url`**: Pure selection logic - finds next URL to try
2. **`sync_identifier_from_url`**: Pure fetch logic - fetches from a specific URL

This separation enables:
- Main sync loop to try non-throttled URLs immediately
- DomainThrottle to process queued identifiers when capacity frees
- Clean testability with mocked SyncContext

### sync_identifier_next_url

```rust
/// Find the next URL to try for an identifier.
/// 
/// When `domain` is None: returns any non-throttled URL not in tried_urls
/// When `domain` is Some: returns a URL from that specific domain not in tried_urls
/// 
/// Returns None if:
/// - No pending events for this identifier
/// - No OIDs needed (sync complete)
/// - No untried URLs available (for the specified domain or all domains)
/// - All available domains are throttled (when domain is None)
pub async fn sync_identifier_next_url<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    domain: Option<&str>,
    tried_urls: &HashSet<String>,
    throttle_manager: &ThrottleManager,
) -> Option<String> {
    // 1. Check if we still have pending events
    if !ctx.has_pending_events(identifier) {
        return None;
    }
    
    // 2. Collect needed OIDs
    let needed_oids = ctx.collect_needed_oids(identifier);
    if needed_oids.is_empty() {
        // No OIDs needed - sync is complete
        return None;
    }
    
    // 3. Get repository data
    let repo_data = match ctx.fetch_repository_data(identifier).await {
        Ok(data) => data,
        Err(_) => return None,
    };
    
    // 4. Collect clone URLs, excluding our domain
    let all_urls: Vec<String> = repo_data
        .announcements
        .iter()
        .flat_map(|a| a.clone_urls.iter().cloned())
        .filter(|url| ctx.our_domain().map_or(true, |d| !url.contains(d)))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    
    // 5. Group by domain
    let urls_by_domain: HashMap<String, Vec<String>> = all_urls
        .iter()
        .fold(HashMap::new(), |mut acc, url| {
            if let Some(d) = extract_domain(url) {
                acc.entry(d).or_default().push(url.clone());
            }
            acc
        });
    
    // 6. Find an available URL
    match domain {
        Some(specific_domain) => {
            // Only look at URLs from this specific domain
            urls_by_domain
                .get(specific_domain)
                .and_then(|urls| {
                    urls.iter()
                        .find(|url| !tried_urls.contains(*url))
                        .cloned()
                })
        }
        None => {
            // Try any non-throttled domain
            for (d, domain_urls) in &urls_by_domain {
                if throttle_manager.is_throttled(d) {
                    continue;
                }
                if let Some(url) = domain_urls.iter().find(|url| !tried_urls.contains(*url)) {
                    return Some(url.clone());
                }
            }
            None
        }
    }
}

/// Information about throttled domains with untried URLs
#[derive(Debug, Clone)]
pub struct ThrottledDomainInfo {
    pub domain: String,
    pub tried_urls_for_domain: HashSet<String>,
}

/// Get information about throttled domains that have untried URLs.
/// 
/// Called by main sync loop to know which DomainThrottle queues to add the identifier to.
pub async fn get_throttled_domains_with_untried_urls<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    tried_urls: &HashSet<String>,
    throttle_manager: &ThrottleManager,
) -> Vec<ThrottledDomainInfo> {
    let repo_data = match ctx.fetch_repository_data(identifier).await {
        Ok(data) => data,
        Err(_) => return vec![],
    };
    
    let all_urls: Vec<String> = repo_data
        .announcements
        .iter()
        .flat_map(|a| a.clone_urls.iter().cloned())
        .filter(|url| ctx.our_domain().map_or(true, |d| !url.contains(d)))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    
    let urls_by_domain: HashMap<String, Vec<String>> = all_urls
        .iter()
        .fold(HashMap::new(), |mut acc, url| {
            if let Some(d) = extract_domain(url) {
                acc.entry(d).or_default().push(url.clone());
            }
            acc
        });
    
    urls_by_domain
        .into_iter()
        .filter_map(|(domain, domain_urls)| {
            if !throttle_manager.is_throttled(&domain) {
                return None; // Not throttled, skip
            }
            
            let untried: Vec<_> = domain_urls
                .iter()
                .filter(|url| !tried_urls.contains(*url))
                .collect();
            
            if untried.is_empty() {
                return None; // All URLs tried for this domain
            }
            
            // Collect tried URLs that belong to this domain
            let tried_urls_for_domain: HashSet<String> = tried_urls
                .iter()
                .filter(|url| extract_domain(url).as_deref() == Some(&domain))
                .cloned()
                .collect();
            
            Some(ThrottledDomainInfo {
                domain,
                tried_urls_for_domain,
            })
        })
        .collect()
}
```

### sync_identifier_from_url

```rust
/// Fetch git data from a specific URL for an identifier.
/// 
/// This function:
/// 1. Records the request with the throttle manager
/// 2. Performs the actual git fetch
/// 3. Processes any events that can now be satisfied
/// 4. Records request completion
/// 
/// Returns the number of OIDs successfully fetched.
pub async fn sync_identifier_from_url<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    url: &str,
    throttle_manager: &Arc<ThrottleManager>,
) -> usize {
    let domain = match extract_domain(url) {
        Some(d) => d,
        None => return 0,
    };
    
    // Get repository data for target repo path
    let repo_data = match ctx.fetch_repository_data(identifier).await {
        Ok(data) => data,
        Err(e) => {
            tracing::debug!(identifier = %identifier, error = %e, "Failed to fetch repo data");
            return 0;
        }
    };
    
    let target_repo = match ctx.find_target_repo(&repo_data) {
        Some(path) => path,
        None => {
            tracing::debug!(identifier = %identifier, "No target repo found");
            return 0;
        }
    };
    
    // Collect needed OIDs
    let needed_oids: Vec<String> = ctx.collect_needed_oids(identifier).into_iter().collect();
    if needed_oids.is_empty() {
        return 0;
    }
    
    // Perform the fetch
    throttle_manager.start_request(&domain);
    let fetch_result = ctx.fetch_oids(&target_repo, url, &needed_oids).await;
    throttle_manager.complete_request(&domain);
    
    let oids_fetched = match fetch_result {
        Ok(fetched) => {
            tracing::debug!(
                identifier = %identifier,
                url = %url,
                oids_fetched = fetched.len(),
                "Fetch succeeded"
            );
            fetched.len()
        }
        Err(e) => {
            tracing::debug!(
                identifier = %identifier,
                url = %url,
                error = %e,
                "Fetch failed"
            );
            0
        }
    };
    
    // Try to process any events that can now be satisfied
    if oids_fetched > 0 {
        let new_oids: HashSet<String> = needed_oids.iter().cloned().collect();
        if let Err(e) = ctx.process_newly_available_git_data(&target_repo, &new_oids).await {
            tracing::warn!(
                identifier = %identifier,
                error = %e,
                "Failed to process newly available git data"
            );
        }
    }
    
    oids_fetched
}
```

### process_newly_available_git_data (Unified Function)

This is the core function that handles the "release from purgatory" logic. It's called after each successful fetch to check if any purgatory events can now be satisfied with the available git data.

**Key Design Decision**: This is a **unified function** shared with the git push handler. Both `handle_receive_pack` (after git push) and `sync_identifier_from_url` (after purgatory sync fetch) call the same function. See [Unified Git Data Sync](unify-git-data-sync.md) for the complete implementation.

**Why unify?**

The post-git-data-available processing is identical regardless of how data arrived:

| Step | After git push | After purgatory fetch |
|------|---------------|----------------------|
| Discover satisfiable events | ✅ Same | ✅ Same |
| Sync OIDs to owner repos | ✅ Same | ✅ Same |
| Align refs (+ set HEAD) | ✅ Same | ✅ Same |
| Save events to database | ✅ Same | ✅ Same |
| Notify WebSocket | ✅ Same | ✅ Same |
| Remove from purgatory | ✅ Same | ✅ Same |

```rust
/// Result of processing newly available git data
#[derive(Debug, Default)]
pub struct ProcessResult {
    /// Number of state events released from purgatory
    pub states_released: usize,
    /// Number of PR events released from purgatory
    pub prs_released: usize,
    /// Number of repositories synced (OIDs copied + refs aligned)
    pub repos_synced: usize,
    /// Number of refs created/updated/deleted
    pub refs_created: usize,
    pub refs_updated: usize,
    pub refs_deleted: usize,
    /// Errors encountered (non-fatal)
    pub errors: Vec<String>,
}

/// Unified processing of newly available git data.
///
/// Called whenever git data becomes available, whether from:
/// - A successful `git push` (handle_receive_pack)
/// - Purgatory sync fetching OIDs from remote servers
///
/// See unify-git-data-sync.md for complete implementation details.
pub async fn process_newly_available_git_data(
    source_repo_path: &Path,
    new_oids: &HashSet<String>,
    database: &SharedDatabase,
    local_relay: Option<&nostr_relay_builder::LocalRelay>,
    purgatory: &Purgatory,
    git_data_path: &Path,
) -> Result<ProcessResult>;
```

**Key properties of the unified function:**

1. **Early release**: If we fetch from `server1.com` and get all OIDs for state event A, we immediately release A even if state event B still needs OIDs from `server2.com`

2. **Idempotent**: The function can be called multiple times safely. It only processes events that are actually satisfiable.

3. **Atomic per-event**: Each event is processed independently. If saving one event fails, others can still succeed.

4. **Authorization at release time**: We check authorization when releasing, not when adding to purgatory. This handles the case where maintainer sets change while an event is in purgatory.

5. **Handles all event types**: Both state events (kind 30618) and PR events (kind 1617/1618) are processed uniformly.

### The Sync Identifier Loop (Main Sync)

```rust
/// Sync git data for an identifier.
/// 
/// This is called by the main sync loop. It:
/// 1. Tries all non-throttled URLs
/// 2. Enqueues with throttled domains for later processing
/// 3. Returns without waiting for throttled domains
/// 
/// Returns true if sync completed (no pending events or no OIDs needed),
/// false if events remain (will be retried after backoff).
pub async fn sync_identifier<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    throttle_manager: &Arc<ThrottleManager>,
) -> bool {
    let mut tried_urls: HashSet<String> = HashSet::new();
    
    // Try all non-throttled URLs
    loop {
        match sync_identifier_next_url(
            ctx,
            identifier,
            None, // Any domain
            &tried_urls,
            throttle_manager,
        ).await {
            Some(url) => {
                // Found a non-throttled URL to try
                sync_identifier_from_url(ctx, identifier, &url, throttle_manager).await;
                tried_urls.insert(url);
                
                // Check if sync is now complete
                if !ctx.has_pending_events(identifier) {
                    tracing::info!(identifier = %identifier, "Sync complete - no pending events");
                    return true;
                }
                
                let needed_oids = ctx.collect_needed_oids(identifier);
                if needed_oids.is_empty() {
                    tracing::info!(identifier = %identifier, "Sync complete - all OIDs available");
                    return true;
                }
                
                // Continue trying more URLs
            }
            None => {
                // No more non-throttled URLs available
                break;
            }
        }
    }
    
    // Check if we're done (no pending events or no needed OIDs)
    if !ctx.has_pending_events(identifier) {
        return true;
    }
    
    let needed_oids = ctx.collect_needed_oids(identifier);
    if needed_oids.is_empty() {
        return true;
    }
    
    // Enqueue with any throttled domains that have untried URLs
    let throttled_domains = get_throttled_domains_with_untried_urls(
        ctx,
        identifier,
        &tried_urls,
        throttle_manager,
    ).await;
    
    for info in throttled_domains {
        tracing::debug!(
            identifier = %identifier,
            domain = %info.domain,
            "Enqueueing with throttled domain"
        );
        throttle_manager.enqueue_identifier(
            &info.domain,
            identifier.to_string(),
            info.tried_urls_for_domain,
        );
    }
    
    // Return false - events remain, will retry after backoff
    // (throttled domains will process independently)
    false
}
```

### The Main Sync Loop

```rust
impl Purgatory {
    pub fn start_sync_loop(
        self: Arc<Self>,
        database: SharedDatabase,
        our_domain: Option<String>,
        local_relay: Option<nostr_relay_builder::LocalRelay>,
        throttle_manager: Arc<ThrottleManager>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            
            loop {
                interval.tick().await;
                
                // Find all ready identifiers
                let ready: Vec<String> = self.sync_queue
                    .iter()
                    .filter(|e| e.value().is_ready())
                    .map(|e| e.key().clone())
                    .collect();
                
                for identifier in ready {
                    // Check if events still exist
                    if !self.has_pending_events(&identifier) {
                        self.sync_queue.remove(&identifier);
                        continue;
                    }
                    
                    // Mark in progress
                    if let Some(mut entry) = self.sync_queue.get_mut(&identifier) {
                        if entry.in_progress {
                            continue;
                        }
                        entry.in_progress = true;
                    } else {
                        continue;
                    }
                    
                    // Spawn sync task
                    let purgatory = self.clone();
                    let db = database.clone();
                    let domain = our_domain.clone();
                    let relay = local_relay.clone();
                    let throttle_manager = throttle_manager.clone();
                    let id = identifier.clone();
                    
                    tokio::spawn(async move {
                        // Create the real SyncContext implementation
                        let ctx = RealSyncContext::new(
                            purgatory.clone(),
                            db,
                            domain,
                            relay,
                        );
                        
                        let complete = sync_identifier(&ctx, &id, &throttle_manager).await;
                        
                        if complete || !purgatory.has_pending_events(&id) {
                            purgatory.sync_queue.remove(&id);
                            tracing::info!(identifier = %id, "Removed from sync queue");
                        } else {
                            // Apply backoff - will retry later
                            // (throttled domains are being processed independently)
                            if let Some(mut entry) = purgatory.sync_queue.get_mut(&id) {
                                entry.on_sync_complete();
                            }
                        }
                    });
                }
            }
        })
    }
}
```

## Testing Strategy

Tests are created **only** as part of each implementation phase. See [Implementation Phases](#implementation-phases) for the complete test plan.

### Design Principles

1. **Tests accompany code**: Each phase specifies exactly which tests to create
2. **Unit tests for mechanics**: Test backoff, throttle, retry logic in isolation using mocks
3. **Integration tests for outcomes**: Verify events sync correctly end-to-end
4. **No speculative tests**: Don't create tests for code that doesn't exist yet

### MockSyncContext

Phases 4-6 use `MockSyncContext` to test sync logic without I/O:

```rust
/// Mock context for testing sync logic
#[cfg(test)]
pub struct MockSyncContext {
    /// Repository data to return
    repo_data: RepositoryData,
    /// OIDs still needed (decremented when "fetched")
    needed_oids: RefCell<HashSet<String>>,
    /// Which OIDs each URL can provide
    url_provides_oids: HashMap<String, HashSet<String>>,
    /// Track fetch attempts for assertions
    fetch_log: RefCell<Vec<String>>,
    /// Whether there are pending events
    has_pending: RefCell<bool>,
}

impl MockSyncContext {
    pub fn new() -> Self;
    pub fn with_urls(self, urls: &[&str]) -> Self;
    pub fn with_needed_oids(self, oids: &[&str]) -> Self;
    pub fn url_provides(self, url: &str, oids: &[&str]) -> Self;
}
```

### Test Locations

| Test Type | Location | Created In |
|-----------|----------|------------|
| SyncQueueEntry | `src/purgatory/sync/queue.rs` | Phase 1 |
| DomainThrottle | `src/purgatory/sync/throttle.rs` | Phase 2 |
| ThrottleManager | `src/purgatory/sync/throttle.rs` | Phase 3 |
| Core sync functions | `src/purgatory/sync/functions.rs` | Phase 5-6 |
| Queue integration | `src/purgatory/mod.rs` | Phase 7 |
| Integration tests | `tests/purgatory_sync.rs` | Phase 10 |

## Implementation Phases

Each phase has clear deliverables, unit tests, and success criteria. Unit tests are created **only** for the code built in that phase.

---

### Phase 1: SyncQueueEntry with Backoff

**Goal**: Implement the sync queue entry struct with backoff calculation.

**Files**:
- `src/purgatory/sync/queue.rs` (new)

**Deliverables**:
```rust
pub struct SyncQueueEntry {
    pub next_attempt: Instant,
    pub attempt_count: u32,
    pub in_progress: bool,
}

impl SyncQueueEntry {
    pub fn new(delay: Duration) -> Self;
    pub fn backoff(&self) -> Duration;
    pub fn is_ready(&self) -> bool;
    pub fn on_new_event(&mut self, delay: Duration);
    pub fn on_sync_complete(&mut self);
}
```

**Unit Tests** (2 tests):
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn backoff_doubles_up_to_cap() {
        // 20s → 40s → 80s → 120s → 120s (capped)
    }
    
    #[test]
    fn new_event_resets_attempt_count() {
        // on_new_event() resets attempt_count to 0
    }
}
```

**Success Criteria**:
- [ ] `SyncQueueEntry::new()` creates entry with given delay
- [ ] `backoff()` returns 20s, 40s, 80s, 120s, 120s for attempts 1-5
- [ ] `on_new_event()` resets `attempt_count` to 0
- [ ] `on_sync_complete()` increments `attempt_count` and updates `next_attempt`
- [ ] Both unit tests pass

---

### Phase 2: DomainThrottle with Rate Limiting and Round-Robin

**Goal**: Implement per-domain throttling with concurrent/rate limits and fair queue processing.

**Files**:
- `src/purgatory/sync/throttle.rs` (new)

**Deliverables**:
```rust
pub struct DomainThrottle {
    domain: String,
    in_flight: u32,
    request_times: VecDeque<Instant>,
    queue: IndexMap<String, IdentifierQueueState>,
    round_robin_index: usize,
    max_concurrent: u32,
    max_per_minute: u32,
}

impl DomainThrottle {
    pub fn new(domain: String, max_concurrent: u32, max_per_minute: u32) -> Self;
    pub fn has_capacity(&self) -> bool;
    pub fn start_request(&mut self);
    pub fn complete_request(&mut self);
    pub fn enqueue_identifier(&mut self, identifier: String, tried_urls: HashSet<String>);
    pub fn next_ready_identifier(&mut self) -> Option<String>;
    pub fn mark_identifier_not_in_progress(&mut self, identifier: &str);
    pub fn remove_identifier(&mut self, identifier: &str);
}
```

**Unit Tests** (4 tests):
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn concurrent_limit_blocks_when_saturated() {
        // has_capacity() returns false when in_flight >= max_concurrent
    }
    
    #[test]
    fn rate_limit_blocks_when_window_full() {
        // has_capacity() returns false when requests in last 60s >= max_per_minute
        // Use deterministic time (pass Instant or mock clock)
    }
    
    #[test]
    fn round_robin_processes_identifiers_fairly() {
        // Enqueue A, B, C → next_ready returns A, B, C, A, B, C...
    }
    
    #[test]
    fn skips_in_progress_identifiers() {
        // next_ready skips identifiers where in_progress=true
    }
}
```

**Success Criteria**:
- [ ] Concurrent limit enforced (blocks at max_concurrent)
- [ ] Rate limit enforced (blocks at max_per_minute within 60s window)
- [ ] Round-robin ordering maintained across calls
- [ ] In-progress identifiers skipped
- [ ] All 4 unit tests pass

---

### Phase 3: ThrottleManager

**Goal**: Implement the manager that owns all domain throttles and provides the sync interface.

**Files**:
- `src/purgatory/sync/throttle.rs` (extend)

**Deliverables**:
```rust
pub struct ThrottleManager {
    throttles: DashMap<String, DomainThrottle>,
    max_concurrent_per_domain: u32,
    max_per_minute_per_domain: u32,
}

impl ThrottleManager {
    pub fn new(max_concurrent: u32, max_per_minute: u32) -> Self;
    pub fn is_throttled(&self, domain: &str) -> bool;
    pub fn start_request(&self, domain: &str);
    pub fn complete_request(&self, domain: &str);
    pub fn enqueue_identifier(&self, domain: &str, identifier: String, tried_urls: HashSet<String>);
}
```

**Unit Tests** (1 test):
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn is_throttled_reflects_domain_capacity() {
        // is_throttled returns true when domain has no capacity
    }
}
```

**Success Criteria**:
- [ ] `is_throttled()` correctly reflects domain capacity
- [ ] `start_request()`/`complete_request()` delegate to correct domain
- [ ] `enqueue_identifier()` creates domain throttle if needed
- [ ] Unit test passes

---

### Phase 4: SyncContext Trait and MockSyncContext

**Goal**: Define the abstraction for sync operations and create the test mock.

**Files**:
- `src/purgatory/sync/context.rs` (new)

**Deliverables**:
```rust
#[async_trait]
pub trait SyncContext: Send + Sync {
    async fn fetch_repository_data(&self, identifier: &str) -> Result<RepositoryData>;
    fn collect_needed_oids(&self, identifier: &str) -> HashSet<String>;
    async fn fetch_oids(&self, repo_path: &Path, url: &str, oids: &[String]) -> Result<Vec<String>>;
    async fn process_newly_available_git_data(
        &self,
        source_repo_path: &Path,
        new_oids: &HashSet<String>,
    ) -> Result<ProcessResult>;
    fn has_pending_events(&self, identifier: &str) -> bool;
    fn find_target_repo(&self, data: &RepositoryData) -> Option<PathBuf>;
    fn our_domain(&self) -> Option<&str>;
}

// Test support
#[cfg(test)]
pub struct MockSyncContext { ... }
```

**Unit Tests** (0 tests):
- This phase creates infrastructure only; tests come in Phase 5

**Success Criteria**:
- [ ] `SyncContext` trait compiles with all required methods
- [ ] `MockSyncContext` implements `SyncContext`
- [ ] Mock supports builder pattern for test setup

---

### Phase 5: Core Sync Functions

**Goal**: Implement `sync_identifier_next_url` and `sync_identifier_from_url`.

**Files**:
- `src/purgatory/sync/functions.rs` (new)

**Deliverables**:
```rust
pub async fn sync_identifier_next_url<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    domain: Option<&str>,
    tried_urls: &HashSet<String>,
    throttle_manager: &ThrottleManager,
) -> Option<String>;

pub async fn sync_identifier_from_url<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    url: &str,
    throttle_manager: &Arc<ThrottleManager>,
) -> usize;
```

**Unit Tests** (3 tests):
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn next_url_skips_throttled_domains() {
        // When domain is throttled, next_url returns URL from different domain
    }
    
    #[tokio::test]
    async fn next_url_skips_tried_urls() {
        // URLs in tried_urls set are not returned
    }
    
    #[tokio::test]
    async fn from_url_fetches_and_processes_on_success() {
        // Successful fetch triggers process_newly_available_git_data
    }
}
```

**Success Criteria**:
- [ ] `sync_identifier_next_url` returns non-throttled, untried URL
- [ ] `sync_identifier_next_url` returns `None` when all URLs tried or throttled
- [ ] `sync_identifier_from_url` calls `fetch_oids` and `process_newly_available_git_data`
- [ ] All 3 unit tests pass

---

### Phase 6: sync_identifier Orchestration

**Goal**: Implement the main sync loop for a single identifier.

**Files**:
- `src/purgatory/sync/functions.rs` (extend)

**Deliverables**:
```rust
pub async fn sync_identifier<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    throttle_manager: &Arc<ThrottleManager>,
) -> bool;  // true if complete, false if pending
```

**Unit Tests** (2 tests):
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn tries_multiple_urls_until_complete() {
        // Tries URL1 (partial), URL2 (partial), URL3 (complete) → returns true
    }
    
    #[tokio::test]
    async fn enqueues_throttled_domains_when_incomplete() {
        // When URLs remain but are throttled, enqueues and returns false
    }
}
```

**Success Criteria**:
- [ ] Loops through available URLs until sync complete or all tried
- [ ] Enqueues with throttled domains when OIDs still needed
- [ ] Returns `true` when all OIDs fetched, `false` otherwise
- [ ] Both unit tests pass

---

### Phase 7: Purgatory Sync Queue Integration

**Goal**: Add sync queue to Purgatory and implement `enqueue_sync`.

**Files**:
- `src/purgatory/mod.rs` (extend)

**Deliverables**:
```rust
impl Purgatory {
    // New field: sync_queue: Arc<DashMap<String, SyncQueueEntry>>
    
    pub fn enqueue_sync(&self, identifier: &str, delay: Duration);
    pub fn has_pending_events(&self, identifier: &str) -> bool;
}
```

**Unit Tests** (1 test):
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn enqueue_sync_debounces_rapid_calls() {
        // Multiple enqueue_sync calls within delay window result in single entry
    }
}
```

**Success Criteria**:
- [ ] `enqueue_sync` adds/updates entry in sync_queue
- [ ] Rapid calls debounce (don't create multiple entries)
- [ ] `has_pending_events` checks both state_events and pr_events
- [ ] Unit test passes

---

### Phase 8: Main Sync Loop

**Goal**: Implement the background sync loop that processes ready identifiers.

**Files**:
- `src/purgatory/sync/loop.rs` (new)

**Deliverables**:
```rust
impl Purgatory {
    pub fn start_sync_loop(
        self: Arc<Self>,
        ctx: Arc<dyn SyncContext>,
        throttle_manager: Arc<ThrottleManager>,
    ) -> JoinHandle<()>;
}
```

**Unit Tests** (0 tests):
- The sync loop is tested via integration tests; unit testing async loops is fragile

**Success Criteria**:
- [ ] Loop runs every 1 second
- [ ] Finds ready identifiers and spawns sync tasks
- [ ] Applies backoff on incomplete syncs
- [ ] Removes completed identifiers from queue

---

### Phase 9: RealSyncContext Implementation

**Goal**: Implement the production `SyncContext` that connects to real systems.

**Files**:
- `src/purgatory/sync/context.rs` (extend)

**Deliverables**:
```rust
pub struct RealSyncContext {
    purgatory: Purgatory,
    database: SharedDatabase,
    git_data_path: PathBuf,
    our_domain: Option<String>,
    local_relay: Option<LocalRelay>,
}

impl SyncContext for RealSyncContext { ... }
```

**Unit Tests** (0 tests):
- `RealSyncContext` is tested via integration tests

**Success Criteria**:
- [ ] All `SyncContext` methods implemented
- [ ] Connects to real database, git, and relay
- [ ] `process_newly_available_git_data` releases events from purgatory

---

### Phase 10: Integration Tests

**Goal**: Verify end-to-end sync behavior with real relay instances.

**Files**:
- `tests/purgatory_sync.rs` (new)

**Integration Tests** (4 tests):
```rust
#[tokio::test]
async fn state_event_syncs_from_remote() {
    // State event enters purgatory, git data fetched, event released
}

#[tokio::test]
async fn pr_event_syncs_from_remote() {
    // PR event enters purgatory, commit fetched, event released
}

#[tokio::test]
async fn concurrent_state_and_pr_sync() {
    // Both event types sync correctly when arriving together
}

#[tokio::test]
async fn partial_oid_aggregation_from_multiple_servers() {
    // OIDs aggregated when no single server has all
}
```

**Success Criteria**:
- [ ] All 4 integration tests pass
- [ ] State events release after git sync
- [ ] PR events release after commit sync
- [ ] Partial OID scenarios handled correctly

---

### Phase 11: Cleanup

**Goal**: Remove old `start_state_sync` code and wire up new system.

**Files**:
- `src/purgatory/mod.rs` (modify)
- `src/main.rs` (modify)

**Deliverables**:
- Remove `start_state_sync` method
- Wire `start_sync_loop` into application startup
- Update `add_state` to call `enqueue_sync`

**Success Criteria**:
- [ ] Old sync code removed
- [ ] New sync loop starts on application boot
- [ ] All existing tests still pass
- [ ] All new tests pass

---

## Test Summary

| Phase | Unit Tests | Integration Tests | Total |
|-------|------------|-------------------|-------|
| 1. SyncQueueEntry | 2 | - | 2 |
| 2. DomainThrottle | 4 | - | 4 |
| 3. ThrottleManager | 1 | - | 1 |
| 4. SyncContext | 0 | - | 0 |
| 5. Core Functions | 3 | - | 3 |
| 6. sync_identifier | 2 | - | 2 |
| 7. Queue Integration | 1 | - | 1 |
| 8. Sync Loop | 0 | - | 0 |
| 9. RealSyncContext | 0 | - | 0 |
| 10. Integration | - | 4 | 4 |
| 11. Cleanup | 0 | - | 0 |
| **Total** | **13** | **4** | **17** |

## Configuration

| Option | CLI Flag | Environment Variable | Default |
|--------|----------|---------------------|---------|
| Sync loop interval | `--sync-loop-interval-ms` | `NGIT_SYNC_LOOP_INTERVAL_MS` | `1000` |
| Domain concurrent limit | `--sync-domain-concurrent` | `NGIT_SYNC_DOMAIN_CONCURRENT` | `5` |
| Domain rate limit | `--sync-domain-rate-limit` | `NGIT_SYNC_DOMAIN_RATE_LIMIT` | `30` |
| Default sync delay | `--sync-default-delay-secs` | `NGIT_SYNC_DEFAULT_DELAY_SECS` | `180` |
| Immediate sync delay | `--sync-immediate-delay-ms` | `NGIT_SYNC_IMMEDIATE_DELAY_MS` | `500` |

## Observability

### Metrics

- `purgatory_sync_queue_size` - Identifiers pending sync
- `purgatory_sync_attempts_total` - Sync attempts per identifier
- `purgatory_sync_oids_fetched_total` - OIDs successfully fetched
- `purgatory_domain_in_flight` - In-flight requests per domain
- `purgatory_domain_requests_total` - Total requests per domain

### Logging

- `INFO`: Successful sync completion, OIDs fetched
- `DEBUG`: URL attempts, throttle decisions, backoff applied
- `WARN`: Fetch failures, processing errors
