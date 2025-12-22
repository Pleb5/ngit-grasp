//! Self-Subscriber for Proactive Sync
//!
//! Monitors the relay's own database for repository announcements and
//! updates the RepoSyncIndex when new relevant events are discovered.
//!
//! This module subscribes to relevant event kinds on our own relay and
//! batches updates before sending them to the SyncManager.
//!
//! See `docs/explanation/grasp-02-proactive-sync-v4.md` for full design details.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use nostr_sdk::prelude::*;
use nostr_sdk::Timestamp;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc};

use super::{AddFilters, RepoSyncIndex, RepoSyncNeeds};

// =============================================================================
// LoopControl - Result of notification processing
// =============================================================================

/// Control flow result from processing a notification
enum LoopControl {
    /// Continue processing the next notification
    Continue,
    /// Break out of the event loop
    Break,
}

// =============================================================================
// PendingUpdates - Accumulator for batching
// =============================================================================

/// Accumulates updates between batch timer firings
struct PendingUpdates {
    /// Repos discovered since last batch, keyed by repo addressable ref
    repos: HashMap<String, RepoSyncNeeds>,
}

impl PendingUpdates {
    /// Create a new empty pending updates accumulator
    fn new() -> Self {
        Self {
            repos: HashMap::new(),
        }
    }

    /// Add or update a repo with its relays and root events
    fn add_repo(
        &mut self,
        repo_id: String,
        relays: HashSet<String>,
        root_events: HashSet<EventId>,
    ) {
        let entry = self.repos.entry(repo_id).or_insert_with(|| RepoSyncNeeds {
            relays: HashSet::new(),
            root_events: HashSet::new(),
        });
        entry.relays.extend(relays);
        entry.root_events.extend(root_events);
    }

    /// Check if there are any pending updates
    fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    /// Take all pending updates, leaving empty
    fn take(&mut self) -> HashMap<String, RepoSyncNeeds> {
        std::mem::take(&mut self.repos)
    }
}

// =============================================================================
// SelfSubscriber - Main Component
// =============================================================================

/// Subscribes to own relay's events to discover repos needing sync
///
/// The SelfSubscriber connects to our own relay and monitors for:
/// - 30617 (Repository Announcements) - to discover repos listing our relay
/// - 1617 (Patches) - root events referencing repos
/// - 1618 (Issues) - root events referencing repos
/// - 1621 (PRs) - root events referencing repos
///
/// Note: 30618 is NOT subscribed to here (per v4 spec - only synced from remote relays)
pub struct SelfSubscriber {
    /// Our own relay URL (to connect to)
    own_relay_url: String,
    /// Our service domain (for filtering relevant repos)
    relay_domain: String,
    /// Shared index of repos to sync
    repo_sync_index: RepoSyncIndex,
    /// Channel to send AddFilters actions to SyncManager
    action_tx: mpsc::Sender<AddFilters>,
    /// Last time we connected - used for since filter on reconnect
    last_connected: Option<Timestamp>,
}

impl SelfSubscriber {
    /// Create a new SelfSubscriber
    ///
    /// # Arguments
    /// * `own_relay_url` - The WebSocket URL of our own relay
    /// * `relay_domain` - Our service domain (used for filtering relevant repos)
    /// * `repo_sync_index` - Shared index to update with discovered repos
    /// * `action_tx` - Channel to send AddFilters actions to the SyncManager
    pub fn new(
        own_relay_url: String,
        relay_domain: String,
        repo_sync_index: RepoSyncIndex,
        action_tx: mpsc::Sender<AddFilters>,
    ) -> Self {
        Self {
            own_relay_url,
            relay_domain,
            repo_sync_index,
            action_tx,
            last_connected: None,
        }
    }

    /// Get batch window from environment or use default
    ///
    /// Reads `NGIT_SYNC_BATCH_WINDOW_MS` environment variable.
    /// Default: 5000ms (5 seconds)
    fn get_batch_window() -> Duration {
        std::env::var("NGIT_SYNC_BATCH_WINDOW_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(5000))
    }

    /// Process a relay pool notification
    ///
    /// Handles incoming events from the subscription, queueing 30617 announcements
    /// for batch processing and immediately processing root events.
    ///
    /// Returns `LoopControl::Break` if the loop should exit, `LoopControl::Continue` otherwise.
    async fn process_notification(
        &self,
        notification: Result<RelayPoolNotification, RecvError>,
        pending: &mut PendingUpdates,
    ) -> LoopControl {
        match notification {
            Ok(RelayPoolNotification::Event { event, .. }) => {
                // Only process 30617 events that list our relay
                if event.kind == Kind::Custom(30617) {
                    if !self.lists_our_relay(&event) {
                        return LoopControl::Continue;
                    }

                    // Extract repo ID and relays
                    if let Some(repo_id) = Self::extract_repo_id(&event) {
                        let relays = Self::extract_relay_urls(&event);
                        // 30617 announcements don't contribute to root_events - those are
                        // the 1617/1618/1621 event IDs that get added when we receive
                        // root events via handle_root_event. See mod.rs:71 for details.
                        pending.add_repo(repo_id.clone(), relays.clone(), HashSet::new());
                        tracing::info!(
                            event_id = %event.id,
                            repo_id = %repo_id,
                            relay_count = relays.len(),
                            relays = ?relays,
                            "[DIAG] Queued 30617 announcement for batch processing"
                        );
                    }
                } else {
                    // For root event kinds (1617, 1618, 1621),
                    // process them to update the RepoSyncIndex AND add to pending
                    // for Layer 3 filter creation
                    tracing::trace!(
                        kind = %event.kind,
                        event_id = %event.id,
                        "Received root event"
                    );
                    self.handle_root_event(&event, pending).await;
                }
                LoopControl::Continue
            }
            Ok(RelayPoolNotification::Shutdown) => {
                tracing::info!("SelfSubscriber received shutdown notification");
                LoopControl::Break
            }
            Err(e) => {
                tracing::error!(error = %e, "Error receiving notification");
                LoopControl::Break
            }
            _ => LoopControl::Continue,
        }
    }

    /// Extract relay URLs from event tags
    ///
    /// Extracts URLs from:
    /// - `relays` tags: ["relays", "wss://relay1.com", "wss://relay2.com", ...]
    /// - `clone` tags: ["clone", "https://example.com/repo.git", ...] (converted to ws://)
    fn extract_relay_urls(event: &Event) -> HashSet<String> {
        let mut relays = HashSet::new();

        for tag in event.tags.iter() {
            let tag_vec = tag.as_slice();
            if tag_vec.is_empty() {
                continue;
            }

            match tag_vec[0].as_str() {
                "relays" => {
                    // All subsequent values are relay URLs
                    for url in tag_vec.iter().skip(1) {
                        relays.insert(url.to_string());
                    }
                }
                "clone" if tag_vec.len() >= 2 => {
                    // Convert ALL http(s) clone URLs to ws(s) relay URLs
                    for clone_url in tag_vec.iter().skip(1) {
                        if let Some(relay_url) = clone_url_to_relay_url(clone_url) {
                            relays.insert(relay_url);
                        }
                    }
                }
                _ => {}
            }
        }

        relays
    }

    /// Extract repo identifier from event
    ///
    /// For kind 30617, uses the `d` tag to build the addressable reference
    /// Format: 30617:pubkey:identifier
    fn extract_repo_id(event: &Event) -> Option<String> {
        // For kind 30617, extract d tag and build addressable ref
        if event.kind == Kind::Custom(30617) {
            for tag in event.tags.iter() {
                let tag_vec = tag.as_slice();
                if tag_vec.len() >= 2 && tag_vec[0] == "d" {
                    return Some(format!("30617:{}:{}", event.pubkey, tag_vec[1]));
                }
            }
        }

        // For other kinds (1617, 1618, 1621), we'd need to look at
        // their 'a' tags to find which repo they belong to.
        // That processing happens in the batch processing, not here.
        None
    }

    /// Check if announcement lists our relay
    ///
    /// Returns true if any extracted relay URL contains our domain
    fn lists_our_relay(&self, event: &Event) -> bool {
        Self::extract_relay_urls(event)
            .iter()
            .any(|url| url.contains(&self.relay_domain) || url == &self.own_relay_url)
    }

    /// Main run loop
    ///
    /// Connects to own relay, subscribes to relevant event kinds,
    /// and batches updates before processing them.
    ///
    /// The optional shutdown receiver allows graceful termination when
    /// received via the broadcast channel.
    pub async fn run(mut self, mut shutdown_rx: Option<broadcast::Receiver<()>>) {
        let client = Client::default();

        // Add own relay
        if let Err(e) = client.add_relay(&self.own_relay_url).await {
            tracing::error!(
                url = %self.own_relay_url,
                error = %e,
                "Failed to add own relay for self-subscription"
            );
            return;
        }

        // Connect
        client.connect().await;

        // Subscribe to announcement and root event kinds
        // Per v4 spec: 30617, 1617, 1618, 1621 (NOT 30618)
        // Plus kind 10317 (User Grasp List) for GRASP discovery
        // Check if we have a last_connected time for reconnect filtering
        let filter = if let Some(last) = self.last_connected {
            // Quick reconnect - use since filter (15 min buffer)
            let since = Timestamp::from(last.as_secs().saturating_sub(15 * 60));
            tracing::debug!(
                since = %since,
                "Using since filter for reconnect"
            );
            Filter::new()
                .kinds(vec![
                    Kind::Custom(30617), // Repository Announcements
                    Kind::Custom(1617),  // Patches
                    Kind::Custom(1621),  // Issues
                    Kind::Custom(1618),  // Pull Requests
                    Kind::Custom(10317), // User Grasp List
                ])
                .since(since)
        } else {
            // First connection - no since filter
            Filter::new().kinds(vec![
                Kind::Custom(30617), // Repository Announcements
                Kind::Custom(1617),  // Patches
                Kind::Custom(1621),  // Issues
                Kind::Custom(1618),  // Pull Requests
                Kind::Custom(10317), // User Grasp List
            ])
        };

        // Update last_connected AFTER creating filter but BEFORE subscribing
        self.last_connected = Some(Timestamp::now());

        if let Err(e) = client.subscribe(filter, None).await {
            tracing::error!(
                error = %e,
                "Failed to subscribe to own relay for self-subscription"
            );
            return;
        }

        tracing::info!(
            url = %self.own_relay_url,
            domain = %self.relay_domain,
            "SelfSubscriber started"
        );

        let mut notifications = client.notifications();
        let batch_window = Self::get_batch_window();
        let mut pending = PendingUpdates::new();

        // Timer does NOT reset on new events - use interval
        let mut timer = tokio::time::interval(batch_window);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            // Build the select based on whether we have a shutdown receiver
            if let Some(ref mut rx) = shutdown_rx {
                tokio::select! {
                    notification = notifications.recv() => {
                        if let LoopControl::Break = self.process_notification(notification, &mut pending).await {
                            break;
                        }
                    }
                    _ = timer.tick() => {
                        if !pending.is_empty() {
                            self.process_batch(&mut pending).await;
                        }
                    }
                    _ = rx.recv() => {
                        tracing::info!("SelfSubscriber received shutdown signal");
                        break;
                    }
                }
            } else {
                // No shutdown receiver - original behavior
                tokio::select! {
                    notification = notifications.recv() => {
                        if let LoopControl::Break = self.process_notification(notification, &mut pending).await {
                            break;
                        }
                    }
                    _ = timer.tick() => {
                        if !pending.is_empty() {
                            self.process_batch(&mut pending).await;
                        }
                    }
                }
            }
        }

        tracing::info!("SelfSubscriber stopped");
    }

    /// Handle a root event (1617/1618/1621)
    ///
    /// Extracts the 'a' tag to find the repo addressable reference,
    /// then updates the RepoSyncIndex with the event ID AND adds to pending
    /// so that Layer 3 filters will be created in the next batch.
    async fn handle_root_event(&self, event: &Event, pending: &mut PendingUpdates) {
        // Extract 'a' tag to find the repo addressable reference
        let repo_a_tag = event.tags.iter().find(|tag| {
            let tag_vec = tag.as_slice();
            !tag_vec.is_empty() && tag_vec[0] == "a"
        });

        let repo_ref = match repo_a_tag {
            Some(tag) => {
                let tag_vec = tag.as_slice();
                // Get first value from tag (the 'a' tag value at index 1)
                if tag_vec.len() >= 2 {
                    tag_vec[1].clone()
                } else {
                    tracing::warn!(
                        event_id = %event.id,
                        "Root event has 'a' tag but no content"
                    );
                    return;
                }
            }
            None => {
                tracing::warn!(
                    event_id = %event.id,
                    "Root event missing 'a' tag"
                );
                return;
            }
        };

        // Look up repo in repo_sync_index - add root event directly and also to pending
        let mut index = self.repo_sync_index.write().await;
        if let Some(repo_sync) = index.get_mut(&repo_ref) {
            // Add event.id to root_events set in the index (immediate availability)
            repo_sync.root_events.insert(event.id);

            // Clone the relays before releasing the lock - Layer 3 filters need to be
            // sent to the same relays as Layer 2 filters for this repo
            let relays = repo_sync.relays.clone();

            // Release lock before modifying pending
            drop(index);

            // Also add root event to pending - this ensures batch processing runs
            // and creates Layer 3 filters for events referencing this root event.
            // CRITICAL: Include relays so derive_relay_targets knows where to send filters!
            let mut root_events = HashSet::new();
            root_events.insert(event.id);
            pending.add_repo(repo_ref.clone(), relays.clone(), root_events);

            tracing::debug!(
                event_id = %event.id,
                repo_ref = %repo_ref,
                relay_count = relays.len(),
                "Added root event to index and pending for Layer 3 filter creation"
            );
        } else {
            tracing::debug!(
                event_id = %event.id,
                repo_ref = %repo_ref,
                "Root event references unknown repo"
            );
        }
    }

    /// Process accumulated batch
    ///
    /// Updates the RepoSyncIndex with discovered repos, then derives per-relay
    /// targets and sends RelayAction messages to the SyncManager.
    async fn process_batch(&self, pending: &mut PendingUpdates) {
        use crate::sync::algorithms::derive_relay_targets;

        let updates = pending.take();

        if updates.is_empty() {
            return;
        }

        tracing::info!(
            repo_count = updates.len(),
            "Processing batch of repo updates"
        );

        // Log what repos and relays we discovered
        for (repo_id, needs) in &updates {
            tracing::info!(
                repo_id = %repo_id,
                relay_urls = ?needs.relays,
                "Discovered repo with relay URLs"
            );
        }

        // Update RepoSyncIndex
        let mut index = self.repo_sync_index.write().await;

        for (repo_id, needs) in updates {
            // Merge with existing entry or insert new
            let entry = index
                .entry(repo_id.clone())
                .or_insert_with(|| RepoSyncNeeds {
                    relays: HashSet::new(),
                    root_events: HashSet::new(),
                });
            entry.relays.extend(needs.relays);
            entry.root_events.extend(needs.root_events);

            tracing::debug!(
                repo_id = %repo_id,
                relay_count = entry.relays.len(),
                event_count = entry.root_events.len(),
                "Updated repo sync needs"
            );
        }

        // Derive per-relay targets from the updated index
        let targets = derive_relay_targets(&index);
        drop(index); // Release lock before async operations

        // For each relay, send AddFilters action directly
        // SyncManager's handle_new_sync_filters auto-spawns connection for unknown relays
        for (relay_url, needs) in targets {
            // Skip our own relay URL (we're subscribed to ourselves via self-subscription)
            if relay_url.contains(&self.relay_domain) {
                continue;
            }

            // Build filters for these repos
            let filters = crate::sync::filters::build_layer2_and_layer3_filters(
                &needs.repos,
                &needs.root_events,
                None,
            );

            // Log before moving values
            let repo_count = needs.repos.len();
            let event_count = needs.root_events.len();

            let action = AddFilters {
                relay_url: relay_url.clone(),
                items: crate::sync::PendingItems {
                    repos: needs.repos,
                    root_events: needs.root_events,
                },
                filters,
            };

            if let Err(e) = self.action_tx.send(action).await {
                tracing::error!(
                    relay = %relay_url,
                    error = %e,
                    "Failed to send AddFilters action"
                );
            } else {
                tracing::info!(
                    relay = %relay_url,
                    repo_count = repo_count,
                    event_count = event_count,
                    "Sent AddFilters action to SyncManager"
                );
            }
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Convert clone URL to relay URL
///
/// Converts http://domain:port/path.git to ws://domain:port
/// Converts https://domain:port/path.git to wss://domain:port
/// Strips the path component to get just the relay URL
/// Returns None for unsupported URL schemes
fn clone_url_to_relay_url(clone_url: &str) -> Option<String> {
    let (ws_scheme, rest) = if clone_url.starts_with("http://") {
        ("ws://", clone_url.strip_prefix("http://")?)
    } else if clone_url.starts_with("https://") {
        ("wss://", clone_url.strip_prefix("https://")?)
    } else {
        return None;
    };

    // Extract just the host:port part (everything before the first /)
    let host_port = rest.split('/').next()?;
    Some(format!("{}{}", ws_scheme, host_port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_url_to_relay_url_https() {
        assert_eq!(
            clone_url_to_relay_url("https://example.com/repo.git"),
            Some("wss://example.com".to_string())
        );
    }

    #[test]
    fn test_clone_url_to_relay_url_http() {
        assert_eq!(
            clone_url_to_relay_url("http://localhost:3000/repo.git"),
            Some("ws://localhost:3000".to_string())
        );
    }

    #[test]
    fn test_clone_url_to_relay_url_with_port() {
        assert_eq!(
            clone_url_to_relay_url("http://127.0.0.1:41463/test-repo.git"),
            Some("ws://127.0.0.1:41463".to_string())
        );
    }

    #[test]
    fn test_clone_url_to_relay_url_unsupported() {
        assert_eq!(clone_url_to_relay_url("git://example.com/repo.git"), None);
        assert_eq!(
            clone_url_to_relay_url("ssh://git@example.com/repo.git"),
            None
        );
    }
}
