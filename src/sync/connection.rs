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

use std::sync::Arc;
use std::time::Duration;

use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

use super::filter::FilterService;

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

        Ok(Self {
            url: url.to_string(),
            client,
            filter_service,
            remote_domain: remote_domain.to_string(),
        })
    }

    /// Start receiving events and send them through the channel
    ///
    /// This method runs indefinitely, handling events from all three filter layers.
    pub async fn run(self, tx: mpsc::Sender<SyncedEvent>) {
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
}

/// Reconnect loop with exponential backoff
///
/// # Arguments
/// * `url` - The relay URL to connect to
/// * `tx` - Channel sender for synced events
/// * `filter_service` - FilterService for building subscriptions
/// * `our_domain` - Our relay's domain (used to extract remote domain)
pub async fn connect_with_retry(
    url: &str,
    tx: mpsc::Sender<SyncedEvent>,
    filter_service: Arc<FilterService>,
    _our_domain: &str,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    // Extract remote domain from URL
    let remote_domain = extract_domain_from_url(url).unwrap_or_else(|| url.to_string());

    loop {
        match SyncConnection::new(url, filter_service.clone(), &remote_domain).await {
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