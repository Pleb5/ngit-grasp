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

use dashmap::DashMap;
use indexmap::IndexMap;
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
            .map_or(false, |t| now.duration_since(*t) >= window)
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
///
/// Note: Trigger-based queue processing (`set_context`, `process_queued_identifier`)
/// will be added in Phase 6 after `SyncContext` is available.
pub struct ThrottleManager {
    /// Per-domain throttle state.
    /// Uses DashMap for concurrent access from multiple sync tasks.
    throttles: DashMap<String, Mutex<DomainThrottle>>,

    /// Maximum concurrent requests per domain.
    max_concurrent_per_domain: u32,

    /// Maximum requests per minute per domain.
    max_per_minute_per_domain: u32,
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
        }
    }

    /// Check if a domain is currently throttled (at capacity).
    ///
    /// Returns true if the domain has no capacity for another request,
    /// either due to concurrent limit or rate limit.
    pub fn is_throttled(&self, domain: &str) -> bool {
        self.throttles
            .get(domain)
            .map_or(false, |entry| {
                let throttle = entry.lock().unwrap();
                !throttle.has_capacity()
            })
    }

    /// Get or create a throttle for a domain.
    fn get_or_create_throttle(&self, domain: &str) -> dashmap::mapref::one::Ref<'_, String, Mutex<DomainThrottle>> {
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

    /// Record that a request completed for a domain.
    ///
    /// Decrements in-flight count and cleans up old timestamps.
    ///
    /// Note: Trigger-based processing (spawning tasks when capacity frees)
    /// will be added in Phase 6 after `SyncContext` is available.
    pub fn complete_request(&self, domain: &str) {
        if let Some(entry) = self.throttles.get(domain) {
            let mut throttle = entry.lock().unwrap();
            throttle.complete_request();
        }
    }

    /// Add an identifier to a domain's waiting queue.
    ///
    /// If the identifier is already queued for this domain, merges the tried_urls sets.
    ///
    /// Note: Trigger-based processing (spawning tasks when capacity available)
    /// will be added in Phase 6 after `SyncContext` is available.
    ///
    /// # Arguments
    /// * `domain` - The domain to queue for
    /// * `identifier` - The repository identifier
    /// * `tried_urls_for_domain` - URLs from this domain that have already been tried
    pub fn enqueue_identifier(
        &self,
        domain: &str,
        identifier: String,
        tried_urls_for_domain: HashSet<String>,
    ) {
        let entry = self.get_or_create_throttle(domain);
        let mut throttle = entry.lock().unwrap();
        throttle.enqueue_identifier(identifier, tried_urls_for_domain);
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

        // Complete one request
        manager.complete_request("example.com");

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

        // Enqueue an identifier
        manager.enqueue_identifier("example.com", "repo1".to_string(), HashSet::new());

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
}
