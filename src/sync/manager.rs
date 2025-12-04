//! SyncManager - Coordinates proactive sync operations
//!
//! The SyncManager spawns connections to configured relays, receives events,
//! validates them through the write policy, and stores accepted events.

use nostr_relay_builder::prelude::*;
use tokio::sync::mpsc;

use super::connection::{connect_with_retry, SyncedEvent};
use super::SYNC_SOURCE_ADDR;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};

/// Coordinates proactive sync from configured relays
pub struct SyncManager {
    /// URL of the relay to sync from
    sync_relay_url: String,
    /// Database for storing accepted events
    database: SharedDatabase,
    /// Write policy for validating events
    write_policy: Nip34WritePolicy,
}

impl SyncManager {
    /// Create a new SyncManager
    pub fn new(
        sync_relay_url: String,
        database: SharedDatabase,
        write_policy: Nip34WritePolicy,
    ) -> Self {
        Self {
            sync_relay_url,
            database,
            write_policy,
        }
    }

    /// Run the sync manager
    ///
    /// This spawns a connection task and processes incoming events.
    /// Runs indefinitely until the task is cancelled.
    pub async fn run(self) {
        tracing::info!("Starting SyncManager for relay: {}", self.sync_relay_url);

        // Create channel for receiving events from connection
        let (tx, mut rx) = mpsc::channel::<SyncedEvent>(100);

        // Spawn connection task with auto-reconnect
        let url = self.sync_relay_url.clone();
        tokio::spawn(async move {
            connect_with_retry(&url, tx).await;
        });

        // Process incoming events
        while let Some(synced_event) = rx.recv().await {
            self.process_event(synced_event).await;
        }

        tracing::warn!("SyncManager event channel closed, shutting down");
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