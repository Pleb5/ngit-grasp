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
use tokio::sync::{broadcast, mpsc};

use super::{RepoSyncIndex, RepoSyncNeeds};

// =============================================================================
// RelayAction - Actions to send to SyncManager
// =============================================================================

/// Actions that the SelfSubscriber sends to the SyncManager
#[derive(Debug)]
pub enum RelayAction {
    /// Spawn a new relay connection
    SpawnRelay {
        /// The relay URL to connect to
        relay_url: String,
        /// Repos to sync, mapped to their root event IDs
        repos: HashMap<String, HashSet<EventId>>,
    },
    /// Add filters to an existing relay connection
    AddFilters {
        /// The relay URL to add filters to
        relay_url: String,
        /// Repos to sync, mapped to their root event IDs
        repos: HashMap<String, HashSet<EventId>>,
    },
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
    fn add_repo(&mut self, repo_id: String, relays: HashSet<String>, root_events: HashSet<EventId>) {
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
/// - 1619 (Replies) - root events referencing repos
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
    /// Channel to send actions to SyncManager
    action_tx: mpsc::Sender<RelayAction>,
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
    /// * `action_tx` - Channel to send RelayActions to the SyncManager
    pub fn new(
        own_relay_url: String,
        relay_domain: String,
        repo_sync_index: RepoSyncIndex,
        action_tx: mpsc::Sender<RelayAction>,
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
                    // Convert http(s) clone URL to ws(s) relay URL
                    if let Some(relay_url) = clone_url_to_relay_url(&tag_vec[1]) {
                        relays.insert(relay_url);
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

        // For other kinds (1617, 1618, 1619, 1621), we'd need to look at
        // their 'a' tags to find which repo they belong to.
        // That processing happens in the batch processing, not here.
        None
    }

    /// Check if announcement lists our relay
    ///
    /// Returns true if any extracted relay URL contains our domain
    fn lists_our_relay(&self, event: &Event) -> bool {
        Self::extract_relay_urls(event).iter().any(|url| {
            url.contains(&self.relay_domain) || url == &self.own_relay_url
        })
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
        // Per v4 spec: 30617, 1617, 1618, 1619, 1621 (NOT 30618)
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
                    Kind::Custom(1618),  // Issues
                    Kind::Custom(1619),  // Replies/Status
                    Kind::Custom(1621),  // Pull Requests
                ])
                .since(since)
        } else {
            // First connection - no since filter
            Filter::new().kinds(vec![
                Kind::Custom(30617), // Repository Announcements
                Kind::Custom(1617),  // Patches
                Kind::Custom(1618),  // Issues
                Kind::Custom(1619),  // Replies/Status
                Kind::Custom(1621),  // Pull Requests
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
                        match notification {
                            Ok(RelayPoolNotification::Event { event, .. }) => {
                                // Only process 30617 events that list our relay
                                if event.kind == Kind::Custom(30617) {
                                    if !self.lists_our_relay(&event) {
                                        continue;
                                    }

                                    // Extract repo ID and relays
                                    if let Some(repo_id) = Self::extract_repo_id(&event) {
                                        let relays = Self::extract_relay_urls(&event);
                                        let mut root_events = HashSet::new();
                                        root_events.insert(event.id);

                                        pending.add_repo(repo_id, relays, root_events);
                                        tracing::debug!(
                                            event_id = %event.id,
                                            "Queued 30617 announcement for batch processing"
                                        );
                                    }
                                } else {
                                    // For root event kinds (1617, 1618, 1619, 1621),
                                    // process them to update the RepoSyncIndex
                                    tracing::trace!(
                                        kind = %event.kind,
                                        event_id = %event.id,
                                        "Received root event"
                                    );
                                    self.handle_root_event(&event).await;
                                }
                            }
                            Ok(RelayPoolNotification::Shutdown) => {
                                tracing::info!("SelfSubscriber received shutdown notification");
                                break;
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Error receiving notification");
                                break;
                            }
                            _ => {}
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
                        match notification {
                            Ok(RelayPoolNotification::Event { event, .. }) => {
                                // Only process 30617 events that list our relay
                                if event.kind == Kind::Custom(30617) {
                                    if !self.lists_our_relay(&event) {
                                        continue;
                                    }

                                    // Extract repo ID and relays
                                    if let Some(repo_id) = Self::extract_repo_id(&event) {
                                        let relays = Self::extract_relay_urls(&event);
                                        let mut root_events = HashSet::new();
                                        root_events.insert(event.id);

                                        pending.add_repo(repo_id, relays, root_events);
                                        tracing::debug!(
                                            event_id = %event.id,
                                            "Queued 30617 announcement for batch processing"
                                        );
                                    }
                                } else {
                                    // For root event kinds (1617, 1618, 1619, 1621),
                                    // process them to update the RepoSyncIndex
                                    tracing::trace!(
                                        kind = %event.kind,
                                        event_id = %event.id,
                                        "Received root event"
                                    );
                                    self.handle_root_event(&event).await;
                                }
                            }
                            Ok(RelayPoolNotification::Shutdown) => {
                                tracing::info!("SelfSubscriber received shutdown notification");
                                break;
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Error receiving notification");
                                break;
                            }
                            _ => {}
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

    /// Handle a root event (1617/1618/1619/1621)
    ///
    /// Extracts the 'a' tag to find the repo addressable reference,
    /// then updates the RepoSyncIndex with the event ID.
    async fn handle_root_event(&self, event: &Event) {
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

        // Look up repo in repo_sync_index
        let mut index = self.repo_sync_index.write().await;
        if let Some(repo_sync) = index.get_mut(&repo_ref) {
            // Add event.id to root_events set
            repo_sync.root_events.insert(event.id);
            tracing::debug!(
                event_id = %event.id,
                repo_ref = %repo_ref,
                "Added root event to repo"
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

        // Update RepoSyncIndex
        let mut index = self.repo_sync_index.write().await;

        for (repo_id, needs) in updates {
            // Merge with existing entry or insert new
            let entry = index.entry(repo_id.clone()).or_insert_with(|| RepoSyncNeeds {
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

        // For each relay, send SpawnRelay action
        // SyncManager will check if relay already exists
        for (relay_url, needs) in targets {
            // Skip our own relay URL (we're subscribed to ourselves via self-subscription)
            if relay_url.contains(&self.relay_domain) {
                continue;
            }

            // Convert needs to HashMap<String, HashSet<EventId>>
            let mut repos = HashMap::new();
            for repo_id in needs.repos {
                repos.insert(repo_id, needs.root_events.clone());
            }

            let action = RelayAction::SpawnRelay { relay_url: relay_url.clone(), repos };

            if let Err(e) = self.action_tx.send(action).await {
                tracing::error!(
                    relay = %relay_url,
                    error = %e,
                    "Failed to send SpawnRelay action"
                );
            } else {
                tracing::debug!(
                    relay = %relay_url,
                    "Sent SpawnRelay action to SyncManager"
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
/// Converts http:// to ws:// and https:// to wss://
/// Returns None for unsupported URL schemes
fn clone_url_to_relay_url(clone_url: &str) -> Option<String> {
    if clone_url.starts_with("http://") {
        Some(clone_url.replacen("http://", "ws://", 1))
    } else if clone_url.starts_with("https://") {
        Some(clone_url.replacen("https://", "wss://", 1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_url_to_relay_url_https() {
        assert_eq!(
            clone_url_to_relay_url("https://example.com/repo.git"),
            Some("wss://example.com/repo.git".to_string())
        );
    }

    #[test]
    fn test_clone_url_to_relay_url_http() {
        assert_eq!(
            clone_url_to_relay_url("http://localhost:3000/repo.git"),
            Some("ws://localhost:3000/repo.git".to_string())
        );
    }

    #[test]
    fn test_clone_url_to_relay_url_unsupported() {
        assert_eq!(clone_url_to_relay_url("git://example.com/repo.git"), None);
        assert_eq!(clone_url_to_relay_url("ssh://git@example.com/repo.git"), None);
    }
}