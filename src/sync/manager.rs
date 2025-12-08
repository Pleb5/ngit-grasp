//! SyncManager - Coordinates proactive sync operations
//!
//! The SyncManager connects to remote relays, receives events, validates them
//! through the write policy, and stores accepted events.
//!
//! ## Simplified Relay Discovery Architecture
//!
//! All relay discovery is centralized in the self-subscriber:
//! - Bootstrap relay: connected immediately (no jitter, single relay)
//! - All other relays: discovered via self-subscriber announcements (with jitter)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        ngit-grasp                           │
//! │                                                             │
//! │  ┌─────────────┐       broadcasts       ┌───────────────┐  │
//! │  │   Relay     │ ─────────────────────▶ │ Self-Subscribe│  │
//! │  │  Database   │                        │    Client     │  │
//! │  └─────────────┘                        └───────┬───────┘  │
//! │        ▲                                        │          │
//! │        │ stores                                 │ extracts │
//! │        │                                        │ relay    │
//! │  ┌─────┴─────┐                                 │ URLs     │
//! │  │  Remote   │◀────────────────────────────────┘          │
//! │  │Connections│           spawns new                       │
//! │  └───────────┘           connections (with jitter)        │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Key Design Decisions
//!
//! - **Single relay discovery path**: Only the self-subscriber discovers new relays
//! - **Jitter at point of discovery**: Applied when spawning connections from announcements
//! - **Since filter on reconnection**: Avoids re-processing old announcements after disconnect
//! - **Bootstrap relay has no jitter**: Single relay doesn't cause thundering herd
//!
//! ## Phase 2 Features
//!
//! - Relay discovery from kind 30617 announcements (via self-subscriber)
//! - Multiple simultaneous relay connections
//! - Three-layer filter strategy via FilterService
//!
//! ## Phase 3 Features
//!
//! - Health tracking with exponential backoff
//! - Dead relay detection after 24h of failures
//! - Startup jitter to prevent thundering herd
//!
//! ## Phase 4 Features
//!
//! - Dynamic subscription updates handled per-connection
//! - Each connection manages its own SubscriptionManager
//! - Announcements trigger Layer 2 subscriptions
//! - PRs/Issues trigger Layer 3 subscriptions
//! - Consolidation when filter count exceeds 150

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use nostr_relay_builder::prelude::*;
use nostr_sdk::prelude::{Client, Filter, Kind, RelayPoolNotification, Timestamp};
use rand::Rng;
use tokio::sync::mpsc;

use super::connection::{connect_with_retry, SyncedEvent};
use super::filter::FilterService;
use super::health::RelayHealthTracker;
use super::metrics::SyncMetrics;
use crate::config::Config;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};
use crate::nostr::events::KIND_REPOSITORY_ANNOUNCEMENT;

/// Default fallback address for sync source when bind_address cannot be parsed
///
/// This distinguishes synced events from directly-submitted events in logs and metrics.
/// Uses 127.0.0.1:8080 as a recognizable default "synced event" marker.
pub const DEFAULT_SYNC_SOURCE_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

/// Derive sync source address from config bind_address
///
/// Parses the bind_address string and returns a SocketAddr.
/// Falls back to 127.0.0.1:8080 if parsing fails.
fn get_sync_source_addr(bind_address: &str) -> SocketAddr {
    bind_address
        .parse()
        .unwrap_or(DEFAULT_SYNC_SOURCE_ADDR)
}

/// Derive the WebSocket URL for our own relay from bind_address
fn derive_own_relay_url(bind_address: &str) -> String {
    format!("ws://{}", bind_address)
}

/// Coordinates proactive sync from configured and discovered relays
pub struct SyncManager {
    /// Bootstrap relay URL for initial sync (from config)
    /// Additional relays are discovered from repository announcements that list our service
    bootstrap_relay_url: Option<String>,
    /// Our relay's domain (for filtering)
    relay_domain: String,
    /// Our relay's WebSocket URL (for self-subscribe)
    own_relay_url: String,
    /// Database for storing accepted events
    database: SharedDatabase,
    /// Write policy for validating events
    write_policy: Nip34WritePolicy,
    /// Health tracker for relay connections
    health_tracker: Arc<RelayHealthTracker>,
    /// Sync metrics for Prometheus
    metrics: Option<SyncMetrics>,
    /// Source address for synced events (derived from config.bind_address)
    sync_source_addr: SocketAddr,
    /// Maximum startup jitter in milliseconds (from config)
    startup_jitter_ms: u64,
}

impl SyncManager {
    /// Create a new SyncManager
    ///
    /// # Arguments
    /// * `bootstrap_relay_url` - Optional bootstrap relay URL from config
    /// * `relay_domain` - Our relay's domain (used to exclude self from sync)
    /// * `database` - Shared database for storing events and querying announcements
    /// * `write_policy` - Write policy for validating synced events
    /// * `config` - Configuration for health tracking settings
    pub fn new(
        bootstrap_relay_url: Option<String>,
        relay_domain: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
        config: &Config,
    ) -> Self {
        let own_relay_url = derive_own_relay_url(&config.bind_address);
        Self {
            bootstrap_relay_url,
            relay_domain,
            own_relay_url,
            database,
            write_policy,
            health_tracker: Arc::new(RelayHealthTracker::new(config)),
            metrics: None,
            sync_source_addr: get_sync_source_addr(&config.bind_address),
            startup_jitter_ms: config.sync_startup_jitter_ms,
        }
    }

    /// Create a new SyncManager with metrics
    ///
    /// # Arguments
    /// * `bootstrap_relay_url` - Optional bootstrap relay URL from config
    /// * `relay_domain` - Our relay's domain (used to exclude self from sync)
    /// * `database` - Shared database for storing events and querying announcements
    /// * `write_policy` - Write policy for validating synced events
    /// * `config` - Configuration for health tracking settings
    /// * `metrics` - Sync metrics for Prometheus
    pub fn with_metrics(
        bootstrap_relay_url: Option<String>,
        relay_domain: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
        config: &Config,
        metrics: SyncMetrics,
    ) -> Self {
        let own_relay_url = derive_own_relay_url(&config.bind_address);
        Self {
            bootstrap_relay_url,
            relay_domain,
            own_relay_url,
            database,
            write_policy,
            health_tracker: Arc::new(RelayHealthTracker::new(config)),
            metrics: Some(metrics),
            sync_source_addr: get_sync_source_addr(&config.bind_address),
            startup_jitter_ms: config.sync_startup_jitter_ms,
        }
    }

    /// Create a SyncManager with a single relay URL (Phase 1 compatibility)
    pub fn with_single_relay(
        bootstrap_url: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
    ) -> Self {
        // Extract domain from URL for filtering
        let relay_domain = extract_domain_from_url(&bootstrap_url).unwrap_or_default();
        let own_relay_url = format!("ws://{}", relay_domain);
        Self {
            bootstrap_relay_url: Some(bootstrap_url),
            relay_domain,
            own_relay_url,
            database,
            write_policy,
            health_tracker: Arc::new(RelayHealthTracker::with_defaults()),
            metrics: None,
            sync_source_addr: DEFAULT_SYNC_SOURCE_ADDR,
            startup_jitter_ms: 10_000, // Default 10 seconds
        }
    }

    /// Set metrics for the sync manager
    pub fn set_metrics(&mut self, metrics: SyncMetrics) {
        self.metrics = Some(metrics);
    }

    /// Get a reference to the metrics
    pub fn metrics(&self) -> Option<&SyncMetrics> {
        self.metrics.as_ref()
    }

    /// Get a reference to the health tracker
    pub fn health_tracker(&self) -> Arc<RelayHealthTracker> {
        self.health_tracker.clone()
    }

    /// Run the sync manager
    ///
    /// This spawns the bootstrap relay connection (if configured), sets up a 
    /// self-subscriber for event-driven relay discovery, and processes incoming
    /// events. The self-subscriber handles ALL relay discovery from announcements.
    /// Runs indefinitely until cancelled.
    ///
    /// ## Simplified Relay Discovery Architecture
    ///
    /// All relay discovery is centralized in the self-subscriber:
    /// - Bootstrap relay: connected immediately (no jitter, single relay)
    /// - All other relays: discovered via self-subscriber announcements (with jitter)
    /// - Jitter applied at point of discovery (not startup)
    ///
    /// This eliminates three redundant discovery paths:
    /// 1. DB query at startup (removed)
    /// 2. Remote event extraction (removed)  
    /// 3. Self-subscriber (sole discovery path)
    pub async fn run(self) {
        tracing::info!(
            "Starting SyncManager (domain: {}, own_relay: {}, bootstrap relay: {:?})",
            self.relay_domain,
            self.own_relay_url,
            self.bootstrap_relay_url
        );

        // Create the filter service
        let filter_service = Arc::new(FilterService::new(
            self.database.clone(),
            self.relay_domain.clone(),
        ));

        // Create channel for receiving events from all connections
        let (tx, mut rx) = mpsc::channel::<SyncedEvent>(100);

        // Track active relay URLs to avoid duplicates (wrapped in Arc for sharing)
        let active_relays = Arc::new(tokio::sync::Mutex::new(HashSet::<String>::new()));

        // Bootstrap relay - connect immediately (no jitter, just one relay)
        if let Some(ref url) = self.bootstrap_relay_url {
            if !self.is_own_relay(url) {
                tracing::info!("Connecting to bootstrap relay: {}", url);
                active_relays.lock().await.insert(url.clone());
                self.spawn_connection(url.clone(), tx.clone(), filter_service.clone(), false);
            } else {
                tracing::info!("Skipping bootstrap relay (is our own relay): {}", url);
            }
        }

        // Record initial tracked relay count
        if let Some(ref metrics) = self.metrics {
            let count = active_relays.lock().await.len();
            metrics.set_tracked_count(count as i64);
        }

        {
            let active = active_relays.lock().await;
            if active.is_empty() {
                tracing::info!(
                    "No bootstrap relay configured, waiting for announcements via self-subscriber..."
                );
            } else {
                tracing::info!(
                    "SyncManager connected to {} relay(s): {:?}",
                    active.len(),
                    *active
                );
            }
        }

        // Spawn self-subscriber task for ALL relay discovery
        let self_subscriber_handle = self.spawn_self_subscriber(
            tx.clone(),
            filter_service.clone(),
            active_relays.clone(),
        );

        // Process incoming events - just validate and store, NO relay discovery
        // (relay discovery is handled solely by the self-subscriber)
        while let Some(synced_event) = rx.recv().await {
            self.process_event(synced_event).await;
        }

        // Clean up self-subscriber
        self_subscriber_handle.abort();
        tracing::warn!("SyncManager event channel closed, shutting down");
    }

    /// Check if a URL points to our own relay
    fn is_own_relay(&self, url: &str) -> bool {
        url.contains(&self.relay_domain)
    }

    /// Spawn a self-subscriber task that connects to our own relay
    /// and watches for kind 30617 announcements to discover new relays.
    ///
    /// This is the SOLE relay discovery path - all relay discovery happens here.
    /// When a new announcement is saved to our database (from direct submission
    /// or synced from another relay), the self-subscriber receives it immediately
    /// and spawns connections to newly discovered relays (with jitter).
    fn spawn_self_subscriber(
        &self,
        tx: mpsc::Sender<SyncedEvent>,
        filter_service: Arc<FilterService>,
        active_relays: Arc<tokio::sync::Mutex<HashSet<String>>>,
    ) -> tokio::task::JoinHandle<()> {
        let own_relay_url = self.own_relay_url.clone();
        let relay_domain = self.relay_domain.clone();
        let metrics = self.metrics.clone();
        let health_tracker = self.health_tracker.clone();
        let startup_jitter_ms = self.startup_jitter_ms;

        tokio::spawn(async move {
            Self::run_self_subscriber_loop(
                own_relay_url,
                relay_domain,
                tx,
                filter_service,
                active_relays,
                metrics,
                health_tracker,
                startup_jitter_ms,
            )
            .await;
        })
    }

    /// Main loop for the self-subscriber
    ///
    /// Connects to our own relay, subscribes to kind 30617 announcements,
    /// and processes events to discover new relays. Handles reconnection
    /// on disconnect.
    ///
    /// ## Reconnection Behavior
    ///
    /// - First connection: no `since` filter (get all historical announcements)
    /// - Reconnections: use `since` filter (15 minutes ago) to avoid re-processing
    #[allow(clippy::too_many_arguments)]
    async fn run_self_subscriber_loop(
        own_relay_url: String,
        relay_domain: String,
        tx: mpsc::Sender<SyncedEvent>,
        filter_service: Arc<FilterService>,
        active_relays: Arc<tokio::sync::Mutex<HashSet<String>>>,
        metrics: Option<SyncMetrics>,
        health_tracker: Arc<RelayHealthTracker>,
        startup_jitter_ms: u64,
    ) {
        let mut reconnect_delay = Duration::from_secs(1);
        let max_reconnect_delay = Duration::from_secs(60);
        let mut is_first_connection = true;

        loop {
            tracing::info!(
                "Self-subscriber connecting to own relay: {}",
                own_relay_url
            );

            match Self::connect_self_subscriber(&own_relay_url).await {
                Ok(client) => {
                    // Reset reconnect delay on successful connection
                    reconnect_delay = Duration::from_secs(1);

                    tracing::info!(
                        "Self-subscriber connected to own relay, subscribing to kind {} announcements{}",
                        KIND_REPOSITORY_ANNOUNCEMENT,
                        if is_first_connection { " (initial, no since filter)" } else { " (reconnection, with since filter)" }
                    );

                    // Subscribe to repository announcements
                    // First connection: get all historical; reconnections: only last 15 minutes
                    let filter = if is_first_connection {
                        Filter::new().kind(Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT))
                    } else {
                        let since = Timestamp::now() - 15 * 60; // 15 minutes ago
                        Filter::new()
                            .kind(Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT))
                            .since(since)
                    };
                    
                    is_first_connection = false;
                    
                    if let Err(e) = client.subscribe(filter, None).await {
                        tracing::error!(
                            "Self-subscriber failed to subscribe on {}: {}",
                            own_relay_url,
                            e
                        );
                        // Will reconnect after delay
                    } else {
                        // Handle notifications until disconnect
                        Self::handle_self_subscriber_notifications(
                            &client,
                            &own_relay_url,
                            &relay_domain,
                            &tx,
                            &filter_service,
                            &active_relays,
                            &metrics,
                            &health_tracker,
                            startup_jitter_ms,
                        )
                        .await;
                    }

                    // Disconnect and cleanup
                    client.disconnect().await;
                }
                Err(e) => {
                    tracing::warn!(
                        "Self-subscriber failed to connect to {}: {}",
                        own_relay_url,
                        e
                    );
                }
            }

            // Wait before reconnecting with exponential backoff
            tracing::debug!(
                "Self-subscriber will reconnect to {} in {:?}",
                own_relay_url,
                reconnect_delay
            );
            tokio::time::sleep(reconnect_delay).await;
            reconnect_delay = std::cmp::min(reconnect_delay * 2, max_reconnect_delay);
        }
    }

    /// Connect to our own relay for self-subscribing
    async fn connect_self_subscriber(
        url: &str,
    ) -> Result<Client, Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::default();
        client.add_relay(url).await?;
        client.connect().await;

        // Wait for connection to establish (with timeout)
        let timeout = Duration::from_secs(10);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            let relays = client.relays().await;
            if relays.values().any(|r| r.is_connected()) {
                return Ok(client);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err("Timeout waiting for self-subscriber connection".into())
    }

    /// Handle notifications from the self-subscriber client
    ///
    /// Processes announcement events to discover new relay URLs.
    /// Applies jitter before spawning connections to prevent thundering herd.
    #[allow(clippy::too_many_arguments)]
    async fn handle_self_subscriber_notifications(
        client: &Client,
        own_relay_url: &str,
        relay_domain: &str,
        tx: &mpsc::Sender<SyncedEvent>,
        filter_service: &Arc<FilterService>,
        active_relays: &Arc<tokio::sync::Mutex<HashSet<String>>>,
        metrics: &Option<SyncMetrics>,
        health_tracker: &Arc<RelayHealthTracker>,
        startup_jitter_ms: u64,
    ) {
        let own_relay_url = own_relay_url.to_string();
        let relay_domain = relay_domain.to_string();
        let filter_service = filter_service.clone();
        let active_relays = active_relays.clone();
        let metrics = metrics.clone();
        let health_tracker = health_tracker.clone();
        let tx = tx.clone();

        client
            .handle_notifications(|notification| {
                let own_relay_url = own_relay_url.clone();
                let relay_domain = relay_domain.clone();
                let filter_service = filter_service.clone();
                let active_relays = active_relays.clone();
                let metrics = metrics.clone();
                let health_tracker = health_tracker.clone();
                let tx = tx.clone();

                async move {
                    match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            // Only process repository announcement events
                            if event.kind.as_u16() != KIND_REPOSITORY_ANNOUNCEMENT {
                                return Ok(false);
                            }

                            tracing::debug!(
                                "Self-subscriber received announcement {} from {}",
                                event.id,
                                own_relay_url
                            );

                            // Extract relay URLs from the announcement
                            let new_urls = filter_service.extract_relay_urls_from_event(&event);

                            for url in new_urls {
                                // Check if we should connect to this relay
                                let should_connect = {
                                    let mut active = active_relays.lock().await;
                                    let is_new = !active.contains(&url);
                                    let is_not_self = !url.contains(&relay_domain);

                                    if is_new && is_not_self {
                                        active.insert(url.clone());
                                        true
                                    } else {
                                        false
                                    }
                                };

                                if should_connect {
                                    tracing::info!(
                                        "Self-subscriber discovered new relay from announcement {}, scheduling connection: {}",
                                        event.id,
                                        url
                                    );

                                    // Update tracked relay count
                                    if let Some(ref m) = metrics {
                                        m.inc_tracked_count();
                                    }

                                    // Spawn connection to the new relay WITH jitter at point of discovery
                                    let url_clone = url.clone();
                                    let tx_clone = tx.clone();
                                    let filter_service_clone = filter_service.clone();
                                    let domain_clone = relay_domain.clone();
                                    let health_tracker_clone = health_tracker.clone();
                                    let metrics_clone = metrics.clone();

                                    tokio::spawn(async move {
                                        // Apply jitter at point of discovery
                                        if startup_jitter_ms > 0 {
                                            let jitter_ms = rand::thread_rng().gen_range(0..startup_jitter_ms);
                                            tracing::debug!(
                                                "Applying {}ms jitter before connecting to discovered relay {}",
                                                jitter_ms,
                                                url_clone
                                            );
                                            tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
                                        }
                                        
                                        connect_with_retry(
                                            &url_clone,
                                            tx_clone,
                                            filter_service_clone,
                                            &domain_clone,
                                            health_tracker_clone,
                                            metrics_clone,
                                        )
                                        .await;
                                    });
                                }
                            }

                            Ok(false) // Continue processing
                        }
                        RelayPoolNotification::Shutdown => {
                            tracing::warn!(
                                "Self-subscriber connection shutdown for {}",
                                own_relay_url
                            );
                            Ok(true) // Stop on shutdown
                        }
                        RelayPoolNotification::Message { .. } => {
                            Ok(false) // Continue processing
                        }
                    }
                }
            })
            .await
            .ok();
    }

    /// Spawn a connection task for a relay
    ///
    /// # Arguments
    /// * `url` - Relay URL to connect to
    /// * `tx` - Channel sender for synced events
    /// * `filter_service` - Filter service for subscriptions
    /// * `apply_jitter` - Whether to apply startup jitter before connecting
    fn spawn_connection(
        &self,
        url: String,
        tx: mpsc::Sender<SyncedEvent>,
        filter_service: Arc<FilterService>,
        apply_jitter: bool,
    ) {
        let domain = self.relay_domain.clone();
        let health_tracker = self.health_tracker.clone();
        let metrics = self.metrics.clone();
        let max_jitter = self.startup_jitter_ms;

        tokio::spawn(async move {
            // Apply startup jitter if requested
            if apply_jitter && max_jitter > 0 {
                let jitter_ms = rand::thread_rng().gen_range(0..max_jitter);
                tracing::debug!(
                    "Applying {}ms jitter before connecting to {}",
                    jitter_ms,
                    url
                );
                tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
            }

            connect_with_retry(&url, tx, filter_service, &domain, health_tracker, metrics).await;
        });
    }

    /// Process a single synced event
    ///
    /// Events are validated through the write policy and stored if accepted.
    /// Dynamic subscription updates are handled by each connection's SubscriptionManager.
    async fn process_event(&self, synced_event: SyncedEvent) {
        let event = &synced_event.event;
        let event_id = event.id.to_hex();
        let kind = event.kind.as_u16();

        tracing::debug!(
            "Processing synced event {} (kind {}) from {}",
            event_id,
            kind,
            synced_event.source_url
        );

        // Log subscription-relevant events for debugging
        match kind {
            30617 | 30618 => {
                tracing::debug!(
                    "Received announcement {} - connection will add Layer 2 subscription",
                    event_id
                );
            }
            1617 | 1618 | 1619 | 1621 | 1622 => {
                tracing::debug!(
                    "Received PR/Issue {} - connection will add Layer 3 subscription",
                    event_id
                );
            }
            _ => {}
        }

        // Validate through write policy using sync_source_addr derived from config
        let result = self
            .write_policy
            .admit_event(event, &self.sync_source_addr)
            .await;

        match result {
            PolicyResult::Accept => {
                tracing::info!(
                    "Synced event {} (kind {}) accepted, storing",
                    event_id,
                    event.kind.as_u16()
                );

                // Store the event in the database
                if let Err(e) = self.database.save_event(event).await {
                    tracing::error!("Failed to store synced event {}: {}", event_id, e);
                } else {
                    tracing::debug!("Synced event {} stored successfully", event_id);
                }
            }
            PolicyResult::Reject(reason) => {
                tracing::info!(
                    "Synced event {} (kind {}) rejected: {}",
                    event_id,
                    event.kind.as_u16(),
                    reason
                );
            }
        }
    }
}

/// Extract domain from a WebSocket URL
///
/// Examples:
/// - "ws://127.0.0.1:8080" -> "127.0.0.1:8080"
/// - "wss://relay.example.com" -> "relay.example.com"
fn extract_domain_from_url(url: &str) -> Option<String> {
    let url = url
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");
    let url = url
        .trim_start_matches("http://")
        .trim_start_matches("https://");

    // Remove path
    let domain = url.split('/').next()?;

    Some(domain.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain_ws() {
        assert_eq!(
            extract_domain_from_url("ws://127.0.0.1:8080"),
            Some("127.0.0.1:8080".to_string())
        );
    }

    #[test]
    fn test_extract_domain_wss() {
        assert_eq!(
            extract_domain_from_url("wss://relay.example.com"),
            Some("relay.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_domain_with_path() {
        assert_eq!(
            extract_domain_from_url("ws://example.com/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_domain_http() {
        assert_eq!(
            extract_domain_from_url("http://example.com:3000"),
            Some("example.com:3000".to_string())
        );
    }

    #[test]
    fn test_derive_own_relay_url() {
        assert_eq!(
            derive_own_relay_url("127.0.0.1:8080"),
            "ws://127.0.0.1:8080".to_string()
        );
    }
}