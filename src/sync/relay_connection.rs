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
    /// 3. Subscribes to Layer 1 filter (kinds 30617 + 30618)
    ///
    /// # Arguments
    /// * `since` - Optional timestamp for incremental sync on reconnect
    ///
    /// # Returns
    /// * `Ok(SubscriptionId)` - The subscription ID on successful connection
    /// * `Err(String)` with error description on failure
    pub async fn connect_and_subscribe(
        &self,
        since: Option<Timestamp>,
    ) -> Result<SubscriptionId, String> {
        // Add relay to client
        self.client
            .add_relay(&self.url)
            .await
            .map_err(|e| format!("Failed to add relay {}: {}", self.url, e))?;

        // Establish connection
        self.client.connect().await;

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
        let mut notifications = self.client.notifications();
        let url = self.url.clone();

        tracing::debug!(relay = %url, "Starting event loop");

        while let Ok(notification) = notifications.recv().await {
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
}