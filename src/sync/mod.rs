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

pub mod algorithms;
pub mod filters;
pub mod health;
pub mod metrics;
pub mod relay_connection;
pub mod self_subscriber;

// Re-export core algorithm types
pub use algorithms::{AddFilters, RelaySyncNeeds};

// Re-export metrics types
pub use metrics::{event_source, SyncMetrics};

// Re-export relay connection types
pub use relay_connection::{RelayConnection, RelayEvent};

// Re-export self-subscriber types
pub use self_subscriber::SelfSubscriber;

// Re-export health tracking types
pub use health::RelayHealthTracker;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use nostr_sdk::prelude::*;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::config::Config;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};
use nostr_relay_builder::prelude::LocalRelay;

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
// SyncManager - Main Entry Point
// =============================================================================

/// Notification from spawned tasks about relay disconnections
#[derive(Debug)]
pub struct DisconnectNotification {
    /// The relay URL that disconnected
    pub relay_url: String,
}

/// Notification from spawned tasks about EOSE (End Of Stored Events)
#[derive(Debug)]
pub struct EoseNotification {
    /// The relay URL that sent EOSE
    pub relay_url: String,
    /// The subscription ID that completed
    pub sub_id: SubscriptionId,
}

/// Notification from spawned tasks about successful connection
#[derive(Debug)]
pub struct ConnectNotification {
    /// The relay URL that connected
    pub relay_url: String,
}

/// Quick reconnect window in seconds (15 minutes)
const QUICK_RECONNECT_WINDOW_SECS: u64 = 15 * 60;

/// Maximum filter count before triggering consolidation
const CONSOLIDATION_THRESHOLD: usize = 70;

/// Maximum time to wait for pending batches (30 seconds)
const CONSOLIDATION_WAIT_TIMEOUT_SECS: u64 = 30;

// =============================================================================
// Daily Timer (Phase 7)
// =============================================================================

/// Run the daily timer for periodic fresh syncs
///
/// This function runs in a loop, sleeping for a random interval between
/// 23-25 hours, then triggering a daily sync for all relays. The random
/// interval prevents thundering herd effects across multiple ngit-grasp instances.
///
/// The daily sync:
/// - Unsubscribes from all current subscriptions
/// - Clears pending batches and sync state
/// - Re-discovers all repos and events from scratch
///
/// This detects state drift over time that might occur from missed events.
async fn run_daily_timer(
    sync_manager: Arc<Mutex<SyncManager>>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    use rand::Rng;

    loop {
        // Random interval between 23-25 hours
        let hours = 23.0 + rand::thread_rng().gen::<f64>() * 2.0;
        let seconds = (hours * 3600.0) as u64;

        tracing::info!(
            hours = format!("{:.1}", hours),
            "Daily timer scheduled to fire in {:.1} hours",
            hours
        );

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(seconds)) => {
                // Timer fired - do daily sync
                // Get list of relays
                let relay_urls: Vec<String> = {
                    let manager = sync_manager.lock().await;
                    let index = manager.relay_sync_index.read().await;
                    let urls: Vec<String> = index.keys().cloned().collect();
                    drop(index);
                    urls
                };

                tracing::info!(
                    relay_count = relay_urls.len(),
                    "Daily timer fired, starting daily sync for all relays"
                );

                // Trigger daily sync for each relay
                for relay_url in relay_urls {
                    let mut manager = sync_manager.lock().await;
                    manager.daily_sync(&relay_url).await;
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("Daily timer received shutdown signal");
                break;
            }
        }
    }
}

// =============================================================================
// Disconnect Checker (Phase 8)
// =============================================================================

/// Run the disconnect checker for periodic cleanup of empty relays
///
/// This function runs in a loop, checking at the configured interval for relays
/// that have no repos or root events to sync. Non-bootstrap relays
/// that are empty will be disconnected to free up resources.
///
/// Bootstrap relays are never disconnected, even if empty.
///
/// The check interval is configurable via `NGIT_SYNC_DISCONNECT_CHECK_INTERVAL_SECS`
/// (default: 60 seconds). Set to a lower value for faster reconnection testing.
async fn run_disconnect_checker(
    sync_manager: Arc<Mutex<SyncManager>>,
    mut shutdown_rx: broadcast::Receiver<()>,
    check_interval_secs: u64,
) {
    let interval = Duration::from_secs(check_interval_secs);
    tracing::info!(
        interval_secs = check_interval_secs,
        "Disconnect checker started with configured interval"
    );
    
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                tracing::debug!("Disconnect checker running");

                let mut manager = sync_manager.lock().await;
                manager.check_disconnects().await;
                manager.check_reconnects().await;
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("Disconnect checker received shutdown signal");
                break;
            }
        }
    }
}

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
    /// Local relay for submitting synced events (enables broadcast to WebSocket subscribers)
    local_relay: LocalRelay,
    /// Configuration reference for sync settings
    config: Config,
    /// What we want to sync (source of truth)
    repo_sync_index: RepoSyncIndex,
    /// What we've confirmed syncing + connection state
    relay_sync_index: RelaySyncIndex,
    /// In-flight subscription batches
    pending_sync_index: PendingSyncIndex,
    /// Active relay connections - keyed by relay URL
    connections: HashMap<String, RelayConnection>,
    /// Health tracker for relay connection state
    health_tracker: Arc<RelayHealthTracker>,
    /// Counter for generating unique batch IDs
    next_batch_id: u64,
    /// Channel for disconnect notifications (set during run)
    disconnect_tx: Option<tokio::sync::mpsc::Sender<DisconnectNotification>>,
    /// Channel for EOSE notifications (set during run)
    eose_tx: Option<tokio::sync::mpsc::Sender<EoseNotification>>,
    /// Channel for connect notifications (set during run)
    connect_tx: Option<tokio::sync::mpsc::Sender<ConnectNotification>>,
    /// Channel for broadcasting shutdown signal to all background tasks
    shutdown_tx: Option<broadcast::Sender<()>>,
    /// Prometheus metrics for sync operations (None if metrics disabled)
    metrics: Option<SyncMetrics>,
}

impl SyncManager {
    /// Create a new SyncManager
    ///
    /// # Arguments
    /// * `bootstrap_relay_url` - Optional relay URL for initial historical sync
    /// * `service_domain` - The domain this relay serves (for filtering repos)
    /// * `database` - Shared database for event storage
    /// * `write_policy` - Policy for validating events before storage
    /// * `local_relay` - Local relay for submitting synced events (enables WebSocket broadcast)
    /// * `config` - Configuration for sync settings
    /// * `sync_metrics` - Optional pre-registered SyncMetrics (passed from Metrics if metrics are enabled)
    pub fn new(
        bootstrap_relay_url: Option<String>,
        service_domain: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
        local_relay: LocalRelay,
        config: &Config,
        sync_metrics: Option<SyncMetrics>,
    ) -> Self {
        Self {
            bootstrap_relay_url,
            service_domain,
            database,
            write_policy,
            local_relay,
            config: config.clone(),
            repo_sync_index: Arc::new(RwLock::new(HashMap::new())),
            relay_sync_index: Arc::new(RwLock::new(HashMap::new())),
            pending_sync_index: Arc::new(RwLock::new(HashMap::new())),
            connections: HashMap::new(),
            health_tracker: Arc::new(RelayHealthTracker::new(config)),
            next_batch_id: 0,
            disconnect_tx: None,
            eose_tx: None,
            connect_tx: None,
            shutdown_tx: None,
            metrics: sync_metrics,
        }
    }

    /// Generate a unique batch ID
    ///
    /// Increments the internal counter and returns the new value.
    /// Used for tracking pending batches and debugging/logging.
    fn next_batch_id(&mut self) -> u64 {
        self.next_batch_id += 1;
        self.next_batch_id
    }

    /// Handle EOSE (End Of Stored Events) for a subscription
    ///
    /// This method:
    /// - Finds the PendingBatch containing this subscription ID
    /// - Removes the subscription from outstanding_subs
    /// - When all subscriptions complete (outstanding_subs empty):
    ///   - Moves repos from pending to confirmed in RelayState
    ///   - Moves root_events from pending to confirmed
    ///   - Removes the batch from pending_sync_index
    async fn handle_eose(&mut self, relay_url: &str, sub_id: SubscriptionId) {
        // 1. Find and update the pending batch
        let mut pending = self.pending_sync_index.write().await;
        
        let Some(batches) = pending.get_mut(relay_url) else {
            tracing::warn!(
                relay = %relay_url,
                sub_id = %sub_id,
                "EOSE received for unknown relay"
            );
            return;
        };

        // Find the batch containing this subscription
        let batch_index = batches.iter().position(|b| b.outstanding_subs.contains(&sub_id));
        
        let Some(batch_idx) = batch_index else {
            tracing::warn!(
                relay = %relay_url,
                sub_id = %sub_id,
                "EOSE received for unknown subscription"
            );
            return;
        };

        // Remove the subscription from outstanding_subs
        let batch = &mut batches[batch_idx];
        batch.outstanding_subs.remove(&sub_id);
        
        tracing::debug!(
            relay = %relay_url,
            sub_id = %sub_id,
            batch_id = batch.batch_id,
            remaining_subs = batch.outstanding_subs.len(),
            "EOSE processed for subscription"
        );

        // Check if batch is complete
        if !batch.outstanding_subs.is_empty() {
            return;
        }

        // 2. Batch complete - extract items and remove batch
        let completed_batch = batches.remove(batch_idx);
        let batch_id = completed_batch.batch_id;
        let repos_count = completed_batch.items.repos.len();
        let events_count = completed_batch.items.root_events.len();
        
        // Clean up empty relay entry
        if batches.is_empty() {
            pending.remove(relay_url);
        }
        
        // Drop the pending lock before acquiring relay_sync_index lock
        drop(pending);

        // 3. Move items to confirmed state in RelayState
        {
            let mut relay_index = self.relay_sync_index.write().await;
            
            if let Some(state) = relay_index.get_mut(relay_url) {
                // Move repos to confirmed
                state.repos.extend(completed_batch.items.repos);
                // Move root_events to confirmed
                state.root_events.extend(completed_batch.items.root_events);
                
                tracing::info!(
                    relay = %relay_url,
                    batch_id = batch_id,
                    repos_confirmed = repos_count,
                    root_events_confirmed = events_count,
                    total_repos = state.repos.len(),
                    total_root_events = state.root_events.len(),
                    "Batch confirmed - items moved from pending to confirmed"
                );
            } else {
                tracing::warn!(
                    relay = %relay_url,
                    batch_id = batch_id,
                    "Batch completed but no RelayState found for relay"
                );
            }
        }
    }

    /// Perform a daily sync for a specific relay
    ///
    /// This method:
    /// - Unsubscribes from all current subscriptions on the relay
    /// - Clears pending batches for this relay
    /// - Clears sync state (repos and root_events) in RelayState
    /// - Recomputes actions to re-discover all repos/events
    ///
    /// This is triggered by the daily timer to detect state drift over time.
    async fn daily_sync(&mut self, relay_url: &str) {
        tracing::info!(relay = %relay_url, "Starting daily sync");

        // Get connection
        let connection = match self.connections.get(relay_url) {
            Some(conn) => conn,
            None => {
                tracing::warn!(
                    relay = %relay_url,
                    "No connection for relay, skipping daily sync"
                );
                return;
            }
        };

        // Unsubscribe all current subscriptions
        connection.unsubscribe_all().await;

        // Clear pending batches for this relay
        {
            let mut pending = self.pending_sync_index.write().await;
            pending.remove(relay_url);
        }

        // Get relay state and clear sync state (repos and root_events)
        {
            let mut index = self.relay_sync_index.write().await;
            if let Some(state) = index.get_mut(relay_url) {
                let repos_cleared = state.repos.len();
                let events_cleared = state.root_events.len();
                state.clear_sync_state();
                tracing::debug!(
                    relay = %relay_url,
                    repos_cleared = repos_cleared,
                    events_cleared = events_cleared,
                    "Cleared sync state for daily sync"
                );
            }
        }

        // Recompute actions - will discover all repos/events again
        self.recompute_actions_for_relay(relay_url).await;

        if let Some(ref metrics) = self.metrics {
            metrics.record_event(event_source::DAILY);
        }

        tracing::info!(relay = %relay_url, "Daily sync complete");
    }

    /// Run the sync manager
    ///
    /// Coordinates all sync components:
    /// 1. Spawns self-subscriber to monitor own relay for announcements
    /// 2. Spawns daily timer for periodic fresh syncs
    /// 3. Connects to bootstrap relay if configured
    /// 4. Handles relay actions from self-subscriber
    /// 5. Handles disconnect, EOSE, and connect notifications from spawned relay tasks
    pub async fn run(mut self) {
        use tokio::sync::mpsc;

        tracing::info!(
            bootstrap_relay = ?self.bootstrap_relay_url,
            service_domain = %self.service_domain,
            "SyncManager starting"
        );

        // 1. Create action channel for self-subscriber -> manager communication
        let (action_tx, mut action_rx) = mpsc::channel::<AddFilters>(100);

        // 2. Create disconnect channel for spawned tasks -> manager communication
        let (disconnect_tx, mut disconnect_rx) = mpsc::channel::<DisconnectNotification>(100);

        // 3. Create EOSE channel for spawned tasks -> manager communication
        let (eose_tx, mut eose_rx) = mpsc::channel::<EoseNotification>(100);

        // 4. Create connect channel for spawned tasks -> manager communication
        let (connect_tx, mut connect_rx) = mpsc::channel::<ConnectNotification>(100);

        // 4b. Create shutdown broadcast channel for graceful shutdown
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);

        // 5. Spawn self-subscriber with shutdown receiver
        let self_subscriber = SelfSubscriber::new(
            format!("ws://{}", self.service_domain),
            self.service_domain.clone(),
            Arc::clone(&self.repo_sync_index),
            action_tx,
        );
        let subscriber_shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move { self_subscriber.run(Some(subscriber_shutdown)).await });

        // 5b. Store channel senders for use by handlers
        self.disconnect_tx = Some(disconnect_tx.clone());
        self.eose_tx = Some(eose_tx.clone());
        self.connect_tx = Some(connect_tx.clone());
        self.shutdown_tx = Some(shutdown_tx.clone());

        // 6. Connect to bootstrap relay if configured
        if let Some(ref bootstrap_url) = self.bootstrap_relay_url.clone() {
            self.spawn_relay_connection(bootstrap_url.clone()).await;
        }

        // 7. Capture config values before moving self into Arc
        let disconnect_check_interval_secs = self.config.sync_disconnect_check_interval_secs;

        // 8. Wrap self in Arc<Mutex> for sharing with timer task
        let sync_manager = Arc::new(Mutex::new(self));

        // 9. Spawn daily timer task with shutdown receiver
        let timer_manager = Arc::clone(&sync_manager);
        let timer_shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move {
            run_daily_timer(timer_manager, timer_shutdown).await;
        });

        // 10. Spawn disconnect checker task with shutdown receiver
        let checker_manager = Arc::clone(&sync_manager);
        let checker_shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move {
            run_disconnect_checker(checker_manager, checker_shutdown, disconnect_check_interval_secs).await;
        });

        // 10. Main loop - handle actions from self-subscriber, disconnect, EOSE, and connect notifications
        loop {
            // Wait for an event without holding the lock
            tokio::select! {
                action = action_rx.recv() => {
                    match action {
                        Some(add_filters) => {
                            // Process AddFilters action directly
                            let mut manager = sync_manager.lock().await;
                            manager.handle_add_filters(add_filters).await;
                        }
                        None => break,
                    }
                }
                disconnect = disconnect_rx.recv() => {
                    match disconnect {
                        Some(notification) => {
                            // Acquire lock to process disconnect
                            let mut manager = sync_manager.lock().await;
                            manager.handle_disconnect(&notification.relay_url).await;
                        }
                        None => {
                            // All disconnect senders dropped - unlikely but handle gracefully
                            tracing::debug!("Disconnect channel closed");
                        }
                    }
                }
                eose = eose_rx.recv() => {
                    match eose {
                        Some(notification) => {
                            // Acquire lock to process EOSE
                            let mut manager = sync_manager.lock().await;
                            manager.handle_eose(&notification.relay_url, notification.sub_id).await;
                        }
                        None => {
                            // All EOSE senders dropped - unlikely but handle gracefully
                            tracing::debug!("EOSE channel closed");
                        }
                    }
                }
                connect = connect_rx.recv() => {
                    match connect {
                        Some(notification) => {
                            // Acquire lock to process connect
                            let mut manager = sync_manager.lock().await;
                            manager.handle_connect_or_reconnect(&notification.relay_url).await;
                        }
                        None => {
                            // All connect senders dropped - unlikely but handle gracefully
                            tracing::debug!("Connect channel closed");
                        }
                    }
                }
            }
        }
    }

    /// Handle AddFilters action - subscribe to filters on a relay
    ///
    /// This method handles all filter additions:
    /// - For new relays: creates entry with Connecting status, spawns connection
    /// - For existing connected relays: subscribes to filters, creates PendingBatch
    /// - For disconnected/connecting relays: returns (will be handled on connection)
    async fn handle_add_filters(&mut self, action: AddFilters) {
        tracing::info!(
            relay = %action.relay_url,
            repo_count = action.repos.len(),
            root_event_count = action.root_events.len(),
            filter_count = action.filters.len(),
            "[DIAG] handle_add_filters called"
        );
        
        // Step 1: Check if relay exists in relay_sync_index
        let connection_status = {
            let index = self.relay_sync_index.read().await;
            index.get(&action.relay_url).map(|s| s.connection_status)
        };

        match connection_status {
            None => {
                // New relay - create entry with Connecting status
                {
                    let mut index = self.relay_sync_index.write().await;
                    let new_state = RelayState {
                        connection_status: ConnectionStatus::Connecting,
                        is_bootstrap: false, // Only bootstrap relays set this to true
                        last_connected: None,
                        disconnected_at: None,
                        repos: HashSet::new(),
                        root_events: HashSet::new(),
                    };
                    index.insert(action.relay_url.clone(), new_state);
                }

                // Track new relay in metrics
                if let Some(ref metrics) = self.metrics {
                    metrics.inc_tracked_count();
                }

                tracing::info!(
                    relay = %action.relay_url,
                    repos = action.repos.len(),
                    "Spawning connection for new relay"
                );

                // Spawn connection for new relay
                self.spawn_relay_connection(action.relay_url.clone()).await;
                // Connection will trigger handle_connect_or_reconnect which will process items
                return;
            }
            Some(ConnectionStatus::Disconnected) | Some(ConnectionStatus::Connecting) => {
                // Will be handled when connection succeeds
                tracing::debug!(
                    relay = %action.relay_url,
                    status = ?connection_status,
                    "Relay not connected, action will be processed on connection"
                );
                return;
            }
            Some(ConnectionStatus::Connected) => {
                // Continue to subscribe
            }
        }

        // Step 2: Check if consolidation is needed BEFORE adding new filters
        self.maybe_consolidate(&action.relay_url, action.filters.len()).await;

        // Step 3: Get connection and subscribe to all filters
        let connection = match self.connections.get(&action.relay_url) {
            Some(conn) => conn,
            None => {
                tracing::warn!(
                    relay = %action.relay_url,
                    "No connection for relay, cannot subscribe"
                );
                return;
            }
        };

        // Subscribe to each filter and collect subscription IDs
        let mut subscription_ids = Vec::new();
        for filter in &action.filters {
            match connection.subscribe_filter(filter.clone()).await {
                Ok(sub_id) => {
                    subscription_ids.push(sub_id);
                }
                Err(e) => {
                    tracing::error!(
                        relay = %action.relay_url,
                        error = %e,
                        "Failed to subscribe to filter"
                    );
                }
            }
        }

        if subscription_ids.is_empty() && !action.filters.is_empty() {
            tracing::warn!(
                relay = %action.relay_url,
                "All filter subscriptions failed, not creating batch"
            );
            return;
        }

        // Step 4: Create PendingBatch
        let batch_id = self.next_batch_id();
        let batch = PendingBatch {
            batch_id,
            items: PendingItems {
                repos: action.repos.clone(),
                root_events: action.root_events.clone(),
            },
            outstanding_subs: subscription_ids.into_iter().collect(),
        };

        // Step 5: Add to pending_sync_index
        {
            let mut pending = self.pending_sync_index.write().await;
            pending
                .entry(action.relay_url.clone())
                .or_insert_with(Vec::new)
                .push(batch);
        }

        tracing::debug!(
            relay = %action.relay_url,
            batch_id = batch_id,
            repos = action.repos.len(),
            root_events = action.root_events.len(),
            filters = action.filters.len(),
            "Created pending batch for filter subscriptions"
        );
    }

    /// Handle a connection success (called when a relay connects or reconnects)
    ///
    /// This method implements smart reconnection logic:
    /// - Fresh sync if never connected or >15 min since last connection
    /// - Quick reconnect with since filter if <15 min since last connection
    ///
    /// For fresh sync:
    /// - Clears any stale state
    /// - Subscribes to Layer 1 without since filter
    /// - Recomputes actions for new items
    ///
    /// For quick reconnect:
    /// - Preserves existing state
    /// - Subscribes to Layer 1 with since filter
    /// - Rebuilds Layer 2 and Layer 3 with since filter
    /// - Recomputes actions for new items
    async fn handle_connect_or_reconnect(&mut self, relay_url: &str) {
        let now = Timestamp::now();

        // Get the relay state to determine reconnect type
        let (is_fresh_sync, last_connected, is_bootstrap) = {
            let index = self.relay_sync_index.read().await;
            if let Some(state) = index.get(relay_url) {
                let last_conn = state.last_connected;
                let is_fresh = match last_conn {
                    None => true, // Never connected before
                    Some(last) => {
                        let elapsed = now.as_secs().saturating_sub(last.as_secs());
                        elapsed > QUICK_RECONNECT_WINDOW_SECS // Stale if > 15 min
                    }
                };
                (is_fresh, last_conn, state.is_bootstrap)
            } else {
                (true, None, false) // No state found, treat as fresh
            }
        };

        // If stale reconnect, clear state
        if is_fresh_sync && last_connected.is_some() {
            let mut index = self.relay_sync_index.write().await;
            if let Some(state) = index.get_mut(relay_url) {
                state.clear_sync_state();
                tracing::info!(
                    relay = %relay_url,
                    "Cleared stale sync state (was disconnected > 15 min)"
                );
            }
        }

        // Update connection state
        {
            let mut index = self.relay_sync_index.write().await;
            let state = index.entry(relay_url.to_string()).or_default();
            state.connection_status = ConnectionStatus::Connected;
            state.last_connected = Some(now);
            state.disconnected_at = None;
        }

        // Record success in health tracker
        self.health_tracker.record_success(relay_url);

        // Update metrics
        if let Some(ref metrics) = self.metrics {
            metrics.set_relay_connected(relay_url, true);
            metrics.inc_connected_count();
            metrics.record_health_state(relay_url, self.health_tracker.get_state(relay_url));
        }

        // Subscribe based on reconnect type
        if is_fresh_sync {
            tracing::info!(
                relay = %relay_url,
                is_bootstrap = is_bootstrap,
                "Fresh sync - subscribing to Layer 1 without since filter"
            );
            // Fresh sync: Layer 1 without since
            // Layer 1 subscription is handled by the connection establishment
            // Just recompute actions for new items
            self.recompute_actions_for_relay(relay_url).await;
        } else {
            // Quick reconnect: use since filter
            let since_ts = Timestamp::from(
                last_connected
                    .unwrap()
                    .as_secs()
                    .saturating_sub(QUICK_RECONNECT_WINDOW_SECS),
            );

            tracing::info!(
                relay = %relay_url,
                since = %since_ts,
                "Quick reconnect - using since filter for incremental sync"
            );

            // Subscribe to Layer 1 (announcements) with since filter to catch new repos
            let layer1_filter = filters::build_announcement_filter(Some(since_ts));
            if let Some(connection) = self.connections.get(relay_url) {
                if let Err(e) = connection.subscribe_filter(layer1_filter).await {
                    tracing::error!(
                        relay = %relay_url,
                        error = %e,
                        "Failed to subscribe to Layer 1 filter on quick reconnect"
                    );
                }
            }

            // Rebuild Layer 2 and Layer 3 with since filter
            self.rebuild_layer2_and_layer3(relay_url, Some(since_ts))
                .await;

            // Recompute actions for any new items discovered while disconnected
            self.recompute_actions_for_relay(relay_url).await;

            if let Some(ref metrics) = self.metrics {
                metrics.record_event(event_source::RECONNECT);
            }
        }
    }

    /// Rebuild Layer 2 and Layer 3 subscriptions for a relay
    ///
    /// Uses the confirmed repos and root_events from RelayState to build filters.
    /// If since is provided, applies it to all filters for incremental sync.
    async fn rebuild_layer2_and_layer3(&self, relay_url: &str, since: Option<Timestamp>) {
        use crate::sync::filters::build_layer2_and_layer3_filters;

        // Get confirmed state from relay_sync_index
        let (repos, root_events) = {
            let index = self.relay_sync_index.read().await;
            match index.get(relay_url) {
                Some(state) => (state.repos.clone(), state.root_events.clone()),
                None => {
                    tracing::warn!(
                        relay = %relay_url,
                        "No RelayState found for rebuild_layer2_and_layer3"
                    );
                    return;
                }
            }
        };

        // Nothing to rebuild if no confirmed items
        if repos.is_empty() && root_events.is_empty() {
            tracing::debug!(
                relay = %relay_url,
                "No confirmed items to rebuild Layer 2/3 for"
            );
            return;
        }

        // Build Layer 2 and Layer 3 filters
        let filters = build_layer2_and_layer3_filters(&repos, &root_events, since);

        tracing::debug!(
            relay = %relay_url,
            filter_count = filters.len(),
            repos_count = repos.len(),
            root_events_count = root_events.len(),
            since = ?since,
            "Rebuilding Layer 2/3 filters"
        );

        // Subscribe to filters on the relay connection
        if let Some(connection) = self.connections.get(relay_url) {
            for filter in filters {
                if let Err(e) = connection.subscribe_filter(filter).await {
                    tracing::error!(
                        relay = %relay_url,
                        error = %e,
                        "Failed to subscribe to Layer 2/3 filter during rebuild"
                    );
                }
            }
        } else {
            tracing::warn!(
                relay = %relay_url,
                "No active connection found for Layer 2/3 rebuild"
            );
        }
    }

    /// Recompute sync actions for a specific relay
    ///
    /// Uses derive_relay_targets and compute_actions to find new items
    /// that need to be synced. Processes AddFilters actions for new items.
    async fn recompute_actions_for_relay(&mut self, relay_url: &str) {
        use crate::sync::algorithms::{compute_actions, derive_relay_targets};

        // Get current state from indexes (need to collect to avoid holding locks)
        let all_targets = {
            let repo_index = self.repo_sync_index.read().await;
            derive_relay_targets(&repo_index)
        };

        // Filter to only targets for this specific relay
        let relay_target = match all_targets.get(relay_url) {
            Some(target) => target.clone(),
            None => {
                tracing::debug!(
                    relay = %relay_url,
                    "No sync targets found for relay"
                );
                return;
            }
        };

        // Build single-relay targets map for compute_actions
        let mut single_relay_targets = std::collections::HashMap::new();
        single_relay_targets.insert(relay_url.to_string(), relay_target);

        // Compute actions for new items
        let actions = {
            let pending_index = self.pending_sync_index.read().await;
            let relay_index = self.relay_sync_index.read().await;
            compute_actions(&single_relay_targets, &pending_index, &relay_index)
        };

        if actions.is_empty() {
            tracing::debug!(
                relay = %relay_url,
                "No new items to sync for relay"
            );
            return;
        }

        // Process each action
        for action in actions {
            tracing::info!(
                relay = %action.relay_url,
                new_repos = action.repos.len(),
                new_root_events = action.root_events.len(),
                filters = action.filters.len(),
                "Processing AddFilters for new items"
            );
            self.handle_add_filters(action).await;
        }
    }

    /// Handle a relay disconnection
    ///
    /// This method:
    /// - Updates the RelayState in relay_sync_index to Disconnected status
    /// - Sets disconnected_at timestamp
    /// - Clears pending sync batches for this relay
    /// - Removes the relay from active connections
    /// - Records the failure in health tracker
    async fn handle_disconnect(&mut self, relay_url: &str) {
        tracing::warn!(relay = %relay_url, "Handling relay disconnect");

        // 1. Update RelayState in relay_sync_index
        {
            let mut index = self.relay_sync_index.write().await;
            if let Some(state) = index.get_mut(relay_url) {
                state.connection_status = ConnectionStatus::Disconnected;
                state.disconnected_at = Some(Timestamp::now());
                tracing::info!(
                    relay = %relay_url,
                    repos_tracked = state.repos.len(),
                    "Relay state updated to disconnected"
                );
            } else {
                tracing::debug!(
                    relay = %relay_url,
                    "No RelayState found for disconnected relay"
                );
            }
        }

        // 2. Clear pending sync batches for this relay
        {
            let mut pending = self.pending_sync_index.write().await;
            if pending.remove(relay_url).is_some() {
                tracing::debug!(
                    relay = %relay_url,
                    "Cleared pending sync batches for disconnected relay"
                );
            }
        }

        // 3. Remove from active connections
        if self.connections.remove(relay_url).is_some() {
            tracing::debug!(
                relay = %relay_url,
                "Removed relay from active connections"
            );
        }

        // 4. Record failure in health tracker
        self.health_tracker.record_failure(relay_url);

        // Update metrics
        if let Some(ref metrics) = self.metrics {
            metrics.set_relay_connected(relay_url, false);
            metrics.dec_connected_count();
            metrics.record_health_state(relay_url, self.health_tracker.get_state(relay_url));
        }

        tracing::info!(
            relay = %relay_url,
            health_state = %self.health_tracker.get_state(relay_url),
            "Relay disconnect handling complete"
        );
    }

    /// Spawn a relay connection and start its event loop
    ///
    /// Creates a new RelayConnection, connects to Layer 1, stores the connection,
    /// and spawns event processing tasks. Uses stored channel senders for notifications.
    async fn spawn_relay_connection(&mut self, relay_url: String) {
        use tokio::sync::mpsc;

        // Get channel senders (must exist during run)
        let disconnect_tx = match &self.disconnect_tx {
            Some(tx) => tx.clone(),
            None => {
                tracing::error!(
                    relay = %relay_url,
                    "Cannot spawn connection - channels not initialized"
                );
                return;
            }
        };
        let eose_tx = match &self.eose_tx {
            Some(tx) => tx.clone(),
            None => {
                tracing::error!(
                    relay = %relay_url,
                    "Cannot spawn connection - channels not initialized"
                );
                return;
            }
        };
        let connect_tx = match &self.connect_tx {
            Some(tx) => tx.clone(),
            None => {
                tracing::error!(
                    relay = %relay_url,
                    "Cannot spawn connection - channels not initialized"
                );
                return;
            }
        };

        let database = Arc::clone(&self.database);
        let write_policy = self.write_policy.clone();
        let local_relay = self.local_relay.clone();
        let relay_sync_index = Arc::clone(&self.relay_sync_index);

        // Check if this is a bootstrap relay
        let is_bootstrap = self.bootstrap_relay_url.as_ref() == Some(&relay_url);

        // Create relay connection
        let connection = RelayConnection::new(relay_url.clone());

        // Get connection timeout from health tracker (capped at base backoff)
        // This ensures the connection attempt completes before the next retry would be scheduled
        let connection_timeout_secs = self.health_tracker.base_backoff_secs();

        // Connect and subscribe to Layer 1
        match connection.connect_and_subscribe(None, connection_timeout_secs).await {
            Ok(_) => {
                // Record successful connection attempt
                if let Some(ref metrics) = self.metrics {
                    metrics.record_connection_attempt(&relay_url, true);
                }
            }
            Err(e) => {
                tracing::error!(relay = %relay_url, error = %e, "Failed to connect to relay");
                
                // Record failed connection attempt
                if let Some(ref metrics) = self.metrics {
                    metrics.record_connection_attempt(&relay_url, false);
                }
                
                // Record failure in health tracker
                self.health_tracker.record_failure(&relay_url);
                
                // Record health state in metrics
                if let Some(ref metrics) = self.metrics {
                    metrics.record_health_state(&relay_url, self.health_tracker.get_state(&relay_url));
                }
                
                // Update state to disconnected on failure
                {
                    let mut index = relay_sync_index.write().await;
                    if let Some(state) = index.get_mut(&relay_url) {
                        state.connection_status = ConnectionStatus::Disconnected;
                    }
                }
                return;
            }
        }

        // Mark as connected in relay sync index
        {
            let mut index = relay_sync_index.write().await;
            let state = index.entry(relay_url.clone()).or_default();
            state.connection_status = ConnectionStatus::Connected;
            state.is_bootstrap = is_bootstrap;
            state.last_connected = Some(Timestamp::now());
            state.disconnected_at = None;
        }

        // Store connection in HashMap BEFORE sending notification
        // This ensures it's available when handle_connect_or_reconnect is called
        self.connections.insert(relay_url.clone(), connection);

        tracing::info!(
            relay = %relay_url,
            is_bootstrap = is_bootstrap,
            "Spawned relay connection"
        );

        // Notify SyncManager of successful connection
        let _ = connect_tx
            .send(ConnectNotification {
                relay_url: relay_url.clone(),
            })
            .await;

        // Clone the connection for the event loop spawn
        // The stored connection is used for subscription management
        let connection_for_loop = self.connections.get(&relay_url).unwrap().clone();

        // Create event channel
        let (event_tx, mut event_rx) = mpsc::channel::<RelayEvent>(1000);

        // Spawn event loop with cloned connection
        tokio::spawn(async move {
            connection_for_loop.run_event_loop(event_tx).await;
        });

        // Spawn event processor
        let relay_url_clone = relay_url.clone();
        let metrics_clone = self.metrics.clone(); // Clone metrics for the spawned task
        let is_bootstrap_clone = is_bootstrap; // Clone is_bootstrap for the spawned task
        tokio::spawn(async move {
            while let Some(relay_event) = event_rx.recv().await {
                match relay_event {
                    RelayEvent::Event(event) => {
                        if let Some(ref metrics) = metrics_clone {
                            let source = if is_bootstrap_clone {
                                event_source::STARTUP
                            } else {
                                event_source::LIVE
                            };
                            metrics.record_event(source);
                        }
                        Self::process_event_static(
                            &event,
                            &relay_url_clone,
                            &database,
                            &write_policy,
                            &local_relay,
                        )
                        .await;
                    }
                    RelayEvent::EndOfStoredEvents(sub_id) => {
                        tracing::debug!(
                            relay = %relay_url_clone,
                            sub_id = %sub_id,
                            "EOSE received, notifying SyncManager"
                        );
                        // Notify SyncManager of EOSE
                        let _ = eose_tx
                            .send(EoseNotification {
                                relay_url: relay_url_clone.clone(),
                                sub_id,
                            })
                            .await;
                    }
                    RelayEvent::Closed(reason) => {
                        tracing::info!(
                            relay = %relay_url_clone,
                            reason = %reason,
                            "Relay connection closed"
                        );
                        // Notify SyncManager of disconnect
                        let _ = disconnect_tx
                            .send(DisconnectNotification {
                                relay_url: relay_url_clone.clone(),
                            })
                            .await;
                        break;
                    }
                    RelayEvent::Shutdown => {
                        tracing::info!(relay = %relay_url_clone, "Relay shutdown detected");
                        // Notify SyncManager of disconnect
                        let _ = disconnect_tx
                            .send(DisconnectNotification {
                                relay_url: relay_url_clone.clone(),
                            })
                            .await;
                        break;
                    }
                }
            }
        });

        tracing::info!(
            relay = %relay_url,
            is_bootstrap = is_bootstrap,
            "Spawned relay connection"
        );
    }

    /// Process a single event from a relay (static version for spawned tasks)
    ///
    /// Processes events with dedup, policy check, database save, and broadcast:
    /// - Deduplication (skips if event already exists)
    /// - Write policy validation
    /// - Database save
    /// - Broadcast to WebSocket subscribers via notify_event (enables recursive relay discovery)
    async fn process_event_static(
        event: &Event,
        relay_url: &str,
        database: &SharedDatabase,
        write_policy: &Nip34WritePolicy,
        local_relay: &LocalRelay,
    ) {
        use nostr_relay_builder::prelude::{PolicyResult, WritePolicy};
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        // Check if event already exists
        match database.event_by_id(&event.id).await {
            Ok(Some(_)) => {
                tracing::trace!(event_id = %event.id, "Event already exists, skipping");
                return;
            }
            Err(e) => {
                tracing::warn!(event_id = %event.id, error = %e, "Database error checking event");
                return;
            }
            Ok(None) => {} // Continue processing
        }

        // Apply write policy using a dummy address (sync events aren't from network clients)
        let dummy_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let result = write_policy.admit_event(event, &dummy_addr).await;

        match result {
            PolicyResult::Accept => {
                // Save event to database
                if let Err(e) = database.save_event(event).await {
                    tracing::error!(
                        event_id = %event.id,
                        relay = %relay_url,
                        error = %e,
                        "Failed to save synced event"
                    );
                    return;
                }

                // Broadcast to WebSocket subscribers (enables recursive relay discovery)
                // This allows SelfSubscriber to receive synced 30617 announcements
                let broadcast_success = local_relay.notify_event(event.clone());

                tracing::debug!(
                    event_id = %event.id,
                    relay = %relay_url,
                    kind = %event.kind.as_u16(),
                    broadcast = broadcast_success,
                    "Synced event saved and broadcast"
                );
            }
            PolicyResult::Reject(reason) => {
                tracing::debug!(
                    event_id = %event.id,
                    relay = %relay_url,
                    reason = %reason,
                    "Event rejected by write policy"
                );
            }
        }
    }

    // =========================================================================
    // Consolidation System (Phase 6)
    // =========================================================================

    /// Get the current filter count for a relay
    ///
    /// Counts both pending subscriptions (outstanding_subs in batches) and
    /// confirmed subscriptions (active Layer 2/3 filters based on RelayState).
    /// This is used to determine if consolidation is needed.
    ///
    /// Confirmed filter counts:
    /// - Layer 1: 1 filter (announcement subscription)
    /// - Layer 2: 3 filters per 100-repo chunk (for kinds 1617/1618/1619/1621)
    /// - Layer 3: 3 filters per 100-event chunk (for replies/reactions/etc)
    async fn get_filter_count(&self, relay_url: &str) -> usize {
        // Count pending subscriptions
        let pending_count = {
            let pending = self.pending_sync_index.read().await;
            match pending.get(relay_url) {
                Some(batches) => batches.iter().map(|b| b.outstanding_subs.len()).sum(),
                None => 0,
            }
        };

        // Count confirmed subscriptions from relay state
        let confirmed_count = {
            let relay_index = self.relay_sync_index.read().await;
            if let Some(state) = relay_index.get(relay_url) {
                // Layer 1: 1 filter for announcements
                // Layer 2: 3 filters per 100-repo chunk (ceiling division)
                // Layer 3: 3 filters per 100-event chunk (ceiling division)
                let repo_count = state.repos.len();
                let event_count = state.root_events.len();
                
                let layer1_filters = 1;
                let layer2_filters = if repo_count > 0 {
                    ((repo_count + 99) / 100) * 3
                } else {
                    0
                };
                let layer3_filters = if event_count > 0 {
                    ((event_count + 99) / 100) * 3
                } else {
                    0
                };
                
                layer1_filters + layer2_filters + layer3_filters
            } else {
                0
            }
        };

        let total_count = pending_count + confirmed_count;

        tracing::debug!(
            relay = %relay_url,
            pending_count = pending_count,
            confirmed_count = confirmed_count,
            total_count = total_count,
            "Counted active filters for relay"
        );

        total_count
    }

    /// Wait until all pending batches for a relay are complete
    ///
    /// Polls the pending_sync_index until the relay has no pending batches.
    /// Returns error if timeout (30 seconds) is exceeded.
    async fn wait_pending_complete(&self, relay_url: &str) -> Result<(), String> {
        use std::time::Duration;
        use tokio::time::{sleep, Instant};

        let start = Instant::now();
        let timeout = Duration::from_secs(CONSOLIDATION_WAIT_TIMEOUT_SECS);

        tracing::debug!(
            relay = %relay_url,
            timeout_secs = CONSOLIDATION_WAIT_TIMEOUT_SECS,
            "Waiting for pending batches to complete"
        );

        loop {
            // Check if no pending batches
            {
                let pending = self.pending_sync_index.read().await;
                if !pending.contains_key(relay_url) {
                    tracing::debug!(
                        relay = %relay_url,
                        elapsed_ms = start.elapsed().as_millis(),
                        "All pending batches complete"
                    );
                    return Ok(());
                }
            }

            // Check timeout
            if start.elapsed() > timeout {
                tracing::warn!(
                    relay = %relay_url,
                    timeout_secs = CONSOLIDATION_WAIT_TIMEOUT_SECS,
                    "Timeout waiting for pending batches"
                );
                return Err(format!(
                    "Timeout waiting for pending batches on {} after {}s",
                    relay_url, CONSOLIDATION_WAIT_TIMEOUT_SECS
                ));
            }

            // Short poll interval
            sleep(Duration::from_millis(100)).await;
        }
    }

    /// Check if consolidation is needed and trigger if threshold exceeded
    ///
    /// Compares current filter count + new filter count against the threshold.
    /// If exceeded, triggers consolidation before adding new filters.
    async fn maybe_consolidate(&mut self, relay_url: &str, new_count: usize) {
        let current_count = self.get_filter_count(relay_url).await;

        if current_count + new_count > CONSOLIDATION_THRESHOLD {
            tracing::info!(
                relay = %relay_url,
                current_count = current_count,
                new_count = new_count,
                threshold = CONSOLIDATION_THRESHOLD,
                "Filter count exceeds threshold, consolidating"
            );
            
            if let Err(e) = self.consolidate(relay_url).await {
                tracing::error!(
                    relay = %relay_url,
                    error = %e,
                    "Consolidation failed"
                );
            }
        }
    }

    /// Consolidate all subscriptions for a relay
    ///
    /// This method:
    /// 1. Waits for all pending batches to complete
    /// 2. Unsubscribes from all active subscriptions
    /// 3. Rebuilds Layer 2 and Layer 3 with since filter
    ///
    /// Layer 1 (announcements) remains active and is NOT unsubscribed.
    async fn consolidate(&mut self, relay_url: &str) -> Result<(), String> {
        tracing::info!(
            relay = %relay_url,
            "Starting consolidation"
        );

        // Step 1: Wait for all pending batches to complete
        self.wait_pending_complete(relay_url).await?;

        // Step 2: Get connection and unsubscribe all
        let connection = match self.connections.get(relay_url) {
            Some(conn) => conn,
            None => {
                tracing::debug!(
                    relay = %relay_url,
                    "No connection found, skipping consolidation"
                );
                return Ok(()); // No connection, nothing to consolidate
            }
        };
        
        connection.unsubscribe_all().await;

        // Step 3: Rebuild all subscriptions with since filter
        let now = Timestamp::now();
        let since = Timestamp::from(now.as_secs().saturating_sub(QUICK_RECONNECT_WINDOW_SECS));

        // Re-subscribe to Layer 1 with since filter
        let layer1_filter = filters::build_announcement_filter(Some(since));
        if let Err(e) = connection.subscribe_filter(layer1_filter).await {
            tracing::error!(
                relay = %relay_url,
                error = %e,
                "Failed to re-subscribe to Layer 1 during consolidation"
            );
        }

        // Rebuild Layer 2 and Layer 3 with since filter
        self.rebuild_layer2_and_layer3(relay_url, Some(since)).await;

        tracing::info!(
            relay = %relay_url,
            since = %since,
            "Consolidation complete - filter count reset"
        );

        Ok(())
    }

    /// Check for relays that should be disconnected
    ///
    /// This method is called periodically by run_disconnect_checker.
    /// It identifies non-bootstrap relays that have no repos or root events
    /// to sync and disconnects them to free up resources.
    ///
    /// Bootstrap relays are NEVER disconnected, even if empty.
    async fn check_disconnects(&mut self) {
        // Collect relays to disconnect
        let to_disconnect: Vec<String> = {
            let index = self.relay_sync_index.read().await;
            index
                .iter()
                .filter_map(|(relay_url, state)| {
                    // Skip bootstrap relays - they stay connected
                    if state.is_bootstrap {
                        return None;
                    }

                    // Disconnect if no repos and no root events
                    if state.repos.is_empty() && state.root_events.is_empty() {
                        Some(relay_url.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        if to_disconnect.is_empty() {
            tracing::trace!("No empty relays to disconnect");
            return;
        }

        tracing::info!(
            count = to_disconnect.len(),
            relays = ?to_disconnect,
            "Found empty non-bootstrap relays to disconnect"
        );

        // Disconnect empty relays
        for relay_url in to_disconnect {
            self.disconnect_relay(&relay_url).await;
        }
    }

    /// Disconnect a relay and clean up all associated state
    ///
    /// This method:
    /// - Removes the relay from relay_sync_index
    /// - Removes the relay from pending_sync_index
    /// - Disconnects the connection if it exists
    ///
    /// Used by check_disconnects for cleanup of empty relays.
    async fn disconnect_relay(&mut self, relay_url: &str) {
        tracing::info!(relay = %relay_url, "Disconnecting empty relay");

        // Remove from relay_sync_index
        {
            let mut index = self.relay_sync_index.write().await;
            if index.remove(relay_url).is_some() {
                tracing::debug!(
                    relay = %relay_url,
                    "Removed relay from relay_sync_index"
                );
            }
        }

        // Remove from pending_sync_index
        {
            let mut pending = self.pending_sync_index.write().await;
            if pending.remove(relay_url).is_some() {
                tracing::debug!(
                    relay = %relay_url,
                    "Removed relay from pending_sync_index"
                );
            }
        }

        // Disconnect the connection if it exists
        if let Some(connection) = self.connections.remove(relay_url) {
            connection.disconnect().await;
            tracing::debug!(
                relay = %relay_url,
                "Disconnected connection"
            );
        }

        tracing::info!(relay = %relay_url, "Relay disconnected and cleaned up");
    }

    /// Check for disconnected relays that should be reconnected
    ///
    /// This method is called periodically by run_disconnect_checker.
    /// It identifies relays that:
    /// - Are currently disconnected
    /// - Have repos or root events to sync (not empty)
    /// - Have passed the exponential backoff period (respects health tracker)
    ///
    /// For each eligible relay, a reconnection is attempted via spawn_relay_connection.
    async fn check_reconnects(&mut self) {
        // Collect relays to reconnect
        let to_reconnect: Vec<String> = {
            let index = self.relay_sync_index.read().await;
            index
                .iter()
                .filter_map(|(relay_url, state)| {
                    // Only consider disconnected relays
                    if state.connection_status != ConnectionStatus::Disconnected {
                        return None;
                    }

                    // Skip empty relays - they'll be cleaned up by check_disconnects
                    if state.repos.is_empty() && state.root_events.is_empty() {
                        return None;
                    }

                    // Check if backoff period has elapsed
                    if self.health_tracker.should_attempt_connection(relay_url) {
                        Some(relay_url.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        if to_reconnect.is_empty() {
            tracing::trace!("No disconnected relays ready for reconnection");
            return;
        }

        tracing::info!(
            count = to_reconnect.len(),
            relays = ?to_reconnect,
            "Attempting reconnection for disconnected relays"
        );

        // Reconnect eligible relays
        for relay_url in to_reconnect {
            tracing::info!(
                relay = %relay_url,
                health_state = %self.health_tracker.get_state(&relay_url),
                "Attempting reconnection"
            );
            self.spawn_relay_connection(relay_url).await;
        }
    }

    /// Gracefully shutdown the SyncManager
    ///
    /// This method:
    /// - Sends shutdown signal to all background tasks (daily timer, disconnect checker)
    /// - Disconnects all relay connections
    /// - Clears all indices (relay_sync_index, pending_sync_index)
    ///
    /// After calling this method, the SyncManager is no longer usable.
    pub async fn shutdown(&mut self) {
        tracing::info!("Starting SyncManager shutdown");

        // 1. Send shutdown signal to all background tasks
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(());
            tracing::debug!("Sent shutdown signal to background tasks");
        }

        // 2. Disconnect all relay connections
        let relay_urls: Vec<String> = self.connections.keys().cloned().collect();
        for relay_url in relay_urls {
            if let Some(connection) = self.connections.remove(&relay_url) {
                tracing::debug!(relay = %relay_url, "Disconnecting relay");
                connection.disconnect().await;
            }
        }

        // 3. Clear all indices
        {
            let mut index = self.relay_sync_index.write().await;
            let count = index.len();
            index.clear();
            tracing::debug!(count = count, "Cleared relay_sync_index");
        }

        {
            let mut pending = self.pending_sync_index.write().await;
            let count = pending.len();
            pending.clear();
            tracing::debug!(count = count, "Cleared pending_sync_index");
        }

        tracing::info!("SyncManager shutdown complete");
    }
}