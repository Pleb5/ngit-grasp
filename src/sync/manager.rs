//! SyncManager - Coordinates proactive sync operations
//!
//! The SyncManager discovers relays from stored announcements, spawns connections
//! to each relay, receives events, validates them through the write policy,
//! and stores accepted events.
//!
//! ## Phase 2 Features
//!
//! - Relay discovery from stored kind 30617 announcements
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
use rand::Rng;
use tokio::sync::mpsc;

use super::connection::{connect_with_retry, SyncedEvent};
use super::filter::FilterService;
use super::health::RelayHealthTracker;
use super::metrics::SyncMetrics;
use crate::config::Config;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};


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

/// Coordinates proactive sync from configured and discovered relays
pub struct SyncManager {
    /// Bootstrap relay URL for initial sync (from config)
    /// Additional relays are discovered from repository announcements that list our service
    bootstrap_relay_url: Option<String>,
    /// Our relay's domain (for filtering)
    relay_domain: String,
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
        Self {
            bootstrap_relay_url,
            relay_domain,
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
        Self {
            bootstrap_relay_url,
            relay_domain,
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
        Self {
            bootstrap_relay_url: Some(bootstrap_url),
            relay_domain,
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
    /// This discovers relays from stored announcements, spawns connection tasks,
    /// and processes incoming events. Runs indefinitely until cancelled.
    pub async fn run(self) {
        tracing::info!(
            "Starting SyncManager (domain: {}, bootstrap relay: {:?})",
            self.relay_domain,
            self.bootstrap_relay_url
        );

        // Create the filter service
        let filter_service = Arc::new(FilterService::new(
            self.database.clone(),
            self.relay_domain.clone(),
        ));

        // Create channel for receiving events from all connections
        let (tx, mut rx) = mpsc::channel::<SyncedEvent>(100);

        // Track active relay URLs to avoid duplicates
        let mut active_relays: HashSet<String> = HashSet::new();

        // Collect all relays to connect to
        let mut relays_to_connect: Vec<String> = Vec::new();

        // Start with bootstrap relay if configured
        if let Some(ref url) = self.bootstrap_relay_url {
            if !self.is_own_relay(url) {
                relays_to_connect.push(url.clone());
                active_relays.insert(url.clone());
            } else {
                tracing::info!("Skipping bootstrap relay (is our own relay): {}", url);
            }
        }

        // Discover additional relays from stored announcements
        let discovered_urls = filter_service.discover_relay_urls().await;
        for url in discovered_urls {
            if !active_relays.contains(&url) && !self.is_own_relay(&url) {
                relays_to_connect.push(url.clone());
                active_relays.insert(url.clone());
            }
        }

        // Record initial tracked relay count
        if let Some(ref metrics) = self.metrics {
            metrics.set_tracked_count(active_relays.len() as i64);
        }

        // Spawn connections with startup jitter to prevent thundering herd
        for url in relays_to_connect {
            tracing::info!("Scheduling connection to sync relay: {}", url);
            self.spawn_connection_with_jitter(url, tx.clone(), filter_service.clone());
        }

        if active_relays.is_empty() {
            tracing::warn!("No sync relays configured or discovered, SyncManager idle");
        } else {
            tracing::info!(
                "SyncManager connected to {} relays: {:?}",
                active_relays.len(),
                active_relays
            );
        }

        // Process incoming events from all connections
        while let Some(synced_event) = rx.recv().await {
            // Check if this event reveals new relays to sync from
            let new_urls = filter_service.extract_relay_urls_from_event(&synced_event.event);
            for url in new_urls {
                if !active_relays.contains(&url) && !self.is_own_relay(&url) {
                    tracing::info!("Discovered new relay from event, connecting: {}", url);
                    active_relays.insert(url.clone());
                    
                    // Update tracked relay count
                    if let Some(ref metrics) = self.metrics {
                        metrics.inc_tracked_count();
                    }
                    
                    // New relays discovered during runtime don't need jitter
                    self.spawn_connection(url, tx.clone(), filter_service.clone());
                }
            }

            self.process_event(synced_event).await;
        }

        tracing::warn!("SyncManager event channel closed, shutting down");
    }

    /// Check if a URL points to our own relay
    fn is_own_relay(&self, url: &str) -> bool {
        url.contains(&self.relay_domain)
    }

    /// Spawn a connection task for a relay with startup jitter
    ///
    /// Adds a random delay (0 to startup_jitter_ms) before connecting to prevent
    /// thundering herd on startup when multiple relays are configured.
    /// Set startup_jitter_ms to 0 to disable jitter (useful for testing).
    fn spawn_connection_with_jitter(
        &self,
        url: String,
        tx: mpsc::Sender<SyncedEvent>,
        filter_service: Arc<FilterService>,
    ) {
        let domain = self.relay_domain.clone();
        let health_tracker = self.health_tracker.clone();
        let metrics = self.metrics.clone();
        let max_jitter = self.startup_jitter_ms;

        tokio::spawn(async move {
            // Apply startup jitter (if configured)
            if max_jitter > 0 {
                let jitter_ms = rand::thread_rng().gen_range(0..max_jitter);
                tracing::debug!(
                    "Applying {}ms startup jitter before connecting to {}",
                    jitter_ms,
                    url
                );
                tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
            }

            connect_with_retry(&url, tx, filter_service, &domain, health_tracker, metrics).await;
        });
    }

    /// Spawn a connection task for a relay without jitter
    ///
    /// Used for relays discovered during runtime (not at startup).
    fn spawn_connection(
        &self,
        url: String,
        tx: mpsc::Sender<SyncedEvent>,
        filter_service: Arc<FilterService>,
    ) {
        let domain = self.relay_domain.clone();
        let health_tracker = self.health_tracker.clone();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
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
        let result = self.write_policy.admit_event(event, &self.sync_source_addr).await;

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
    let url = url.trim_start_matches("ws://").trim_start_matches("wss://");
    let url = url.trim_start_matches("http://").trim_start_matches("https://");
    
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
}