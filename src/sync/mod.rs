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
use std::sync::Arc;

use nostr_sdk::EventId;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::nostr::builder::Nip34WritePolicy;
use crate::nostr::SharedDatabase;

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
/// This is a stub implementation. The core data structures are:
/// - [`FollowingRepoRootEvents`]: Repository root events we're following
/// - [`SyncRelays`]: Relays we sync from with their repos and events
///
/// Full implementation will come in later phases.
pub struct SyncManager {
    /// Bootstrap relay URL if configured
    #[allow(dead_code)]
    bootstrap_relay_url: Option<String>,

    /// Our service domain for filtering repo announcements
    #[allow(dead_code)]
    service_domain: String,

    /// Database for querying/storing events
    #[allow(dead_code)]
    database: SharedDatabase,

    /// Write policy for applying acceptance rules
    #[allow(dead_code)]
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
        Self {
            bootstrap_relay_url,
            service_domain,
            database,
            write_policy,
            following_repo_root_events: new_following_repo_root_events(),
            sync_relays: new_sync_relays(),
            max_backoff_secs: config.sync_max_backoff_secs,
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
    /// ## Current Status
    ///
    /// This is a stub that logs and then waits indefinitely.
    /// Full implementation includes:
    /// - Phase 2: Database initialization queries
    /// - Phase 3: Self-subscription for incremental updates
    /// - Phase 4-6: Filter building, connection management
    /// - Phase 7: Full sync loop
    pub async fn run(self) {
        tracing::info!(
            "SyncManager stub started (bootstrap_relay={:?}, domain={})",
            self.bootstrap_relay_url,
            self.service_domain
        );

        tracing::info!(
            "Phase 1 data structures initialized: following_repo_root_events, sync_relays"
        );

        // Stub: just wait indefinitely until full implementation
        // This prevents the spawned task from immediately completing
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
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