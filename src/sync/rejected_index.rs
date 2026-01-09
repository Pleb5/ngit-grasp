//! Two-tier rejected events index for efficient re-processing
//!
//! This module provides a two-tier storage system for rejected repository announcements:
//!
//! 1. **Hot Cache (Tier 1)**: Stores full event objects for 2 minutes
//!    - Enables immediate re-processing when dependencies resolve
//!    - Auto-expires to prevent memory growth
//!    - Typical memory: ~200 KB, worst case: ~20 MB
//!
//! 2. **Cold Index (Tier 2)**: Stores metadata only for 7 days
//!    - Prevents repeated downloads of rejected events
//!    - Enables invalidation when dependencies change
//!    - Typical memory: ~1 MB
//!
//! # Problem Solved
//!
//! Without this system, maintainer announcements face a timing gap:
//!
//! ```text
//! 00:00 - Maintainer announcement rejected → Event discarded
//! 00:02 - Owner announcement accepted (lists maintainer) → Want to re-process
//! 00:02 - ❌ Maintainer announcement GONE → Must wait 24h for next sync
//! ```
//!
//! With the two-tier system:
//!
//! ```text
//! 00:00 - Maintainer announcement rejected → Stored in hot cache + cold index
//! 00:02 - Owner announcement accepted → Invalidate + get from hot cache
//! 00:02 - ✅ Re-process immediately → Accepted in <1 second
//! ```
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Tier 1: Hot Cache (2 minutes)                               │
//! │ - Stores FULL EVENT objects                                 │
//! │ - Enables IMMEDIATE re-processing                           │
//! │ - Auto-expires after 2 minutes                              │
//! │ - Memory: ~200 KB typical, ~20 MB worst case                │
//! └─────────────────────────────────────────────────────────────┘
//!                         │
//!                         │ After 2 minutes
//!                         ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Tier 2: Cold Index (7 days)                                 │
//! │ - Stores METADATA only (event_id, pubkey, identifier)       │
//! │ - Prevents repeated downloads                               │
//! │ - Enables invalidation                                      │
//! │ - Memory: ~1 MB typical                                     │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use ngit_grasp::sync::rejected_index::{RejectedEventsIndex, RejectionReason, EventType};
//! use nostr_sdk::{Event, PublicKey};
//! use std::time::Duration;
//!
//! let index = RejectedEventsIndex::new(
//!     Duration::from_secs(120),  // hot cache: 2 minutes
//!     Duration::from_secs(604800), // cold index: 7 days
//! );
//!
//! // Add rejected announcement (event is a nostr_sdk::Event)
//! index.add_announcement(
//!     event.clone(),
//!     event.pubkey,
//!     "my-repo".to_string(),
//!     RejectionReason::DoesNotListService,
//! );
//!
//! // Later, when owner announcement accepted...
//! let (removed, hot_events) = index.invalidate_and_get(
//!     &maintainer_pubkey,
//!     "my-repo",
//!     Some(EventType::Announcement),
//! );
//!
//! // Re-process events from hot cache immediately
//! for event in hot_events {
//!     process_event(&event).await;
//! }
//! ```

use nostr_sdk::{Event, EventId, PublicKey};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Type of event stored in the rejected events index
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    /// Repository announcement (kind 30617)
    Announcement,
    /// Repository state event (kind 30618)
    State,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Announcement => write!(f, "announcement"),
            Self::State => write!(f, "state"),
        }
    }
}

/// Reason why a repository announcement was rejected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    /// Announcement doesn't list this service in clone/web URLs
    DoesNotListService,
    /// Maintainer announcement rejected (owner not yet accepted)
    MaintainerNotYetValid,
    /// Other validation failure
    Other,
}

impl std::fmt::Display for RejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DoesNotListService => write!(f, "does_not_list_service"),
            Self::MaintainerNotYetValid => write!(f, "maintainer_not_yet_valid"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// Entry in the hot cache (full event)
#[derive(Debug, Clone)]
struct HotCacheEntry {
    event: Event,
    pubkey: PublicKey,
    identifier: String,
    event_type: EventType,
    #[allow(dead_code)] // Used for metrics/debugging in future
    reason: RejectionReason,
    cached_at: Instant,
}

/// Entry in the cold index (metadata only)
///
/// Note: event_id is stored as the HashMap key, not in this struct
#[derive(Debug, Clone)]
struct ColdIndexEntry {
    pubkey: PublicKey,
    identifier: String,
    event_type: EventType,
    #[allow(dead_code)] // Used for metrics/debugging in future
    reason: RejectionReason,
    rejected_at: Instant,
}

/// Hot cache: Stores full events for immediate re-processing
///
/// Events are stored for a short duration (default: 2 minutes) to enable
/// immediate re-processing when dependencies resolve. After expiry, events
/// are dropped from the hot cache but remain in the cold index.
#[derive(Debug, Clone)]
struct HotCache {
    /// Map of event_id -> full event entry
    entries: Arc<RwLock<HashMap<EventId, HotCacheEntry>>>,
    /// Duration before entries expire
    expiry_duration: Duration,
}

impl HotCache {
    fn new(expiry_duration: Duration) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            expiry_duration,
        }
    }

    /// Add event to hot cache
    fn add(
        &self,
        event: Event,
        pubkey: PublicKey,
        identifier: String,
        event_type: EventType,
        reason: RejectionReason,
    ) {
        let entry = HotCacheEntry {
            event,
            pubkey,
            identifier,
            event_type,
            reason,
            cached_at: Instant::now(),
        };

        self.entries.write().unwrap().insert(entry.event.id, entry);
    }

    /// Get events for a specific maintainer/identifier from hot cache
    ///
    /// If `event_type` is `Some`, only returns events of that type.
    /// If `event_type` is `None`, returns all event types.
    fn get_maintainer_events(
        &self,
        pubkey: &PublicKey,
        identifier: &str,
        event_type: Option<EventType>,
    ) -> Vec<Event> {
        let entries = self.entries.read().unwrap();
        let now = Instant::now();

        entries
            .values()
            .filter(|entry| {
                // Check if entry matches and hasn't expired
                let matches_type = event_type.is_none_or(|et| entry.event_type == et);
                entry.pubkey == *pubkey
                    && entry.identifier == identifier
                    && matches_type
                    && now.duration_since(entry.cached_at) < self.expiry_duration
            })
            .map(|entry| entry.event.clone())
            .collect()
    }

    /// Remove expired entries from hot cache
    fn cleanup_expired(&self) -> usize {
        let mut entries = self.entries.write().unwrap();
        let now = Instant::now();
        let initial_count = entries.len();

        entries.retain(|_, entry| now.duration_since(entry.cached_at) < self.expiry_duration);

        initial_count - entries.len()
    }

    /// Get current number of entries in hot cache
    fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Check if event is in hot cache
    fn contains(&self, event_id: &EventId) -> bool {
        self.entries.read().unwrap().contains_key(event_id)
    }
}

/// Cold index: Stores metadata only for long-term deduplication
///
/// Events are stored for a long duration (default: 7 days) to prevent
/// repeated downloads of rejected events. Only metadata is stored to
/// minimize memory usage.
#[derive(Debug, Clone)]
struct ColdIndex {
    /// Map of event_id -> metadata entry
    entries: Arc<RwLock<HashMap<EventId, ColdIndexEntry>>>,
    /// Duration before entries expire
    expiry_duration: Duration,
}

impl ColdIndex {
    fn new(expiry_duration: Duration) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            expiry_duration,
        }
    }

    /// Add metadata to cold index
    fn add(
        &self,
        event_id: EventId,
        pubkey: PublicKey,
        identifier: String,
        event_type: EventType,
        reason: RejectionReason,
    ) {
        let entry = ColdIndexEntry {
            pubkey,
            identifier,
            event_type,
            reason,
            rejected_at: Instant::now(),
        };

        self.entries.write().unwrap().insert(event_id, entry);
    }

    /// Check if event is in cold index
    fn contains(&self, event_id: &EventId) -> bool {
        let entries = self.entries.read().unwrap();
        if let Some(entry) = entries.get(event_id) {
            let now = Instant::now();
            now.duration_since(entry.rejected_at) < self.expiry_duration
        } else {
            false
        }
    }

    /// Invalidate (remove) entries from cold index
    ///
    /// Called when an owner announcement is accepted that lists this maintainer.
    /// Removes the cold index entries so they can be re-fetched on next sync.
    ///
    /// If `event_type` is `Some`, only removes entries of that type.
    /// If `event_type` is `None`, removes all event types matching pubkey/identifier.
    fn invalidate_maintainer_announcements(
        &self,
        maintainer_pubkey: &PublicKey,
        identifier: &str,
        event_type: Option<EventType>,
    ) -> usize {
        let mut entries = self.entries.write().unwrap();
        let initial_count = entries.len();

        entries.retain(|_, entry| {
            // Keep entries that DON'T match the maintainer/identifier/type
            let matches_type = event_type.is_none_or(|et| entry.event_type == et);
            !(entry.pubkey == *maintainer_pubkey && entry.identifier == identifier && matches_type)
        });

        initial_count - entries.len()
    }

    /// Remove expired entries from cold index
    fn cleanup_expired(&self) -> usize {
        let mut entries = self.entries.write().unwrap();
        let now = Instant::now();
        let initial_count = entries.len();

        entries.retain(|_, entry| now.duration_since(entry.rejected_at) < self.expiry_duration);

        initial_count - entries.len()
    }

    /// Get current number of entries in cold index
    fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }
}

/// Two-tier rejected events index
///
/// Combines hot cache (full events, short duration) with cold index
/// (metadata only, long duration) for efficient re-processing and deduplication.
#[derive(Clone)]
pub struct RejectedEventsIndex {
    hot_cache: HotCache,
    cold_index: ColdIndex,
    metrics: Option<super::metrics::SyncMetrics>,
}

// Manual Debug impl to avoid requiring Debug on SyncMetrics
impl std::fmt::Debug for RejectedEventsIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RejectedEventsIndex")
            .field("hot_cache", &self.hot_cache)
            .field("cold_index", &self.cold_index)
            .field("metrics", &self.metrics.is_some())
            .finish()
    }
}

impl RejectedEventsIndex {
    /// Create new rejected events index
    ///
    /// # Arguments
    ///
    /// * `hot_cache_duration` - How long to keep full events in hot cache (default: 2 minutes)
    /// * `cold_index_duration` - How long to keep metadata in cold index (default: 7 days)
    pub fn new(hot_cache_duration: Duration, cold_index_duration: Duration) -> Self {
        Self {
            hot_cache: HotCache::new(hot_cache_duration),
            cold_index: ColdIndex::new(cold_index_duration),
            metrics: None,
        }
    }

    /// Create new rejected events index with metrics
    ///
    /// # Arguments
    ///
    /// * `hot_cache_duration` - How long to keep full events in hot cache (default: 2 minutes)
    /// * `cold_index_duration` - How long to keep metadata in cold index (default: 7 days)
    /// * `metrics` - Prometheus metrics for tracking index operations
    pub fn with_metrics(
        hot_cache_duration: Duration,
        cold_index_duration: Duration,
        metrics: super::metrics::SyncMetrics,
    ) -> Self {
        let index = Self {
            hot_cache: HotCache::new(hot_cache_duration),
            cold_index: ColdIndex::new(cold_index_duration),
            metrics: Some(metrics),
        };

        // Initialize metrics with current sizes for both event types
        index.update_metrics_for_type("announcement");
        index.update_metrics_for_type("state");
        index
    }

    /// Update metrics with current sizes for a specific event type
    ///
    /// # Arguments
    ///
    /// * `event_type` - The event type label ("announcement" or "state")
    fn update_metrics_for_type(&self, event_type: &str) {
        if let Some(ref metrics) = self.metrics {
            metrics.update_rejected_hot_cache_size(event_type, self.hot_cache.len());
            metrics.update_rejected_cold_index_size(event_type, self.cold_index.len());
        }
    }

    /// Add rejected announcement to both tiers
    ///
    /// # Arguments
    ///
    /// * `event` - Full event object (stored in hot cache)
    /// * `pubkey` - Author's public key
    /// * `identifier` - Repository identifier (d tag)
    /// * `reason` - Why the announcement was rejected
    pub fn add_announcement(
        &self,
        event: Event,
        pubkey: PublicKey,
        identifier: String,
        reason: RejectionReason,
    ) {
        // Add to hot cache (full event)
        self.hot_cache.add(
            event.clone(),
            pubkey,
            identifier.clone(),
            EventType::Announcement,
            reason,
        );

        // Add to cold index (metadata only)
        self.cold_index.add(
            event.id,
            pubkey,
            identifier,
            EventType::Announcement,
            reason,
        );

        // Update metrics
        self.update_metrics_for_type("announcement");
    }

    /// Add rejected state event to both tiers
    ///
    /// # Arguments
    ///
    /// * `event` - Full event object (stored in hot cache)
    /// * `pubkey` - Author's public key
    /// * `identifier` - Repository identifier (d tag)
    /// * `reason` - Why the state event was rejected
    pub fn add_state(
        &self,
        event: Event,
        pubkey: PublicKey,
        identifier: String,
        reason: RejectionReason,
    ) {
        // Add to hot cache (full event)
        self.hot_cache.add(
            event.clone(),
            pubkey,
            identifier.clone(),
            EventType::State,
            reason,
        );

        // Add to cold index (metadata only)
        self.cold_index
            .add(event.id, pubkey, identifier, EventType::State, reason);

        // Update metrics
        self.update_metrics_for_type("state");
    }

    /// Check if event is already rejected (in either tier)
    pub fn contains(&self, event_id: &EventId) -> bool {
        self.hot_cache.contains(event_id) || self.cold_index.contains(event_id)
    }

    /// Invalidate events and get them for immediate re-processing (unified method)
    ///
    /// This is called when a dependency is satisfied (e.g., owner announcement accepted,
    /// or announcement accepted for state events). It removes the cold index entries
    /// (so they can be re-fetched on next sync) and returns any events still in the
    /// hot cache for immediate re-processing.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - Public key to match (maintainer for announcements, author for states)
    /// * `identifier` - Repository identifier (d tag)
    /// * `event_type` - If `Some`, filter to that event type; if `None`, return all types
    ///
    /// # Returns
    ///
    /// Tuple of (number of cold index entries removed, events from hot cache)
    pub fn invalidate_and_get(
        &self,
        pubkey: &PublicKey,
        identifier: &str,
        event_type: Option<EventType>,
    ) -> (usize, Vec<Event>) {
        // Remove from cold index
        let removed = self
            .cold_index
            .invalidate_maintainer_announcements(pubkey, identifier, event_type);

        // Get from hot cache (for immediate re-processing)
        let events = self
            .hot_cache
            .get_maintainer_events(pubkey, identifier, event_type);

        // Track metrics based on event type
        if let Some(ref metrics) = self.metrics {
            let type_label = match event_type {
                Some(EventType::State) => "state",
                Some(EventType::Announcement) | None => "announcement",
            };

            if removed > 0 {
                metrics.record_rejected_invalidation(type_label, removed);
            }
            if events.is_empty() {
                metrics.record_rejected_hot_cache_miss(type_label);
            } else {
                for _ in &events {
                    metrics.record_rejected_hot_cache_hit(type_label);
                }
            }
        }

        // Update size metrics based on event type
        let type_label = match event_type {
            Some(EventType::State) => "state",
            Some(EventType::Announcement) | None => "announcement",
        };
        self.update_metrics_for_type(type_label);

        (removed, events)
    }

    /// Clean up expired entries from both tiers
    ///
    /// # Arguments
    ///
    /// * `event_type` - The event type label for metrics ("announcement" or "state")
    ///
    /// # Returns
    ///
    /// Tuple of (hot cache expired, cold index expired)
    pub fn cleanup_expired_for_type(&self, event_type: &str) -> (usize, usize) {
        let hot_expired = self.hot_cache.cleanup_expired();
        let cold_expired = self.cold_index.cleanup_expired();

        // Track metrics
        if let Some(ref metrics) = self.metrics {
            if hot_expired > 0 {
                metrics.record_rejected_hot_cache_expired(event_type, hot_expired);
            }
            if cold_expired > 0 {
                metrics.record_rejected_cold_index_expired(event_type, cold_expired);
            }
        }

        // Update size metrics
        self.update_metrics_for_type(event_type);

        (hot_expired, cold_expired)
    }

    /// Get current number of entries in hot cache
    pub fn hot_cache_len(&self) -> usize {
        self.hot_cache.len()
    }

    /// Get current number of entries in cold index
    pub fn cold_index_len(&self) -> usize {
        self.cold_index.len()
    }

    /// Get all rejected event IDs (from both hot cache and cold index)
    ///
    /// Used for excluding rejected events from negentropy sync.
    /// Note: This creates a snapshot - events may be added/removed concurrently.
    pub fn get_all_event_ids(&self) -> HashSet<EventId> {
        let mut ids = HashSet::new();

        // Add from hot cache
        let hot_entries = self.hot_cache.entries.read().unwrap();
        ids.extend(hot_entries.keys().cloned());

        // Add from cold index
        let cold_entries = self.cold_index.entries.read().unwrap();
        ids.extend(cold_entries.keys().cloned());

        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{Keys, NostrSigner};

    async fn create_test_event() -> Event {
        let keys = Keys::generate();
        let unsigned = nostr_sdk::EventBuilder::text_note("test").build(keys.public_key());
        keys.sign_event(unsigned).await.unwrap()
    }

    #[tokio::test]
    async fn test_hot_cache_stores_and_retrieves_events() {
        let cache = HotCache::new(Duration::from_secs(120));
        let event = create_test_event().await;
        let pubkey = event.pubkey;
        let identifier = "test-repo".to_string();

        cache.add(
            event.clone(),
            pubkey,
            identifier.clone(),
            EventType::Announcement,
            RejectionReason::DoesNotListService,
        );

        assert!(cache.contains(&event.id));

        let retrieved = cache.get_maintainer_events(&pubkey, &identifier, None);
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].id, event.id);
    }

    #[tokio::test]
    async fn test_hot_cache_expires_after_duration() {
        let cache = HotCache::new(Duration::from_millis(50));
        let event = create_test_event().await;

        cache.add(
            event.clone(),
            event.pubkey,
            "test-repo".to_string(),
            EventType::Announcement,
            RejectionReason::DoesNotListService,
        );

        assert!(cache.contains(&event.id));

        // Wait for expiry
        std::thread::sleep(Duration::from_millis(60));

        let expired = cache.cleanup_expired();
        assert_eq!(expired, 1);
        assert!(!cache.contains(&event.id));
    }

    #[tokio::test]
    async fn test_cold_index_tracks_metadata() {
        let index = ColdIndex::new(Duration::from_secs(604800));
        let event = create_test_event().await;

        index.add(
            event.id,
            event.pubkey,
            "test-repo".to_string(),
            EventType::Announcement,
            RejectionReason::DoesNotListService,
        );

        assert!(index.contains(&event.id));
        assert_eq!(index.len(), 1);
    }

    #[tokio::test]
    async fn test_cold_index_invalidation() {
        let index = ColdIndex::new(Duration::from_secs(604800));
        let event = create_test_event().await;
        let pubkey = event.pubkey;
        let identifier = "test-repo".to_string();

        index.add(
            event.id,
            pubkey,
            identifier.clone(),
            EventType::Announcement,
            RejectionReason::MaintainerNotYetValid,
        );

        assert!(index.contains(&event.id));

        let removed = index.invalidate_maintainer_announcements(
            &pubkey,
            &identifier,
            Some(EventType::Announcement),
        );
        assert_eq!(removed, 1);
        assert!(!index.contains(&event.id));
    }

    #[tokio::test]
    async fn test_two_tier_index_add_and_contains() {
        let index = RejectedEventsIndex::new(Duration::from_secs(120), Duration::from_secs(604800));
        let event = create_test_event().await;

        index.add_announcement(
            event.clone(),
            event.pubkey,
            "test-repo".to_string(),
            RejectionReason::DoesNotListService,
        );

        assert!(index.contains(&event.id));
        assert_eq!(index.hot_cache_len(), 1);
        assert_eq!(index.cold_index_len(), 1);
    }

    #[tokio::test]
    async fn test_invalidate_and_get_announcements() {
        let index = RejectedEventsIndex::new(Duration::from_secs(120), Duration::from_secs(604800));
        let event = create_test_event().await;
        let pubkey = event.pubkey;
        let identifier = "test-repo".to_string();

        index.add_announcement(
            event.clone(),
            pubkey,
            identifier.clone(),
            RejectionReason::MaintainerNotYetValid,
        );

        let (removed, hot_events) =
            index.invalidate_and_get(&pubkey, &identifier, Some(EventType::Announcement));

        assert_eq!(removed, 1); // Removed from cold index
        assert_eq!(hot_events.len(), 1); // Retrieved from hot cache
        assert_eq!(hot_events[0].id, event.id);

        // Cold index entry removed, hot cache still has it
        assert_eq!(index.cold_index_len(), 0);
        assert_eq!(index.hot_cache_len(), 1);
    }

    #[tokio::test]
    async fn test_cleanup_expired_both_tiers() {
        let index = RejectedEventsIndex::new(
            Duration::from_millis(50),  // Hot cache expires quickly
            Duration::from_millis(100), // Cold index expires slower
        );
        let event = create_test_event().await;

        index.add_announcement(
            event.clone(),
            event.pubkey,
            "test-repo".to_string(),
            RejectionReason::DoesNotListService,
        );

        // Wait for hot cache to expire
        std::thread::sleep(Duration::from_millis(60));

        let (hot_expired, cold_expired) = index.cleanup_expired_for_type("announcement");
        assert_eq!(hot_expired, 1);
        assert_eq!(cold_expired, 0); // Not expired yet

        // Wait for cold index to expire
        std::thread::sleep(Duration::from_millis(50));

        let (hot_expired, cold_expired) = index.cleanup_expired_for_type("announcement");
        assert_eq!(hot_expired, 0); // Already cleaned up
        assert_eq!(cold_expired, 1);
    }

    #[tokio::test]
    async fn test_hot_cache_miss_after_expiry() {
        let index =
            RejectedEventsIndex::new(Duration::from_millis(50), Duration::from_secs(604800));
        let event = create_test_event().await;
        let pubkey = event.pubkey;
        let identifier = "test-repo".to_string();

        index.add_announcement(
            event.clone(),
            pubkey,
            identifier.clone(),
            RejectionReason::MaintainerNotYetValid,
        );

        // Wait for hot cache to expire
        std::thread::sleep(Duration::from_millis(60));

        let (removed, hot_events) =
            index.invalidate_and_get(&pubkey, &identifier, Some(EventType::Announcement));

        assert_eq!(removed, 1); // Removed from cold index
        assert_eq!(hot_events.len(), 0); // Hot cache expired - miss!

        // This is expected: events arrive >2 minutes apart, must wait for next sync
    }

    #[tokio::test]
    async fn test_multiple_maintainer_repos() {
        let index = RejectedEventsIndex::new(Duration::from_secs(120), Duration::from_secs(604800));

        let keys1 = Keys::generate();
        let keys2 = Keys::generate();

        let unsigned1 = nostr_sdk::EventBuilder::text_note("test1").build(keys1.public_key());
        let event1 = keys1.sign_event(unsigned1).await.unwrap();

        let unsigned2 = nostr_sdk::EventBuilder::text_note("test2").build(keys2.public_key());
        let event2 = keys2.sign_event(unsigned2).await.unwrap();

        // Add two different maintainer repos
        index.add_announcement(
            event1.clone(),
            event1.pubkey,
            "repo1".to_string(),
            RejectionReason::MaintainerNotYetValid,
        );

        index.add_announcement(
            event2.clone(),
            event2.pubkey,
            "repo2".to_string(),
            RejectionReason::MaintainerNotYetValid,
        );

        assert_eq!(index.hot_cache_len(), 2);
        assert_eq!(index.cold_index_len(), 2);

        // Invalidate only first maintainer
        let (removed, hot_events) =
            index.invalidate_and_get(&event1.pubkey, "repo1", Some(EventType::Announcement));

        assert_eq!(removed, 1);
        assert_eq!(hot_events.len(), 1);
        assert_eq!(hot_events[0].id, event1.id);

        // Second maintainer still in index
        assert_eq!(index.cold_index_len(), 1);
        assert!(index.contains(&event2.id));
    }

    #[tokio::test]
    async fn test_invalidate_and_get_unified_with_event_type_filter() {
        let index = RejectedEventsIndex::new(Duration::from_secs(120), Duration::from_secs(604800));
        let keys = Keys::generate();

        // Create an announcement event
        let unsigned_ann =
            nostr_sdk::EventBuilder::text_note("announcement").build(keys.public_key());
        let event_ann = keys.sign_event(unsigned_ann).await.unwrap();

        // Create a state event
        let unsigned_state = nostr_sdk::EventBuilder::text_note("state").build(keys.public_key());
        let event_state = keys.sign_event(unsigned_state).await.unwrap();

        let pubkey = event_ann.pubkey;
        let identifier = "test-repo".to_string();

        // Add announcement and state for same pubkey/identifier
        index.add_announcement(
            event_ann.clone(),
            pubkey,
            identifier.clone(),
            RejectionReason::MaintainerNotYetValid,
        );

        index.add_state(
            event_state.clone(),
            pubkey,
            identifier.clone(),
            RejectionReason::Other,
        );

        assert_eq!(index.hot_cache_len(), 2);
        assert_eq!(index.cold_index_len(), 2);

        // Invalidate only announcements
        let (removed, hot_events) =
            index.invalidate_and_get(&pubkey, &identifier, Some(EventType::Announcement));

        assert_eq!(removed, 1); // Only announcement removed from cold index
        assert_eq!(hot_events.len(), 1);
        assert_eq!(hot_events[0].id, event_ann.id);

        // State is still in cold index
        assert_eq!(index.cold_index_len(), 1);
        assert!(index.contains(&event_state.id));

        // Now invalidate states
        let (removed, hot_events) =
            index.invalidate_and_get(&pubkey, &identifier, Some(EventType::State));

        assert_eq!(removed, 1);
        assert_eq!(hot_events.len(), 1);
        assert_eq!(hot_events[0].id, event_state.id);

        // Cold index now empty
        assert_eq!(index.cold_index_len(), 0);
    }

    #[tokio::test]
    async fn test_invalidate_and_get_unified_without_filter() {
        let index = RejectedEventsIndex::new(Duration::from_secs(120), Duration::from_secs(604800));
        let keys = Keys::generate();

        // Create an announcement event
        let unsigned_ann =
            nostr_sdk::EventBuilder::text_note("announcement").build(keys.public_key());
        let event_ann = keys.sign_event(unsigned_ann).await.unwrap();

        // Create a state event
        let unsigned_state = nostr_sdk::EventBuilder::text_note("state").build(keys.public_key());
        let event_state = keys.sign_event(unsigned_state).await.unwrap();

        let pubkey = event_ann.pubkey;
        let identifier = "test-repo".to_string();

        // Add announcement and state for same pubkey/identifier
        index.add_announcement(
            event_ann.clone(),
            pubkey,
            identifier.clone(),
            RejectionReason::MaintainerNotYetValid,
        );

        index.add_state(
            event_state.clone(),
            pubkey,
            identifier.clone(),
            RejectionReason::Other,
        );

        assert_eq!(index.hot_cache_len(), 2);
        assert_eq!(index.cold_index_len(), 2);

        // Invalidate all types (None filter)
        let (removed, hot_events) = index.invalidate_and_get(&pubkey, &identifier, None);

        assert_eq!(removed, 2); // Both removed from cold index
        assert_eq!(hot_events.len(), 2); // Both returned from hot cache

        // Both should be in the results
        let event_ids: Vec<_> = hot_events.iter().map(|e| e.id).collect();
        assert!(event_ids.contains(&event_ann.id));
        assert!(event_ids.contains(&event_state.id));

        // Cold index now empty
        assert_eq!(index.cold_index_len(), 0);
    }
}
