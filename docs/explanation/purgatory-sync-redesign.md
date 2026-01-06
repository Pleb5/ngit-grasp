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
┌──────────────────────────────────────────────────────────────────────────────┐
│                              Purgatory                                        │
│                                                                              │
│  ┌─────────────────┐  ┌─────────────────┐                                    │
│  │  State Events   │  │   PR Events     │                                    │
│  │  (by identifier)│  │  (by event_id)  │                                    │
│  └────────┬────────┘  └────────┬────────┘                                    │
│           │                    │                                             │
│           └──────────┬─────────┘                                             │
│                      │ add_state() / add_pr() / trigger_immediate_sync()     │
│                      ▼                                                       │
│           ┌──────────────────────────┐                                       │
│           │      Sync Queue          │                                       │
│           │  DashMap<id, Entry>      │                                       │
│           │                          │                                       │
│           │  Entry {                 │                                       │
│           │    next_attempt,         │  ← delay/backoff timer                │
│           │    attempt_count,        │  ← for backoff calculation            │
│           │    in_progress,          │  ← prevents concurrent runs           │
│           │  }                       │                                       │
│           └────────────┬─────────────┘                                       │
│                        │                                                     │
│  ┌─────────────────────┼─────────────────────────────────────────────────┐   │
│  │                     ▼                                                 │   │
│  │          ┌─────────────────────┐                                      │   │
│  │          │   Main Sync Loop    │  (every 1s)                          │   │
│  │          │                     │                                      │   │
│  │          │  1. Find ALL ready  │                                      │   │
│  │          │     identifiers     │                                      │   │
│  │          │  2. Spawn parallel  │                                      │   │
│  │          │     tasks for each  │───────┐                              │   │
│  │          │  3. Apply backoff   │       │  (parallel tasks)            │   │
│  │          │     when done       │       │                              │   │
│  │          └─────────────────────┘       │                              │   │
│  │                                        ▼                              │   │
│  │                             ┌─────────────────────────────────────┐   │   │
│  │                             │       sync_identifier()             │   │   │
│  │                             │                                     │   │   │
│  │                             │  Owns its own tried_urls: HashSet   │   │   │
│  │                             │                                     │   │   │
│  │                             │  loop:                              │   │   │
│  │                             │    sync_identifier_step()           │   │   │
│  │                             │      → (url_tried, complete)        │   │   │
│  │                             │    if complete: break               │   │   │
│  │                             │    tried_urls.insert(url_tried)     │   │   │
│  │                             └─────────────────────────────────────┘   │   │
│  │                                        │                              │   │
│  │                                        ▼                              │   │
│  │                             ┌─────────────────────────────────────┐   │   │
│  │                             │       Domain Throttle               │   │   │
│  │                             │       (rate limiting only)          │   │   │
│  │                             │                                     │   │   │
│  │                             │  Per-domain state:                  │   │   │
│  │                             │    - in_flight: u32                 │   │   │
│  │                             │    - request_times: VecDeque        │   │   │
│  │                             │                                     │   │   │
│  │                             │  Round-robin via:                   │   │   │
│  │                             │    - last_used_index per domain     │   │   │
│  │                             │    - Caller passes tried_urls       │   │   │
│  │                             └─────────────────────────────────────┘   │   │
│  │                                                                       │   │
│  └───────────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Key Design Principle: Separation of Concerns

The previous design conflated two concerns in `DomainThrottle`:
1. **Rate limiting** (per-domain): How many requests can we make to a domain?
2. **URL tracking** (per-identifier): Which URLs have we tried for this sync?

The new design cleanly separates these:

- **`DomainThrottle`**: Only handles rate limiting. Tracks in-flight requests and request timestamps per domain. Uses round-robin internally to distribute load across URLs.
- **`sync_identifier`**: Owns its `tried_urls: HashSet<String>`. Passes this to the throttle when requesting a URL to try.

This separation enables:
- **Unit testing** of sync logic with a mock throttle
- **Simpler state management** - throttle doesn't need cleanup when identifiers complete
- **Clearer reasoning** about each component's responsibility

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
     - Loops calling `sync_identifier_step()` until complete
     - Step returns `(Option<url_tried>, complete)` - clean testable interface
   - When task completes: apply backoff or remove from queue

3. **Domain throttle**:
   - Pure rate limiting: tracks in_flight count and request timestamps per domain
   - `get_next_url()` takes `available_urls` and `tried_urls`, returns next URL to try
   - Uses round-robin internally to distribute load
   - No per-identifier state needed

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

### DomainThrottle (Rate Limiting Only)

```rust
/// Domain-level rate limiting with round-robin URL selection.
/// 
/// This struct ONLY handles rate limiting. It does not track which URLs
/// have been tried - that's the caller's responsibility.
/// 
/// Rate limits: 5 concurrent requests, 30 requests/minute per domain
pub struct DomainThrottle {
    /// In-flight request count per domain
    in_flight: DashMap<String, u32>,
    
    /// Request timestamps per domain (sliding window)
    request_times: DashMap<String, VecDeque<Instant>>,
    
    /// Round-robin index per domain (for fair URL distribution)
    round_robin_index: DashMap<String, usize>,
    
    max_concurrent: u32,
    max_per_minute: u32,
}

impl DomainThrottle {
    pub fn new(max_concurrent: u32, max_per_minute: u32) -> Self {
        Self {
            in_flight: DashMap::new(),
            request_times: DashMap::new(),
            round_robin_index: DashMap::new(),
            max_concurrent,
            max_per_minute,
        }
    }
    
    /// Check if domain has capacity for another request
    pub fn has_capacity(&self, domain: &str) -> bool {
        let in_flight = self.in_flight.get(domain).map_or(0, |v| *v);
        if in_flight >= self.max_concurrent {
            return false;
        }
        
        let now = Instant::now();
        let window = Duration::from_secs(60);
        self.request_times
            .get(domain)
            .map_or(true, |times| {
                times.iter().filter(|t| now.duration_since(**t) < window).count() 
                    < self.max_per_minute as usize
            })
    }
    
    /// Get next URL to try from available URLs, excluding already-tried URLs.
    /// Uses round-robin to distribute load across URLs for a domain.
    /// 
    /// Returns None if:
    /// - Domain is at capacity (rate limited)
    /// - All available URLs have been tried
    pub fn get_next_url(
        &self,
        domain: &str,
        available_urls: &[String],
        tried_urls: &HashSet<String>,
    ) -> Option<String> {
        if !self.has_capacity(domain) {
            return None;
        }
        
        // Filter to untried URLs
        let untried: Vec<_> = available_urls
            .iter()
            .filter(|url| !tried_urls.contains(*url))
            .collect();
        
        if untried.is_empty() {
            return None;
        }
        
        // Round-robin selection
        let mut index = self.round_robin_index.entry(domain.to_string()).or_insert(0);
        let selected_index = *index % untried.len();
        *index = (*index + 1) % untried.len();
        
        Some(untried[selected_index].clone())
    }
    
    /// Record that a request is starting
    pub fn start_request(&self, domain: &str) {
        *self.in_flight.entry(domain.to_string()).or_insert(0) += 1;
        self.request_times
            .entry(domain.to_string())
            .or_default()
            .push_back(Instant::now());
    }
    
    /// Record that a request completed
    pub fn complete_request(&self, domain: &str) {
        if let Some(mut count) = self.in_flight.get_mut(domain) {
            *count = count.saturating_sub(1);
        }
        
        // Clean old timestamps
        let now = Instant::now();
        let window = Duration::from_secs(60);
        if let Some(mut times) = self.request_times.get_mut(domain) {
            while times.front().map_or(false, |t| now.duration_since(*t) >= window) {
                times.pop_front();
            }
        }
    }
    
    /// Get time until domain has capacity (for scheduling retries)
    pub fn time_until_capacity(&self, domain: &str) -> Option<Duration> {
        // Check concurrent limit first
        let in_flight = self.in_flight.get(domain).map_or(0, |v| *v);
        if in_flight >= self.max_concurrent {
            // Can't predict when a request will complete
            return Some(Duration::from_millis(100));
        }
        
        // Check rate limit
        let now = Instant::now();
        let window = Duration::from_secs(60);
        if let Some(times) = self.request_times.get(domain) {
            let recent_count = times.iter().filter(|t| now.duration_since(**t) < window).count();
            if recent_count >= self.max_per_minute as usize {
                // Find oldest request in window, wait until it expires
                if let Some(oldest) = times.front() {
                    let age = now.duration_since(*oldest);
                    if age < window {
                        return Some(window - age);
                    }
                }
            }
        }
        
        None // Has capacity now
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

### The Sync Step Function

This is the key abstraction that enables clean testing:

```rust
/// Result of a single sync step
#[derive(Debug, Clone, PartialEq)]
pub enum SyncStepResult {
    /// Successfully tried a URL, may or may not have fetched OIDs
    TriedUrl { url: String, oids_fetched: usize },
    
    /// All available URLs have been tried, need to wait for throttle
    AllUrlsThrottled { wait_duration: Duration },
    
    /// No more URLs to try (all exhausted)
    NoMoreUrls,
    
    /// All OIDs are now available, sync complete
    Complete,
    
    /// No pending events remain
    NoPendingEvents,
}

/// Execute one step of the sync process.
/// 
/// This function is pure logic - all I/O goes through the SyncContext trait.
/// This makes it trivially unit testable.
pub async fn sync_identifier_step<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    tried_urls: &HashSet<String>,
    throttle: &DomainThrottle,
) -> Result<SyncStepResult> {
    // 1. Check if we still have pending events
    if !ctx.has_pending_events(identifier) {
        return Ok(SyncStepResult::NoPendingEvents);
    }
    
    // 2. Collect needed OIDs (fresh each step - may have changed)
    let needed_oids = ctx.collect_needed_oids(identifier);
    if needed_oids.is_empty() {
        // No OIDs needed - try to process events
        ctx.process_satisfiable_events(identifier).await?;
        return Ok(SyncStepResult::Complete);
    }
    
    // 3. Get repository data (fresh each step - announcements may have arrived)
    let db_repo_data = ctx.fetch_repository_data(identifier).await?;
    
    // 4. Collect clone URLs, excluding our domain
    let all_urls: Vec<String> = db_repo_data
        .announcements
        .iter()
        .flat_map(|a| a.clone_urls.iter().cloned())
        .filter(|url| ctx.our_domain().map_or(true, |d| !url.contains(d)))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    
    if all_urls.is_empty() {
        return Ok(SyncStepResult::NoMoreUrls);
    }
    
    // 5. Group by domain and find an available URL
    let urls_by_domain: HashMap<String, Vec<String>> = all_urls
        .iter()
        .fold(HashMap::new(), |mut acc, url| {
            acc.entry(extract_domain(url)).or_default().push(url.clone());
            acc
        });
    
    // 6. Try to get a URL from any domain that has capacity
    let mut min_wait: Option<Duration> = None;
    
    for (domain, domain_urls) in &urls_by_domain {
        if let Some(url) = throttle.get_next_url(domain, domain_urls, tried_urls) {
            // Found a URL to try!
            let target_repo = match ctx.find_target_repo(&db_repo_data) {
                Some(path) => path,
                None => return Ok(SyncStepResult::NoMoreUrls),
            };
            
            // Start the fetch
            throttle.start_request(domain);
            let oids_to_fetch: Vec<String> = needed_oids.iter().cloned().collect();
            let fetch_result = ctx.fetch_oids(&target_repo, &url, &oids_to_fetch).await;
            throttle.complete_request(domain);
            
            let oids_fetched = match fetch_result {
                Ok(fetched) => fetched.len(),
                Err(e) => {
                    tracing::debug!(url = %url, error = %e, "Fetch failed");
                    0
                }
            };
            
            // Try to process any events that can now be satisfied
            if oids_fetched > 0 {
                let _ = ctx.process_satisfiable_events(identifier).await;
            }
            
            return Ok(SyncStepResult::TriedUrl { url, oids_fetched });
        } else {
            // Domain throttled or all URLs tried
            let untried_exist = domain_urls.iter().any(|u| !tried_urls.contains(u));
            if untried_exist {
                // URLs exist but domain is throttled
                if let Some(wait) = throttle.time_until_capacity(domain) {
                    min_wait = Some(min_wait.map_or(wait, |m| m.min(wait)));
                }
            }
        }
    }
    
    // Check if all URLs have been tried
    let all_tried = all_urls.iter().all(|url| tried_urls.contains(url));
    if all_tried {
        return Ok(SyncStepResult::NoMoreUrls);
    }
    
    // Some URLs exist but all domains are throttled
    Ok(SyncStepResult::AllUrlsThrottled {
        wait_duration: min_wait.unwrap_or(Duration::from_millis(100)),
    })
}
```

### The Sync Identifier Loop

```rust
/// Sync git data for an identifier.
/// 
/// Returns true if sync completed successfully (no more pending events),
/// false if we exhausted all options but events remain.
pub async fn sync_identifier<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    throttle: &DomainThrottle,
) -> bool {
    let mut tried_urls: HashSet<String> = HashSet::new();
    
    loop {
        match sync_identifier_step(ctx, identifier, &tried_urls, throttle).await {
            Ok(SyncStepResult::TriedUrl { url, oids_fetched }) => {
                tried_urls.insert(url.clone());
                tracing::debug!(
                    identifier = %identifier,
                    url = %url,
                    oids_fetched = oids_fetched,
                    "Tried URL"
                );
                // Continue looping
            }
            
            Ok(SyncStepResult::AllUrlsThrottled { wait_duration }) => {
                tracing::debug!(
                    identifier = %identifier,
                    wait_ms = wait_duration.as_millis(),
                    "All domains throttled, waiting"
                );
                tokio::time::sleep(wait_duration).await;
                // Continue looping
            }
            
            Ok(SyncStepResult::NoMoreUrls) => {
                tracing::debug!(identifier = %identifier, "No more URLs to try");
                return false; // Events remain but no URLs left
            }
            
            Ok(SyncStepResult::Complete) => {
                tracing::info!(identifier = %identifier, "Sync complete");
                return true;
            }
            
            Ok(SyncStepResult::NoPendingEvents) => {
                tracing::debug!(identifier = %identifier, "No pending events");
                return true;
            }
            
            Err(e) => {
                tracing::warn!(identifier = %identifier, error = %e, "Sync step error");
                return false;
            }
        }
    }
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
        throttle: Arc<DomainThrottle>,
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
                    let throttle = throttle.clone();
                    let id = identifier.clone();
                    
                    tokio::spawn(async move {
                        // Create the real SyncContext implementation
                        let ctx = RealSyncContext::new(
                            purgatory.clone(),
                            db,
                            domain,
                            relay,
                        );
                        
                        let complete = sync_identifier(&ctx, &id, &throttle).await;
                        
                        if complete || !purgatory.has_pending_events(&id) {
                            purgatory.sync_queue.remove(&id);
                            tracing::info!(identifier = %id, "Removed from sync queue");
                        } else {
                            // Apply backoff
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
            // Return mock data with our available_urls
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
            // Return pre-configured fetch result for this URL
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
    async fn test_sync_step_no_pending_events() {
        let ctx = MockSyncContext {
            pending_events: RefCell::new(false),
            ..Default::default()
        };
        let throttle = DomainThrottle::new(5, 30);
        let tried = HashSet::new();
        
        let result = sync_identifier_step(&ctx, "test", &tried, &throttle).await.unwrap();
        assert_eq!(result, SyncStepResult::NoPendingEvents);
    }
    
    #[tokio::test]
    async fn test_sync_step_no_oids_needed() {
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(HashSet::new()), // Empty = no OIDs needed
            ..Default::default()
        };
        let throttle = DomainThrottle::new(5, 30);
        let tried = HashSet::new();
        
        let result = sync_identifier_step(&ctx, "test", &tried, &throttle).await.unwrap();
        assert_eq!(result, SyncStepResult::Complete);
        assert_eq!(*ctx.processed_count.borrow(), 1);
    }
    
    #[tokio::test]
    async fn test_sync_step_tries_url() {
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
        let throttle = DomainThrottle::new(5, 30);
        let tried = HashSet::new();
        
        let result = sync_identifier_step(&ctx, "test", &tried, &throttle).await.unwrap();
        
        match result {
            SyncStepResult::TriedUrl { url, oids_fetched } => {
                assert_eq!(url, "https://example.com/repo.git");
                assert_eq!(oids_fetched, 1);
            }
            _ => panic!("Expected TriedUrl, got {:?}", result),
        }
    }
    
    #[tokio::test]
    async fn test_sync_step_all_urls_tried() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed),
            available_urls: vec!["https://example.com/repo.git".to_string()],
            fetch_results: RefCell::new(HashMap::new()),
            processed_count: RefCell::new(0),
        };
        let throttle = DomainThrottle::new(5, 30);
        
        // Mark the only URL as tried
        let mut tried = HashSet::new();
        tried.insert("https://example.com/repo.git".to_string());
        
        let result = sync_identifier_step(&ctx, "test", &tried, &throttle).await.unwrap();
        assert_eq!(result, SyncStepResult::NoMoreUrls);
    }
    
    #[tokio::test]
    async fn test_sync_step_domain_throttled() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        
        let ctx = MockSyncContext {
            pending_events: RefCell::new(true),
            needed_oids: RefCell::new(needed),
            available_urls: vec!["https://example.com/repo.git".to_string()],
            fetch_results: RefCell::new(HashMap::new()),
            processed_count: RefCell::new(0),
        };
        
        // Create throttle with 0 concurrent limit
        let throttle = DomainThrottle::new(0, 30);
        let tried = HashSet::new();
        
        let result = sync_identifier_step(&ctx, "test", &tried, &throttle).await.unwrap();
        
        match result {
            SyncStepResult::AllUrlsThrottled { .. } => {}
            _ => panic!("Expected AllUrlsThrottled, got {:?}", result),
        }
    }
    
    #[tokio::test]
    async fn test_full_sync_loop() {
        let mut needed = HashSet::new();
        needed.insert("abc123".to_string());
        needed.insert("def456".to_string());
        
        let mut fetch_results = HashMap::new();
        // First URL returns one OID
        fetch_results.insert(
            "https://server1.com/repo.git".to_string(),
            vec!["abc123".to_string()],
        );
        // Second URL returns the other
        fetch_results.insert(
            "https://server2.com/repo.git".to_string(),
            vec!["def456".to_string()],
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
        
        // Simulate OIDs being removed as they're fetched
        // (In real code, collect_needed_oids would return fewer OIDs)
        
        let throttle = DomainThrottle::new(5, 30);
        let complete = sync_identifier(&ctx, "test", &throttle).await;
        
        // Should have tried both URLs
        assert!(*ctx.processed_count.borrow() >= 1);
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

1. **Phase 1**: Add new data structures (SyncQueueEntry, DomainThrottle, SyncContext trait)
2. **Phase 2**: Implement `sync_identifier_step` with unit tests
3. **Phase 3**: Implement main sync loop alongside existing `start_state_sync`
4. **Phase 4**: Add PR event syncing
5. **Phase 5**: Remove old `start_state_sync` code

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
