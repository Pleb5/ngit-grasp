//! Relay Connection Management for Proactive Sync
//!
//! This module provides relay connection management for external relay connections.
//! Each RelayConnection manages a single connection to an external relay and handles
//! subscriptions using the three-layer sync strategy.
//!
//! ## NIP-77 Negentropy Support
//!
//! RelayConnection supports NIP-77 negentropy for efficient set reconciliation:
//! - `supports_negentropy()` - Check if remote relay supports NIP-77
//! - `negentropy_sync_filter()` - Perform negentropy sync for a filter
//!
//! When NIP-77 is supported, historical sync uses negentropy instead of REQ+EOSE,
//! significantly reducing bandwidth for relays with overlapping event sets.
//!
//! See `docs/explanation/grasp-02-proactive-sync.md` for full design details.

use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

use crate::nostr::builder::SharedDatabase;

/// Events from a relay connection
#[derive(Debug)]
pub enum RelayEvent {
    /// A new event was received (event, subscription_id)
    Event(Event, SubscriptionId),
    /// End of stored events for a subscription
    EndOfStoredEvents(SubscriptionId),
    /// NOTICE message from relay
    Notice(String),
    /// Connection was closed
    Closed(String),
    /// Shutdown notification
    Shutdown,
}

/// Result of a negentropy sync operation
#[derive(Debug)]
pub struct NegentropySyncResult {
    /// Event IDs that exist on remote but not locally (discovered but not fetched)
    pub remote_only: Vec<EventId>,
    /// Event IDs that exist locally but not on remote (could push)
    pub local_only: Vec<EventId>,
    /// Event IDs that were fetched during sync
    pub received: Vec<EventId>,
}

/// Manages connection to a single external relay
///
/// RelayConnection wraps a nostr-sdk Client to manage a WebSocket connection
/// to an external relay. It handles:
/// - Connection establishment
/// - Layer 1 subscription (announcements)
/// - Additional filter subscriptions (Layers 2 & 3)
/// - Event notification loop
/// - NIP-77 negentropy synchronization
///
/// # Why Client instead of Relay directly?
///
/// While it would be cleaner to hold a `Relay` directly (since we only manage
/// one relay per connection), the nostr-sdk API makes `Relay::new()` private
/// (`pub(crate)`). Relays can only be created through `Client::add_relay()` or
/// `RelayPool::add_relay()`. This is an intentional design in nostr-sdk to
/// ensure proper lifecycle management.
///
/// The Client adds minimal overhead since we configure it with a single relay,
/// and we retrieve the `Relay` reference for notification handling.
#[derive(Clone)]
pub struct RelayConnection {
    /// The relay URL this connection is for
    url: String,
    /// The underlying nostr-sdk client
    client: Client,
    /// Local database for negentropy comparison (used for NIP-77 sync)
    database: Option<SharedDatabase>,
    /// Whether we've logged NIP-77 not supported for this relay (log once)
    nip77_warning_logged: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl RelayConnection {
    /// Create a new relay connection (not yet connected)
    ///
    /// # Arguments
    /// * `url` - The relay URL to connect to (e.g., "wss://relay.example.com")
    pub fn new(url: String) -> Self {
        let client = Client::default();
        Self {
            url,
            client,
            database: None,
            nip77_warning_logged: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Create a new relay connection with database for negentropy sync
    ///
    /// # Arguments
    /// * `url` - The relay URL to connect to (e.g., "wss://relay.example.com")
    /// * `database` - Shared database for local event comparison during negentropy sync
    pub fn new_with_database(url: String, database: SharedDatabase) -> Self {
        let client = Client::default();
        Self {
            url,
            client,
            database: Some(database),
            nip77_warning_logged: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Connect to the relay
    ///
    /// This method:
    /// 1. Adds the relay to the client
    /// 2. Establishes the WebSocket connection
    /// 3. Verifies connection was established
    ///
    /// Subscriptions are handled separately via handle_connect_or_reconnect.
    ///
    /// # Arguments
    /// * `connection_timeout_secs` - Timeout for the connection attempt in seconds.
    ///   Should be no larger than base_backoff_secs to ensure the connection attempt
    ///   completes before the next retry would be scheduled.
    ///
    /// # Returns
    /// * `Ok(())` - Connection established successfully
    /// * `Err(String)` with error description on failure
    pub async fn connect(&self, connection_timeout_secs: u64) -> Result<(), String> {
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
            .try_connect_relay(
                &self.url,
                std::time::Duration::from_secs(connection_timeout_secs),
            )
            .await
            .map_err(|e| format!("Failed to connect to relay {}: {}", self.url, e))?;

        tracing::info!(url = %self.url, "Connected to relay");
        Ok(())
    }

    /// Run the event loop, sending events through the provided channel
    ///
    /// This method blocks and processes notifications from the relay using
    /// nostr-sdk's `Relay::notifications()` channel, which provides event-driven
    /// disconnect detection via `RelayNotification::RelayStatus`.
    ///
    /// Notification types handled:
    /// - `RelayNotification::Event` -> sends `RelayEvent::Event`
    /// - `RelayNotification::Message` with EOSE -> sends `RelayEvent::EndOfStoredEvents`
    /// - `RelayNotification::RelayStatus { Disconnected }` -> terminates loop (disconnect detected)
    /// - `RelayNotification::Shutdown` -> sends `RelayEvent::Shutdown`
    ///
    /// The loop terminates when:
    /// - The sender channel is closed (receiver dropped)
    /// - A shutdown notification is received
    /// - Relay status changes to Disconnected or Terminated
    /// - An error occurs receiving notifications
    ///
    /// # Arguments
    /// * `event_sender` - Channel to send relay events through
    ///
    /// # Note
    /// This uses `Relay::notifications()` instead of `Client::notifications()` because
    /// `RelayNotification::RelayStatus` events are not forwarded to the pool-level channel.
    /// This enables immediate, event-driven disconnect detection without polling.
    ///
    /// We must retrieve the Relay from the Client because nostr-sdk does not expose
    /// `Relay::new()` publicly - relays can only be created through Client or RelayPool.
    pub async fn run_event_loop(self, event_sender: mpsc::Sender<RelayEvent>) {
        let url = self.url.clone();

        // Get the Relay from the client to access relay-level notifications
        // which include RelayStatus changes (not available at pool level)
        let relay = match self.client.relay(&self.url).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(relay = %url, error = %e, "Failed to get relay from client");
                return;
            }
        };

        // Subscribe to relay-level notifications (includes RelayStatus)
        let mut notifications = relay.notifications();

        tracing::debug!(relay = %url, "Starting event loop with relay-level notifications");

        loop {
            match notifications.recv().await {
                Ok(notification) => {
                    match notification {
                        RelayNotification::Event {
                            event,
                            subscription_id,
                        } => {
                            tracing::trace!(
                                relay = %url,
                                event_id = %event.id,
                                sub_id = %subscription_id,
                                "Received event"
                            );
                            if event_sender.send(RelayEvent::Event(*event, subscription_id.clone())).await.is_err() {
                                tracing::debug!(relay = %url, "Event sender closed, stopping event loop");
                                break;
                            }
                        }
                        RelayNotification::Message { message } => match message {
                            RelayMessage::EndOfStoredEvents(sub_id) => {
                                tracing::debug!(relay = %url, sub_id = ?sub_id, "Received EOSE");
                                // Convert Cow<SubscriptionId> to owned SubscriptionId
                                let owned_sub_id = sub_id.into_owned();
                                if event_sender
                                    .send(RelayEvent::EndOfStoredEvents(owned_sub_id))
                                    .await
                                    .is_err()
                                {
                                    tracing::debug!(
                                        relay = %url,
                                        "Event sender closed, stopping event loop"
                                    );
                                    break;
                                }
                            }
                            RelayMessage::Notice(msg) => {
                                tracing::debug!(relay = %url, message = %msg, "Received NOTICE");
                                let _ = event_sender.send(RelayEvent::Notice(msg.to_string())).await;
                                // Don't break - continue processing events
                            }
                            RelayMessage::Closed { message: msg, .. } => {
                                tracing::info!(relay = %url, message = %msg, "Relay closed subscription");
                                let _ =
                                    event_sender.send(RelayEvent::Closed(msg.to_string())).await;
                                // Don't break - CLOSED is subscription-specific, not connection-specific
                                // The event loop should continue running for other active subscriptions
                            }
                            _ => {}
                        },
                        RelayNotification::RelayStatus { status } => {
                            // Event-driven disconnect detection - no polling needed!
                            match status {
                                RelayStatus::Disconnected => {
                                    tracing::info!(
                                        relay = %url,
                                        "Relay disconnected (detected via RelayNotification)"
                                    );
                                    break;
                                }
                                RelayStatus::Terminated => {
                                    tracing::info!(
                                        relay = %url,
                                        "Relay terminated (detected via RelayNotification)"
                                    );
                                    break;
                                }
                                _ => {
                                    // Log other status changes for debugging
                                    tracing::trace!(
                                        relay = %url,
                                        status = ?status,
                                        "Relay status changed"
                                    );
                                }
                            }
                        }
                        RelayNotification::Shutdown => {
                            tracing::info!(relay = %url, "Relay shutdown notification");
                            let _ = event_sender.send(RelayEvent::Shutdown).await;
                            break;
                        }
                        RelayNotification::Authenticated => {
                            tracing::debug!(relay = %url, "Authenticated to relay (NIP-42)");
                        }
                        RelayNotification::AuthenticationFailed => {
                            tracing::warn!(relay = %url, "Authentication failed to relay (NIP-42)");
                            // Don't break - relay may still work for public data
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
    /// * `auto_close` - If true, subscription automatically closes after EOSE (for historic sync). If false, stays open for new events (for live sync).
    ///
    /// # Returns
    /// * `Ok(SubscriptionId)` - The subscription ID on success
    /// * `Err(String)` - Error description on failure
    pub async fn subscribe_filter(
        &self,
        filter: Filter,
        auto_close: bool,
    ) -> Result<SubscriptionId, String> {
        // DEBUG TRACING: Log the filter being subscribed to
        tracing::debug!(
            relay = %self.url,
            filter = ?filter,
            auto_close = auto_close,
            "subscribe_filter called with filter"
        );

        let opts = if auto_close {
            Some(SubscribeAutoCloseOptions::default().exit_policy(ReqExitPolicy::ExitOnEOSE))
        } else {
            None
        };

        let output = self
            .client
            .subscribe(filter, opts)
            .await
            .map_err(|e| format!("Failed to subscribe on {}: {}", self.url, e))?;

        tracing::debug!(
            relay = %self.url,
            subscription_id = %output.val,
            "subscribe_filter succeeded"
        );

        Ok(output.val)
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

    // =========================================================================
    // NIP-77 Negentropy Support
    // =========================================================================

    /// Check if negentropy sync should be attempted
    ///
    /// Rather than relying on NIP-11 document detection (which can be unreliable),
    /// this returns true to indicate we should try negentropy sync. The actual
    /// sync will handle failures gracefully with fallback to REQ+EOSE.
    ///
    /// # Note
    /// This uses a "try and fallback" approach because:
    /// - Some relays support NIP-77 but don't advertise it in NIP-11
    /// - Some relays claim NIP-77 support but have bugs
    /// - The nostr-sdk 0.44 API for relay document access varies
    pub async fn supports_negentropy(&self) -> bool {
        // Always return true to attempt negentropy - we handle failure gracefully
        // in negentropy_sync_filter() which logs a warning and returns an error
        // that the caller can use to fall back to REQ+EOSE
        true
    }

    /// Perform a negentropy sync diff (dry run) to identify missing events
    ///
    /// This method performs NIP-77 negentropy reconciliation without downloading events.
    /// It returns the list of event IDs that need to be fetched. The caller should then
    /// manually fetch these events and pass them through the write policy for validation.
    ///
    /// # Arguments
    /// * `filter` - The filter to sync
    ///
    /// # Returns
    /// * `Ok(Reconciliation)` - Reconciliation result with remote/local/sent event IDs
    /// * `Err(String)` - Sync failed (relay may not support NIP-77, or other error)
    ///
    /// # Usage Pattern
    /// ```ignore
    /// // 1. Get the diff
    /// let reconciliation = conn.negentropy_sync_diff(filter).await?;
    ///
    /// // 2. Fetch missing events by ID
    /// if !reconciliation.remote.is_empty() {
    ///     let ids: Vec<EventId> = reconciliation.remote.into_iter().collect();
    ///     let filter = Filter::new().ids(ids);
    ///     conn.subscribe_filter(filter, tx).await?;
    /// }
    ///
    /// // 3. Events come through normal flow and get validated via process_event_static
    /// ```
    pub async fn negentropy_sync_diff(&self, filter: Filter) -> Result<Reconciliation, String> {
        // Use dry_run to only identify differences without downloading events
        let sync_opts = SyncOptions::default().dry_run();

        match self.client.sync(filter.clone(), &sync_opts).await {
            Ok(output) => {
                let reconciliation = output.val;

                tracing::debug!(
                    relay = %self.url,
                    local_count = reconciliation.local.len(),
                    remote_count = reconciliation.remote.len(),
                    "Negentropy diff completed (dry run)"
                );

                // Check for any failures
                if !output.failed.is_empty() {
                    tracing::warn!(
                        relay = %self.url,
                        failures = ?output.failed,
                        "Some relays failed during negentropy diff"
                    );
                }

                Ok(reconciliation)
            }
            Err(e) => {
                // Log warning only once per relay to avoid spam
                if !self
                    .nip77_warning_logged
                    .swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    tracing::warn!(
                        relay = %self.url,
                        error = %e,
                        "Negentropy diff failed, will fall back to REQ+EOSE"
                    );
                }
                Err(format!("Negentropy diff failed: {}", e))
            }
        }
    }

    /// Check if this connection has a database configured for negentropy
    pub fn has_database(&self) -> bool {
        self.database.is_some()
    }
}
