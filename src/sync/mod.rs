//! Proactive Sync Module - GRASP-02 v4 Implementation
//!
//! This module implements proactive synchronization of repository data from external
//! relays based on relay URLs listed in 30617 repository announcements.
//!
//! ## Architecture
//!
//! The sync system uses three index structures:
//! - `RepoSyncIndex` - What we WANT to sync (source of truth from self-subscription)
//! - `RelaySyncIndex` - What we have CONFIRMED syncing + connection state
//! - `PendingSyncIndex` - In-flight batches awaiting EOSE confirmation
//!
//! See `docs/explanation/grasp-02-proactive-sync-v4.md` for full design details.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nostr_sdk::prelude::*;
use prometheus::{IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry};
use tokio::sync::RwLock;

use crate::config::Config;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};

// =============================================================================
// Type Aliases for Index Structures
// =============================================================================

/// What we WANT to sync - derived from events received via self-subscription.
/// Updated immediately when self-subscriber batch fires.
/// Key: repo addressable ref - 30617:pubkey:identifier
pub type RepoSyncIndex = Arc<RwLock<HashMap<String, RepoSyncNeeds>>>;

/// What we have CONFIRMED syncing - includes connection state for integrated lifecycle.
/// Key: relay URL
pub type RelaySyncIndex = Arc<RwLock<HashMap<String, RelayState>>>;

/// Tracks batches of subscriptions that are in-flight, awaiting EOSE.
/// Each batch has its own ID and can confirm independently.
/// Key: relay URL
pub type PendingSyncIndex = Arc<RwLock<HashMap<String, Vec<PendingBatch>>>>;

// =============================================================================
// Supporting Data Structures
// =============================================================================

/// What repos and root events need to be synced
#[derive(Debug, Clone, Default)]
pub struct RepoSyncNeeds {
    /// Relay URLs listed in this repo's 30617 announcement
    pub relays: HashSet<String>,
    /// Root event IDs - 1617/1618/1619/1621 - that reference this repo
    pub root_events: HashSet<EventId>,
}

/// Connection status for a relay
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    /// Not currently connected
    #[default]
    Disconnected,
    /// Connection attempt in progress
    Connecting,
    /// Successfully connected and subscribed
    Connected,
}

/// Complete state for a single relay - combines sync needs with connection lifecycle
#[derive(Debug)]
pub struct RelayState {
    /// Repos we have confirmed syncing from this relay
    pub repos: HashSet<String>,
    /// Root events we have confirmed tracking
    pub root_events: HashSet<EventId>,
    /// If true, never disconnect this relay
    pub is_bootstrap: bool,
    /// Current connection status
    pub connection_status: ConnectionStatus,
    /// When we last successfully connected - used for since filter on reconnect
    pub last_connected: Option<Timestamp>,
    /// When we disconnected - for 15-minute state retention rule
    pub disconnected_at: Option<Timestamp>,
    // The active connection - will be added in Phase 4
    // pub connection: Option<RelayConnection>,
}

impl Default for RelayState {
    fn default() -> Self {
        Self {
            repos: HashSet::new(),
            root_events: HashSet::new(),
            is_bootstrap: false,
            connection_status: ConnectionStatus::Disconnected,
            last_connected: None,
            disconnected_at: None,
        }
    }
}

impl RelayState {
    /// Check if state should be cleared based on 15-minute rule
    pub fn should_clear_state(&self) -> bool {
        match self.disconnected_at {
            Some(disconnected) => {
                let now = Timestamp::now();
                now.as_secs().saturating_sub(disconnected.as_secs()) > 900 // 15 minutes
            }
            None => false, // Still connected or never connected
        }
    }

    /// Clear repos and root_events - called when reconnect takes > 15 minutes
    pub fn clear_sync_state(&mut self) {
        self.repos.clear();
        self.root_events.clear();
    }
}

/// A batch of items pending EOSE confirmation
#[derive(Debug, Clone)]
pub struct PendingBatch {
    /// Unique ID for this batch - for debugging/logging
    pub batch_id: u64,
    /// The items this batch is syncing
    pub items: PendingItems,
    /// Subscription IDs that must ALL receive EOSE before confirming
    pub outstanding_subs: HashSet<SubscriptionId>,
}

/// Items included in a pending batch
#[derive(Debug, Clone, Default)]
pub struct PendingItems {
    /// Repos being synced in this batch
    pub repos: HashSet<String>,
    /// Root events being synced in this batch
    pub root_events: HashSet<EventId>,
}

// =============================================================================
// SyncMetrics - Prometheus Metrics for Sync System
// =============================================================================

/// Prometheus metrics for the proactive sync system.
///
/// Tracks relay connections, sync progress, and operational statistics.
/// Following the comprehensive v3 metrics design.
#[derive(Clone)]
pub struct SyncMetrics {
    // === Connection metrics ===
    /// Per-relay connection status (1=connected, 0=disconnected)
    relay_connected: IntGaugeVec,
    /// Connection attempts by relay and result (success/failure)
    connection_attempts_total: IntCounterVec,

    // === Event metrics ===
    /// Events synced by source (live/startup/reconnect/daily)
    events_total: IntCounterVec,

    // === Summary metrics ===
    /// Total relays discovered and tracked
    relays_tracked_total: IntGauge,
    /// Currently connected relay count
    relays_connected_total: IntGauge,
}

impl SyncMetrics {
    /// Register sync metrics with a Prometheus registry.
    ///
    /// Returns an error if metrics are already registered (e.g., in tests).
    pub fn register(registry: &Registry) -> Result<Self, prometheus::Error> {
        // Connection metrics
        let relay_connected = IntGaugeVec::new(
            Opts::new(
                "ngit_sync_relay_connected",
                "Relay connection status (1=connected, 0=disconnected)",
            ),
            &["relay"],
        )?;
        registry.register(Box::new(relay_connected.clone()))?;

        let connection_attempts_total = IntCounterVec::new(
            Opts::new(
                "ngit_sync_connection_attempts_total",
                "Total connection attempts by relay and result",
            ),
            &["relay", "result"],
        )?;
        registry.register(Box::new(connection_attempts_total.clone()))?;

        // Event metrics
        let events_total = IntCounterVec::new(
            Opts::new(
                "ngit_sync_events_total",
                "Total events synced by source type",
            ),
            &["source"],
        )?;
        registry.register(Box::new(events_total.clone()))?;

        // Summary metrics
        let relays_tracked_total = IntGauge::with_opts(Opts::new(
            "ngit_sync_relays_tracked_total",
            "Total number of relays discovered and tracked",
        ))?;
        registry.register(Box::new(relays_tracked_total.clone()))?;

        let relays_connected_total = IntGauge::with_opts(Opts::new(
            "ngit_sync_relays_connected_total",
            "Number of currently connected relays",
        ))?;
        registry.register(Box::new(relays_connected_total.clone()))?;

        Ok(Self {
            relay_connected,
            connection_attempts_total,
            events_total,
            relays_tracked_total,
            relays_connected_total,
        })
    }

    // === Connection Recording Methods ===

    /// Record a connection attempt (success or failure)
    pub fn record_connection_attempt(&self, relay: &str, success: bool) {
        let result = if success { "success" } else { "failure" };
        self.connection_attempts_total
            .with_label_values(&[relay, result])
            .inc();
    }

    /// Set relay connection status
    pub fn set_relay_connected(&self, relay: &str, connected: bool) {
        self.relay_connected
            .with_label_values(&[relay])
            .set(if connected { 1 } else { 0 });
    }

    /// Increment connected count
    pub fn inc_connected_count(&self) {
        self.relays_connected_total.inc();
    }

    /// Decrement connected count
    pub fn dec_connected_count(&self) {
        self.relays_connected_total.dec();
    }

    // === Event Recording Methods ===

    /// Record a synced event by source type
    ///
    /// Source types:
    /// - "live" - Real-time subscription events
    /// - "startup" - Events from startup catchup
    /// - "reconnect" - Events from reconnection catchup
    pub fn record_event(&self, source: &str) {
        self.events_total.with_label_values(&[source]).inc();
    }

    /// Record multiple events synced by source type
    pub fn record_events(&self, source: &str, count: u64) {
        self.events_total
            .with_label_values(&[source])
            .inc_by(count);
    }

    // === Summary Recording Methods ===

    /// Set the total tracked relay count
    pub fn set_tracked_count(&self, count: i64) {
        self.relays_tracked_total.set(count);
    }

    /// Increment tracked relay count
    pub fn inc_tracked_count(&self) {
        self.relays_tracked_total.inc();
    }

    /// Get current tracked relay count
    pub fn get_tracked_count(&self) -> i64 {
        self.relays_tracked_total.get()
    }

    /// Get current connected relay count
    pub fn get_connected_count(&self) -> i64 {
        self.relays_connected_total.get()
    }
}

/// Event source types for metrics tracking
pub mod event_source {
    /// Real-time subscription events
    pub const LIVE: &str = "live";
    /// Events from startup catchup
    pub const STARTUP: &str = "startup";
    /// Events from reconnection catchup
    pub const RECONNECT: &str = "reconnect";
}

// =============================================================================
// SyncManager - Main Entry Point
// =============================================================================

/// Manages proactive synchronization with external relays
///
/// The SyncManager runs as a background task, subscribing to repository
/// announcements on the local relay and syncing data from external relays
/// listed in those announcements.
#[allow(dead_code)] // Fields will be used in later phases
pub struct SyncManager {
    /// Bootstrap relay URL for initial sync (optional)
    bootstrap_relay_url: Option<String>,
    /// Our service domain - used for filtering relevant repos
    service_domain: String,
    /// Database for event storage and queries
    database: SharedDatabase,
    /// Write policy for validating incoming events
    write_policy: Nip34WritePolicy,
    /// Configuration reference for sync settings
    config: Config,
    /// What we want to sync (source of truth)
    repo_sync_index: RepoSyncIndex,
    /// What we've confirmed syncing + connection state
    relay_sync_index: RelaySyncIndex,
    /// In-flight subscription batches
    pending_sync_index: PendingSyncIndex,
}

impl SyncManager {
    /// Create a new SyncManager
    ///
    /// # Arguments
    /// * `bootstrap_relay_url` - Optional relay URL for initial historical sync
    /// * `service_domain` - The domain this relay serves (for filtering repos)
    /// * `database` - Shared database for event storage
    /// * `write_policy` - Policy for validating events before storage
    /// * `config` - Configuration for sync settings
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
            config: config.clone(),
            repo_sync_index: Arc::new(RwLock::new(HashMap::new())),
            relay_sync_index: Arc::new(RwLock::new(HashMap::new())),
            pending_sync_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Run the sync manager (placeholder for Phase 1)
    ///
    /// This will be implemented in later phases to:
    /// 1. Subscribe to local relay for 30617 events
    /// 2. Process events to build RepoSyncIndex
    /// 3. Compute and execute sync actions
    /// 4. Handle reconnection and catch-up logic
    pub async fn run(self) {
        tracing::info!(
            bootstrap_relay = ?self.bootstrap_relay_url,
            service_domain = %self.service_domain,
            "SyncManager starting (placeholder - not yet implemented)"
        );

        // Phase 1: Just log and return
        // Full implementation will be added in subsequent phases
    }
}