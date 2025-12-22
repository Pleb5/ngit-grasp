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
pub use metrics::SyncMetrics;

// Re-export relay connection types
pub use relay_connection::{NegentropySyncResult, RelayConnection, RelayEvent};

// Re-export self-subscriber types
pub use self_subscriber::SelfSubscriber;

// Re-export health tracking types
pub use health::RelayHealthTracker;
use tokio::time::sleep;

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
    /// Root event IDs - 1617/1618/1621 - that reference this repo
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
    /// Whether announcement filter historic sync has completed for this relay
    /// Used to determine if we can use `since` filter on reconnect for Layer 1
    pub announcements_synced: bool,
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
            announcements_synced: false,
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
        self.announcements_synced = false;
    }
}

/// Method used for synchronization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMethod {
    /// Traditional REQ+EOSE flow - waits for EOSE on subscriptions
    ReqEose,
    /// NIP-77 negentropy sync - confirms immediately after sync completes
    Negentropy,
}

/// Result of processing an event from sync
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessResult {
    /// Event was new and saved to database
    Saved,
    /// Event already existed in database
    Duplicate,
    /// Event rejected by write policy
    Rejected,
}

/// Pagination state for a subscription in non-Negentropy historic sync
#[derive(Debug, Clone)]
pub struct PaginationState {
    /// Number of events received for this subscription
    pub event_count: usize,
    /// Smallest created_at timestamp seen (for pagination with `until`)
    pub min_created_at: Option<Timestamp>,
    /// Original filter to reconstruct for next page
    pub original_filter: Filter,
}

/// A batch of items pending confirmation
#[derive(Debug, Clone)]
pub struct PendingBatch {
    /// Unique ID for this batch - for debugging/logging
    pub batch_id: u64,
    /// The items this batch is syncing
    pub items: PendingItems,
    /// Subscription IDs that must ALL receive EOSE before confirming (for ReqEose)
    /// Empty for Negentropy sync method
    pub outstanding_subs: HashSet<SubscriptionId>,
    /// The sync method used for this batch
    pub sync_method: SyncMethod,
    /// Pagination tracking for REQ+EOSE subscriptions (empty for Negentropy)
    /// Maps subscription ID to its pagination state
    pub pagination_state: HashMap<SubscriptionId, PaginationState>,
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

/// Page size threshold for historic sync pagination (non-negentropy)
/// If a subscription receives >= 75 events, we fetch the next page
const PAGINATION_THRESHOLD: usize = 75;

// =============================================================================
// Daily Timer
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
    use ::rand::Rng;

    loop {
        // Random interval between 23-25 hours
        let hours = 23.0 + ::rand::thread_rng().gen::<f64>() * 2.0;
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

// Combined Health and Metrics Checker

/// Run the combined health and metrics checker
///
/// This function runs in a loop with a 2-second interval, performing three tasks:
/// 1. **Disconnect checking**: Check for empty relays and disconnect non-bootstrap ones
/// 2. **Rate limit recovery**: Check for relays whose rate limit cooldown has expired
/// 3. **Metrics update**: Update Prometheus metrics with current health states from health_tracker
///
/// The metrics update ensures that health states are kept current in metrics even when
/// they change due to timeouts, cooldowns expiring, or stability periods completing.
///
/// The 2-second interval provides a good balance between responsiveness and overhead.
/// While disconnect checking traditionally ran at 60s intervals, the faster cadence here
/// is acceptable since the operations are lightweight (just index checks, no I/O).
async fn run_health_and_metrics_checker(
    sync_manager: Arc<Mutex<SyncManager>>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let interval = Duration::from_secs(2);
    tracing::info!("Health and metrics checker started with 2s interval");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                tracing::debug!("Health and metrics checker running");

                let mut manager = sync_manager.lock().await;

                // 1. Check for disconnects and retry disconnected relays
                manager.check_disconnects().await;
                manager.retry_disconnected_relays().await;

                // 2. Check for rate limit recovery
                manager.check_rate_limit_recovery().await;

                // 3. Update metrics with current health states
                if let Some(ref metrics) = manager.metrics {
                    // Get all tracked relay URLs
                    let relay_urls: Vec<String> = {
                        let index = manager.relay_sync_index.read().await;
                        index.keys().cloned().collect()
                    };

                    // Update health state for each relay
                    for relay_url in relay_urls {
                        let state = manager.health_tracker.get_state(&relay_url);
                        metrics.record_health_state(&relay_url, state);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("Health and metrics checker received shutdown signal");
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
    ///   - Calls confirm_batch to move items to confirmed state
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
        let batch_index = batches
            .iter()
            .position(|b| b.outstanding_subs.contains(&sub_id));

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

        // Check for pagination: if this subscription hit the threshold, fetch next page
        if let Some(pagination_state) = batch.pagination_state.remove(&sub_id) {
            if pagination_state.event_count >= PAGINATION_THRESHOLD {
                if let Some(min_created_at) = pagination_state.min_created_at {
                    tracing::info!(
                        relay = %relay_url,
                        sub_id = %sub_id,
                        batch_id = batch.batch_id,
                        event_count = pagination_state.event_count,
                        min_created_at = %min_created_at,
                        "Subscription hit pagination threshold, fetching next page"
                    );

                    // Create next page filter: same as original but with .until(min_created_at)
                    // dont subtract 1 second to avoid duplicate events at the boundary
                    // as this would lead to missed events with the same created_at timestamp
                    let until_timestamp = Timestamp::from(min_created_at.as_secs());
                    let mut next_filter = pagination_state.original_filter.clone();
                    next_filter = next_filter.until(until_timestamp);

                    // Store relay_url for spawning the subscription after releasing the lock
                    let relay_url_for_pagination = relay_url.to_string();
                    let batch_id = batch.batch_id;

                    // Drop the lock before async operations
                    drop(pending);

                    // Wait for rate limiting to clear before pagination continues
                    if self.health_tracker.is_rate_limited(relay_url) {
                        tracing::debug!(
                            relay = %relay_url,
                            batch_id = batch_id,
                            "Relay is rate limited, waiting before pagination"
                        );

                        // Loop until rate limit clears, sleeping with jitter between checks
                        while self.health_tracker.is_rate_limited(relay_url) {
                            let jitter_secs = 1 + (rand::random::<u64>() % 5); // 1-5 seconds
                            sleep(Duration::from_secs(jitter_secs)).await;
                        }

                        tracing::debug!(
                            relay = %relay_url,
                            batch_id = batch_id,
                            "Rate limit cleared, continuing pagination"
                        );
                        let batch_exists = {
                            let pending = self.pending_sync_index.read().await;
                            pending
                                .get(&relay_url_for_pagination)
                                .map(|batches| batches.iter().any(|b| b.batch_id == batch_id))
                                .unwrap_or(false)
                        };

                        // If we were rate limited, verify batch still exists after waiting
                        // (batches are wiped during disconnect, so avoid orphaned pagination)
                        if !batch_exists {
                            tracing::debug!(
                                relay = %relay_url_for_pagination,
                                batch_id = batch_id,
                                "Batch no longer exists after rate limit wait, skipping pagination"
                            );
                            return;
                        }
                    }

                    // Subscribe to next page and add to outstanding_subs
                    if let Some(conn) = self.connections.get(&relay_url_for_pagination) {
                        match conn.subscribe_filter(next_filter.clone(), true).await {
                            Ok(new_sub_id) => {
                                // Re-acquire lock to update the batch
                                let mut pending = self.pending_sync_index.write().await;
                                if let Some(batches) = pending.get_mut(&relay_url_for_pagination) {
                                    if let Some(batch) =
                                        batches.iter_mut().find(|b| b.batch_id == batch_id)
                                    {
                                        batch.outstanding_subs.insert(new_sub_id.clone());
                                        // Initialize pagination state for new subscription
                                        batch.pagination_state.insert(
                                            new_sub_id.clone(),
                                            PaginationState {
                                                event_count: 0,
                                                min_created_at: None,
                                                original_filter: next_filter,
                                            },
                                        );
                                        tracing::info!(
                                            relay = %relay_url_for_pagination,
                                            new_sub_id = %new_sub_id,
                                            batch_id = batch_id,
                                            until = %until_timestamp,
                                            "Next page subscription created"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    relay = %relay_url_for_pagination,
                                    batch_id = batch_id,
                                    error = %e,
                                    "Failed to create pagination subscription, continuing without next page"
                                );
                            }
                        }
                    }

                    // Early return since we've released and re-acquired locks
                    return;
                }
            }
        }

        // Check if batch is complete
        if !batch.outstanding_subs.is_empty() {
            return;
        }

        // 2. Batch complete - extract and remove
        let completed_batch = batches.remove(batch_idx);

        // Clean up empty relay entry
        if batches.is_empty() {
            pending.remove(relay_url);
        }

        // Drop the pending lock before confirm_batch
        drop(pending);

        // 3. Confirm the batch (moves items to RelayState)
        self.confirm_batch(relay_url, completed_batch).await;
    }

    /// Confirm a completed batch by moving items to RelayState
    ///
    /// This method is used by both sync paths (REQ+EOSE and Negentropy) to
    /// move repos and root_events from pending to confirmed state. This unified
    /// flow ensures consistent state tracking regardless of sync method.
    ///
    /// For generic filter batches (identified by empty repos and root_events),
    /// this sets the announcements_synced flag to enable incremental sync on reconnect.
    ///
    /// # Arguments
    /// * `relay_url` - The relay URL the batch belongs to
    /// * `batch` - The completed batch to confirm
    async fn confirm_batch(&self, relay_url: &str, batch: PendingBatch) {
        let batch_id = batch.batch_id;
        let repos_count = batch.items.repos.len();
        let events_count = batch.items.root_events.len();
        let sync_method = batch.sync_method;
        let is_generic_filter = repos_count == 0 && events_count == 0;

        let mut relay_index = self.relay_sync_index.write().await;

        if let Some(state) = relay_index.get_mut(relay_url) {
            // Move repos to confirmed
            state.repos.extend(batch.items.repos);
            // Move root_events to confirmed
            state.root_events.extend(batch.items.root_events.clone());

            // Set announcements_synced flag for generic filter batches
            if is_generic_filter {
                state.announcements_synced = true;
                tracing::info!(
                    relay = %relay_url,
                    batch_id = batch_id,
                    sync_method = ?sync_method,
                    "Generic filter (announcements) historic sync complete - announcements_synced set to true"
                );
            }

            // DEBUG TRACING: Log the root events being confirmed
            tracing::info!(
                relay = %relay_url,
                batch_id = batch_id,
                sync_method = ?sync_method,
                repos_confirmed = repos_count,
                root_events_confirmed = events_count,
                root_events_ids = ?batch.items.root_events.iter().map(|id| id.to_hex()).collect::<Vec<_>>(),
                total_repos = state.repos.len(),
                total_root_events = state.root_events.len(),
                all_root_events = ?state.root_events.iter().map(|id| id.to_hex()).collect::<Vec<_>>(),
                is_generic_filter = is_generic_filter,
                announcements_synced = state.announcements_synced,
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

        // maybe we just run start fresh with a daily flag? make sture so start layer 1 filters
        self.fresh_start(relay_url).await;

        // if let Some(ref metrics) = self.metrics {
        //     metrics.record_event(event_source::DAILY);
        // }

        // tracing::info!(relay = %relay_url, "Daily sync complete");
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
            self.register_relay(bootstrap_url.clone()).await;
            self.try_connect_relay(bootstrap_url).await;
        }

        // 7. Wrap self in Arc<Mutex> for sharing with timer task
        let sync_manager = Arc::new(Mutex::new(self));

        // 8. Spawn daily timer task with shutdown receiver
        let timer_manager = Arc::clone(&sync_manager);
        let timer_shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move {
            run_daily_timer(timer_manager, timer_shutdown).await;
        });

        // 9. Spawn health and metrics checker task with shutdown receiver
        // This combines disconnect checking, rate limit recovery, and metrics updates
        let checker_manager = Arc::clone(&sync_manager);
        let checker_shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move {
            run_health_and_metrics_checker(checker_manager, checker_shutdown).await;
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
                            manager.handle_new_sync_filters(add_filters).await;
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
    async fn handle_new_sync_filters(&mut self, action: AddFilters) {
        // Step 1: Check if relay exists in relay_sync_index
        let connection_status = {
            let index = self.relay_sync_index.read().await;
            index.get(&action.relay_url).map(|s| s.connection_status)
        };

        match connection_status {
            None => {
                // New relay - register and connect
                tracing::info!(
                    relay = %action.relay_url,
                    repos = action.items.repos.len(),
                    "Registering and connecting to new relay"
                );

                // Register relay (creates RelayConnection, initializes RelayState, updates metrics)
                self.register_relay(action.relay_url.clone()).await;
                self.try_connect_relay(&action.relay_url).await;
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

        // Step 2: Check if relay is rate-limited before creating new pending items
        if self.health_tracker.is_rate_limited(&action.relay_url) {
            tracing::debug!(
                relay = %action.relay_url,
                repos = action.items.repos.len(),
                root_events = action.items.root_events.len(),
                "Skipping AddFilters for rate-limited relay, will recompute after cooldown"
            );
            return;
        }

        // Step 3: Check if consolidation is needed BEFORE adding new filters
        self.maybe_consolidate(&action.relay_url, action.filters.len())
            .await;

        // Subscribe to each filter and collect subscription IDs
        tracing::info!(
            relay = %action.relay_url,
            filter_count = action.filters.len(),
            repo_count = action.items.repos.len(),
            root_event_count = action.items.root_events.len(),
            "handle_add_filters: calling sync_live and historic_sync"
        );

        self.sync_live(&action.relay_url, &action.filters).await;
        self.historic_sync(&action.relay_url, action.filters, action.items, None)
            .await;
    }

    /// Handle a connection success (called when a relay connects or reconnects)
    ///
    /// This method:
    /// 1. Updates RelayState to Connected
    /// 2. Spawns event loop (MUST happen on every connection/reconnect)
    /// 3. Dispatches to appropriate reconnection strategy based on disconnect time
    async fn handle_connect_or_reconnect(&mut self, relay_url: &str) {
        use tokio::sync::mpsc;

        // 1. Capture old last_connected BEFORE updating state
        // This is critical for correct first-connection detection
        let old_last_connected = {
            let index = self.relay_sync_index.read().await;
            index.get(relay_url).and_then(|s| s.last_connected)
        };

        // 2. Update state to Connected
        {
            let mut index = self.relay_sync_index.write().await;
            let state = index.entry(relay_url.to_string()).or_default();
            state.connection_status = ConnectionStatus::Connected;
            state.last_connected = Some(Timestamp::now());
            state.disconnected_at = None;
        }

        // Update metrics
        if let Some(ref metrics) = self.metrics {
            metrics.set_relay_connected(relay_url, true);
            metrics.inc_connected_count();
        }

        // 2. SPAWN EVENT LOOP (moved from spawn_relay_connection)
        // This MUST happen on every connection (initial or reconnect)
        // because event loops die on disconnect and cannot be reused
        let connection = match self.connections.get(relay_url) {
            Some(c) => c.clone(),
            None => {
                tracing::error!(relay = %relay_url, "No RelayConnection found for connected relay");
                return;
            }
        };

        let (event_tx, mut event_rx) = mpsc::channel::<RelayEvent>(1000);

        // Spawn event loop task
        let relay_url_for_loop = relay_url.to_string();
        tokio::spawn(async move {
            connection.run_event_loop(event_tx).await;
            tracing::debug!(relay = %relay_url_for_loop, "Event loop terminated");
        });

        // Spawn event processor task
        let relay_url_clone = relay_url.to_string();
        let database = Arc::clone(&self.database);
        let write_policy = self.write_policy.clone();
        let local_relay = self.local_relay.clone();
        let disconnect_tx = self.disconnect_tx.as_ref().unwrap().clone();
        let eose_tx = self.eose_tx.as_ref().unwrap().clone();
        let metrics_clone = self.metrics.clone();
        let pending_sync_index = Arc::clone(&self.pending_sync_index);
        let health_tracker = Arc::clone(&self.health_tracker);

        tokio::spawn(async move {
            let mut disconnect_sent = false;

            while let Some(relay_event) = event_rx.recv().await {
                match relay_event {
                    RelayEvent::Event(event, subscription_id) => {
                        let result = Self::process_event_static(
                            &event,
                            &relay_url_clone,
                            &database,
                            &write_policy,
                            &local_relay,
                        )
                        .await;
                        // Only record metric when event is actually saved
                        if result == ProcessResult::Saved {
                            if let Some(ref metrics) = metrics_clone {
                                metrics.record_synced_event();
                            }
                        }

                        // Track pagination state for this subscription
                        if result == ProcessResult::Saved || result == ProcessResult::Duplicate {
                            let mut pending = pending_sync_index.write().await;
                            if let Some(batches) = pending.get_mut(&relay_url_clone) {
                                for batch in batches.iter_mut() {
                                    if let Some(state) =
                                        batch.pagination_state.get_mut(&subscription_id)
                                    {
                                        state.event_count += 1;
                                        // Track minimum created_at timestamp
                                        match state.min_created_at {
                                            None => state.min_created_at = Some(event.created_at),
                                            Some(min) if event.created_at < min => {
                                                state.min_created_at = Some(event.created_at);
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    RelayEvent::EndOfStoredEvents(sub_id) => {
                        tracing::debug!(
                            relay = %relay_url_clone,
                            sub_id = %sub_id,
                            "EOSE received, notifying SyncManager"
                        );
                        let _ = eose_tx
                            .send(EoseNotification {
                                relay_url: relay_url_clone.clone(),
                                sub_id,
                            })
                            .await;
                    }
                    RelayEvent::Notice(notice) => {
                        // Check for rate limiting indicators
                        let notice_lower = notice.to_lowercase();
                        let is_rate_limit = (notice_lower.contains("rate")
                            && notice_lower.contains("limit"))
                            || notice_lower.contains("too many")
                            || notice_lower.contains("slow down")
                            || notice_lower.contains("throttl");

                        if is_rate_limit {
                            tracing::warn!(
                                relay = %relay_url_clone,
                                notice = %notice,
                                "Rate limiting NOTICE detected from relay"
                            );

                            // Mark relay as rate limited
                            health_tracker.record_rate_limit(&relay_url_clone);

                            // Update metrics with new health state
                            if let Some(ref metrics) = metrics_clone {
                                let state = health_tracker.get_state(&relay_url_clone);
                                metrics.record_health_state(&relay_url_clone, state);
                            }
                        } else {
                            tracing::debug!(
                                relay = %relay_url_clone,
                                notice = %notice,
                                "Relay issued notice"
                            );
                        }
                    }
                    RelayEvent::Closed(reason) => {
                        // CLOSED message means one subscription was closed, not the whole connection
                        // This is normal behavior (e.g., when historic_sync completes)
                        tracing::debug!(
                            relay = %relay_url_clone,
                            reason = %reason,
                            "Relay closed a subscription (not a connection close)"
                        );
                        // Don't break - other subscriptions remain active
                        // Don't send disconnect - connection is still alive
                    }
                    RelayEvent::Shutdown => {
                        tracing::info!(relay = %relay_url_clone, "Relay shutdown detected");
                        if !disconnect_sent {
                            let _ = disconnect_tx
                                .send(DisconnectNotification {
                                    relay_url: relay_url_clone.clone(),
                                })
                                .await;
                            disconnect_sent = true;
                        }
                        break;
                    }
                }
            }

            // If the event channel closed without a Closed/Shutdown event
            if !disconnect_sent {
                tracing::info!(
                    relay = %relay_url_clone,
                    "Event channel closed, notifying SyncManager of disconnect"
                );
                let _ = disconnect_tx
                    .send(DisconnectNotification {
                        relay_url: relay_url_clone,
                    })
                    .await;
            }
        });

        tracing::info!(
            relay = %relay_url,
            "Event loop and processor spawned for connected relay"
        );

        // 3. Decide reconnection strategy based on OLD last_connected time
        // Use the value captured BEFORE the update to correctly detect first connections
        if let Some(last) = old_last_connected {
            let elapsed = Timestamp::now().as_secs().saturating_sub(last.as_secs());
            if elapsed < QUICK_RECONNECT_WINDOW_SECS {
                // Short disconnect - quick reconnect
                tracing::info!(
                    relay = %relay_url,
                    disconnect_secs = elapsed,
                    "Short disconnection - initiating quick_reconnect"
                );
                self.quick_reconnect(relay_url, Timestamp::from(elapsed))
                    .await;
            } else {
                // Long disconnect - fresh start
                tracing::info!(
                    relay = %relay_url,
                    disconnect_secs = elapsed,
                    "Long disconnection - initiating fresh_start"
                );
                self.fresh_start(relay_url).await;
            }
        } else {
            // First connection - fresh start
            tracing::info!(
                relay = %relay_url,
                "First connection - initiating fresh_start"
            );
            self.fresh_start(relay_url).await;
        }
    }

    /// Fresh start - clears state and does full sync
    ///
    /// Called by: initial connect, long_reconnect, daily_sync
    ///
    /// Flow:
    /// 1. Clear PendingSyncIndex for this relay
    /// 2. Clear RelaySyncIndex sync state (repos/root_events)
    /// 3. Update connection state to Connected
    /// 4. L1 live + L1 historic (negentropy if available)
    /// 5. compute_actions → AddFilters → sync_computed_filters for L2+L3
    async fn fresh_start(&mut self, relay_url: &str) {
        let _now = Timestamp::now();

        tracing::info!(relay = %relay_url, "Starting fresh_start");

        // Step 1: Clear PendingSyncIndex for this relay
        {
            let mut pending = self.pending_sync_index.write().await;
            if pending.remove(relay_url).is_some() {
                tracing::debug!(
                    relay = %relay_url,
                    "Cleared pending batches in fresh_start"
                );
            }
        }

        // Step 2: Clear RelaySyncIndex sync state (but preserve connection metadata)
        {
            let mut index = self.relay_sync_index.write().await;
            if let Some(state) = index.get_mut(relay_url) {
                let repos_cleared = state.repos.len();
                let events_cleared = state.root_events.len();
                state.clear_sync_state();
                if repos_cleared > 0 || events_cleared > 0 {
                    tracing::debug!(
                        relay = %relay_url,
                        repos_cleared = repos_cleared,
                        events_cleared = events_cleared,
                        "Cleared sync state in fresh_start"
                    );
                }
                if state.connection_status == ConnectionStatus::Connected {
                    drop(index);
                    self.sync_generic_filters(relay_url, None).await;
                    // Step 5: compute_actions for L2+L3 (will be triggered by EOSE)
                    self.recompute_new_sync_filters_for_relay(relay_url).await;
                }
            } else {
                drop(index);
            }
        }
    }

    async fn sync_generic_filters(&mut self, relay_url: &str, since: Option<Timestamp>) {
        let filters = vec![filters::build_announcement_filter(None)];

        // Create live subscription for ongoing announcements
        let _sub_ids = self.sync_live(relay_url, &filters).await;

        // Use historic_sync with empty PendingItems for generic filters
        // Generic filters (announcements) don't have associated repos or root_events
        let items = PendingItems::default();
        let _batch_id = self.historic_sync(relay_url, filters, items, since).await;
    }

    /// Quick reconnect - for disconnections < 15 minutes
    ///
    /// Re-establishes subscriptions after a brief disconnection by:
    /// 1. Clearing stale PendingSyncIndex entries
    /// 2. Syncing L1 filters with since timestamp (announcements)
    /// 3. Rebuilding L2+L3 from preserved RelaySyncIndex state
    /// 4. Computing actions for new items discovered during catchup
    ///
    /// Basic connection state and metrics are managed by handle_connect_or_reconnect.
    /// This method handles reconnect-specific concerns (health tracking, reconnect metrics).
    async fn quick_reconnect(&mut self, relay_url: &str, since: Timestamp) {
        // Step 1: Clear PendingSyncIndex for this relay
        // Old subscriptions are dead after disconnect
        {
            let mut pending = self.pending_sync_index.write().await;
            pending.remove(relay_url);
        }

        // Record successful reconnection in health tracker
        self.health_tracker.record_success(relay_url);

        // Record reconnect-specific metrics (not basic connection metrics)
        if let Some(ref metrics) = self.metrics {
            metrics.record_health_state(relay_url, self.health_tracker.get_state(relay_url));
        }

        // Step 2: L1 live + L1 historic with since filter (or full sync if announcements never completed)
        let announcement_since = {
            let index = self.relay_sync_index.read().await;
            if let Some(state) = index.get(relay_url) {
                if state.announcements_synced {
                    Some(since) // Can use incremental sync
                } else {
                    None // Need full sync - announcements never completed
                }
            } else {
                None
            }
        };

        self.sync_generic_filters(relay_url, announcement_since)
            .await;

        // Step 3: Rebuild L2+L3 from confirmed state with since filter
        // This uses the preserved repos/root_events from RelaySyncIndex
        self.rebuild_layer2_and_layer3(relay_url, Some(since)).await;

        // Step 4: compute_actions for any NEW items discovered while disconnected
        self.recompute_new_sync_filters_for_relay(relay_url).await;
    }

    /// Rebuild Layer 2 and Layer 3 subscriptions for a relay
    ///
    /// Uses the confirmed repos and root_events from RelayState to build filters.
    /// If since is provided, applies it to all filters for incremental sync.
    ///
    /// CRITICAL: This method now creates a PendingBatch to track subscriptions,
    /// ensuring EOSE handling works correctly for live sync scenarios.
    async fn rebuild_layer2_and_layer3(&mut self, relay_url: &str, since: Option<Timestamp>) {
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

        if filters.is_empty() {
            tracing::debug!(
                relay = %relay_url,
                "No filters generated for Layer 2/3 rebuild"
            );
            return;
        }
        self.sync_live(relay_url, &filters).await;
    }

    /// Register a relay for managed connection/reconnection
    ///
    /// Creates a RelayConnection object and stores it in the connections HashMap.
    /// Also initializes RelayState if it doesn't exist.
    /// Does NOT connect - connection happens via try_connect_relay or retry_disconnected_relays.
    /// The RelayConnection persists forever and is reused on reconnects.
    async fn register_relay(&mut self, relay_url: String) {
        // Create RelayConnection if not exists
        if !self.connections.contains_key(&relay_url) {
            let connection =
                RelayConnection::new_with_database(relay_url.clone(), Arc::clone(&self.database));
            self.connections.insert(relay_url.clone(), connection);
            tracing::debug!(relay = %relay_url, "Registered new relay connection");
        }

        // Initialize RelayState if not exists
        let is_new = {
            let mut index = self.relay_sync_index.write().await;
            if !index.contains_key(&relay_url) {
                let new_state = RelayState {
                    connection_status: ConnectionStatus::Disconnected,
                    is_bootstrap: false,
                    last_connected: None,
                    disconnected_at: None,
                    repos: HashSet::new(),
                    root_events: HashSet::new(),
                    announcements_synced: false,
                };
                index.insert(relay_url.clone(), new_state);
                true
            } else {
                false
            }
        };

        // Track new relay in metrics
        if is_new {
            if let Some(ref metrics) = self.metrics {
                metrics.inc_tracked_count();
                // Initialize connection status to disconnected
                metrics.set_relay_connected(&relay_url, false);
            }
            tracing::info!(relay = %relay_url, "Registered new relay for tracking");
        }
    }

    /// Attempt a single connection to a registered relay
    ///
    /// Uses the existing RelayConnection from the HashMap and attempts to connect.
    /// On success, sends ConnectNotification which triggers handle_connect_or_reconnect.
    /// On failure, updates state and health tracker.
    async fn try_connect_relay(&mut self, relay_url: &str) {
        // 1. Mark attempting (optional, helpful for debugging)
        {
            let mut index = self.relay_sync_index.write().await;
            if let Some(state) = index.get_mut(relay_url) {
                state.connection_status = ConnectionStatus::Connecting;
            }
        }

        // 2. Record attempt in health tracker
        self.health_tracker.record_attempt(relay_url);

        // 3. Get connection and attempt
        let connection = match self.connections.get(relay_url) {
            Some(c) => c,
            None => {
                tracing::error!(relay = %relay_url, "No RelayConnection registered");
                return;
            }
        };

        let timeout = self.health_tracker.base_backoff_secs();

        match connection.connect(timeout).await {
            Ok(()) => {
                // Success - record and send notification
                self.health_tracker.record_success(relay_url);

                if let Some(ref metrics) = self.metrics {
                    metrics.record_connection_attempt(relay_url, true);
                }

                if let Some(ref connect_tx) = self.connect_tx {
                    let _ = connect_tx
                        .send(ConnectNotification {
                            relay_url: relay_url.to_string(),
                        })
                        .await;
                }
            }
            Err(e) => {
                tracing::error!(relay = %relay_url, error = %e, "Connection failed");

                // 4. Update state back to Disconnected on failure
                {
                    let mut index = self.relay_sync_index.write().await;
                    if let Some(state) = index.get_mut(relay_url) {
                        state.connection_status = ConnectionStatus::Disconnected;
                    }
                }

                // 5. Record failure in health tracker
                self.health_tracker.record_failure(relay_url);

                // 6. Update metrics
                if let Some(ref metrics) = self.metrics {
                    metrics.record_connection_attempt(relay_url, false);
                    metrics
                        .record_health_state(relay_url, self.health_tracker.get_state(relay_url));
                }
            }
        }
    }

    /// Recompute sync actions for a specific relay
    ///
    /// Uses derive_relay_targets and compute_actions to find new items
    /// that need to be synced. Processes AddFilters actions for new items.
    async fn recompute_new_sync_filters_for_relay(&mut self, relay_url: &str) {
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
                new_repos = action.items.repos.len(),
                new_root_events = action.items.root_events.len(),
                filters = action.filters.len(),
                "Processing AddFilters for new items"
            );
            self.handle_new_sync_filters(action).await;
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

        // 3. Keep RelayConnection in HashMap for reuse on reconnect
        // The connection object persists and will be reused when retry_disconnected_relays
        // calls try_connect_relay -> connection.connect()
        tracing::debug!(
            relay = %relay_url,
            "Keeping RelayConnection in HashMap for reconnection"
        );

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

    /// Process a single event from a relay (static version for spawned tasks)
    ///
    /// Processes events with dedup, policy check, database save, and broadcast:
    /// - Deduplication (skips if event already exists)
    /// - Write policy validation
    /// - Database save
    /// - Broadcast to WebSocket subscribers via notify_event (enables recursive relay discovery)
    ///
    /// Returns `ProcessResult` to indicate whether the event was saved, duplicate, or rejected.
    async fn process_event_static(
        event: &Event,
        relay_url: &str,
        database: &SharedDatabase,
        write_policy: &Nip34WritePolicy,
        local_relay: &LocalRelay,
    ) -> ProcessResult {
        use nostr_relay_builder::prelude::{WritePolicyResult, WritePolicy};
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        // Check if event already exists
        match database.event_by_id(&event.id).await {
            Ok(Some(_)) => {
                tracing::trace!(event_id = %event.id, "Event already exists, skipping");
                return ProcessResult::Duplicate;
            }
            Err(e) => {
                tracing::warn!(event_id = %event.id, error = %e, "Database error checking event");
                return ProcessResult::Rejected;
            }
            Ok(None) => {} // Continue processing
        }

        // Apply write policy using a dummy address (sync events aren't from network clients)
        let dummy_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let result = write_policy.admit_event(event, &dummy_addr).await;

        match result {
            WritePolicyResult::Accept => {
                // Save event to database
                if let Err(e) = database.save_event(event).await {
                    tracing::error!(
                        event_id = %event.id,
                        relay = %relay_url,
                        error = %e,
                        "Failed to save synced event"
                    );
                    return ProcessResult::Rejected;
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
                ProcessResult::Saved
            }
            WritePolicyResult::Reject { message, .. } => {
                tracing::debug!(
                    event_id = %event.id,
                    relay = %relay_url,
                    reason = %message,
                    "Event rejected by write policy"
                );
                ProcessResult::Rejected
            }
        }
    }

    // =========================================================================
    // Consolidation System
    // =========================================================================

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
        let current_count = if let Some(connection) = self.connections.get(relay_url) {
            connection.subscription_count().await
        } else {
            0
        };

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
        self.sync_generic_filters(relay_url, Some(since)).await;
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

    /// Retry disconnected relays that are ready for reconnection
    ///
    /// This method is called periodically by run_disconnect_checker.
    /// It identifies relays that:
    /// - Are currently disconnected
    /// - Have repos or root events to sync (not empty)
    /// - Have passed the exponential backoff period (respects health tracker)
    ///
    /// For each eligible relay, a reconnection is attempted via try_connect_relay.
    async fn retry_disconnected_relays(&mut self) {
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
            self.try_connect_relay(&relay_url).await;
        }
    }

    /// Check for rate-limited relays that have exceeded cooldown
    ///
    /// This method is called periodically by run_rate_limit_checker (every 1 second).
    /// For each relay in RateLimited state that has exceeded the 65-second cooldown:
    /// 1. Clears the rate limit state (sets to Healthy)
    /// 2. Recomputes required actions for that relay
    /// 3. Submits those actions
    async fn check_rate_limit_recovery(&mut self) {
        use crate::sync::algorithms::{compute_actions, derive_relay_targets};

        // Exit rate limiting for relays whose cooldown has expired
        let relays_to_recover: Vec<String> = self.health_tracker.exit_expired_rate_limits();

        if relays_to_recover.is_empty() {
            return;
        }

        // Recompute actions - could optimise by adding relays: Option<&[]> to derive_relay_targets
        let repo_index = self.repo_sync_index.read().await;
        let targets = derive_relay_targets(&repo_index);
        drop(repo_index);

        for relay_url in relays_to_recover {
            tracing::info!(
                relay = %relay_url,
                "Rate limit cooldown expired, recovering"
            );

            // Clear rate limit state
            self.health_tracker.clear_rate_limit(&relay_url);

            // Only compute actions for this specific relay
            if let Some(relay_needs) = targets.get(&relay_url) {
                let mut single_relay_targets = std::collections::HashMap::new();
                single_relay_targets.insert(relay_url.clone(), relay_needs.clone());

                let pending = self.pending_sync_index.read().await;
                let confirmed = self.relay_sync_index.read().await;

                let actions = compute_actions(&single_relay_targets, &pending, &confirmed);
                drop(pending);
                drop(confirmed);

                // Submit each action
                for action in actions {
                    tracing::info!(
                        relay = %action.relay_url,
                        repo_count = action.items.repos.len(),
                        event_count = action.items.root_events.len(),
                        "Submitting recovered actions after rate limit"
                    );
                    self.handle_new_sync_filters(action).await;
                }
            }
        }
    }

    /// Subscribe to filters for live (ongoing) events - NOT tracked in PendingSyncIndex
    ///
    /// This method applies limit(0) to all filters to receive ONLY new events.
    /// Per NIP-01, limit 0 means "send no stored events, only future events", which
    /// ensures EOSE is received immediately and all subsequent events are tagged as "live"
    /// in metrics (not "startup").
    ///
    /// **Important**: Callers pass the SAME filters to both sync_live() and historic_sync().
    /// This method applies limit(0) to prevent fetching historic events.
    ///
    /// Live subscriptions are NOT tracked in PendingSyncIndex because they don't have
    /// a definite "completion" - they stay open indefinitely.
    ///
    /// Used for:
    /// - Layer 1 live subscription (new announcements after initial sync)
    /// - Layer 2+3 live subscriptions (new events after initial sync)
    ///
    /// # Arguments
    /// * `relay_url` - The relay URL to subscribe on
    /// * `filters` - Filters to subscribe to (limit(0) will be applied)
    ///
    /// # Returns
    /// Vec of subscription IDs for the live subscriptions, or empty if connection not found
    async fn sync_live(&self, relay_url: &str, filters: &[Filter]) -> Vec<SubscriptionId> {
        if filters.is_empty() {
            return vec![];
        }

        let connection = match self.connections.get(relay_url) {
            Some(conn) => conn,
            None => {
                tracing::debug!(relay = %relay_url, "No connection found for live sync");
                return vec![];
            }
        };

        let mut sub_ids = Vec::new();

        for filter in filters.iter() {
            // Live subscriptions MUST use limit(0) to receive ONLY new events
            // This prevents fetching historic events that would be miscounted as "live" in metrics
            // The caller passes the same filters to both sync_live() and historic_sync()
            // Live subscriptions do NOT auto-close - we want them to stay open for new events
            match connection
                .subscribe_filter(filter.clone().limit(0), false)
                .await
            {
                Ok(sub_id) => {
                    sub_ids.push(sub_id);
                }
                Err(e) => {
                    tracing::error!(relay = %relay_url, error = %e, "Failed to create live subscription");
                }
            }
        }

        sub_ids
    }

    /// Sync historical events and track in PendingSyncIndex
    ///
    /// This method handles historical synchronization for a set of filters,
    /// creating a PendingBatch to track completion. It dispatches to either
    /// negentropy sync or traditional REQ+EOSE based on relay capability and config.
    ///
    /// Used for:
    /// - Initial sync (no since filter)
    /// - Reconnect sync (with since filter)
    /// - Daily sync (no since filter, full re-sync)
    ///
    /// # Arguments
    /// * `relay_url` - The relay URL to sync from
    /// * `filters` - Filters to sync (will have `since` applied if provided)
    /// * `items` - Items being synced (for tracking in PendingBatch)
    /// * `since` - Optional timestamp for incremental sync
    ///
    /// # Returns
    /// * `Some(batch_id)` - Batch was created and sync initiated
    /// * `None` - No connection or sync failed to start
    async fn historic_sync(
        &mut self,
        relay_url: &str,
        filters: Vec<Filter>,
        items: PendingItems,
        since: Option<Timestamp>,
    ) -> Option<u64> {
        // DEBUG TRACING: Log all filters being passed to historic_sync
        tracing::debug!(
            relay = %relay_url,
            filter_count = filters.len(),
            filters = ?filters,
            repos_count = items.repos.len(),
            root_events_count = items.root_events.len(),
            since = ?since,
            "historic_sync called"
        );

        if filters.is_empty() && items.repos.is_empty() && items.root_events.is_empty() {
            tracing::debug!(
                relay = %relay_url,
                "historic_sync called with empty filters and items, skipping"
            );
            return None;
        }

        // Check connection exists and clone for async usage
        let connection = match self.connections.get(relay_url) {
            Some(conn) => conn.clone(),
            None => {
                tracing::warn!(
                    relay = %relay_url,
                    "No connection found for historic_sync"
                );
                return None;
            }
        };

        // Apply since filter if provided
        let filters_with_since: Vec<Filter> = if let Some(ts) = since {
            filters.into_iter().map(|f| f.since(ts)).collect()
        } else {
            filters
        };

        // Check if we should use negentropy
        let use_negentropy =
            !self.config.sync_disable_negentropy && connection.supports_negentropy().await;

        // Generate batch ID
        let batch_id = self.next_batch_id();

        if use_negentropy && !filters_with_since.is_empty() {
            // NIP-77 negentropy path
            tracing::debug!(
                relay = %relay_url,
                batch_id = batch_id,
                filter_count = filters_with_since.len(),
                repos = items.repos.len(),
                root_events = items.root_events.len(),
                "Starting historic_sync with negentropy"
            );

            // Create PendingBatch for negentropy (empty outstanding_subs and pagination_state)
            let batch = PendingBatch {
                batch_id,
                items: items.clone(),
                outstanding_subs: HashSet::new(),
                sync_method: SyncMethod::Negentropy,
                pagination_state: HashMap::new(), // Negentropy doesn't use pagination
            };

            // Add to pending_sync_index
            {
                let mut pending = self.pending_sync_index.write().await;
                pending
                    .entry(relay_url.to_string())
                    .or_insert_with(Vec::new)
                    .push(batch);
            }

            // Perform negentropy sync for all filters concurrently
            // Note: We sync each filter separately because negentropy works on a single filter
            let diff_futures: Vec<_> = filters_with_since
                .iter()
                .enumerate()
                .map(|(idx, filter)| {
                    let filter = filter.clone();
                    let conn = connection.clone();
                    async move { (idx, conn.negentropy_sync_diff(filter).await) }
                })
                .collect();

            let diff_results = futures_util::future::join_all(diff_futures).await;

            // Process results - collect all event IDs we need to fetch
            let mut all_remote_ids = Vec::new();
            let mut failed_count = 0;

            for (idx, result) in diff_results {
                match result {
                    Ok(reconciliation) => {
                        let remote_count = reconciliation.remote.len();
                        let local_count = reconciliation.local.len();
                        tracing::debug!(
                            relay = %relay_url,
                            filter_idx = idx,
                            remote_count = remote_count,
                            local_count = local_count,
                            remote_ids = ?reconciliation.remote,
                            "[DIAG TRACE] ✓ Negentropy diff results for filter {}", idx
                        );
                        if remote_count > 0 {
                            all_remote_ids.extend(reconciliation.remote.into_iter());
                        }
                    }
                    Err(e) => {
                        failed_count += 1;
                        tracing::warn!(
                            relay = %relay_url,
                            filter_idx = idx,
                            error = %e,
                            "Negentropy diff failed for filter in historic_sync"
                        );
                    }
                }
            }

            // Require ALL filters to succeed to confirm the batch
            if failed_count > 0 {
                // Leave pending batch so it doesnt appear as synced. we can try again later.
                tracing::warn!(
                    relay = %relay_url,
                    batch_id = batch_id,
                    failed_count = failed_count,
                    total_filters = filters_with_since.len(),
                    "historic_sync (negentropy) failed - not all filters succeeded"
                );
                return None;
            } else if all_remote_ids.is_empty() {
                // Remove batch from pending and confirm it (no items to download)
                let completed_batch = {
                    let mut pending = self.pending_sync_index.write().await;
                    if let Some(batches) = pending.get_mut(relay_url) {
                        let batch_idx = batches.iter().position(|b| b.batch_id == batch_id);
                        if let Some(idx) = batch_idx {
                            let batch = batches.remove(idx);
                            if batches.is_empty() {
                                pending.remove(relay_url);
                            }
                            Some(batch)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(batch) = completed_batch {
                    self.confirm_batch(relay_url, batch).await;
                }

                tracing::info!(
                    relay = %relay_url,
                    batch_id = batch_id,
                    total_received = 0,
                    "historic_sync (negentropy) completed - already up-to-date"
                );

                // Batch already confirmed, nothing more to do
                return Some(batch_id);
            }

            // launch subscriptions to fetch missing events by id
            let ids_filters: Vec<_> = all_remote_ids
                .chunks(300)
                .map(|c| Filter::new().ids(c.iter().copied()))
                .collect();

            // DEBUG TRACING: Log that we're requesting events by ID
            tracing::info!(
                relay = %relay_url,
                batch_id = batch_id,
                total_event_ids = all_remote_ids.len(),
                filter_chunks = ids_filters.len(),
                event_ids = ?all_remote_ids,
                "[DIAG TRACE] ✓ Creating {} subscription(s) to fetch {} missing event(s) by ID",
                ids_filters.len(),
                all_remote_ids.len()
            );

            let mut subscription_ids = HashSet::new();
            for (idx, filter) in ids_filters.iter().enumerate() {
                if let Some(conn) = self.connections.get(relay_url) {
                    match conn.subscribe_filter(filter.clone(), true).await {
                        Ok(sub_id) => {
                            subscription_ids.insert(sub_id);
                        }
                        Err(e) => {
                            tracing::error!(
                                relay = %relay_url,
                                batch_id = batch_id,
                                chunk_idx = idx,
                                error = %e,
                                "Failed to subscribe to ID filter chunk"
                            );
                        }
                    }
                }
            }
            {
                let mut pending = self.pending_sync_index.write().await;
                if let Some(relay_batches) = pending.get_mut(relay_url) {
                    if let Some(batch) = relay_batches.iter_mut().find(|b| b.batch_id == batch_id) {
                        batch.outstanding_subs.extend(subscription_ids.clone());
                    }
                }
            }
            tracing::debug!(
                relay = %relay_url,
                batch_id = batch_id,
                subscription_ids = subscription_ids.len(),
                events = all_remote_ids.len(),
                "historic_sync (Negentropy) created subscritions to fetch missing events by id, awaiting EOSE"
            );
        } else {
            // Traditional REQ+EOSE path
            tracing::debug!(
                relay = %relay_url,
                batch_id = batch_id,
                filter_count = filters_with_since.len(),
                repos = items.repos.len(),
                root_events = items.root_events.len(),
                use_negentropy = use_negentropy,
                "Starting historic_sync with REQ+EOSE"
            );

            // Subscribe to each filter and collect subscription IDs
            let mut subscription_ids = HashSet::new();
            let mut pagination_state = HashMap::new();

            // DEBUG TRACING: Log each filter in REQ+EOSE path
            for (idx, filter) in filters_with_since.iter().enumerate() {
                tracing::debug!(
                    relay = %relay_url,
                    batch_id = batch_id,
                    filter_idx = idx,
                    filter = ?filter,
                    "Subscribing to filter in REQ+EOSE path"
                );

                if let Some(conn) = self.connections.get(relay_url) {
                    match conn.subscribe_filter(filter.clone(), true).await {
                        Ok(sub_id) => {
                            subscription_ids.insert(sub_id.clone());
                            // Initialize pagination state for this subscription
                            pagination_state.insert(
                                sub_id,
                                PaginationState {
                                    event_count: 0,
                                    min_created_at: None,
                                    original_filter: filter.clone(),
                                },
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                relay = %relay_url,
                                error = %e,
                                "Failed to subscribe to filter in historic_sync"
                            );
                        }
                    }
                }
            }

            if subscription_ids.is_empty() && !filters_with_since.is_empty() {
                tracing::warn!(
                    relay = %relay_url,
                    "All filter subscriptions failed in historic_sync"
                );
                return None;
            }

            // Create PendingBatch for REQ+EOSE
            let batch = PendingBatch {
                batch_id,
                items,
                outstanding_subs: subscription_ids,
                sync_method: SyncMethod::ReqEose,
                pagination_state,
            };

            // Add to pending_sync_index
            {
                let mut pending = self.pending_sync_index.write().await;
                pending
                    .entry(relay_url.to_string())
                    .or_insert_with(Vec::new)
                    .push(batch);
            }

            tracing::debug!(
                relay = %relay_url,
                batch_id = batch_id,
                "historic_sync (REQ+EOSE) batch created, awaiting EOSE"
            );
        }

        Some(batch_id)
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
