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
    
    /// Process events that can now be satisfied (save to DB, notify, remove from purgatory)
    async fn process_satisfiable_events(&self, identifier: &str) -> Result<()>;
    
    /// Check if there are still pending events for this identifier
    fn has_pending_events(&self, identifier: &str) -> bool;
    
    /// Find the best local repo to fetch into
    fn find_target_repo(&self, db_repo_data: &RepositoryData) -> Option<PathBuf>;
    
    /// Our domain (to exclude from clone URLs)
    fn our_domain(&self) -> Option<&str>;
}
```

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
        if let Err(e) = ctx.process_satisfiable_events(identifier).await {
            tracing::warn!(
                identifier = %identifier,
                error = %e,
                "Failed to process satisfiable events"
            );
        }
    }
    
    oids_fetched
}
```

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
                    // Process any remaining satisfiable events
                    let _ = ctx.process_satisfiable_events(identifier).await;
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
        let _ = ctx.process_satisfiable_events(identifier).await;
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

### Unit Tests for Sync Logic

The `SyncContext` trait enables pure unit tests without any I/O:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    /// Mock context for testing sync logic
    struct MockSyncContext {
        pending_events: RefCell<bool>,
        needed_oids: RefCell<HashSet<String>>,
        available_urls: Vec<String>,
        fetch_results: RefCell<HashMap<String, Vec<String>>>,
        processed_count: RefCell<usize>,
    }
    
    #[async_trait]
    impl SyncContext for MockSyncContext {
        async fn fetch_repository_data(&self, _id: &str) -> Result<RepositoryData> {
            Ok(RepositoryData {
                announcements: vec![MockAnnouncement {
                    clone_urls: self.available_urls.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            })
        }
        
        fn collect_needed_oids(&self, _id: &str) -> HashSet<String> {
            self.needed_oids.borrow().clone()
        }
        
        fn oid_exists(&self, _path: &Path, oid: &str) -> bool {
            !self.needed_oids.borrow().contains(oid)
        }
        
        async fn fetch_oids(&self, _path: &Path, url: &str, _oids: &[String]) -> Result<Vec<String>> {
            Ok(self.fetch_results.borrow().get(url).cloned().unwrap_or_default())
        }
        
        async fn process_satisfiable_events(&self, _id: &str) -> Result<()> {
            *self.processed_count.borrow_mut() += 1;
            Ok(())
        }
        
        fn has_pending_events(&self, _id: &str) -> bool {
            *self.pending_events.borrow()
        }
        
        fn find_target_repo(&self, _data: &RepositoryData) -> Option<PathBuf> {
            Some(PathBuf::from("/tmp/test-repo"))
        }
        
        fn our_domain(&self) -> Option<&str> {
            None
        }
    }
    
    #[tokio::test]
    async fn test_next_url_no_pending_events() {
        let ctx = MockSyncContext {
            pending_events: RefCell::new(false),
            needed_oids: RefCell::new(HashSet::new()),
            available_urls: vec!["https://example.com/repo.git".to_string()],
            ..Default::default()
        };
        let throttle_manager = ThrottleManager::new(5, 30);
        let tried = HashSet::new();
        
        let result = sync_identifier_next_url(&ctx, "test", None, &tried, &throttle_manager).await;
        assert!(result.is_none());
    }
    
    #[tokio::test]
    async fn test_next_url_no_oids_needed() {
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(HashSet::new()), // Empty = no OIDs needed
            available_urls: vec!["https://example.com/repo.git".to_string()],
            ..Default::default()
        };
        let throttle_manager = ThrottleManager::new(5, 30);
        let tried = HashSet::new();
        
        let result = sync_identifier_next_url(&ctx, "test", None, &tried, &throttle_manager).await;
        assert!(result.is_none()); // No URL needed, sync is complete
    }
    
    #[tokio::test]
    async fn test_next_url_returns_non_throttled() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed),
            available_urls: vec!["https://example.com/repo.git".to_string()],
            ..Default::default()
        };
        let throttle_manager = ThrottleManager::new(5, 30);
        let tried = HashSet::new();
        
        let result = sync_identifier_next_url(&ctx, "test", None, &tried, &throttle_manager).await;
        assert_eq!(result, Some("https://example.com/repo.git".to_string()));
    }
    
    #[tokio::test]
    async fn test_next_url_skips_tried() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed),
            available_urls: vec![
                "https://example.com/repo.git".to_string(),
                "https://other.com/repo.git".to_string(),
            ],
            ..Default::default()
        };
        let throttle_manager = ThrottleManager::new(5, 30);
        
        let mut tried = HashSet::new();
        tried.insert("https://example.com/repo.git".to_string());
        
        let result = sync_identifier_next_url(&ctx, "test", None, &tried, &throttle_manager).await;
        assert_eq!(result, Some("https://other.com/repo.git".to_string()));
    }
    
    #[tokio::test]
    async fn test_next_url_specific_domain() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed),
            available_urls: vec![
                "https://example.com/repo.git".to_string(),
                "https://other.com/repo.git".to_string(),
            ],
            ..Default::default()
        };
        let throttle_manager = ThrottleManager::new(5, 30);
        let tried = HashSet::new();
        
        // Request specific domain
        let result = sync_identifier_next_url(
            &ctx, "test", Some("other.com"), &tried, &throttle_manager
        ).await;
        assert_eq!(result, Some("https://other.com/repo.git".to_string()));
    }
    
    #[tokio::test]
    async fn test_next_url_none_when_all_tried() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed),
            available_urls: vec!["https://example.com/repo.git".to_string()],
            ..Default::default()
        };
        let throttle_manager = ThrottleManager::new(5, 30);
        
        let mut tried = HashSet::new();
        tried.insert("https://example.com/repo.git".to_string());
        
        let result = sync_identifier_next_url(&ctx, "test", None, &tried, &throttle_manager).await;
        assert!(result.is_none());
    }
    
    #[tokio::test]
    async fn test_from_url_fetches_and_processes() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let mut fetch_results = HashMap::new();
        fetch_results.insert(
            "https://example.com/repo.git".to_string(),
            vec!["abc123".to_string()],
        );
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed),
            available_urls: vec!["https://example.com/repo.git".to_string()],
            fetch_results: RefCell::new(fetch_results),
            processed_count: RefCell::new(0),
        };
        let throttle_manager = Arc::new(ThrottleManager::new(5, 30));
        
        let oids_fetched = sync_identifier_from_url(
            &ctx, "test", "https://example.com/repo.git", &throttle_manager
        ).await;
        
        assert_eq!(oids_fetched, 1);
        assert_eq!(*ctx.processed_count.borrow(), 1);
    }
    
    #[tokio::test]
    async fn test_full_sync_with_throttled_domains() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let mut fetch_results = HashMap::new();
        fetch_results.insert(
            "https://server1.com/repo.git".to_string(),
            vec![], // First server doesn't have the OID
        );
        fetch_results.insert(
            "https://server2.com/repo.git".to_string(),
            vec!["abc123".to_string()], // Second server has it
        );
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed.clone()),
            available_urls: vec![
                "https://server1.com/repo.git".to_string(),
                "https://server2.com/repo.git".to_string(),
            ],
            fetch_results: RefCell::new(fetch_results),
            processed_count: RefCell::new(0),
        };
        
        let throttle_manager = Arc::new(ThrottleManager::new(5, 30));
        
        // Manually throttle server2.com to test enqueueing
        // (In real code, this would happen due to rate limits)
        // For this test, we just verify the sync tries available URLs
        
        let complete = sync_identifier(&ctx, "test", &throttle_manager).await;
        
        // Should have processed events (found OID from server2)
        assert!(*ctx.processed_count.borrow() >= 1);
    }
    
    #[tokio::test]
    async fn test_domain_throttle_queue_round_robin() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 30);
        
        // Enqueue three identifiers
        throttle.enqueue_identifier("id1".to_string(), HashSet::new());
        throttle.enqueue_identifier("id2".to_string(), HashSet::new());
        throttle.enqueue_identifier("id3".to_string(), HashSet::new());
        
        // Should get them in round-robin order
        assert_eq!(throttle.next_ready_identifier(), Some("id1".to_string()));
        throttle.mark_identifier_not_in_progress("id1");
        
        assert_eq!(throttle.next_ready_identifier(), Some("id2".to_string()));
        throttle.mark_identifier_not_in_progress("id2");
        
        assert_eq!(throttle.next_ready_identifier(), Some("id3".to_string()));
        throttle.mark_identifier_not_in_progress("id3");
        
        // Back to id1
        assert_eq!(throttle.next_ready_identifier(), Some("id1".to_string()));
    }
    
    #[tokio::test]
    async fn test_domain_throttle_skips_in_progress() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 30);
        
        throttle.enqueue_identifier("id1".to_string(), HashSet::new());
        throttle.enqueue_identifier("id2".to_string(), HashSet::new());
        
        // Get id1 (marks it in_progress)
        assert_eq!(throttle.next_ready_identifier(), Some("id1".to_string()));
        
        // Next should skip id1 and return id2
        assert_eq!(throttle.next_ready_identifier(), Some("id2".to_string()));
        
        // Both in progress, should return None
        assert_eq!(throttle.next_ready_identifier(), None);
        
        // Mark id1 not in progress
        throttle.mark_identifier_not_in_progress("id1");
        
        // Now id1 should be available again
        assert_eq!(throttle.next_ready_identifier(), Some("id1".to_string()));
    }
    
    #[tokio::test]
    async fn test_domain_throttle_remove_adjusts_index() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 30);
        
        throttle.enqueue_identifier("id1".to_string(), HashSet::new());
        throttle.enqueue_identifier("id2".to_string(), HashSet::new());
        throttle.enqueue_identifier("id3".to_string(), HashSet::new());
        
        // Advance to id2
        assert_eq!(throttle.next_ready_identifier(), Some("id1".to_string()));
        throttle.mark_identifier_not_in_progress("id1");
        
        // Remove id1 (before current index)
        throttle.remove_identifier("id1");
        
        // Should continue with id2 (not skip to id3)
        assert_eq!(throttle.next_ready_identifier(), Some("id2".to_string()));
    }
    
    #[tokio::test]
    async fn test_domain_throttle_has_queued_work() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 30);
        
        assert!(!throttle.has_queued_work());
        
        throttle.enqueue_identifier("id1".to_string(), HashSet::new());
        assert!(throttle.has_queued_work());
        
        throttle.remove_identifier("id1");
        assert!(!throttle.has_queued_work());
    }
    
    #[tokio::test]
    async fn test_domain_throttle_tried_urls_merge() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 30);
        
        let mut urls1 = HashSet::new();
        urls1.insert("url1".to_string());
        throttle.enqueue_identifier("id1".to_string(), urls1);
        
        // Enqueue again with different tried URLs - should merge
        let mut urls2 = HashSet::new();
        urls2.insert("url2".to_string());
        throttle.enqueue_identifier("id1".to_string(), urls2);
        
        let tried = throttle.get_tried_urls("id1");
        assert!(tried.contains("url1"));
        assert!(tried.contains("url2"));
    }
}
```

### Integration Tests

1. **Sync against own implementation**: Two ngit-grasp instances syncing
2. **Burst handling**: 10 events in 100ms, verify debounce
3. **Backoff behavior**: Unreachable URLs, verify timing
4. **Rate limiting**: Verify 30 req/min and 5 concurrent limits
5. **Parallel identifiers**: 5 identifiers sync in parallel

## Migration Path

1. **Phase 1**: Add new data structures (SyncQueueEntry, ThrottleManager, DomainThrottle, SyncContext trait)
2. **Phase 2**: Implement `sync_identifier_next_url` and `sync_identifier_from_url` with unit tests
3. **Phase 3**: Implement `sync_identifier` and main sync loop alongside existing `start_state_sync`
4. **Phase 4**: Implement ThrottleManager trigger-based processing
5. **Phase 5**: Add PR event syncing
6. **Phase 6**: Remove old `start_state_sync` code

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
