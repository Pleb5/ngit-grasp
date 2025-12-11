//! Relay Connection Management for Proactive Sync
//!
//! This module provides relay connection management for external relay connections.
//! Each RelayConnection manages a single connection to an external relay and handles
//! subscriptions using the three-layer sync strategy.
//!
//! See `docs/explanation/grasp-02-proactive-sync-v4.md` for full design details.

use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

use super::filters::build_announcement_filter;

/// Events from a relay connection
#[derive(Debug)]
pub enum RelayEvent {
    /// A new event was received
    Event(Event),
    /// End of stored events for a subscription
    EndOfStoredEvents(SubscriptionId),
    /// Connection was closed
    Closed(String),
    /// Shutdown notification
    Shutdown,
}

/// Manages connection to a single external relay
///
/// RelayConnection wraps a nostr-sdk Client to manage a WebSocket connection
/// to an external relay. It handles:
/// - Connection establishment
/// - Layer 1 subscription (announcements)
/// - Additional filter subscriptions (Layers 2 & 3)
/// - Event notification loop
#[derive(Clone)]
pub struct RelayConnection {
    /// The relay URL this connection is for
    url: String,
    /// The underlying nostr-sdk client
    client: Client,
}

impl RelayConnection {
    /// Create a new relay connection (not yet connected)
    ///
    /// # Arguments
    /// * `url` - The relay URL to connect to (e.g., "wss://relay.example.com")
    pub fn new(url: String) -> Self {
        let client = Client::default();
        Self { url, client }
    }

    /// Connect to the relay and subscribe to Layer 1 (announcements)
    ///
    /// This method:
    /// 1. Adds the relay to the client
    /// 2. Establishes the WebSocket connection
    /// 3. Verifies connection was established
    /// 4. Subscribes to Layer 1 filter (kinds 30617 + 30618)
    ///
    /// # Arguments
    /// * `since` - Optional timestamp for incremental sync on reconnect
    /// * `connection_timeout_secs` - Timeout for the connection attempt in seconds.
    ///   Should be no larger than base_backoff_secs to ensure the connection attempt
    ///   completes before the next retry would be scheduled.
    ///
    /// # Returns
    /// * `Ok(SubscriptionId)` - The subscription ID on successful connection
    /// * `Err(String)` with error description on failure
    pub async fn connect_and_subscribe(
        &self,
        since: Option<Timestamp>,
        connection_timeout_secs: u64,
    ) -> Result<SubscriptionId, String> {
        // Add relay to client
        self.client
            .add_relay(&self.url)
            .await
            .map_err(|e| format!("Failed to add relay {}: {}", self.url, e))?;

        // Establish connection using try_connect_relay for immediate failure detection
        //
        // Key difference from client.connect():
        // - try_connect_relay: Single attempt with timeout, returns Err on failure,
        //   does NOT spawn background retry task (we control retries via HealthTracker)
        // - connect(): Spawns background task, returns immediately, auto-retries forever
        //
        // Using try_connect_relay gives us:
        // 1. Immediate error return on connection failure
        // 2. Configurable timeout (set to base_backoff_secs to ensure retry timing works)
        // 3. No conflicting retry logic (we use HealthTracker for backoff)
        // 4. Cleaner error messages for metrics recording
        //
        // See: nostr-sdk-0.44 Client::try_connect_relay documentation
        self.client
            .try_connect_relay(&self.url, std::time::Duration::from_secs(connection_timeout_secs))
            .await
            .map_err(|e| format!("Failed to connect to relay {}: {}", self.url, e))?;

        // Subscribe to Layer 1 (announcements)
        let filter = build_announcement_filter(since);
        let output = self
            .client
            .subscribe(filter, None)
            .await
            .map_err(|e| format!("Failed to subscribe to announcements on {}: {}", self.url, e))?;

        tracing::info!(url = %self.url, sub_id = %output.val, "Connected and subscribed to Layer 1 (announcements)");
        Ok(output.val)
    }

    /// Run the event loop, sending events through the provided channel
    ///
    /// This method blocks and processes notifications from the relay:
    /// - `RelayPoolNotification::Event` -> sends `RelayEvent::Event`
    /// - `RelayPoolNotification::Message` with EOSE -> sends `RelayEvent::EndOfStoredEvents`
    /// - `RelayPoolNotification::Shutdown` -> sends `RelayEvent::Shutdown`
    ///
    /// The loop terminates when:
    /// - The sender channel is closed (receiver dropped)
    /// - A shutdown notification is received
    /// - An error occurs receiving notifications
    ///
    /// # Arguments
    /// * `event_sender` - Channel to send relay events through
    pub async fn run_event_loop(self, event_sender: mpsc::Sender<RelayEvent>) {
        use std::time::Duration;
        use tokio::time::interval;
        
        let mut notifications = self.client.notifications();
        let url = self.url.clone();
        
        // Check connection status every second to detect dead connections
        let mut check_interval = interval(Duration::from_secs(1));

        tracing::debug!(relay = %url, "Starting event loop");

        loop {
            tokio::select! {
                // Check for new notifications
                notification_result = notifications.recv() => {
                    match notification_result {
                        Ok(notification) => {
                            match notification {
                                RelayPoolNotification::Event { event, .. } => {
                                    tracing::trace!(relay = %url, event_id = %event.id, "Received event");
                                    if event_sender.send(RelayEvent::Event(*event)).await.is_err() {
                                        tracing::debug!(relay = %url, "Event sender closed, stopping event loop");
                                        break;
                                    }
                                }
                                RelayPoolNotification::Message { message, .. } => {
                                    match message {
                                        RelayMessage::EndOfStoredEvents(sub_id) => {
                                            tracing::debug!(relay = %url, sub_id = ?sub_id, "Received EOSE");
                                            // Convert Cow<SubscriptionId> to owned SubscriptionId
                                            let owned_sub_id = sub_id.into_owned();
                                            if event_sender
                                                .send(RelayEvent::EndOfStoredEvents(owned_sub_id))
                                                .await
                                                .is_err()
                                            {
                                                tracing::debug!(relay = %url, "Event sender closed, stopping event loop");
                                                break;
                                            }
                                        }
                                        RelayMessage::Closed { message: msg, .. } => {
                                            tracing::info!(relay = %url, message = %msg, "Relay closed subscription");
                                            let _ = event_sender
                                                .send(RelayEvent::Closed(msg.to_string()))
                                                .await;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                RelayPoolNotification::Shutdown => {
                                    tracing::info!(relay = %url, "Relay pool shutdown");
                                    let _ = event_sender.send(RelayEvent::Shutdown).await;
                                    break;
                                }
                            }
                        }
                        Err(_) => {
                            // Notification channel closed - connection lost
                            tracing::debug!(relay = %url, "Notification channel error, stopping event loop");
                            break;
                        }
                    }
                }
                // Periodic connection health check
                _ = check_interval.tick() => {
                    // Check if relay is still connected via nostr-sdk
                    if let Ok(relay) = self.client.relay(&self.url).await {
                        if !relay.is_connected() {
                            tracing::info!(relay = %url, "Relay disconnected (detected by health check)");
                            break;
                        }
                    } else {
                        // Relay not found in client - must be disconnected
                        tracing::info!(relay = %url, "Relay not found (detected by health check)");
                        break;
                    }
                }
            }
        }

        tracing::debug!(relay = %url, "Event loop terminated");
    }

    /// Add additional filter subscription (for Layer 2 + 3)
    ///
    /// Use this to subscribe to:
    /// - Layer 2: Events tagging our repos (a/A/q tags)
    /// - Layer 3: Events tagging our root events (e/E/q tags)
    ///
    /// # Arguments
    /// * `filter` - The filter to subscribe to
    ///
    /// # Returns
    /// * `Ok(SubscriptionId)` - The subscription ID on success
    /// * `Err(String)` - Error description on failure
    pub async fn subscribe_filter(&self, filter: Filter) -> Result<SubscriptionId, String> {
        let output = self
            .client
            .subscribe(filter, None)
            .await
            .map_err(|e| format!("Failed to subscribe on {}: {}", self.url, e))?;
        Ok(output.val)
    }

    /// Subscribe to multiple filters at once
    ///
    /// Each filter creates its own subscription. Returns when all subscriptions
    /// are established. This is useful for Layer 2 + 3 filters together.
    ///
    /// # Arguments
    /// * `filters` - Vec of filters to subscribe to
    ///
    /// # Returns
    /// * `Ok(Vec<SubscriptionId>)` - The subscription IDs on success
    /// * `Err(String)` - Error description on failure
    pub async fn subscribe_filters(
        &self,
        filters: Vec<Filter>,
    ) -> Result<Vec<SubscriptionId>, String> {
        if filters.is_empty() {
            return Ok(vec![]);
        }

        let mut sub_ids = Vec::with_capacity(filters.len());
        for filter in filters {
            let output = self
                .client
                .subscribe(filter, None)
                .await
                .map_err(|e| format!("Failed to subscribe on {}: {}", self.url, e))?;
            sub_ids.push(output.val);
        }
        Ok(sub_ids)
    }

    /// Get the relay URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Disconnect from the relay
    pub async fn disconnect(&self) {
        self.client.disconnect().await;
        tracing::debug!(relay = %self.url, "Disconnected from relay");
    }

    /// Unsubscribe from all active subscriptions
    ///
    /// Used during consolidation to reset all subscriptions before rebuilding
    /// with consolidated filters. This sends CLOSE messages for all active
    /// subscriptions on the relay.
    pub async fn unsubscribe_all(&self) {
        self.client.unsubscribe_all().await;
        tracing::debug!(relay = %self.url, "Unsubscribed from all subscriptions");
    }
}