//! Domain-based rate limiting and identifier queue management.
//!
//! This module provides per-domain throttling to prevent overwhelming remote
//! git servers during purgatory sync operations. Each domain has:
//! - Concurrent request limit (max in-flight requests)
//! - Rate limit (max requests per minute)
//! - Queue of identifiers waiting for capacity (with round-robin processing)
//!
//! The `ThrottleManager` owns all `DomainThrottle` instances and provides the
//! interface for checking throttle status and managing identifier queues.
//!
//! ## Trigger-based Processing
//!
//! When capacity frees up (via `complete_request`) or a new identifier is enqueued
//! (via `enqueue_identifier`), the manager automatically spawns tasks to process
//! queued identifiers. This is trigger-based, not polling-based.

use dashmap::DashMap;
use indexmap::IndexMap;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::debug;

use super::context::SyncContext;
use super::functions::{sync_identifier_from_url, sync_identifier_next_url};

/// State for an identifier waiting in a domain's queue.
///
/// Tracks which URLs from this domain have been tried and whether
/// a fetch is currently in progress for this identifier.
#[derive(Debug, Clone)]
struct IdentifierQueueState {
    /// URLs from this domain that have been tried.
    tried_urls: HashSet<String>,

    /// Whether a fetch is currently in progress for this identifier on this domain.
    ///
    /// Prevents starting multiple concurrent fetches for the same identifier,
    /// which is important when the queue is small (e.g., 2 identifiers with 5
    /// concurrent slots would otherwise try to process the same identifier multiple times).
    in_progress: bool,
}

impl IdentifierQueueState {
    fn new(tried_urls: HashSet<String>) -> Self {
        Self {
            tried_urls,
            in_progress: false,
        }
    }
}

/// Per-domain rate limiting and identifier queue.
///
/// Handles:
/// - Rate limiting (concurrent requests, requests per minute)
/// - Queue of identifiers waiting for capacity (using IndexMap for round-robin order)
/// - Tracking tried URLs per identifier (for this domain only)
/// - In-progress flag per identifier (prevents concurrent fetches for same identifier
///   on this domain, important when queue is small and we have multiple concurrent slots)
#[derive(Debug)]
pub struct DomainThrottle {
    /// Domain this throttle manages (for debugging/logging).
    #[allow(dead_code)]
    domain: String,

    /// Current in-flight request count.
    in_flight: u32,

    /// Request timestamps (sliding window for rate limiting).
    request_times: VecDeque<Instant>,

    /// Queued identifiers with their state.
    /// IndexMap preserves insertion order for round-robin processing.
    queue: IndexMap<String, IdentifierQueueState>,

    /// Round-robin index for fair processing across identifiers.
    round_robin_index: usize,

    /// Maximum concurrent requests for this domain.
    max_concurrent: u32,

    /// Maximum requests per minute for this domain.
    max_per_minute: u32,
}

impl DomainThrottle {
    /// Create a new domain throttle with the specified limits.
    ///
    /// # Arguments
    /// * `domain` - The domain name (for logging)
    /// * `max_concurrent` - Maximum concurrent in-flight requests
    /// * `max_per_minute` - Maximum requests per 60-second window
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

    /// Check if domain has capacity for another request.
    ///
    /// Returns false if:
    /// - Already at max concurrent requests
    /// - Already at max requests per minute (sliding window)
    pub fn has_capacity(&self) -> bool {
        // Check concurrent limit
        if self.in_flight >= self.max_concurrent {
            return false;
        }

        // Check rate limit (sliding window of 60 seconds)
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let recent_count = self
            .request_times
            .iter()
            .filter(|t| now.duration_since(**t) < window)
            .count();

        recent_count < self.max_per_minute as usize
    }

    /// Check if there are any identifiers in the queue.
    pub fn has_queued_work(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Record that a request is starting.
    ///
    /// Increments in-flight count and records timestamp for rate limiting.
    pub fn start_request(&mut self) {
        self.in_flight += 1;
        self.request_times.push_back(Instant::now());
    }

    /// Record that a request completed.
    ///
    /// Decrements in-flight count and cleans up old timestamps.
    pub fn complete_request(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);

        // Clean old timestamps outside the 60-second window
        let now = Instant::now();
        let window = Duration::from_secs(60);
        while self
            .request_times
            .front()
            .is_some_and(|t| now.duration_since(*t) >= window)
        {
            self.request_times.pop_front();
        }
    }

    /// Add an identifier to the queue.
    ///
    /// If the identifier is already queued, merges the tried_urls sets.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier
    /// * `tried_urls` - URLs from this domain that have already been tried
    pub fn enqueue_identifier(&mut self, identifier: String, tried_urls: HashSet<String>) {
        self.queue
            .entry(identifier)
            .and_modify(|state| {
                // Merge tried_urls if already exists
                state.tried_urls.extend(tried_urls.iter().cloned());
            })
            .or_insert(IdentifierQueueState::new(tried_urls));
    }

    /// Get next identifier ready for processing (round-robin, not in_progress).
    ///
    /// Iterates through the queue starting from round_robin_index, skipping
    /// any identifiers that are already in_progress. This ensures fair
    /// distribution even when some identifiers have active fetches.
    ///
    /// Returns the identifier and marks it as in_progress.
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

    /// Get tried URLs for an identifier.
    pub fn get_tried_urls(&self, identifier: &str) -> HashSet<String> {
        self.queue
            .get(identifier)
            .map(|s| s.tried_urls.clone())
            .unwrap_or_default()
    }

    /// Mark a URL as tried for an identifier.
    pub fn mark_url_tried(&mut self, identifier: &str, url: String) {
        if let Some(state) = self.queue.get_mut(identifier) {
            state.tried_urls.insert(url);
        }
    }

    /// Mark identifier as not in progress (fetch completed).
    pub fn mark_identifier_not_in_progress(&mut self, identifier: &str) {
        if let Some(state) = self.queue.get_mut(identifier) {
            state.in_progress = false;
        }
    }

    /// Remove an identifier from the queue entirely.
    ///
    /// Adjusts round_robin_index if needed to maintain fair processing.
    pub fn remove_identifier(&mut self, identifier: &str) {
        if let Some((index, _, _)) = self.queue.shift_remove_full(identifier) {
            // Adjust round_robin_index if we removed an entry before it
            if index < self.round_robin_index && self.round_robin_index > 0 {
                self.round_robin_index -= 1;
            }
            // Clamp to valid range
            if !self.queue.is_empty() {
                self.round_robin_index %= self.queue.len();
            } else {
                self.round_robin_index = 0;
            }
        }
    }
}

/// Manages rate limiting across all domains.
///
/// Owns a collection of `DomainThrottle` instances and provides:
/// - Throttle status checking for `sync_identifier_next_url`
/// - Identifier queue management
/// - Request tracking (start/complete)
/// - Trigger-based queue processing when capacity frees up
pub struct ThrottleManager {
    /// Per-domain throttle state.
    /// Uses DashMap for concurrent access from multiple sync tasks.
    throttles: DashMap<String, Mutex<DomainThrottle>>,

    /// Maximum concurrent requests per domain.
    max_concurrent_per_domain: u32,

    /// Maximum requests per minute per domain.
    max_per_minute_per_domain: u32,

    /// Sync context for processing queued identifiers.
    /// Set once at startup via `set_context()`.
    ctx: OnceLock<Arc<dyn SyncContext>>,
}

impl ThrottleManager {
    /// Create a new throttle manager with the specified limits.
    ///
    /// # Arguments
    /// * `max_concurrent` - Maximum concurrent in-flight requests per domain
    /// * `max_per_minute` - Maximum requests per 60-second window per domain
    pub fn new(max_concurrent: u32, max_per_minute: u32) -> Self {
        Self {
            throttles: DashMap::new(),
            max_concurrent_per_domain: max_concurrent,
            max_per_minute_per_domain: max_per_minute,
            ctx: OnceLock::new(),
        }
    }

    /// Set the sync context (called once at startup).
    ///
    /// The context is used for processing queued identifiers when capacity
    /// becomes available. Must be called before any trigger-based processing
    /// can occur.
    ///
    /// # Arguments
    /// * `ctx` - The sync context implementation
    pub fn set_context(&self, ctx: Arc<dyn SyncContext>) {
        let _ = self.ctx.set(ctx);
    }

    /// Check if a domain is currently throttled (at capacity).
    ///
    /// Returns true if the domain has no capacity for another request,
    /// either due to concurrent limit or rate limit.
    pub fn is_throttled(&self, domain: &str) -> bool {
        self.throttles.get(domain).is_some_and(|entry| {
            let throttle = entry.lock().unwrap();
            !throttle.has_capacity()
        })
    }

    /// Get or create a throttle for a domain.
    fn get_or_create_throttle(
        &self,
        domain: &str,
    ) -> dashmap::mapref::one::Ref<'_, String, Mutex<DomainThrottle>> {
        // First, try to get existing
        if let Some(entry) = self.throttles.get(domain) {
            return entry;
        }

        // Create new throttle
        self.throttles.entry(domain.to_string()).or_insert_with(|| {
            Mutex::new(DomainThrottle::new(
                domain.to_string(),
                self.max_concurrent_per_domain,
                self.max_per_minute_per_domain,
            ))
        });

        // Return the entry (we know it exists now)
        self.throttles.get(domain).unwrap()
    }

    /// Record that a request is starting for a domain.
    ///
    /// Increments in-flight count and records timestamp for rate limiting.
    pub fn start_request(&self, domain: &str) {
        let entry = self.get_or_create_throttle(domain);
        let mut throttle = entry.lock().unwrap();
        throttle.start_request();
    }

    /// Record that a request completed for a domain (internal, no trigger).
    ///
    /// Decrements in-flight count and cleans up old timestamps.
    /// Does not trigger processing of queued identifiers.
    #[cfg(test)]
    fn complete_request_internal(&self, domain: &str) {
        if let Some(entry) = self.throttles.get(domain) {
            let mut throttle = entry.lock().unwrap();
            throttle.complete_request();
        }
    }

    /// Record that a request completed for a domain.
    ///
    /// Decrements in-flight count, cleans up old timestamps, and triggers
    /// processing of queued identifiers if capacity is available.
    ///
    /// # Arguments
    /// * `domain` - The domain that completed a request
    pub fn complete_request(self: &Arc<Self>, domain: &str) {
        let should_trigger = {
            if let Some(entry) = self.throttles.get(domain) {
                let mut throttle = entry.lock().unwrap();
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

    /// Add an identifier to a domain's waiting queue (internal, no trigger).
    ///
    /// If the identifier is already queued for this domain, merges the tried_urls sets.
    /// Does not trigger processing.
    #[cfg(test)]
    fn enqueue_identifier_internal(
        &self,
        domain: &str,
        identifier: String,
        tried_urls_for_domain: HashSet<String>,
    ) {
        let entry = self.get_or_create_throttle(domain);
        let mut throttle = entry.lock().unwrap();
        throttle.enqueue_identifier(identifier, tried_urls_for_domain);
    }

    /// Add an identifier to a domain's waiting queue.
    ///
    /// If the identifier is already queued for this domain, merges the tried_urls sets.
    /// Triggers processing if capacity is available.
    ///
    /// # Arguments
    /// * `domain` - The domain to queue for
    /// * `identifier` - The repository identifier
    /// * `tried_urls_for_domain` - URLs from this domain that have already been tried
    pub fn enqueue_identifier(
        self: &Arc<Self>,
        domain: &str,
        identifier: String,
        tried_urls_for_domain: HashSet<String>,
    ) {
        let should_trigger = {
            let entry = self.get_or_create_throttle(domain);
            let mut throttle = entry.lock().unwrap();
            throttle.enqueue_identifier(identifier, tried_urls_for_domain);
            throttle.has_capacity()
        };

        if should_trigger {
            self.try_process_next(domain);
        }
    }

    /// Try to process the next queued identifier for a domain.
    ///
    /// This is called when capacity becomes available (either via `complete_request`
    /// or when a new identifier is enqueued). Spawns a task to process the next
    /// ready identifier if one exists.
    fn try_process_next(self: &Arc<Self>, domain: &str) {
        // Get next ready identifier (not in_progress)
        let identifier = {
            if let Some(entry) = self.throttles.get(domain) {
                let mut throttle = entry.lock().unwrap();
                throttle.next_ready_identifier()
            } else {
                None
            }
        };

        if let Some(identifier) = identifier {
            let manager = self.clone();
            let domain = domain.to_string();

            tokio::spawn(async move {
                manager
                    .process_queued_identifier(&domain, &identifier)
                    .await;
            });
        }
    }

    /// Process a single identifier from a domain's queue.
    ///
    /// This function:
    /// 1. Gets the next URL to try for this identifier on this domain
    /// 2. If a URL is found, fetches from it and marks it as tried
    /// 3. If no URL is found, removes the identifier from this domain's queue
    /// 4. Triggers processing of the next identifier if capacity is available
    async fn process_queued_identifier(self: &Arc<Self>, domain: &str, identifier: &str) {
        let ctx = match self.ctx.get() {
            Some(ctx) => ctx,
            None => {
                debug!(
                    domain = %domain,
                    identifier = %identifier,
                    "No sync context set - cannot process queued identifier"
                );
                // Mark not in progress so it can be retried
                if let Some(entry) = self.throttles.get(domain) {
                    let mut throttle = entry.lock().unwrap();
                    throttle.mark_identifier_not_in_progress(identifier);
                }
                return;
            }
        };

        // Get tried URLs for this identifier on this domain
        let tried_urls = {
            self.throttles
                .get(domain)
                .map(|entry| {
                    let throttle = entry.lock().unwrap();
                    throttle.get_tried_urls(identifier)
                })
                .unwrap_or_default()
        };

        // Get next URL for this identifier on this specific domain
        let url =
            sync_identifier_next_url(ctx.as_ref(), identifier, Some(domain), &tried_urls, self)
                .await;

        match url {
            Some(url) => {
                debug!(
                    domain = %domain,
                    identifier = %identifier,
                    url = %url,
                    "Processing queued identifier - fetching from URL"
                );

                // Fetch from this URL
                sync_identifier_from_url(ctx.as_ref(), identifier, &url, self).await;

                // Record URL as tried and mark not in_progress
                if let Some(entry) = self.throttles.get(domain) {
                    let mut throttle = entry.lock().unwrap();
                    throttle.mark_url_tried(identifier, url);
                    throttle.mark_identifier_not_in_progress(identifier);
                }

                // complete_request was already called by sync_identifier_from_url,
                // which will trigger try_process_next if capacity is available
            }
            None => {
                debug!(
                    domain = %domain,
                    identifier = %identifier,
                    "No more URLs for identifier on this domain - removing from queue"
                );

                // No more URLs for this identifier on this domain - remove from queue
                if let Some(entry) = self.throttles.get(domain) {
                    let mut throttle = entry.lock().unwrap();
                    throttle.remove_identifier(identifier);
                }

                // Try next identifier since we didn't use any capacity
                self.try_process_next(domain);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_limit_blocks_when_saturated() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 3, 100);

        // Initially has capacity
        assert!(throttle.has_capacity());

        // Start 3 requests (at limit)
        throttle.start_request();
        throttle.start_request();
        throttle.start_request();

        // Should be at capacity now
        assert!(!throttle.has_capacity());

        // Complete one request
        throttle.complete_request();

        // Should have capacity again
        assert!(throttle.has_capacity());
    }

    #[test]
    fn rate_limit_blocks_when_window_full() {
        // Use a very small rate limit for testing
        let mut throttle = DomainThrottle::new("example.com".to_string(), 100, 2);

        // Initially has capacity
        assert!(throttle.has_capacity());

        // Make 2 requests (at rate limit)
        throttle.start_request();
        throttle.complete_request();
        throttle.start_request();
        throttle.complete_request();

        // Should be at rate limit now (2 requests in last 60s)
        assert!(!throttle.has_capacity());

        // Note: In a real test we'd need to wait 60 seconds or mock time
        // For this test, we just verify the blocking behavior
    }

    #[test]
    fn round_robin_processes_identifiers_fairly() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 100);

        // Enqueue A, B, C
        throttle.enqueue_identifier("A".to_string(), HashSet::new());
        throttle.enqueue_identifier("B".to_string(), HashSet::new());
        throttle.enqueue_identifier("C".to_string(), HashSet::new());

        // First round: should get A, B, C in order
        let first = throttle.next_ready_identifier();
        assert_eq!(first, Some("A".to_string()));
        throttle.mark_identifier_not_in_progress("A");

        let second = throttle.next_ready_identifier();
        assert_eq!(second, Some("B".to_string()));
        throttle.mark_identifier_not_in_progress("B");

        let third = throttle.next_ready_identifier();
        assert_eq!(third, Some("C".to_string()));
        throttle.mark_identifier_not_in_progress("C");

        // Second round: should cycle back to A, B, C
        let fourth = throttle.next_ready_identifier();
        assert_eq!(fourth, Some("A".to_string()));
        throttle.mark_identifier_not_in_progress("A");

        let fifth = throttle.next_ready_identifier();
        assert_eq!(fifth, Some("B".to_string()));
    }

    #[test]
    fn skips_in_progress_identifiers() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 100);

        // Enqueue A, B, C
        throttle.enqueue_identifier("A".to_string(), HashSet::new());
        throttle.enqueue_identifier("B".to_string(), HashSet::new());
        throttle.enqueue_identifier("C".to_string(), HashSet::new());

        // Get A (marks it in_progress)
        let first = throttle.next_ready_identifier();
        assert_eq!(first, Some("A".to_string()));

        // Get B (A is still in_progress)
        let second = throttle.next_ready_identifier();
        assert_eq!(second, Some("B".to_string()));

        // Get C (A and B are in_progress)
        let third = throttle.next_ready_identifier();
        assert_eq!(third, Some("C".to_string()));

        // All are in_progress now, should return None
        let fourth = throttle.next_ready_identifier();
        assert_eq!(fourth, None);

        // Mark A as not in_progress
        throttle.mark_identifier_not_in_progress("A");

        // Should get A again (it's the only one not in_progress)
        let fifth = throttle.next_ready_identifier();
        assert_eq!(fifth, Some("A".to_string()));
    }

    #[test]
    fn remove_identifier_adjusts_round_robin_index() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 100);

        // Enqueue A, B, C, D
        throttle.enqueue_identifier("A".to_string(), HashSet::new());
        throttle.enqueue_identifier("B".to_string(), HashSet::new());
        throttle.enqueue_identifier("C".to_string(), HashSet::new());
        throttle.enqueue_identifier("D".to_string(), HashSet::new());

        // Get A (round_robin_index now points to B)
        let first = throttle.next_ready_identifier();
        assert_eq!(first, Some("A".to_string()));
        throttle.mark_identifier_not_in_progress("A");

        // Get B (round_robin_index now points to C)
        let second = throttle.next_ready_identifier();
        assert_eq!(second, Some("B".to_string()));
        throttle.mark_identifier_not_in_progress("B");

        // Remove A (before current index)
        throttle.remove_identifier("A");

        // Next should be C (not B again, index was adjusted)
        let third = throttle.next_ready_identifier();
        assert_eq!(third, Some("C".to_string()));
    }

    #[test]
    fn enqueue_merges_tried_urls() {
        let mut throttle = DomainThrottle::new("example.com".to_string(), 5, 100);

        // First enqueue with some tried URLs
        let mut tried1 = HashSet::new();
        tried1.insert("url1".to_string());
        throttle.enqueue_identifier("A".to_string(), tried1);

        // Second enqueue with different tried URLs
        let mut tried2 = HashSet::new();
        tried2.insert("url2".to_string());
        throttle.enqueue_identifier("A".to_string(), tried2);

        // Should have both URLs
        let tried = throttle.get_tried_urls("A");
        assert!(tried.contains("url1"));
        assert!(tried.contains("url2"));
        assert_eq!(tried.len(), 2);
    }

    // ThrottleManager tests

    #[test]
    fn is_throttled_reflects_domain_capacity() {
        let manager = ThrottleManager::new(2, 100);

        // New domain should not be throttled (has capacity)
        assert!(!manager.is_throttled("example.com"));

        // Start 2 requests (at concurrent limit)
        manager.start_request("example.com");
        manager.start_request("example.com");

        // Should now be throttled
        assert!(manager.is_throttled("example.com"));

        // Complete one request (using internal method for non-Arc test)
        manager.complete_request_internal("example.com");

        // Should have capacity again
        assert!(!manager.is_throttled("example.com"));

        // Different domain should be independent
        assert!(!manager.is_throttled("other.com"));
    }

    #[test]
    fn enqueue_identifier_creates_domain_throttle() {
        let manager = ThrottleManager::new(5, 100);

        // Domain doesn't exist yet
        assert!(!manager.throttles.contains_key("example.com"));

        // Enqueue an identifier (using internal method for non-Arc test)
        manager.enqueue_identifier_internal("example.com", "repo1".to_string(), HashSet::new());

        // Domain throttle should now exist
        assert!(manager.throttles.contains_key("example.com"));
    }

    #[test]
    fn start_request_creates_domain_throttle() {
        let manager = ThrottleManager::new(5, 100);

        // Domain doesn't exist yet
        assert!(!manager.throttles.contains_key("example.com"));

        // Start a request
        manager.start_request("example.com");

        // Domain throttle should now exist
        assert!(manager.throttles.contains_key("example.com"));
    }

    #[test]
    fn has_queued_work_reflects_queue_state() {
        let manager = ThrottleManager::new(5, 100);

        // Initially no queued work
        let has_work = manager
            .throttles
            .get("example.com")
            .map(|e| e.lock().unwrap().has_queued_work())
            .unwrap_or(false);
        assert!(!has_work);

        // Enqueue an identifier
        manager.enqueue_identifier_internal("example.com", "repo1".to_string(), HashSet::new());

        // Now should have queued work
        let has_work = manager
            .throttles
            .get("example.com")
            .map(|e| e.lock().unwrap().has_queued_work())
            .unwrap_or(false);
        assert!(has_work);
    }
}
