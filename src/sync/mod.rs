//! Proactive Sync Module for GRASP-02
//!
//! This module implements the proactive sync system that ensures data availability
//! for repositories hosted on this relay by syncing from other relays in the ecosystem.
//!
//! ## Architecture Overview
//!
//! The sync system is built around two core data structures:
//!
//! - **FollowingRepoRootEvents**: Tracks repository root events we're following
//! - **SyncRelays**: Tracks relays we sync from, including their repos and events
//!
//! These type aliases are colocated with SyncManager (following the pattern of
//! `src/http/mod.rs` and `src/metrics/mod.rs`) to reduce file count while maintaining clarity.
//!
//! ## Submodules
//!
//! - [`health`]: Relay health tracking with exponential backoff and dead relay detection
//! - [`metrics`]: Prometheus metrics for sync operations
//!
//! ## Memory Estimates (from design doc)
//!
//! At target scale (1,000 repos, 100 relays):
//! - `FollowingRepoRootEvents`: ~1,000 entries × 50 EventIds = ~3-5 MB
//! - `SyncRelays`: ~100 entries × varying repo counts = ~2-3 MB
//! - **Total in-memory state**: ~10 MB
//!
//! ## Upper Bounds (triggers for redesign)
//!
//! - 10,000+ repos: Consider database-backed state
//! - 500+ sync relays: Consider connection pooling
//! - 500+ root events per repo: Consider per-repo pagination
//!
//! ## Design References
//!
//! See [`docs/explanation/grasp-02-proactive-sync-v2.md`](../../docs/explanation/grasp-02-proactive-sync-v2.md)
//! for the complete design context.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use nostr_relay_builder::prelude::{
    DatabaseEventStatus, Event, Filter, Kind, PolicyResult, SaveEventStatus, TagKind, WritePolicy,
};
use nostr_sdk::prelude::*;
use nostr_sdk::EventId;
use tokio::sync::{mpsc, RwLock};

use crate::config::Config;
use crate::nostr::builder::Nip34WritePolicy;
use crate::nostr::events::{KIND_PR, KIND_PR_UPDATE, KIND_REPOSITORY_ANNOUNCEMENT};
use crate::nostr::SharedDatabase;

mod relay_connection;
mod self_subscriber;
pub use relay_connection::{RelayConnection, RelayEvent};
pub use self_subscriber::{RelayAction, SelfSubscriber};

// =============================================================================
// Type Aliases for Sync State
// =============================================================================

/// Repository root events we're following.
///
/// This structure tracks which repository root events (kinds 1617, 1618, 1619, 1621)
/// we need to follow for each repository we host.
///
/// ## Key Format
///
/// The key is a repository addressable reference in the format:
/// `"30617:<pubkey>:<identifier>"`
///
/// For example: `"30617:abc123...def:my-project"`
///
/// ## Value
///
/// A set of event IDs representing root events (PRs, Issues, Patches, Status events)
/// that reference this repository via an `a` tag.
///
/// ## Event Kinds Tracked
///
/// - **1617**: Patches (NIP-34)
/// - **1618**: Issues (NIP-34)
/// - **1619**: PRs (Pull Requests, NIP-34)
/// - **1621**: Status events (NIP-34)
///
/// ## Invariants
///
/// - May include a few extra repo refs that aren't in `SyncRelays`
/// - This is acceptable - we won't query other relays for them
/// - Updated incrementally via self-subscription
///
/// ## Thread Safety
///
/// Wrapped in `Arc<RwLock<...>>` for safe concurrent access from multiple
/// async tasks performing sync operations.
///
/// ## Example Usage
///
/// ```rust,ignore
/// use ngit_grasp::sync::FollowingRepoRootEvents;
/// use std::collections::HashSet;
/// use nostr_sdk::EventId;
///
/// async fn check_repo(state: &FollowingRepoRootEvents, repo_ref: &str) {
///     let guard = state.read().await;
///     if let Some(events) = guard.get(repo_ref) {
///         println!("Tracking {} root events for {}", events.len(), repo_ref);
///     }
/// }
/// ```
pub type FollowingRepoRootEvents = Arc<RwLock<HashMap<String, HashSet<EventId>>>>;

/// Relays we sync from, including their repos and events.
///
/// This structure tracks which relays we need to connect to for syncing,
/// and for each relay, which repositories and their root events we're interested in.
///
/// ## Key Format (Outer HashMap)
///
/// The outer key is a relay WebSocket URL, e.g., `"wss://relay.example.com"`
///
/// ## Value Format (Inner HashMap)
///
/// For each relay, we maintain a map of:
/// - Key: Repository addressable reference (`"30617:<pubkey>:<identifier>"`)
/// - Value: Set of event IDs for that repo which should be synced from this relay
///
/// ## Relay Selection Criteria
///
/// A relay is included if its URL appears in a repository announcement (kind 30617)
/// that **also** lists our service URL. This ensures we only sync from relays
/// for repositories that are actually hosted on our relay.
///
/// ## Bootstrap Relay
///
/// If configured, the bootstrap relay is always present in this map and is
/// excluded from automatic removal logic. The bootstrap relay is used for
/// initial sync and discovery even when no repositories explicitly list it.
///
/// ## Thread Safety
///
/// Wrapped in `Arc<RwLock<...>>` for safe concurrent access from multiple
/// async tasks performing sync operations.
///
/// ## Example Usage
///
/// ```rust,ignore
/// use ngit_grasp::sync::SyncRelays;
/// use std::collections::{HashMap, HashSet};
///
/// async fn get_relay_repos(state: &SyncRelays, relay_url: &str) {
///     let guard = state.read().await;
///     if let Some(repos) = guard.get(relay_url) {
///         println!("Relay {} tracks {} repos", relay_url, repos.len());
///         for (repo_ref, events) in repos {
///             println!("  {} -> {} events", repo_ref, events.len());
///         }
///     }
/// }
/// ```
pub type SyncRelays = Arc<RwLock<HashMap<String, HashMap<String, HashSet<EventId>>>>>;

/// Creates a new empty `FollowingRepoRootEvents` state.
///
/// Use this to initialize the state before populating from database queries.
pub fn new_following_repo_root_events() -> FollowingRepoRootEvents {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Creates a new empty `SyncRelays` state.
///
/// Use this to initialize the state before populating from database queries.
pub fn new_sync_relays() -> SyncRelays {
    Arc::new(RwLock::new(HashMap::new()))
}

// =============================================================================
// SyncManager
// =============================================================================

/// Manages proactive synchronization with external relays.
///
/// The SyncManager is responsible for:
/// - Discovering relays from stored repository announcements
/// - Maintaining connections to sync relays
/// - Subscribing to events at external relays
/// - Applying the acceptance policy to synced events
///
/// ## Lifecycle
///
/// 1. `new()` - Creates manager with database and config
/// 2. `run()` - Main async loop (call in a spawned task)
///
/// ## Current Status
///
/// Phase 2 implementation supports:
/// - Layer 1 sync: Bootstrap relay connection with 30617/30618 filter
/// - Event processing through write policy
/// - Storage of accepted events
///
/// Core data structures:
/// - [`FollowingRepoRootEvents`]: Repository root events we're following
/// - [`SyncRelays`]: Relays we sync from with their repos and events
pub struct SyncManager {
    /// Bootstrap relay URL if configured
    bootstrap_relay_url: Option<String>,

    /// Our service domain for filtering repo announcements
    #[allow(dead_code)]
    service_domain: String,

    /// Database for querying/storing events
    database: SharedDatabase,

    /// Write policy for applying acceptance rules
    write_policy: Nip34WritePolicy,

    /// Repository root events we're following (Phase 1 data structure)
    #[allow(dead_code)]
    following_repo_root_events: FollowingRepoRootEvents,

    /// Relays we sync from (Phase 1 data structure)
    #[allow(dead_code)]
    sync_relays: SyncRelays,

    /// Max backoff duration for relay reconnection
    #[allow(dead_code)]
    max_backoff_secs: u64,

    /// Socket address used for sync source (for write policy)
    sync_source_addr: SocketAddr,
}

impl SyncManager {
    /// Creates a new SyncManager.
    ///
    /// # Arguments
    ///
    /// * `bootstrap_relay_url` - Optional bootstrap relay for initial sync
    /// * `service_domain` - Our domain for filtering announcements
    /// * `database` - Database for event storage/queries
    /// * `write_policy` - Policy for accepting events
    /// * `config` - Configuration for sync parameters
    pub fn new(
        bootstrap_relay_url: Option<String>,
        service_domain: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
        config: &Config,
    ) -> Self {
        // Create a synthetic SocketAddr for sync source identification
        // This is used when calling write_policy.admit_event() for synced events
        let sync_source_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();

        Self {
            bootstrap_relay_url,
            service_domain,
            database,
            write_policy,
            following_repo_root_events: new_following_repo_root_events(),
            sync_relays: new_sync_relays(),
            max_backoff_secs: config.sync_max_backoff_secs,
            sync_source_addr,
        }
    }

    /// Returns a reference to the following repo root events state.
    ///
    /// This is the Phase 1 data structure tracking which repository root events
    /// (kinds 1617, 1618, 1619, 1621) we're following.
    pub fn following_repo_root_events(&self) -> &FollowingRepoRootEvents {
        &self.following_repo_root_events
    }

    /// Returns a reference to the sync relays state.
    ///
    /// This is the Phase 1 data structure tracking which relays we sync from
    /// and their associated repositories/events.
    pub fn sync_relays(&self) -> &SyncRelays {
        &self.sync_relays
    }

    // =========================================================================
    // Phase 2: Database Initialization
    // =========================================================================

    /// Initialize sync state from database queries at startup.
    ///
    /// This method performs two database queries:
    /// 1. Query kinds 1617/1618/1619/1621 to build `following_repo_root_events`
    /// 2. Query kind 30617 to build `sync_relays`
    ///
    /// The bootstrap relay (if configured) is always added to `sync_relays`.
    ///
    /// # Errors
    ///
    /// Returns an error if database queries fail.
    pub async fn initialize_from_database(&self) -> Result<(), String> {
        // Initialize bootstrap relay if configured (never removed)
        if let Some(bootstrap_url) = &self.bootstrap_relay_url {
            self.sync_relays.write().await.insert(
                bootstrap_url.clone(),
                HashMap::new(), // Repos potentially populated below but may stay empty (Layer 1 only)
            );
            tracing::info!("Added bootstrap relay to sync_relays: {}", bootstrap_url);
        }

        // Query 1: Build following_repo_root_events
        // Find all 1617/1618/1619/1621 events and extract their repo references
        let root_event_kinds = vec![
            Kind::GitPatch,             // 1617
            Kind::from(KIND_PR),        // 1618
            Kind::from(KIND_PR_UPDATE), // 1619
            Kind::GitIssue,             // 1621
        ];

        let filter = Filter::new().kinds(root_event_kinds);
        let root_events = self
            .database
            .query(filter)
            .await
            .map_err(|e| format!("Failed to query root events: {}", e))?;

        let mut root_events_count = 0;
        for event in root_events {
            // An event may have multiple 'a' tags pointing to different repos
            let repo_refs = Self::extract_all_repo_refs(&event);
            for repo_ref in repo_refs {
                self.following_repo_root_events
                    .write()
                    .await
                    .entry(repo_ref)
                    .or_default()
                    .insert(event.id);
                root_events_count += 1;
            }
        }
        tracing::info!(
            "Populated following_repo_root_events with {} repo-event mappings",
            root_events_count
        );

        // Query 2: Build sync_relays from kind 30617 announcements
        let announcement_filter = Filter::new().kind(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT));
        let announcements = self
            .database
            .query(announcement_filter)
            .await
            .map_err(|e| format!("Failed to query announcements: {}", e))?;

        let mut sync_relays_count = 0;
        for event in announcements {
            let repo_ref = Self::build_repo_ref(&event);
            let relay_urls = Self::extract_relay_urls(&event);

            // Only track repos that list BOTH a remote relay AND our service
            if self.lists_our_service(&event) {
                for relay_url in relay_urls {
                    if !self.is_own_relay(&relay_url) {
                        // Get events for this repo from following_repo_root_events
                        let events = self
                            .following_repo_root_events
                            .read()
                            .await
                            .get(&repo_ref)
                            .cloned()
                            .unwrap_or_default();

                        self.sync_relays
                            .write()
                            .await
                            .entry(relay_url)
                            .or_default()
                            .insert(repo_ref.clone(), events);
                        sync_relays_count += 1;
                    }
                }
            }
        }
        tracing::info!(
            "Populated sync_relays with {} relay-repo mappings",
            sync_relays_count
        );

        Ok(())
    }

    // =========================================================================
    // Helper Methods for Event Extraction
    // =========================================================================

    /// Extract ALL repo refs from an event (it may tag multiple repos).
    ///
    /// Looks for 'a' tags that reference kind 30617 (repository announcements).
    /// Returns refs in format "30617:pubkey:identifier".
    pub fn extract_all_repo_refs(event: &Event) -> Vec<String> {
        event
            .tags
            .iter()
            .filter_map(|tag| {
                let tag_vec = tag.clone().to_vec();
                if tag_vec.len() >= 2 && tag_vec[0] == "a" {
                    // Validate it's a 30617 reference
                    if tag_vec[1].starts_with("30617:") {
                        Some(tag_vec[1].clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// Build a repo ref string from a 30617 announcement event.
    ///
    /// Returns format "30617:pubkey:identifier".
    pub fn build_repo_ref(event: &Event) -> String {
        // Extract 'd' tag for identifier
        let identifier = event
            .tags
            .iter()
            .find(|tag| tag.kind() == TagKind::d())
            .and_then(|tag| tag.content())
            .map(|s| s.to_string())
            .unwrap_or_default();

        format!("30617:{}:{}", event.pubkey.to_hex(), identifier)
    }

    /// Extract relay URLs from a repository announcement event.
    ///
    /// Looks for the 'relays' tag and returns all relay URLs.
    pub fn extract_relay_urls(event: &Event) -> Vec<String> {
        event
            .tags
            .iter()
            .filter(|tag| matches!(tag.kind(), TagKind::Relays))
            .flat_map(|tag| {
                let vec = tag.clone().to_vec();
                // Skip first element (tag name), rest are values
                vec.into_iter().skip(1)
            })
            .collect()
    }

    /// Check if event lists our service in the relays tag.
    ///
    /// Compares relay URLs against our service domain.
    fn lists_our_service(&self, event: &Event) -> bool {
        let relay_urls = Self::extract_relay_urls(event);
        relay_urls.iter().any(|url| self.is_own_relay(url))
    }

    /// Check if a relay URL matches our relay.
    ///
    /// Compares the URL against our service domain.
    fn is_own_relay(&self, relay_url: &str) -> bool {
        // Normalize comparison: check if URL contains our domain
        relay_url.contains(&self.service_domain)
    }

    // =========================================================================
    // Main Run Loop
    // =========================================================================

    /// Runs the sync manager main loop.
    ///
    /// This method should be called in a spawned task:
    ///
    /// ```rust,ignore
    /// tokio::spawn(async move {
    ///     sync_manager.run().await;
    /// });
    /// ```
    ///
    /// ## Implementation Status
    ///
    /// - Phase 2: Layer 1 sync from bootstrap relay ✓
    /// - Phase 3: Self-subscription and relay discovery ✓
    /// - Phase 4-6: Filter building, connection management (TODO)
    /// - Phase 7: Full sync loop (TODO)
    pub async fn run(self) {
        tracing::info!(
            "SyncManager starting (bootstrap_relay={:?}, domain={})",
            self.bootstrap_relay_url,
            self.service_domain
        );

        // Phase 3: Initialize state from database BEFORE spawning connections
        if let Err(e) = self.initialize_from_database().await {
            tracing::error!("Failed to initialize from database: {}", e);
            // Continue anyway - we can still sync from bootstrap
        }

        // Create channel for relay actions from self-subscriber
        let (action_tx, mut action_rx) = mpsc::channel::<RelayAction>(100);

        // Construct our own relay URL for self-subscription
        let own_relay_url = format!("ws://{}", self.service_domain);

        // Spawn self-subscriber task
        let self_subscriber = SelfSubscriber::new(
            own_relay_url.clone(),
            self.service_domain.clone(),
            Arc::clone(&self.following_repo_root_events),
            Arc::clone(&self.sync_relays),
            action_tx,
        );

        tokio::spawn(async move {
            self_subscriber.run().await;
        });

        tracing::info!("SelfSubscriber spawned for {}", own_relay_url);

        // Track active relay connections (relay_url -> event_sender)
        let mut active_relays: HashMap<String, mpsc::Sender<RelayEvent>> = HashMap::new();

        // Phase 2: Connect to bootstrap relay if configured
        if let Some(ref bootstrap_url) = self.bootstrap_relay_url {
            if let Some(event_tx) = self
                .spawn_relay_connection(bootstrap_url.clone(), None)
                .await
            {
                active_relays.insert(bootstrap_url.clone(), event_tx);
            }
        }

        // Main coordination loop
        loop {
            tokio::select! {
                // Handle relay actions from self-subscriber
                action = action_rx.recv() => {
                    match action {
                        Some(RelayAction::SpawnRelay { relay_url, repos_and_root_events }) => {
                            tracing::info!("Spawning new relay connection to {}", relay_url);
                            if !active_relays.contains_key(&relay_url) {
                                if let Some(event_tx) = self.spawn_relay_connection(relay_url.clone(), Some(repos)).await {
                                    active_relays.insert(relay_url, event_tx);
                                }
                            }
                        }
                        Some(RelayAction::AddFilters { relay_url, repos_and_new_root_event }) => {
                            tracing::debug!("AddFilters for {} - {} repos (not yet implemented)", relay_url, repos.len());
                            // TODO: Implement filter updates for existing connections
                        }
                        None => {
                            tracing::info!("Action channel closed, continuing without self-subscriber");
                        }
                    }
                }
                // Sleep to prevent busy loop when no events
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                    // Periodic maintenance could go here
                }
            }
        }
    }

    /// Spawn a relay connection with optional Layer 2 filters.
    ///
    /// Returns the event sender channel if successfully spawned.
    async fn spawn_relay_connection(
        &self,
        relay_url: String,
        repos: Option<HashMap<String, HashSet<EventId>>>,
    ) -> Option<mpsc::Sender<RelayEvent>> {
        // Create channel for receiving events
        let (event_tx, event_rx) = mpsc::channel::<RelayEvent>(100);

        // Create connection
        let connection = RelayConnection::new(relay_url.clone());

        // Determine if this is bootstrap (no repos) or discovered relay (with repos)
        let is_bootstrap = repos.is_none();

        match connection.connect_and_subscribe().await {
            Ok(()) => {
                if is_bootstrap {
                    tracing::info!("Bootstrap relay connection established: {}", relay_url);
                } else {
                    tracing::info!(
                        "Discovered relay connection established: {} (with Layer 2 filters)",
                        relay_url
                    );

                    // Add Layer 2 subscription for repo events
                    if let Some(ref repos) = repos {
                        if let Err(e) = self.add_layer2_subscription(&connection, repos).await {
                            tracing::warn!("Failed to add Layer 2 subscription: {}", e);
                        }
                    }
                }

                // Clone refs needed for event processing task
                let database = Arc::clone(&self.database);
                let write_policy = self.write_policy.clone();
                let sync_source_addr = self.sync_source_addr;

                // Clone event_tx for the spawned task
                let event_tx_clone = event_tx.clone();

                // Spawn event loop task
                let conn_url = relay_url.clone();
                tokio::spawn(async move {
                    connection.run_event_loop(event_tx_clone).await;
                });

                // Spawn event processing task
                tokio::spawn(async move {
                    Self::process_relay_events(
                        event_rx,
                        database,
                        write_policy,
                        sync_source_addr,
                        conn_url,
                    )
                    .await;
                });

                Some(event_tx)
            }
            Err(e) => {
                tracing::error!("Failed to connect to relay {}: {}", relay_url, e);
                None
            }
        }
    }

    /// Add Layer 2 subscription for repo-related events.
    ///
    /// Layer 2 filters subscribe to events with 'a' tags referencing repos we track.
    async fn add_layer2_subscription(
        &self,
        connection: &RelayConnection,
        repos: &HashMap<String, HashSet<EventId>>,
    ) -> Result<(), String> {
        if repos.is_empty() {
            return Ok(());
        }

        // Build repo refs list for filter
        let repo_refs: Vec<String> = repos.keys().cloned().collect();

        tracing::debug!(
            "Adding Layer 2 subscription for {} repos to {}",
            repo_refs.len(),
            connection.url()
        );

        // Chunk repo_refs into groups of 100 (per plan)
        for chunk in repo_refs.chunks(100) {
            // Build filter with lowercase 'a' tag for each repo ref
            let mut filter = Filter::new().kinds([
                Kind::GitPatch,     // 1617
                Kind::Custom(1618), // PR
                Kind::Custom(1619), // PR update
                Kind::GitIssue,     // 1621
            ]);

            // Add each repo ref as a custom tag filter
            for repo_ref in chunk {
                filter =
                    filter.custom_tag(SingleLetterTag::lowercase(Alphabet::A), repo_ref.clone());
            }

            // Subscribe to this filter
            if let Err(e) = connection.subscribe_filter(filter).await {
                return Err(format!("Failed to subscribe with Layer 2 filter: {}", e));
            }
        }

        Ok(())
    }

    /// Process events from a single relay connection.
    ///
    /// This is a static method that runs in its own task.
    async fn process_relay_events(
        mut event_rx: mpsc::Receiver<RelayEvent>,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
        sync_source_addr: SocketAddr,
        relay_url: String,
    ) {
        tracing::debug!("Starting event processing for relay: {}", relay_url);

        while let Some(relay_event) = event_rx.recv().await {
            match relay_event {
                RelayEvent::Event(event) => {
                    Self::process_single_event_static(
                        &event,
                        &database,
                        &write_policy,
                        &sync_source_addr,
                        &relay_url,
                    )
                    .await;
                }
                RelayEvent::EndOfStoredEvents => {
                    tracing::debug!("EOSE received from {}", relay_url);
                }
                RelayEvent::Closed(reason) => {
                    tracing::warn!("Connection to {} closed: {}", relay_url, reason);
                    break;
                }
            }
        }

        tracing::info!("Event processing ended for relay: {}", relay_url);
    }

    /// Process a single event (static version for use in spawned tasks).
    async fn process_single_event_static(
        event: &Event,
        database: &SharedDatabase,
        write_policy: &Nip34WritePolicy,
        sync_source_addr: &SocketAddr,
        relay_url: &str,
    ) {
        let event_id = event.id;
        let kind = event.kind.as_u16();

        // Check if event already exists in database
        match database.check_id(&event_id).await {
            Ok(DatabaseEventStatus::Saved) | Ok(DatabaseEventStatus::Deleted) => {
                tracing::trace!("Event {} already exists, skipping", event_id);
                return;
            }
            Ok(DatabaseEventStatus::NotExistent) => {} // Continue processing
            Err(e) => {
                tracing::warn!("Failed to check if event {} exists: {}", event_id, e);
            }
        }

        // Pass through write policy
        let policy_result = write_policy.admit_event(event, sync_source_addr).await;

        match policy_result {
            PolicyResult::Accept => match database.save_event(event).await {
                Ok(SaveEventStatus::Success) => {
                    tracing::info!(
                        "Synced event {} (kind {}) from {}",
                        event_id,
                        kind,
                        relay_url
                    );
                }
                Ok(_) => {
                    tracing::debug!(
                        "Event {} (kind {}) already stored or rejected by database",
                        event_id,
                        kind
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to save synced event {}: {}", event_id, e);
                }
            },
            PolicyResult::Reject(reason) => {
                tracing::debug!(
                    "Rejected synced event {} (kind {}): {}",
                    event_id,
                    kind,
                    reason
                );
            }
        }
    }
}

// =============================================================================
// Submodules
// =============================================================================

pub mod health;
pub mod metrics;

// Re-export commonly used types
pub use health::{create_health_tracker, HealthState, RelayHealth, RelayHealthTracker};
pub use metrics::{event_source, SyncMetrics};

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_relay_builder::prelude::{EventBuilder, Keys, Tag};

    /// Helper to create a test event with specific tags
    fn create_test_event(kind: Kind, tags: Vec<Tag>) -> Event {
        let keys = Keys::generate();
        EventBuilder::new(kind, "test content")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("Failed to sign test event")
    }

    // =========================================================================
    // Tests for extract_all_repo_refs
    // =========================================================================

    #[test]
    fn test_extract_all_repo_refs_single_ref() {
        let event = create_test_event(
            Kind::GitPatch,
            vec![Tag::custom(
                nostr_relay_builder::prelude::TagKind::custom("a"),
                vec!["30617:abc123def456:my-project"],
            )],
        );

        let refs = SyncManager::extract_all_repo_refs(&event);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], "30617:abc123def456:my-project");
    }

    #[test]
    fn test_extract_all_repo_refs_multiple_refs() {
        let event = create_test_event(
            Kind::GitPatch,
            vec![
                Tag::custom(
                    nostr_relay_builder::prelude::TagKind::custom("a"),
                    vec!["30617:abc123:project1"],
                ),
                Tag::custom(
                    nostr_relay_builder::prelude::TagKind::custom("a"),
                    vec!["30617:def456:project2"],
                ),
            ],
        );

        let refs = SyncManager::extract_all_repo_refs(&event);
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"30617:abc123:project1".to_string()));
        assert!(refs.contains(&"30617:def456:project2".to_string()));
    }

    #[test]
    fn test_extract_all_repo_refs_ignores_non_30617() {
        let event = create_test_event(
            Kind::GitPatch,
            vec![
                Tag::custom(
                    nostr_relay_builder::prelude::TagKind::custom("a"),
                    vec!["30617:abc123:valid-repo"],
                ),
                Tag::custom(
                    nostr_relay_builder::prelude::TagKind::custom("a"),
                    vec!["30618:def456:state-event"], // Not a repo ref
                ),
            ],
        );

        let refs = SyncManager::extract_all_repo_refs(&event);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], "30617:abc123:valid-repo");
    }

    #[test]
    fn test_extract_all_repo_refs_empty_when_no_a_tags() {
        let event = create_test_event(
            Kind::GitPatch,
            vec![Tag::custom(
                nostr_relay_builder::prelude::TagKind::custom("e"),
                vec!["some-event-id"],
            )],
        );

        let refs = SyncManager::extract_all_repo_refs(&event);
        assert!(refs.is_empty());
    }

    // =========================================================================
    // Tests for build_repo_ref
    // =========================================================================

    #[test]
    fn test_build_repo_ref() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(30617_u16), "announcement")
            .tags(vec![Tag::custom(
                nostr_relay_builder::prelude::TagKind::d(),
                vec!["my-identifier"],
            )])
            .sign_with_keys(&keys)
            .expect("Failed to sign test event");

        let repo_ref = SyncManager::build_repo_ref(&event);
        assert!(repo_ref.starts_with("30617:"));
        assert!(repo_ref.ends_with(":my-identifier"));
        assert!(repo_ref.contains(&event.pubkey.to_hex()));
    }

    #[test]
    fn test_build_repo_ref_empty_identifier() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(30617_u16), "announcement")
            .sign_with_keys(&keys)
            .expect("Failed to sign test event");

        let repo_ref = SyncManager::build_repo_ref(&event);
        assert!(repo_ref.starts_with("30617:"));
        assert!(repo_ref.ends_with(":")); // Empty identifier
    }

    // =========================================================================
    // Tests for extract_relay_urls
    // =========================================================================

    #[test]
    fn test_extract_relay_urls_single() {
        let event = create_test_event(
            Kind::from(30617_u16),
            vec![Tag::custom(
                nostr_relay_builder::prelude::TagKind::Relays,
                vec!["wss://relay.example.com"],
            )],
        );

        let urls = SyncManager::extract_relay_urls(&event);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "wss://relay.example.com");
    }

    #[test]
    fn test_extract_relay_urls_multiple() {
        let event = create_test_event(
            Kind::from(30617_u16),
            vec![Tag::custom(
                nostr_relay_builder::prelude::TagKind::Relays,
                vec!["wss://relay1.example.com", "wss://relay2.example.com"],
            )],
        );

        let urls = SyncManager::extract_relay_urls(&event);
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"wss://relay1.example.com".to_string()));
        assert!(urls.contains(&"wss://relay2.example.com".to_string()));
    }

    #[test]
    fn test_extract_relay_urls_empty_when_no_relays_tag() {
        let event = create_test_event(
            Kind::from(30617_u16),
            vec![Tag::custom(
                nostr_relay_builder::prelude::TagKind::custom("d"),
                vec!["my-project"],
            )],
        );

        let urls = SyncManager::extract_relay_urls(&event);
        assert!(urls.is_empty());
    }

    // =========================================================================
    // Original data structure tests
    // =========================================================================

    #[tokio::test]
    async fn test_following_repo_root_events_basic_operations() {
        let state = new_following_repo_root_events();

        // Insert some events
        {
            let mut guard = state.write().await;
            let repo_ref = "30617:abc123:my-project".to_string();
            guard
                .entry(repo_ref)
                .or_default()
                .insert(EventId::all_zeros());
        }

        // Read back
        {
            let guard = state.read().await;
            assert_eq!(guard.len(), 1);
            assert!(guard.contains_key("30617:abc123:my-project"));
        }
    }

    #[tokio::test]
    async fn test_sync_relays_basic_operations() {
        let state = new_sync_relays();

        // Insert relay with repos
        {
            let mut guard = state.write().await;
            let relay_url = "wss://relay.example.com".to_string();
            let repo_ref = "30617:abc123:my-project".to_string();

            guard
                .entry(relay_url)
                .or_default()
                .entry(repo_ref)
                .or_default()
                .insert(EventId::all_zeros());
        }

        // Read back
        {
            let guard = state.read().await;
            assert_eq!(guard.len(), 1);
            let relay_repos = guard.get("wss://relay.example.com").unwrap();
            assert_eq!(relay_repos.len(), 1);
            let events = relay_repos.get("30617:abc123:my-project").unwrap();
            assert_eq!(events.len(), 1);
        }
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let state = new_following_repo_root_events();
        let state_clone = Arc::clone(&state);

        // Writer task
        let writer = tokio::spawn(async move {
            let mut guard = state_clone.write().await;
            guard
                .entry("30617:writer:repo".to_string())
                .or_default()
                .insert(EventId::all_zeros());
        });

        // Wait for writer
        writer.await.unwrap();

        // Reader should see the change
        let guard = state.read().await;
        assert!(guard.contains_key("30617:writer:repo"));
    }
}
