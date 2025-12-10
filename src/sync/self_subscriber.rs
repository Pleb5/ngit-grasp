//! Self-Subscriber for Proactive Sync
//!
//! This module handles subscribing to our own relay to detect new events
//! and trigger relay discovery from announcements.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use nostr_sdk::prelude::*;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::nostr::events::{KIND_PR, KIND_PR_UPDATE, KIND_REPOSITORY_ANNOUNCEMENT};

use super::{FollowingRepoRootEvents, SyncManager, SyncRelays};

// =============================================================================
// Types
// =============================================================================

/// Actions to be taken by the SyncManager based on self-subscription events.
#[derive(Debug, Clone)]
pub enum RelayAction {
    /// Spawn a new relay connection to sync from.
    /// Contains: relay_url, map of repo_refs to their event IDs for Layer 2 filtering.
    SpawnRelay {
        relay_url: String,
        repos_and_root_events: HashMap<String, HashSet<EventId>>,
    },
    /// Add filters to an existing relay connection.
    /// Contains: relay_url, additional repos to add.
    AddFilters {
        relay_url: String,
        repos_and_new_root_event: HashMap<String, HashSet<EventId>>,
    },
}

/// Pending updates collected during batch window.
#[derive(Debug, Default)]
struct PendingUpdates {
    /// New announcements (kind 30617) - triggers relay discovery
    announcements: Vec<Event>,
    /// New root events (kinds 1617, 1618, 1619, 1621) - updates following set
    root_events: Vec<Event>,
}

// =============================================================================
// SelfSubscriber
// =============================================================================

/// Subscribes to our own relay to detect new events.
///
/// The self-subscriber:
/// 1. Connects to our own relay
/// 2. Subscribes to kinds 30617, 1617, 1618, 1619, 1621 (NOT 30618)
/// 3. When events arrive, batches them
/// 4. On batch timer fire, processes updates and sends relay actions
pub struct SelfSubscriber {
    /// URL of our own relay to subscribe to
    own_relay_url: String,
    /// Our relay domain for checking if announcements list us
    relay_domain: String,
    /// Reference to following repo root events (shared with SyncManager)
    following_repo_root_events: FollowingRepoRootEvents,
    /// Reference to sync relays (shared with SyncManager)
    sync_relays: SyncRelays,
    /// Channel to send relay actions back to manager
    action_tx: mpsc::Sender<RelayAction>,
}

impl SelfSubscriber {
    /// Create a new self-subscriber.
    pub fn new(
        own_relay_url: String,
        relay_domain: String,
        following_repo_root_events: FollowingRepoRootEvents,
        sync_relays: SyncRelays,
        action_tx: mpsc::Sender<RelayAction>,
    ) -> Self {
        Self {
            own_relay_url,
            relay_domain,
            following_repo_root_events,
            sync_relays,
            action_tx,
        }
    }

    /// Get the batch window duration from environment variable.
    ///
    /// Default is 5 seconds, but can be overridden via NGIT_SYNC_BATCH_WINDOW_MS
    /// for faster tests (typically 200ms).
    fn get_batch_window() -> Duration {
        std::env::var("NGIT_SYNC_BATCH_WINDOW_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(5))
    }

    /// Run the self-subscriber event loop.
    ///
    /// This method:
    /// 1. Connects to our own relay
    /// 2. Subscribes to relevant event kinds
    /// 3. Receives events and batches them
    /// 4. On batch timer fire, processes and sends relay actions
    pub async fn run(self) {
        tracing::info!("SelfSubscriber starting for {}", self.own_relay_url);

        // Create nostr-sdk client
        let keys = Keys::generate();
        let client = Client::new(keys);

        // Connect to our own relay
        if let Err(e) = client.add_relay(&self.own_relay_url).await {
            tracing::error!("Failed to add own relay {}: {}", self.own_relay_url, e);
            return;
        }

        client.connect().await;

        // Wait for connection
        let mut connected = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let relays = client.relays().await;
            if relays.values().any(|r| r.is_connected()) {
                connected = true;
                break;
            }
        }

        if !connected {
            tracing::error!(
                "Failed to connect to own relay {} after 3 seconds",
                self.own_relay_url
            );
            return;
        }

        tracing::info!("SelfSubscriber connected to {}", self.own_relay_url);

        // Subscribe to kinds 30617, 1617, 1618, 1619, 1621 (NOT 30618 per v2 design)
        let filter = Filter::new()
            .kinds([
                Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT), // 30617
                Kind::GitPatch,                             // 1617
                Kind::Custom(KIND_PR),                      // 1618
                Kind::Custom(KIND_PR_UPDATE),               // 1619
                Kind::GitIssue,                             // 1621
            ])
            .since(Timestamp::now());

        if let Err(e) = client.subscribe(filter, None).await {
            tracing::error!("Failed to subscribe to own relay: {}", e);
            return;
        }

        tracing::info!("SelfSubscriber subscribed to event kinds on own relay");

        // Batch state
        let mut pending = PendingUpdates::default();
        let mut batch_timer_started: Option<Instant> = None;
        let batch_window = Self::get_batch_window();

        // Main event loop using notifications stream
        loop {
            // Calculate timeout for batch processing
            let timeout = if let Some(started) = batch_timer_started {
                let elapsed = started.elapsed();
                if elapsed >= batch_window {
                    Duration::ZERO
                } else {
                    batch_window - elapsed
                }
            } else {
                Duration::from_secs(60) // Long timeout when no batch pending
            };

            // Wait for notification with timeout
            let notification = tokio::time::timeout(timeout, client.notifications().recv()).await;

            match notification {
                Ok(Ok(notification)) => {
                    match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            let kind = event.kind.as_u16();

                            // Start batch timer on first event (does NOT reset)
                            if batch_timer_started.is_none() {
                                batch_timer_started = Some(Instant::now());
                                tracing::debug!("Batch timer started");
                            }

                            // Classify and add to pending
                            if kind == KIND_REPOSITORY_ANNOUNCEMENT {
                                tracing::debug!(
                                    "SelfSubscriber received announcement {}",
                                    event.id
                                );
                                pending.announcements.push(*event);
                            } else {
                                tracing::debug!(
                                    "SelfSubscriber received root event {} (kind {})",
                                    event.id,
                                    kind
                                );
                                pending.root_events.push(*event);
                            }
                        }
                        RelayPoolNotification::Message { message, .. } => {
                            if let RelayMessage::EndOfStoredEvents(_) = message {
                                tracing::debug!("SelfSubscriber EOSE received");
                                // Process any pending events after EOSE
                                if !pending.announcements.is_empty()
                                    || !pending.root_events.is_empty()
                                {
                                    self.process_batch(&mut pending).await;
                                    batch_timer_started = None;
                                }
                            }
                        }
                        RelayPoolNotification::Shutdown => {
                            tracing::info!("SelfSubscriber shutting down");
                            break;
                        }
                    }
                }
                Ok(Err(_)) => {
                    // Channel closed
                    tracing::warn!("SelfSubscriber notification channel closed");
                    break;
                }
                Err(_) => {
                    // Timeout - check if batch should be processed
                    if let Some(started) = batch_timer_started {
                        if started.elapsed() >= batch_window {
                            if !pending.announcements.is_empty() || !pending.root_events.is_empty()
                            {
                                self.process_batch(&mut pending).await;
                            }
                            batch_timer_started = None;
                        }
                    }
                }
            }
        }

        client.disconnect().await;
        tracing::info!("SelfSubscriber disconnected");
    }

    /// Process a batch of pending updates.
    async fn process_batch(&self, pending: &mut PendingUpdates) {
        tracing::debug!(
            "Processing batch: {} announcements, {} root events",
            pending.announcements.len(),
            pending.root_events.len()
        );

        // Process root events first (update following_repo_root_events)
        for event in pending.root_events.drain(..) {
            let repo_refs = SyncManager::extract_all_repo_refs(&event);
            if !repo_refs.is_empty() {
                let mut guard = self.following_repo_root_events.write().await;
                for repo_ref in repo_refs {
                    guard.entry(repo_ref).or_default().insert(event.id);
                }
            }
        }

        // Process announcements (relay discovery)
        for event in pending.announcements.drain(..) {
            self.process_announcement(&event).await;
        }
    }

    /// Process an announcement event for relay discovery.
    async fn process_announcement(&self, event: &Event) {
        let repo_ref = SyncManager::build_repo_ref(event);
        let relay_urls = Self::extract_relay_urls_from_announcement(event);

        // Check if this announcement lists our relay
        if !self.lists_our_service(event) {
            tracing::debug!(
                "Announcement {} does not list our service, skipping relay discovery",
                event.id
            );
            return;
        }

        tracing::info!(
            "Processing announcement {} for repo {}, found {} relay URLs",
            event.id,
            repo_ref,
            relay_urls.len()
        );

        // Get current events for this repo from following_repo_root_events
        let events = self
            .following_repo_root_events
            .read()
            .await
            .get(&repo_ref)
            .cloned()
            .unwrap_or_default();

        // For each relay URL in the announcement, check if we need to spawn or update
        for relay_url in relay_urls {
            if self.is_own_relay(&relay_url) {
                continue; // Skip our own relay
            }

            let sync_relays_guard = self.sync_relays.read().await;
            let exists = sync_relays_guard.contains_key(&relay_url);
            drop(sync_relays_guard);

            if exists {
                // Relay already known - check if we need to add this repo
                let mut guard = self.sync_relays.write().await;
                let relay_repos = guard.entry(relay_url.clone()).or_default();
                let is_new_repo = !relay_repos.contains_key(&repo_ref);

                if is_new_repo {
                    relay_repos.insert(repo_ref.clone(), events.clone());
                    drop(guard);

                    // Send action to add filters
                    let mut repos_filters = HashMap::new();
                    repos_filters.insert(repo_ref.clone(), events.clone());

                    if let Err(e) = self
                        .action_tx
                        .send(RelayAction::AddFilters {
                            relay_url: relay_url.clone(),
                            repos_and_new_root_event: repos_filters,
                        })
                        .await
                    {
                        tracing::warn!("Failed to send AddFilters action: {}", e);
                    }
                }
            } else {
                // New relay - add to sync_relays and spawn
                let mut guard = self.sync_relays.write().await;
                let mut repos = HashMap::new();
                repos.insert(repo_ref.clone(), events.clone());
                guard.insert(relay_url.clone(), repos.clone());
                drop(guard);

                tracing::info!("Discovered new relay to sync from: {}", relay_url);

                // Send action to spawn relay
                if let Err(e) = self
                    .action_tx
                    .send(RelayAction::SpawnRelay {
                        relay_url: relay_url.clone(),
                        repos_and_root_events: repos,
                    })
                    .await
                {
                    tracing::warn!("Failed to send SpawnRelay action: {}", e);
                }
            }
        }
    }

    /// Extract relay URLs from an announcement event.
    ///
    /// Looks for both 'relays' and 'clone' tags.
    fn extract_relay_urls_from_announcement(event: &Event) -> Vec<String> {
        let mut urls = Vec::new();

        // Extract from 'relays' tag
        for tag in event.tags.iter() {
            if matches!(tag.kind(), TagKind::Relays) {
                let vec = tag.clone().to_vec();
                urls.extend(vec.into_iter().skip(1)); // Skip tag name
            }
        }

        // Extract from 'clone' tag - parse URLs to get relay hints
        // Clone URLs look like: http://domain/repo.git or git://domain/repo.git
        // We want to construct ws://domain from these
        for tag in event.tags.iter() {
            if matches!(tag.kind(), TagKind::Clone) {
                let vec = tag.clone().to_vec();
                for url in vec.into_iter().skip(1) {
                    if let Some(relay_url) = Self::clone_url_to_relay_url(&url) {
                        if !urls.contains(&relay_url) {
                            urls.push(relay_url);
                        }
                    }
                }
            }
        }

        urls
    }

    /// Convert a clone URL to a potential relay URL.
    ///
    /// E.g., "http://127.0.0.1:8080/repo.git" -> "ws://127.0.0.1:8080"
    fn clone_url_to_relay_url(clone_url: &str) -> Option<String> {
        // Parse the URL to extract host:port
        if let Ok(url) = url::Url::parse(clone_url) {
            let host = url.host_str()?;
            let port = url.port();
            let scheme = if url.scheme() == "https" { "wss" } else { "ws" };

            if let Some(port) = port {
                Some(format!("{}://{}:{}", scheme, host, port))
            } else {
                Some(format!("{}://{}", scheme, host))
            }
        } else {
            None
        }
    }

    /// Check if event lists our service in the relays or clone tags.
    fn lists_our_service(&self, event: &Event) -> bool {
        // Check relays tag
        for tag in event.tags.iter() {
            if matches!(tag.kind(), TagKind::Relays) {
                let vec = tag.clone().to_vec();
                for url in vec.into_iter().skip(1) {
                    if self.is_own_relay(&url) {
                        return true;
                    }
                }
            }
        }

        // Check clone tag
        for tag in event.tags.iter() {
            if matches!(tag.kind(), TagKind::Clone) {
                let vec = tag.clone().to_vec();
                for url in vec.into_iter().skip(1) {
                    if url.contains(&self.relay_domain) {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Check if a relay URL matches our relay.
    fn is_own_relay(&self, relay_url: &str) -> bool {
        relay_url.contains(&self.relay_domain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_url_to_relay_url_http() {
        let url = "http://127.0.0.1:8080/repo.git";
        let relay = SelfSubscriber::clone_url_to_relay_url(url);
        assert_eq!(relay, Some("ws://127.0.0.1:8080".to_string()));
    }

    #[test]
    fn test_clone_url_to_relay_url_https() {
        let url = "https://example.com/repo.git";
        let relay = SelfSubscriber::clone_url_to_relay_url(url);
        assert_eq!(relay, Some("wss://example.com".to_string()));
    }

    #[test]
    fn test_clone_url_to_relay_url_invalid() {
        let url = "not-a-valid-url";
        let relay = SelfSubscriber::clone_url_to_relay_url(url);
        assert_eq!(relay, None);
    }

    #[test]
    fn test_get_batch_window_default() {
        // Clear env var if set
        std::env::remove_var("NGIT_SYNC_BATCH_WINDOW_MS");
        let window = SelfSubscriber::get_batch_window();
        assert_eq!(window, Duration::from_secs(5));
    }

    #[test]
    fn test_get_batch_window_from_env() {
        std::env::set_var("NGIT_SYNC_BATCH_WINDOW_MS", "200");
        let window = SelfSubscriber::get_batch_window();
        assert_eq!(window, Duration::from_millis(200));
        std::env::remove_var("NGIT_SYNC_BATCH_WINDOW_MS");
    }
}
