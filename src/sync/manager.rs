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

use std::collections::HashSet;
use std::sync::Arc;

use nostr_relay_builder::prelude::*;
use tokio::sync::mpsc;

use super::connection::{connect_with_retry, SyncedEvent};
use super::filter::FilterService;
use super::SYNC_SOURCE_ADDR;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};

/// Coordinates proactive sync from configured and discovered relays
pub struct SyncManager {
    /// Initial relay URL to sync from (from config)
    initial_relay_url: Option<String>,
    /// Our relay's domain (for filtering)
    relay_domain: String,
    /// Database for storing accepted events
    database: SharedDatabase,
    /// Write policy for validating events
    write_policy: Nip34WritePolicy,
}

impl SyncManager {
    /// Create a new SyncManager
    ///
    /// # Arguments
    /// * `initial_relay_url` - Optional initial relay URL from config
    /// * `relay_domain` - Our relay's domain (used to exclude self from sync)
    /// * `database` - Shared database for storing events and querying announcements
    /// * `write_policy` - Write policy for validating synced events
    pub fn new(
        initial_relay_url: Option<String>,
        relay_domain: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
    ) -> Self {
        Self {
            initial_relay_url,
            relay_domain,
            database,
            write_policy,
        }
    }

    /// Create a SyncManager with a single relay URL (Phase 1 compatibility)
    pub fn with_single_relay(
        sync_relay_url: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
    ) -> Self {
        // Extract domain from URL for filtering
        let relay_domain = extract_domain_from_url(&sync_relay_url).unwrap_or_default();
        Self {
            initial_relay_url: Some(sync_relay_url),
            relay_domain,
            database,
            write_policy,
        }
    }

    /// Run the sync manager
    ///
    /// This discovers relays from stored announcements, spawns connection tasks,
    /// and processes incoming events. Runs indefinitely until cancelled.
    pub async fn run(self) {
        tracing::info!(
            "Starting SyncManager (domain: {}, initial relay: {:?})",
            self.relay_domain,
            self.initial_relay_url
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

        // Start with initial relay if configured
        if let Some(ref url) = self.initial_relay_url {
            if !self.is_own_relay(url) {
                tracing::info!("Connecting to initial sync relay: {}", url);
                active_relays.insert(url.clone());
                self.spawn_connection(url.clone(), tx.clone(), filter_service.clone());
            } else {
                tracing::info!("Skipping initial relay (is our own relay): {}", url);
            }
        }

        // Discover additional relays from stored announcements
        let discovered_urls = filter_service.discover_relay_urls().await;
        for url in discovered_urls {
            if !active_relays.contains(&url) && !self.is_own_relay(&url) {
                tracing::info!("Connecting to discovered relay: {}", url);
                active_relays.insert(url.clone());
                self.spawn_connection(url, tx.clone(), filter_service.clone());
            }
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

    /// Spawn a connection task for a relay
    fn spawn_connection(
        &self,
        url: String,
        tx: mpsc::Sender<SyncedEvent>,
        filter_service: Arc<FilterService>,
    ) {
        let domain = self.relay_domain.clone();
        tokio::spawn(async move {
            connect_with_retry(&url, tx, filter_service, &domain).await;
        });
    }

    /// Process a single synced event
    async fn process_event(&self, synced_event: SyncedEvent) {
        let event = &synced_event.event;
        let event_id = event.id.to_hex();

        tracing::debug!(
            "Processing synced event {} (kind {}) from {}",
            event_id,
            event.kind.as_u16(),
            synced_event.source_url
        );

        // Validate through write policy using SYNC_SOURCE_ADDR
        let result = self.write_policy.admit_event(event, &SYNC_SOURCE_ADDR).await;

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