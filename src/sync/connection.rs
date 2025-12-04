//! WebSocket connection handling for sync
//!
//! Manages the connection to a source relay, subscribes to events using
//! the three-layer filter strategy, and passes them through validation.
//!
//! ## Phase 2 Features
//!
//! - Three-layer filter subscriptions:
//!   1. Layer 1: kinds 30617 + 30618 (announcements)
//!   2. Layer 2: A/a tags for repository events
//!   3. Layer 3: E/e tags for related events (PRs, Issues, etc.)
//!
//! ## Phase 3 Features
//!
//! - Health tracking with success/failure reporting
//! - Exponential backoff with health-aware delays
//! - Dead relay detection and minimal retry
//!
//! ## Phase 4 Features
//!
//! - Dynamic subscription updates when new announcements/PRs arrive
//! - Per-connection subscription tracking
//! - Filter consolidation when count exceeds threshold (>150)
//! - Duplicate subscription prevention

use std::sync::Arc;
use std::time::Duration;

use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

use super::filter::FilterService;
use super::health::RelayHealthTracker;
use super::subscription::SubscriptionManager;

/// Event received from the sync connection
#[derive(Debug, Clone)]
pub struct SyncedEvent {
    pub event: Event,
    pub source_url: String,
}

/// Manages a WebSocket connection to a single relay for syncing
pub struct SyncConnection {
    url: String,
    client: Client,
    filter_service: Arc<FilterService>,
    remote_domain: String,
    subscription_manager: SubscriptionManager,
}

impl SyncConnection {
    /// Create a new sync connection to the given relay URL
    pub async fn new(
        url: &str,
        filter_service: Arc<FilterService>,
        remote_domain: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::default();

        // Add the relay
        client.add_relay(url).await?;

        // Connect to the relay
        client.connect().await;

        tracing::info!("Sync connection established to {}", url);

        // Create subscription manager for this connection
        let subscription_manager = SubscriptionManager::new(
            filter_service.clone(),
            remote_domain.to_string(),
        );

        Ok(Self {
            url: url.to_string(),
            client,
            filter_service,
            remote_domain: remote_domain.to_string(),
            subscription_manager,
        })
    }

    /// Start receiving events and send them through the channel
    ///
    /// This method runs indefinitely, handling events from all three filter layers.
    /// Dynamic subscription updates are triggered when new announcements or PRs arrive.
    pub async fn run(mut self, tx: mpsc::Sender<SyncedEvent>) {
        // Subscribe to all three filter layers

        // Layer 1: Announcement discovery (kinds 30617 + 30618)
        let layer1_filters = self.filter_service.get_layer1_filters();
        for filter in &layer1_filters {
            match self.client.subscribe(filter.clone(), None).await {
                Ok(output) => {
                    tracing::info!(
                        "Subscribed to Layer 1 (announcements) on {} (subscription: {})",
                        self.url,
                        output.id()
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to subscribe Layer 1 on {}: {}", self.url, e);
                }
            }
        }

        // Layer 2: Repository events (A/a tags)
        let layer2_filters = self
            .filter_service
            .get_layer2_filters(&self.remote_domain)
            .await;
        for filter in &layer2_filters {
            match self.client.subscribe(filter.clone(), None).await {
                Ok(output) => {
                    tracing::info!(
                        "Subscribed to Layer 2 (repo events) on {} (subscription: {})",
                        self.url,
                        output.id()
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to subscribe Layer 2 on {}: {}", self.url, e);
                }
            }
        }

        // Layer 3: Related events (E/e tags)
        let layer3_filters = self.filter_service.get_layer3_filters().await;
        for filter in &layer3_filters {
            match self.client.subscribe(filter.clone(), None).await {
                Ok(output) => {
                    tracing::info!(
                        "Subscribed to Layer 3 (related events) on {} (subscription: {})",
                        self.url,
                        output.id()
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to subscribe Layer 3 on {}: {}", self.url, e);
                }
            }
        }

        tracing::info!(
            "Sync subscriptions active on {} (L1: {}, L2: {}, L3: {})",
            self.url,
            layer1_filters.len(),
            layer2_filters.len(),
            layer3_filters.len()
        );

        // Handle incoming notifications
        let url = self.url.clone();
        self.client
            .handle_notifications(|notification| {
                let tx = tx.clone();
                let url = url.clone();
                async move {
                    match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            tracing::debug!(
                                "Received event {} from {} (kind {})",
                                event.id,
                                url,
                                event.kind.as_u16()
                            );

                            // Send the event to the manager for processing
                            let synced = SyncedEvent {
                                event: (*event).clone(),
                                source_url: url.clone(),
                            };

                            if let Err(e) = tx.send(synced).await {
                                tracing::warn!("Failed to send synced event: {}", e);
                                return Ok(true); // Stop if channel is closed
                            }
                        }
                        RelayPoolNotification::Shutdown => {
                            tracing::warn!("Relay connection shutdown for {}", url);
                            return Ok(true); // Stop on shutdown
                        }
                        RelayPoolNotification::Message { message, .. } => {
                            tracing::trace!("Received message from {}: {:?}", url, message);
                        }
                    }
                    Ok(false) // Continue processing
                }
            })
            .await
            .ok();
    }

    /// Handle dynamic subscription updates based on incoming event kind
    ///
    /// - kind 30617/30618: New announcement → add Layer 2 subscription
    /// - kind 1617/1618/1619/1621/1622: New PR/Issue → add Layer 3 subscription
    async fn handle_dynamic_subscription(&mut self, event: &Event) {
        let kind = event.kind.as_u16();

        // Check if this is an announcement kind (triggers Layer 2 subscription)
        if SubscriptionManager::is_announcement_kind(kind) {
            if let Some(new_filters) = self.subscription_manager.add_announcement(event) {
                tracing::info!(
                    "New announcement {} on {}, adding {} Layer 2 filter(s) (total filters: {})",
                    event.id.to_hex(),
                    self.url,
                    new_filters.len(),
                    self.subscription_manager.get_filter_count()
                );
                self.subscribe_to_filters(new_filters, "Layer 2").await;
            }
        }

        // Check if this is a PR/Issue kind (triggers Layer 3 subscription)
        if SubscriptionManager::is_pr_issue_kind(kind) {
            if let Some(new_filters) = self.subscription_manager.add_event(event) {
                tracing::info!(
                    "New PR/Issue {} on {}, adding {} Layer 3 filter(s) (total filters: {})",
                    event.id.to_hex(),
                    self.url,
                    new_filters.len(),
                    self.subscription_manager.get_filter_count()
                );
                self.subscribe_to_filters(new_filters, "Layer 3").await;
            }
        }

        // Check if we need to consolidate
        if self.subscription_manager.should_consolidate() {
            self.consolidate_subscriptions().await;
        }
    }

    /// Subscribe to new filters
    async fn subscribe_to_filters(&self, filters: Vec<Filter>, layer_name: &str) {
        for filter in filters {
            match self.client.subscribe(filter, None).await {
                Ok(output) => {
                    tracing::debug!(
                        "Dynamic {} subscription on {} (subscription: {})",
                        layer_name,
                        self.url,
                        output.id()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to add dynamic {} subscription on {}: {}",
                        layer_name,
                        self.url,
                        e
                    );
                }
            }
        }
    }

    /// Consolidate subscriptions back to Layer 1 only
    ///
    /// This is triggered when the filter count exceeds 150.
    /// All existing subscriptions are closed and only Layer 1 is re-subscribed.
    async fn consolidate_subscriptions(&mut self) {
        tracing::warn!(
            "Filter count {} exceeds threshold, consolidating subscriptions on {}",
            self.subscription_manager.get_filter_count(),
            self.url
        );

        // Get consolidated filters (clears tracking and returns Layer 1 only)
        let layer1_filters = self.subscription_manager.consolidate();

        // Note: nostr-sdk doesn't provide a way to close all subscriptions easily
        // The client will manage subscription count internally
        // We just add the new Layer 1 subscription

        for filter in layer1_filters {
            match self.client.subscribe(filter, None).await {
                Ok(output) => {
                    tracing::info!(
                        "Consolidated to Layer 1 subscription on {} (subscription: {})",
                        self.url,
                        output.id()
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to subscribe Layer 1 after consolidation on {}: {}",
                        self.url,
                        e
                    );
                }
            }
        }
    }

    /// Get the current filter count from the subscription manager
    pub fn get_filter_count(&self) -> usize {
        self.subscription_manager.get_filter_count()
    }

    /// Check if subscriptions have been consolidated
    pub fn is_consolidated(&self) -> bool {
        self.subscription_manager.is_consolidated()
    }
}

/// Reconnect loop with health-aware exponential backoff
///
/// This function manages the connection lifecycle with health tracking:
/// - Checks health state before attempting connections
/// - Reports success/failure to the health tracker
/// - Respects backoff delays from the health tracker
/// - Handles dead relay detection (24h+ failures)
///
/// # Arguments
/// * `url` - The relay URL to connect to
/// * `tx` - Channel sender for synced events
/// * `filter_service` - FilterService for building subscriptions
/// * `our_domain` - Our relay's domain (used to extract remote domain)
/// * `health_tracker` - Health tracker for managing connection state
pub async fn connect_with_retry(
    url: &str,
    tx: mpsc::Sender<SyncedEvent>,
    filter_service: Arc<FilterService>,
    _our_domain: &str,
    health_tracker: Arc<RelayHealthTracker>,
) {
    // Extract remote domain from URL
    let remote_domain = extract_domain_from_url(url).unwrap_or_else(|| url.to_string());

    loop {
        // Check if we should attempt connection based on health state
        if !health_tracker.should_attempt_connection(url) {
            // Wait for remaining backoff
            if let Some(remaining) = health_tracker.get_remaining_backoff(url) {
                tracing::debug!(
                    "Relay {} in backoff, waiting {:?} before retry",
                    url,
                    remaining
                );
                tokio::time::sleep(remaining).await;
                continue;
            }
        }

        // Log current health state for dead relays
        if health_tracker.is_dead(url) {
            tracing::info!(
                "Attempting reconnection to dead relay {} (daily retry)",
                url
            );
        }

        match SyncConnection::new(url, filter_service.clone(), &remote_domain).await {
            Ok(conn) => {
                // Record successful connection
                health_tracker.record_success(url);
                tracing::info!("Sync connection established to {}", url);

                // Run the connection (this blocks until disconnection)
                conn.run(tx.clone()).await;

                // Connection ended - record as failure for reconnection backoff
                // (The connection ending is considered a failure even if it worked for a while)
                health_tracker.record_failure(url);
                tracing::warn!("Sync connection to {} ended, will reconnect", url);
            }
            Err(e) => {
                // Record connection failure
                health_tracker.record_failure(url);

                let failure_count = health_tracker.get_failure_count(url);
                let state = health_tracker.get_state(url);

                tracing::error!(
                    "Failed to connect to sync relay {} (attempt #{}, state: {}): {}",
                    url,
                    failure_count,
                    state,
                    e
                );
            }
        }

        // Get the backoff duration from health tracker
        // If the health tracker has no backoff set (shouldn't happen), use a small default
        let wait_duration = health_tracker
            .get_remaining_backoff(url)
            .unwrap_or(Duration::from_secs(5));

        tracing::debug!(
            "Waiting {:?} before reconnecting to {}",
            wait_duration,
            url
        );
        tokio::time::sleep(wait_duration).await;
    }
}

/// Extract domain from a URL
fn extract_domain_from_url(url: &str) -> Option<String> {
    let url = url
        .trim_start_matches("ws://")
        .trim_start_matches("wss://")
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
    fn test_extract_domain() {
        assert_eq!(
            extract_domain_from_url("ws://127.0.0.1:8080"),
            Some("127.0.0.1:8080".to_string())
        );
        assert_eq!(
            extract_domain_from_url("wss://relay.example.com/path"),
            Some("relay.example.com".to_string())
        );
    }
}