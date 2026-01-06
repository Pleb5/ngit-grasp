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
│  │                             │  while (events && oids && urls):    │   │   │
│  │                             │    1. Recalc URLs/OIDs (fresh)      │   │   │
│  │                             │    2. Get untried URLs per domain   │   │   │
│  │                             │    3. Skip throttled domains        │   │   │
│  │                             │    4. Try available URLs            │   │   │
│  │                             │       (respecting domain limits)    │   │   │
│  │                             │    5. Process satisfiable events    │   │   │
│  │                             │    6. Loop catches new events/URLs  │   │   │
│  │                             └─────────────────────────────────────┘   │   │
│  │                                        │                              │   │
│  │                                        ▼                              │   │
│  │                             ┌─────────────────────────────────────┐   │   │
│  │                             │       Domain Throttle               │   │   │
│  │                             │                                     │   │   │
│  │                             │  Per-domain state:                  │   │   │
│  │                             │    - 5 concurrent requests max      │   │   │
│  │                             │    - 30 requests/min sliding window │   │   │
│  │                             │                                     │   │   │
│  │                             │  Per (domain, identifier) state:    │   │   │
│  │                             │    - in_progress: bool              │   │   │
│  │                             │    - urls_tried: HashSet<String>    │   │   │
│  │                             │                                     │   │   │
│  │                             │  Identifier removed when all URLs   │   │   │
│  │                             │  tried. Re-added on next sync.      │   │   │
│  │                             └─────────────────────────────────────┘   │   │
│  │                                                                       │   │
│  └───────────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Flow Summary

1. **Event arrives** → added to state_events/pr_events + sync_queue with delay
   - User-submitted: 3 minute delay (expect git push to follow)
   - Sync-triggered: 500ms delay (batch burst arrivals)
   - `enqueue_sync()` resets `attempt_count` to 0 and updates `next_attempt` if needed

2. **Main sync loop** (every 1s):
   - Finds ALL ready identifiers (where `!in_progress && next_attempt <= now`)
   - Spawns parallel tasks for each (marks `in_progress = true`)
   - Each `sync_identifier()` task runs a while loop:
     - Recalculates URLs and OIDs fresh each iteration (catches new events/announcements)
     - For each domain, gets untried URLs (via `domain_throttle`)
     - Skips domains that are fully throttled
     - Tries non-throttled domains first, then throttled if still need OIDs
     - Respects per-domain concurrency (5 max) and rate limits (30/min)
     - Processes satisfiable events
     - Continues while: events remain AND OIDs missing AND untried URLs exist
   - When task completes (regardless of outcome):
     - If `next_attempt` is in the future (new event arrived): just clear `in_progress`
     - Otherwise: increment `attempt_count`, set `next_attempt = now + backoff`, clear `in_progress`
   - If no pending events remain: remove identifier from sync_queue

3. **Domain throttle state management**:
   - Tracks `(in_progress, urls_tried)` per `(domain, identifier)` pair
   - `in_progress` prevents parallel fetches to same domain for same identifier
   - `urls_tried` tracks which URLs have been attempted
   - When all URLs for a domain+identifier are tried, that entry is removed
   - Entry may be re-added on next sync attempt if new URLs available

### New Data Structures

#### SyncQueueEntry

Tracks sync state for each identifier in the main sync queue:

```rust
/// Entry in the sync queue tracking when/how to sync an identifier
#[derive(Debug, Clone)]
pub struct SyncQueueEntry {
    /// Don't attempt sync before this time
    /// Set for: initial delay (3min user / 500ms sync), backoff after attempts
    pub next_attempt: Instant,
    
    /// Number of sync attempts (for backoff calculation)
    /// Reset to 0 when new event arrives for this identifier
    pub attempt_count: u32,
    
    /// Whether a sync is currently in progress for this identifier
    /// Prevents concurrent sync runs for the same identifier
    pub in_progress: bool,
}

impl SyncQueueEntry {
    /// Create new entry with specified delay
    pub fn new(delay: Duration) -> Self {
        Self {
            next_attempt: Instant::now() + delay,
            attempt_count: 0,
            in_progress: false,
        }
    }
    
    /// Calculate backoff duration: 20s, 40s, 80s, 120s (max 2min)
    pub fn backoff(&self) -> Duration {
        let base = Duration::from_secs(20);
        let multiplier = 2u32.saturating_pow(self.attempt_count.saturating_sub(1).min(3));
        let backoff = base * multiplier;
        backoff.min(Duration::from_secs(120)) // Cap at 2 minutes
    }
    
    /// Check if this entry is ready to sync (not in progress, delay passed)
    pub fn is_ready(&self) -> bool {
        !self.in_progress && Instant::now() >= self.next_attempt
    }
    
    /// Called when new event arrives - resets attempt_count, may update next_attempt
    pub fn on_new_event(&mut self, delay: Duration) {
        self.attempt_count = 0;
        let new_attempt = Instant::now() + delay;
        // Only bring forward if new time is sooner
        if new_attempt < self.next_attempt {
            self.next_attempt = new_attempt;
        }
    }
    
    /// Called when sync attempt completes
    /// If next_attempt is in the future (new event arrived during sync), just clear in_progress
    /// Otherwise, apply backoff
    pub fn on_sync_complete(&mut self) {
        self.in_progress = false;
        let now = Instant::now();
        if self.next_attempt <= now {
            // No new event arrived during sync - apply backoff
            self.attempt_count += 1;
            self.next_attempt = now + self.backoff();
        }
        // else: new event arrived during sync, next_attempt already set, don't apply backoff
    }
}
```

#### DomainThrottle

Tracks per-domain rate limiting and per-(domain, identifier) fetch state:

```rust
/// Tracks domain-level rate limiting and per-identifier fetch state
/// 
/// Rate limits: 5 concurrent requests, 30 requests/minute per domain
/// 
/// Per (domain, identifier): tracks which URLs have been tried and whether
/// a fetch is currently in progress. When all URLs are tried, the identifier
/// is removed from that domain's tracking. It may be re-added on the next
/// sync attempt if new URLs become available.
pub struct DomainThrottle {
    /// Per-domain, per-identifier state: (in_progress, urls_tried)
    /// domain -> identifier -> (in_progress, urls_tried)
    state: DashMap<String, DashMap<String, (bool, HashSet<String>)>>,
    
    /// Count of currently in-flight requests per domain
    in_flight: DashMap<String, u32>,
    
    /// Request timestamps per domain (sliding window for rate limiting)
    request_times: DashMap<String, VecDeque<Instant>>,
    
    /// Maximum concurrent requests per domain
    max_concurrent: u32,
    
    /// Maximum requests per minute per domain
    max_per_minute: u32,
}

impl DomainThrottle {
    pub fn new(max_concurrent: u32, max_per_minute: u32) -> Self {
        Self {
            state: DashMap::new(),
            in_flight: DashMap::new(),
            request_times: DashMap::new(),
            max_concurrent,
            max_per_minute,
        }
    }
    
    /// Check if domain has capacity for another request
    /// (both concurrent limit and rate limit)
    pub fn has_capacity(&self, domain: &str) -> bool {
        // Check concurrent limit
        let current_in_flight = self.in_flight.get(domain).map_or(0, |v| *v);
        if current_in_flight >= self.max_concurrent {
            return false;
        }
        
        // Check rate limit (sliding window)
        let now = Instant::now();
        let window = Duration::from_secs(60);
        self.request_times
            .get(domain)
            .map_or(true, |times| {
                times.iter().filter(|t| now.duration_since(**t) < window).count() 
                    < self.max_per_minute as usize
            })
    }
    
    /// Check if identifier is currently fetching from this domain
    pub fn is_in_progress(&self, domain: &str, identifier: &str) -> bool {
        self.state
            .get(domain)
            .and_then(|domain_state| domain_state.get(identifier))
            .map_or(false, |entry| entry.0)
    }
    
    /// Get untried URLs for an identifier from a specific domain
    /// Returns URLs from `available_urls` that haven't been tried yet
    pub fn get_untried_urls(
        &self, 
        domain: &str, 
        identifier: &str, 
        available_urls: &[String]
    ) -> Vec<String> {
        let tried = self.state
            .get(domain)
            .and_then(|domain_state| domain_state.get(identifier))
            .map(|entry| entry.1.clone())
            .unwrap_or_default();
        
        available_urls
            .iter()
            .filter(|url| !tried.contains(*url))
            .cloned()
            .collect()
    }
    
    /// Mark a URL as being fetched (in progress)
    /// Returns false if domain has no capacity or identifier already in progress for this domain
    pub fn start_fetch(&self, domain: &str, identifier: &str, url: &str) -> bool {
        // Check capacity
        if !self.has_capacity(domain) {
            return false;
        }
        
        // Check if already in progress for this domain+identifier
        if self.is_in_progress(domain, identifier) {
            return false;
        }
        
        // Increment in-flight counter
        *self.in_flight.entry(domain.to_string()).or_insert(0) += 1;
        
        // Record request time
        self.request_times
            .entry(domain.to_string())
            .or_default()
            .push_back(Instant::now());
        
        // Mark in progress and add URL to tried set
        self.state
            .entry(domain.to_string())
            .or_default()
            .entry(identifier.to_string())
            .and_modify(|(in_progress, tried)| {
                *in_progress = true;
                tried.insert(url.to_string());
            })
            .or_insert_with(|| {
                let mut tried = HashSet::new();
                tried.insert(url.to_string());
                (true, tried)
            });
        
        true
    }
    
    /// Mark a fetch as complete
    pub fn complete_fetch(&self, domain: &str, identifier: &str) {
        // Decrement in-flight counter
        if let Some(mut count) = self.in_flight.get_mut(domain) {
            *count = count.saturating_sub(1);
        }
        
        // Clear in_progress flag
        if let Some(domain_state) = self.state.get(domain) {
            if let Some(mut entry) = domain_state.get_mut(identifier) {
                entry.0 = false;
            }
        }
        
        // Clean old request times
        let now = Instant::now();
        let window = Duration::from_secs(60);
        if let Some(mut times) = self.request_times.get_mut(domain) {
            while times.front().map_or(false, |t| now.duration_since(*t) >= window) {
                times.pop_front();
            }
        }
    }
    
    /// Check if all URLs have been tried for this domain+identifier
    pub fn all_urls_tried(
        &self, 
        domain: &str, 
        identifier: &str, 
        available_urls: &[String]
    ) -> bool {
        self.get_untried_urls(domain, identifier, available_urls).is_empty()
    }
    
    /// Remove identifier from a domain's tracking (called when all URLs tried)
    pub fn remove_identifier_from_domain(&self, domain: &str, identifier: &str) {
        if let Some(domain_state) = self.state.get(domain) {
            domain_state.remove(identifier);
        }
    }
    
    /// Remove identifier from all domains (called when sync complete or events expired)
    pub fn remove_identifier(&self, identifier: &str) {
        for domain_entry in self.state.iter() {
            domain_entry.value().remove(identifier);
        }
    }
    
    /// Get all domains where this identifier has untried URLs
    pub fn domains_with_untried_urls(&self, identifier: &str) -> Vec<String> {
        self.state
            .iter()
            .filter(|entry| entry.value().contains_key(identifier))
            .map(|entry| entry.key().clone())
            .collect()
    }
}
```

### Modified Purgatory Structure

```rust
pub struct Purgatory {
    /// State events (kind 30618) indexed by repository identifier
    state_events: Arc<DashMap<String, Vec<StatePurgatoryEntry>>>,

    /// PR events (kind 1617/1618) indexed by event ID
    pr_events: Arc<DashMap<String, PrPurgatoryEntry>>,

    /// NEW: Sync queue - identifiers pending git data sync
    sync_queue: Arc<DashMap<String, SyncQueueEntry>>,
    
    /// NEW: Domain-level throttling and per-identifier fetch state
    domain_throttle: Arc<DomainThrottle>,

    git_data_path: PathBuf,
}
```

### Key Methods

#### Adding Events to Purgatory

```rust
impl Purgatory {
    /// Add a state event to purgatory (user-submitted, 3min delay)
    pub fn add_state(&self, event: Event, identifier: String, author: PublicKey) {
        // ... existing logic to add to state_events ...
        
        // Add to sync queue with 3 minute delay
        self.enqueue_sync(&identifier, Duration::from_secs(180));
    }
    
    /// Add a PR event to purgatory (user-submitted, 3min delay)  
    pub fn add_pr(&self, event: Event, event_id: String, commit: String) {
        // ... existing logic to add to pr_events ...
        
        // Extract identifier from event's `a` tag and enqueue sync
        if let Some(identifier) = extract_identifier_from_pr(&event) {
            self.enqueue_sync(&identifier, Duration::from_secs(180));
        }
    }
    
    /// Trigger immediate sync for an identifier (called from negentropy sync)
    /// Still applies 500ms debounce for batching burst arrivals
    pub fn trigger_immediate_sync(&self, identifier: &str) {
        self.enqueue_sync(identifier, Duration::from_millis(500));
    }
    
    /// Internal: Add identifier to sync queue with specified delay
    fn enqueue_sync(&self, identifier: &str, delay: Duration) {
        self.sync_queue
            .entry(identifier.to_string())
            .and_modify(|entry| {
                // New event arrived - reset backoff, potentially update timing
                entry.reset(delay);
            })
            .or_insert_with(|| SyncQueueEntry::new(delay));
    }
}
```

#### Extracting Identifier from PR Events

```rust
/// Extract repository identifier from PR event's `a` tag
fn extract_identifier_from_pr(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let tag_vec = tag.clone().to_vec();
        if tag_vec.len() >= 2 && tag_vec[0] == "a" && tag_vec[1].starts_with("30617:") {
            // Format: 30617:<owner>:<identifier>
            let parts: Vec<&str> = tag_vec[1].split(':').collect();
            if parts.len() >= 3 {
                return Some(parts[2].to_string());
            }
        }
        None
    })
}
```

### Sync Loop

A single background loop handles syncing. The `DomainThrottle` is just state tracking, not a separate processing queue.

```rust
impl Purgatory {
    /// Start the background sync loop
    pub fn start_sync_loop(
        self: Arc<Self>,
        database: SharedDatabase,
        our_domain: Option<String>,
        local_relay: Option<nostr_relay_builder::LocalRelay>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            
            loop {
                interval.tick().await;
                
                // Find ALL ready identifiers
                let ready_identifiers: Vec<String> = self.sync_queue
                    .iter()
                    .filter(|entry| entry.value().is_ready())
                    .map(|entry| entry.key().clone())
                    .collect();
                
                // Spawn sync tasks in parallel for each ready identifier
                for identifier in ready_identifiers {
                    // Check if there are still events in purgatory for this identifier
                    if !self.has_pending_events(&identifier) {
                        self.sync_queue.remove(&identifier);
                        self.domain_throttle.remove_identifier(&identifier);
                        continue;
                    }
                    
                    // Mark as in progress (prevents re-spawning on next tick)
                    if let Some(mut entry) = self.sync_queue.get_mut(&identifier) {
                        if entry.in_progress {
                            continue; // Already running
                        }
                        entry.in_progress = true;
                    }
                    
                    // Spawn task for this identifier
                    let purgatory = self.clone();
                    let db = database.clone();
                    let domain = our_domain.clone();
                    let relay = local_relay.clone();
                    let id = identifier.clone();
                    
                    tokio::spawn(async move {
                        purgatory.sync_identifier(
                            &id,
                            &db,
                            domain.as_deref(),
                            relay.as_ref(),
                        ).await;
                        
                        // Check if events remain
                        if !purgatory.has_pending_events(&id) {
                            purgatory.sync_queue.remove(&id);
                            purgatory.domain_throttle.remove_identifier(&id);
                            tracing::info!(identifier = %id, "Sync complete, removed from queues");
                        } else {
                            // Apply backoff (or not, if new event arrived during sync)
                            if let Some(mut entry) = purgatory.sync_queue.get_mut(&id) {
                                entry.on_sync_complete();
                                tracing::debug!(
                                    identifier = %id,
                                    attempt = entry.attempt_count,
                                    next_attempt_secs = entry.next_attempt.duration_since(Instant::now()).as_secs(),
                                    "Sync attempt complete, scheduled next attempt"
                                );
                            }
                        }
                    });
                }
            }
        })
    }
    
    /// Check if there are pending events for an identifier
    fn has_pending_events(&self, identifier: &str) -> bool {
        // Check state events
        if self.state_events.get(identifier).map_or(false, |v| !v.is_empty()) {
            return true;
        }
        
        // Check PR events (need to scan for matching identifier)
        for entry in self.pr_events.iter() {
            if let Some(ref event) = entry.value().event {
                if extract_identifier_from_pr(event).as_deref() == Some(identifier) {
                    return true;
                }
            }
        }
        
        false
    }
}
```

### Core Sync Logic

```rust
impl Purgatory {
    /// Sync git data for all purgatory events with this identifier
    /// 
    /// Uses a while loop that:
    /// 1. Recalculates URLs and OIDs fresh each iteration (catches new events/announcements)
    /// 2. Gets untried URLs per domain from DomainThrottle
    /// 3. Skips domains that are fully throttled, tries available ones
    /// 4. Continues while: events remain AND OIDs missing AND untried URLs exist
    async fn sync_identifier(
        &self,
        identifier: &str,
        database: &SharedDatabase,
        our_domain: Option<&str>,
        local_relay: Option<&nostr_relay_builder::LocalRelay>,
    ) {
        loop {
            // 1. Check if any events remain (may have expired or been processed)
            if !self.has_pending_events(identifier) {
                return;
            }
            
            // 2. Collect all OIDs needed (fresh calculation each iteration)
            let needed_oids = self.collect_needed_oids(identifier);
            
            if needed_oids.is_empty() {
                // No OIDs needed - try to process events and exit
                self.try_process_events(identifier, database, our_domain, local_relay).await;
                return;
            }
            
            // 3. Get repository data and clone URLs (fresh calculation each iteration)
            let db_repo_data = match fetch_repository_data(database, identifier).await {
                Ok(data) => data,
                Err(e) => {
                    tracing::warn!(identifier = %identifier, error = %e, "Failed to fetch repo data");
                    return;
                }
            };
            
            // 4. Collect clone URLs, excluding our domain
            let all_clone_urls: Vec<String> = db_repo_data
                .announcements
                .iter()
                .flat_map(|a| a.clone_urls.iter().cloned())
                .filter(|url| our_domain.map_or(true, |d| !url.contains(d)))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            
            if all_clone_urls.is_empty() {
                tracing::debug!(identifier = %identifier, "No external clone URLs available");
                return;
            }
            
            // 5. Group URLs by domain
            let urls_by_domain: HashMap<String, Vec<String>> = all_clone_urls
                .iter()
                .fold(HashMap::new(), |mut acc, url| {
                    let domain = extract_domain(url);
                    acc.entry(domain).or_default().push(url.clone());
                    acc
                });
            
            // 6. Find best local repo to fetch into
            let target_repo = match self.find_target_repo(&db_repo_data, identifier) {
                Some(path) => path,
                None => {
                    tracing::debug!(identifier = %identifier, "No local repository found");
                    return;
                }
            };
            
            // 7. Partition domains: available (have capacity) vs throttled
            let (available_domains, throttled_domains): (Vec<_>, Vec<_>) = urls_by_domain
                .keys()
                .cloned()
                .partition(|domain| self.domain_throttle.has_capacity(domain));
            
            let mut remaining_oids: HashSet<String> = needed_oids.clone();
            let mut any_fetch_started = false;
            
            // 8. Try available domains first
            for domain in &available_domains {
                if remaining_oids.is_empty() {
                    break;
                }
                
                let domain_urls = urls_by_domain.get(domain).unwrap();
                let untried_urls = self.domain_throttle.get_untried_urls(domain, identifier, domain_urls);
                
                for url in untried_urls {
                    if remaining_oids.is_empty() {
                        break;
                    }
                    
                    // Skip if already in progress for this domain+identifier
                    if self.domain_throttle.is_in_progress(domain, identifier) {
                        break; // Wait for current fetch to complete
                    }
                    
                    // Try to start fetch (checks capacity again, marks in_progress)
                    if !self.domain_throttle.start_fetch(domain, identifier, &url) {
                        break; // Domain at capacity
                    }
                    
                    any_fetch_started = true;
                    
                    // Fetch OIDs
                    let oids_to_fetch: Vec<String> = remaining_oids.iter().cloned().collect();
                    let fetch_result = fetch_oids_from_server(&target_repo, &url, &oids_to_fetch).await;
                    
                    // Mark fetch complete
                    self.domain_throttle.complete_fetch(domain, identifier);
                    
                    match fetch_result {
                        Ok(fetched_oids) => {
                            if !fetched_oids.is_empty() {
                                let fetched_count = fetched_oids.len();
                                for oid in fetched_oids {
                                    remaining_oids.remove(&oid);
                                }
                                tracing::info!(
                                    identifier = %identifier,
                                    url = %url,
                                    fetched = fetched_count,
                                    remaining = remaining_oids.len(),
                                    "Fetched OIDs from server"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::debug!(
                                identifier = %identifier,
                                url = %url,
                                error = %e,
                                "Failed to fetch from server"
                            );
                        }
                    }
                }
                
                // Clean up if all URLs tried for this domain
                if self.domain_throttle.all_urls_tried(domain, identifier, domain_urls) {
                    self.domain_throttle.remove_identifier_from_domain(domain, identifier);
                }
            }
            
            // 9. If still need OIDs, try throttled domains (they might have capacity now)
            if !remaining_oids.is_empty() {
                for domain in &throttled_domains {
                    if remaining_oids.is_empty() {
                        break;
                    }
                    
                    // Re-check capacity (might have freed up)
                    if !self.domain_throttle.has_capacity(domain) {
                        continue;
                    }
                    
                    let domain_urls = urls_by_domain.get(domain).unwrap();
                    let untried_urls = self.domain_throttle.get_untried_urls(domain, identifier, domain_urls);
                    
                    for url in untried_urls {
                        if remaining_oids.is_empty() {
                            break;
                        }
                        
                        if self.domain_throttle.is_in_progress(domain, identifier) {
                            break;
                        }
                        
                        if !self.domain_throttle.start_fetch(domain, identifier, &url) {
                            break;
                        }
                        
                        any_fetch_started = true;
                        
                        let oids_to_fetch: Vec<String> = remaining_oids.iter().cloned().collect();
                        let fetch_result = fetch_oids_from_server(&target_repo, &url, &oids_to_fetch).await;
                        
                        self.domain_throttle.complete_fetch(domain, identifier);
                        
                        if let Ok(fetched_oids) = fetch_result {
                            for oid in fetched_oids {
                                remaining_oids.remove(&oid);
                            }
                        }
                    }
                    
                    if self.domain_throttle.all_urls_tried(domain, identifier, domain_urls) {
                        self.domain_throttle.remove_identifier_from_domain(domain, identifier);
                    }
                }
            }
            
            // 10. Try to process events that can now be satisfied
            self.try_process_events(identifier, database, our_domain, local_relay).await;
            
            // 11. Decide whether to continue looping
            let still_have_events = self.has_pending_events(identifier);
            if !still_have_events {
                return;
            }
            
            let still_need_oids = !self.collect_needed_oids(identifier).is_empty();
            if !still_need_oids {
                // Events remain but no OIDs needed - loop to try processing again
                continue;
            }
            
            // Check if there are any untried URLs left across all domains
            let have_untried_urls = urls_by_domain.iter().any(|(domain, urls)| {
                !self.domain_throttle.get_untried_urls(domain, identifier, urls).is_empty()
            });
            
            if !have_untried_urls {
                // No more URLs to try - exit and let backoff handle retry
                return;
            }
            
            // If no fetch was started this iteration (all throttled), yield briefly
            if !any_fetch_started {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    
    /// Collect all OIDs needed for purgatory events with this identifier
    fn collect_needed_oids(&self, identifier: &str) -> HashSet<String> {
        let mut oids = HashSet::new();
        
        // Collect from state events
        if let Some(entries) = self.state_events.get(identifier) {
            for entry in entries.iter() {
                if let Ok(state) = RepositoryState::from_event(entry.event.clone()) {
                    for branch in &state.branches {
                        if !branch.commit.starts_with("ref: ") {
                            oids.insert(branch.commit.clone());
                        }
                    }
                    for tag in &state.tags {
                        if !tag.commit.starts_with("ref: ") {
                            oids.insert(tag.commit.clone());
                        }
                    }
                }
            }
        }
        
        // Collect from PR events
        for entry in self.pr_events.iter() {
            if let Some(ref event) = entry.value().event {
                if extract_identifier_from_pr(event).as_deref() == Some(identifier) {
                    if let Some(commit) = extract_commit_from_pr(event) {
                        oids.insert(commit);
                    }
                }
            }
        }
        
        oids
    }
    
    /// Try to process events that can now be satisfied
    async fn try_process_events(
        &self,
        identifier: &str,
        database: &SharedDatabase,
        our_domain: Option<&str>,
        local_relay: Option<&nostr_relay_builder::LocalRelay>,
    ) {
        if !self.has_pending_events(identifier) {
            return;
        }
        
        let db_repo_data = match fetch_repository_data(database, identifier).await {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!(identifier = %identifier, error = %e, "Failed to fetch repo data for processing");
                return;
            }
        };
        
        // Process state events (oldest first)
        if let Some(mut entries) = self.state_events.get_mut(identifier) {
            entries.sort_by_key(|e| e.event.created_at);
            
            let mut to_remove = Vec::new();
            
            for entry in entries.iter() {
                if let Ok(state) = RepositoryState::from_event(entry.event.clone()) {
                    if self.can_satisfy_state(&state, &db_repo_data) {
                        match self.process_state_event(&state, &db_repo_data, database, local_relay).await {
                            Ok(()) => {
                                to_remove.push(entry.event.id);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    event_id = %entry.event.id,
                                    error = %e,
                                    "Failed to process state event"
                                );
                            }
                        }
                    }
                }
            }
            
            entries.retain(|e| !to_remove.contains(&e.event.id));
        }
        
        // Process PR events (oldest first)
        let mut pr_to_remove = Vec::new();
        let mut pr_entries: Vec<_> = self.pr_events
            .iter()
            .filter_map(|entry| {
                entry.value().event.as_ref().and_then(|event| {
                    if extract_identifier_from_pr(event).as_deref() == Some(identifier) {
                        Some((entry.key().clone(), event.clone(), entry.value().commit.clone()))
                    } else {
                        None
                    }
                })
            })
            .collect();
        
        pr_entries.sort_by_key(|(_, event, _)| event.created_at);
        
        for (event_id, event, commit) in pr_entries {
            if self.can_satisfy_pr(&commit, &db_repo_data) {
                match self.process_pr_event(&event, &commit, &db_repo_data, database, local_relay).await {
                    Ok(()) => {
                        pr_to_remove.push(event_id);
                    }
                    Err(e) => {
                        tracing::warn!(
                            event_id = %event.id,
                            error = %e,
                            "Failed to process PR event"
                        );
                    }
                }
            }
        }
        
        for event_id in pr_to_remove {
            self.pr_events.remove(&event_id);
        }
    }
    
    /// Check if a state event can be satisfied (all OIDs available locally)
    fn can_satisfy_state(&self, state: &RepositoryState, db_repo_data: &RepositoryData) -> bool {
        for announcement in &db_repo_data.announcements {
            let repo_path = self.git_data_path.join(announcement.repo_path());
            if !repo_path.exists() {
                continue;
            }
            
            let all_present = state.branches.iter().all(|b| {
                b.commit.starts_with("ref: ") || oid_exists(&repo_path, &b.commit)
            }) && state.tags.iter().all(|t| {
                oid_exists(&repo_path, &t.commit)
            });
            
            if all_present {
                return true;
            }
        }
        false
    }
    
    /// Check if a PR event can be satisfied (commit available locally)
    fn can_satisfy_pr(&self, commit: &str, db_repo_data: &RepositoryData) -> bool {
        for announcement in &db_repo_data.announcements {
            let repo_path = self.git_data_path.join(announcement.repo_path());
            if repo_path.exists() && oid_exists(&repo_path, commit) {
                return true;
            }
        }
        false
    }
}

/// Extract commit hash from PR event's `c` tag
fn extract_commit_from_pr(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let tag_vec = tag.clone().to_vec();
        if tag_vec.len() >= 2 && tag_vec[0] == "c" {
            Some(tag_vec[1].clone())
        } else {
            None
        }
    })
}
```

### Fetching OIDs

```rust
/// Fetch specific OIDs from a remote git server
///
/// Returns the list of OIDs that were successfully fetched (now exist locally).
/// Git fetch may partially succeed, so we check which OIDs are available after.
async fn fetch_oids_from_server(
    repo_path: &Path,
    server_url: &str,
    oids: &[String],
) -> Result<Vec<String>> {
    if oids.is_empty() {
        return Ok(Vec::new());
    }
    
    let repo_path = repo_path.to_path_buf();
    let server_url = server_url.to_string();
    let oids = oids.to_vec();
    
    tokio::task::spawn_blocking(move || {
        // Build git fetch command with all OIDs
        let mut args = vec!["fetch", "--depth=1", &server_url];
        args.extend(oids.iter().map(|s| s.as_str()));
        
        tracing::debug!(
            oids_count = oids.len(),
            server = %server_url,
            "Fetching OIDs"
        );
        
        let output = Command::new("git")
            .args(&args)
            .current_dir(&repo_path)
            .output();
        
        match output {
            Ok(result) => {
                // Check which OIDs we now have (regardless of command success)
                // git fetch may partially succeed
                let fetched: Vec<String> = oids
                    .iter()
                    .filter(|oid| oid_exists(&repo_path, oid))
                    .cloned()
                    .collect();
                
                if !result.status.success() {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    tracing::debug!(
                        server = %server_url,
                        stderr = %stderr,
                        fetched_count = fetched.len(),
                        "git fetch returned non-zero but may have fetched some OIDs"
                    );
                }
                
                Ok(fetched)
            }
            Err(e) => {
                bail!("git fetch command error: {}", e)
            }
        }
    })
    .await?
}

/// Extract domain from a URL for throttling
fn extract_domain(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| url.to_string())
}
```

### Helper Methods

```rust
impl Purgatory {
    /// Find the best local repository to fetch OIDs into
    /// 
    /// Prefers the repo with the most recent commit on its default branch,
    /// as it's most likely to have related git history.
    fn find_target_repo(&self, db_repo_data: &RepositoryData, identifier: &str) -> Option<PathBuf> {
        let mut best: Option<(Timestamp, PathBuf)> = None;
        
        for announcement in &db_repo_data.announcements {
            let repo_path = self.git_data_path.join(announcement.repo_path());
            if !repo_path.exists() {
                continue;
            }
            
            let commit_date = get_date_of_most_recent_commit_on_default_branch(&repo_path)
                .unwrap_or(Timestamp::zero());
            
            if best.as_ref().map_or(true, |(d, _)| commit_date > *d) {
                best = Some((commit_date, repo_path));
            }
        }
        
        best.map(|(_, path)| path)
    }
    
    /// Process a state event that can now be satisfied
    /// 
    /// Syncs OIDs to all owner repos, aligns refs, saves to DB, notifies subscribers.
    async fn process_state_event(
        &self,
        state: &RepositoryState,
        db_repo_data: &RepositoryData,
        database: &SharedDatabase,
        local_relay: Option<&nostr_relay_builder::LocalRelay>,
    ) -> Result<()> {
        // Find source repo (one that has all OIDs)
        let source_repo = self.find_repo_with_all_oids(state, db_repo_data)
            .ok_or_else(|| anyhow::anyhow!("No repo has all required OIDs"))?;
        
        // Sync to other owner repos and align refs
        let sync_result = sync_to_owner_repos(&source_repo, state, db_repo_data, &self.git_data_path);
        
        tracing::info!(
            identifier = %state.identifier,
            event_id = %state.event.id,
            repos_synced = sync_result.repos_synced,
            "Synced state from purgatory"
        );
        
        // Save to database
        database.save_event(&state.event).await?;
        
        // Notify subscribers
        if let Some(relay) = local_relay {
            relay.notify_event(state.event.clone());
        }
        
        Ok(())
    }
    
    /// Process a PR event that can now be satisfied
    async fn process_pr_event(
        &self,
        event: &Event,
        commit: &str,
        db_repo_data: &RepositoryData,
        database: &SharedDatabase,
        local_relay: Option<&nostr_relay_builder::LocalRelay>,
    ) -> Result<()> {
        // Save to database
        database.save_event(event).await?;
        
        // Notify subscribers
        if let Some(relay) = local_relay {
            relay.notify_event(event.clone());
        }
        
        tracing::info!(
            event_id = %event.id,
            commit = %commit,
            "Processed PR event from purgatory"
        );
        
        Ok(())
    }
    
    /// Find a repository that has all OIDs required by a state event
    fn find_repo_with_all_oids(&self, state: &RepositoryState, db_repo_data: &RepositoryData) -> Option<PathBuf> {
        for announcement in &db_repo_data.announcements {
            let repo_path = self.git_data_path.join(announcement.repo_path());
            if !repo_path.exists() {
                continue;
            }
            
            let all_present = state.branches.iter().all(|b| {
                b.commit.starts_with("ref: ") || oid_exists(&repo_path, &b.commit)
            }) && state.tags.iter().all(|t| {
                oid_exists(&repo_path, &t.commit)
            });
            
            if all_present {
                return Some(repo_path);
            }
        }
        None
    }
}
```
```

## Integration Points

### 1. Negentropy Sync (src/purgatory/mod.rs:105-107)

When events are added via negentropy sync, call `trigger_immediate_sync`:

```rust
// In negentropy sync handler
purgatory.add_state(event, identifier.clone(), author);
purgatory.trigger_immediate_sync(&identifier);  // NEW: triggers 500ms debounced sync
```

### 2. Relay Startup

Start the sync loop when the relay starts:

```rust
// In main.rs or server setup
let domain_throttle = Arc::new(DomainThrottle::new(5, 30)); // 5 concurrent, 30/min
let purgatory = Arc::new(Purgatory::new(git_data_path, domain_throttle));

let sync_handle = purgatory.clone().start_sync_loop(
    database.clone(),
    Some(domain.clone()),
    Some(local_relay.clone()),
);
```

### 3. Shutdown

The sync loop will naturally stop when the purgatory is dropped. No special shutdown handling needed since all state is in-memory.

## Testing Strategy

### Unit Tests

1. **SyncQueueEntry**
   - Verify backoff calculation: 20s → 40s → 80s → 120s → 120s
   - Verify `on_new_event()` resets attempt_count and updates next_attempt if sooner
   - Verify `on_sync_complete()` applies backoff only if next_attempt is in the past
   - Verify `is_ready()` respects both `next_attempt` and `in_progress`

2. **DomainThrottle**
   - Verify concurrent limit: 6th request to same domain blocked
   - Verify rate limit: 31st request in a minute blocked
   - Verify `has_capacity()` checks both limits
   - Verify `get_untried_urls()` returns only URLs not in urls_tried
   - Verify `start_fetch()` fails if already in_progress for domain+identifier
   - Verify `start_fetch()` adds URL to urls_tried
   - Verify `complete_fetch()` decrements in_flight and clears in_progress
   - Verify `all_urls_tried()` correctly identifies when done
   - Verify `remove_identifier_from_domain()` cleans up state
   - Verify `remove_identifier()` removes from all domains

3. **OID collection**
   - Verify OIDs extracted from state events correctly
   - Verify OIDs extracted from PR events correctly
   - Verify deduplication works across state and PR events

4. **Identifier extraction**
   - Verify `extract_identifier_from_pr()` handles various `a` tag formats
   - Verify `extract_commit_from_pr()` extracts `c` tag correctly

### Integration Tests

1. **Sync against own implementation**
   - Start two ngit-grasp instances
   - Push to one, verify other can sync via purgatory
   - Verify partial OID availability handled correctly (some OIDs fetched, others missing)

2. **Burst handling**
   - Submit 10 events for same identifier within 100ms
   - Verify debounce: sync doesn't start until 500ms after last event
   - Verify only one sync operation runs (not 10)

3. **Backoff behavior**
   - Configure with unreachable clone URLs
   - Verify backoff timing: 20s, 40s, 80s, 120s, then stays at 120s
   - Verify new event arriving resets attempt_count to 0
   - Verify new event during sync prevents backoff (next_attempt already in future)

4. **Rate limiting**
   - Configure with single domain having multiple URLs
   - Trigger many sync operations
   - Verify only 30 requests made in first minute
   - Verify only 5 concurrent requests per domain

5. **Concurrent limit per domain+identifier**
   - Start fetch for domain+identifier
   - Verify second fetch attempt for same domain+identifier blocked
   - Verify fetch for different identifier on same domain allowed (up to 5)

6. **Parallel identifier processing**
   - Add events for 5 different identifiers
   - Verify all 5 sync tasks start in parallel (not serial)
   - Verify `in_progress` flag prevents duplicate tasks for same identifier

7. **Dynamic URL/OID recalculation**
   - Start sync for identifier
   - While sync is running, add new announcement with additional clone URL
   - Verify sync picks up new URL in next while loop iteration
   - Similarly for new events adding new OIDs

8. **urls_tried cleanup**
   - Sync identifier, exhaust all URLs for a domain
   - Verify identifier removed from that domain's state
   - Add new announcement with new URL for same domain
   - Verify new URL is tried on next sync attempt

9. **Mixed state and PR events**
   - Add state event and PR event for same identifier
   - Verify both OID sets collected
   - Verify both events processed when OIDs arrive

10. **Available domains first**
    - Have 10 URLs: 8 from available domains, 2 from throttled domain
    - Verify available domains tried first
    - Verify throttled domain only contacted if available didn't satisfy all OIDs

## Migration Path

1. **Phase 1**: Add new data structures (SyncQueueEntry, DomainThrottle)
2. **Phase 2**: Implement sync loop alongside existing `start_state_sync`
3. **Phase 3**: Migrate state events to use new sync loop
4. **Phase 4**: Add PR event syncing
5. **Phase 5**: Remove old `start_state_sync` code

## Configuration

New configuration options:

| Option | CLI Flag | Environment Variable | Default |
|--------|----------|---------------------|---------|
| Sync loop interval | `--sync-loop-interval-ms` | `NGIT_SYNC_LOOP_INTERVAL_MS` | `1000` |
| Domain concurrent limit | `--sync-domain-concurrent` | `NGIT_SYNC_DOMAIN_CONCURRENT` | `5` |
| Domain rate limit | `--sync-domain-rate-limit` | `NGIT_SYNC_DOMAIN_RATE_LIMIT` | `30` |
| Default sync delay | `--sync-default-delay-secs` | `NGIT_SYNC_DEFAULT_DELAY_SECS` | `180` |
| Immediate sync delay | `--sync-immediate-delay-ms` | `NGIT_SYNC_IMMEDIATE_DELAY_MS` | `500` |

## Observability

### Metrics

- `purgatory_sync_queue_size` - Number of identifiers pending sync
- `purgatory_sync_attempts_total` - Counter of sync attempts per identifier
- `purgatory_sync_oids_fetched_total` - Counter of OIDs successfully fetched
- `purgatory_domain_in_flight` - Gauge of in-flight requests per domain
- `purgatory_domain_requests_total` - Counter of requests per domain
- `purgatory_sync_backoff_seconds` - Histogram of backoff durations applied

### Logging

Key log points:
- `INFO`: Successful sync completion, OIDs fetched
- `DEBUG`: Domain capacity checks, backoff applied, urls_tried state
- `WARN`: Fetch failures, processing errors

## Open Questions

1. **PR placeholder handling**: Current code has `add_pr_placeholder()` for git-data-first scenario. How should this interact with the new sync system? (Probably: placeholders don't need syncing since git data already exists)

2. **Memory bounds**: Should we limit sync queue size? What happens if thousands of identifiers are pending?

3. **Persistence**: Currently all purgatory state is in-memory. Should sync queue state survive restarts? (Probably no - events will be re-synced via negentropy on restart)
