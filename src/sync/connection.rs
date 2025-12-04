//! WebSocket connection handling for sync
//!
//! Manages the connection to a source relay, subscribes to kind 30617 events,
//! and passes them through validation.

use std::time::Duration;

use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

use super::KIND_REPOSITORY_STATE;

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
}

impl SyncConnection {
    /// Create a new sync connection to the given relay URL
    pub async fn new(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::default();

        // Add the relay
        client.add_relay(url).await?;

        // Connect to the relay
        client.connect().await;

        tracing::info!("Sync connection established to {}", url);

        Ok(Self {
            url: url.to_string(),
            client,
        })
    }

    /// Start receiving events and send them through the channel
    ///
    /// This method runs indefinitely, reconnecting as needed.
    pub async fn run(self, tx: mpsc::Sender<SyncedEvent>) {
        // Create filter for kind 30617 (repository state) events
        let filter = Filter::new().kind(Kind::Custom(KIND_REPOSITORY_STATE));

        // Subscribe to events
        match self.client.subscribe(filter, None).await {
            Ok(output) => {
                tracing::info!(
                    "Subscribed to kind {} events on {} (subscription: {})",
                    KIND_REPOSITORY_STATE,
                    self.url,
                    output.id()
                );
            }
            Err(e) => {
                tracing::error!("Failed to subscribe on {}: {}", self.url, e);
                return;
            }
        };

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

}

/// Reconnect loop with exponential backoff
pub async fn connect_with_retry(
    url: &str,
    tx: mpsc::Sender<SyncedEvent>,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        match SyncConnection::new(url).await {
            Ok(conn) => {
                backoff = Duration::from_secs(1); // Reset backoff on successful connection
                conn.run(tx.clone()).await;
                tracing::warn!("Sync connection to {} ended, will reconnect", url);
            }
            Err(e) => {
                tracing::error!(
                    "Failed to connect to sync relay {}: {} (retrying in {:?})",
                    url,
                    e,
                    backoff
                );
            }
        }

        // Wait before reconnecting
        tokio::time::sleep(backoff).await;

        // Exponential backoff
        backoff = std::cmp::min(backoff * 2, max_backoff);
    }
}