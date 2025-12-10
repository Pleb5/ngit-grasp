//! Relay Connection for Proactive Sync
//!
//! This module handles connecting to external relays and receiving events
//! for the proactive sync system.

use std::time::Duration;

use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

use crate::nostr::events::{KIND_REPOSITORY_ANNOUNCEMENT, KIND_REPOSITORY_STATE};

/// Events received from a relay connection
#[derive(Debug)]
pub enum RelayEvent {
    /// A nostr event was received
    Event(Event),
    /// End of stored events (EOSE) received
    EndOfStoredEvents,
    /// Connection was closed
    Closed(String),
}

/// Connection to an external relay for syncing events.
///
/// RelayConnection handles:
/// - Connecting to the relay
/// - Subscribing with appropriate filters (Layer 1 for bootstrap)
/// - Receiving events and sending them through a channel
pub struct RelayConnection {
    /// The relay URL
    url: String,
    /// The nostr-sdk client
    client: Client,
}

impl RelayConnection {
    /// Create a new relay connection.
    ///
    /// # Arguments
    ///
    /// * `url` - The WebSocket URL of the relay to connect to
    pub fn new(url: String) -> Self {
        // Create a client with generated keys (we're just subscribing, not publishing)
        let keys = Keys::generate();
        let client = Client::new(keys);

        Self { url, client }
    }

    /// Connect to the relay and subscribe with Layer 1 filter.
    ///
    /// Layer 1 filter syncs announcement events (30617, 30618) which are
    /// the foundation for discovering repository relationships.
    ///
    /// Returns the notification stream for receiving events.
    pub async fn connect_and_subscribe(&self) -> Result<(), String> {
        // Add the relay
        self.client
            .add_relay(&self.url)
            .await
            .map_err(|e| format!("Failed to add relay {}: {}", self.url, e))?;

        // Connect to relay
        self.client.connect().await;

        // Wait for connection to establish
        let mut connected = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let relays = self.client.relays().await;
            if relays.values().any(|r| r.is_connected()) {
                connected = true;
                break;
            }
        }

        if !connected {
            return Err(format!(
                "Failed to connect to relay {} after 3 seconds",
                self.url
            ));
        }

        tracing::info!("Connected to bootstrap relay: {}", self.url);

        // Layer 1 filter: Repository announcements and state events
        // These are addressable events that define repositories
        let filter = Filter::new().kinds([
            Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT), // 30617
            Kind::Custom(KIND_REPOSITORY_STATE),        // 30618
        ]);

        // Subscribe to the filter
        self.client
            .subscribe(filter, None)
            .await
            .map_err(|e| format!("Failed to subscribe: {}", e))?;

        tracing::debug!(
            "Subscribed to Layer 1 events (kinds 30617, 30618) from {}",
            self.url
        );

        Ok(())
    }

    /// Run the event loop, sending received events through the channel.
    ///
    /// This method runs until the connection is closed or an error occurs.
    ///
    /// # Arguments
    ///
    /// * `event_sender` - Channel to send received events
    pub async fn run_event_loop(self, event_sender: mpsc::Sender<RelayEvent>) {
        tracing::debug!("Starting event loop for relay: {}", self.url);

        // Handle notifications
        self.client
            .handle_notifications(|notification| async {
                match notification {
                    RelayPoolNotification::Event { event, .. } => {
                        tracing::debug!(
                            "Received event {} (kind {}) from {}",
                            event.id,
                            event.kind.as_u16(),
                            self.url
                        );
                        if event_sender.send(RelayEvent::Event(*event)).await.is_err() {
                            tracing::warn!("Event channel closed, stopping relay connection");
                            return Ok(true); // Stop handling
                        }
                    }
                    RelayPoolNotification::Message { message, .. } => {
                        if let RelayMessage::EndOfStoredEvents(_) = message {
                            tracing::debug!("EOSE received from {}", self.url);
                            if event_sender
                                .send(RelayEvent::EndOfStoredEvents)
                                .await
                                .is_err()
                            {
                                return Ok(true); // Stop handling
                            }
                        }
                    }
                    RelayPoolNotification::Shutdown => {
                        tracing::info!("Relay {} shutting down", self.url);
                        let _ = event_sender
                            .send(RelayEvent::Closed("Shutdown".to_string()))
                            .await;
                        return Ok(true); // Stop handling
                    }
                }
                Ok(false) // Continue handling
            })
            .await
            .ok(); // Ignore errors on shutdown

        // Disconnect when done
        self.client.disconnect().await;
        tracing::info!("Disconnected from relay: {}", self.url);
    }

    /// Get the relay URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Subscribe to an additional filter.
    ///
    /// This is used to add Layer 2 filters for repo-related events after
    /// the initial connection is established.
    pub async fn subscribe_filter(&self, filter: Filter) -> Result<(), String> {
        self.client
            .subscribe(filter, None)
            .await
            .map_err(|e| format!("Failed to subscribe with filter: {}", e))?;
        Ok(())
    }

    /// Get a reference to the client for additional operations.
    pub fn client(&self) -> &Client {
        &self.client
    }
}
