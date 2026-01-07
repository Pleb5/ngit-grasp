//! Purgatory: In-memory holding area for events awaiting git data.
//!
//! Solves the "which arrives first?" problem where either nostr events or git pushes
//! can arrive in any order. Events and git data are held temporarily until their
//! counterpart arrives, at which point they can be processed together.
//!
//! ## Architecture
//!
//! - **In-memory only**: Data is lost on restart (acceptable per spec)
//! - **Thread-safe**: Uses DashMap for concurrent access from multiple handlers
//! - **Automatic expiry**: Entries expire after 30 minutes by default
//! - **Separate stores**: State events and PR events use different indexing strategies

mod helpers;
pub mod sync;
mod types;

pub use helpers::{can_apply_state, can_satisfy_state, extract_refs_from_state, get_unpushed_refs};
pub use types::{PrPurgatoryEntry, RefPair, RefUpdate, StatePurgatoryEntry};

use dashmap::DashMap;
use nostr_sdk::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use sync::SyncQueueEntry;

/// Default expiry duration for purgatory entries (30 minutes)
const DEFAULT_EXPIRY: Duration = Duration::from_secs(1800);

/// Default delay before syncing user-submitted events (3 minutes).
/// This gives time for the git push to arrive after the nostr event.
const DEFAULT_SYNC_DELAY: Duration = Duration::from_secs(180);

/// Delay for sync-triggered events (500ms).
/// Used for batching burst arrivals during negentropy sync.
const IMMEDIATE_SYNC_DELAY: Duration = Duration::from_millis(500);

/// Main purgatory structure holding events awaiting git data.
///
/// Provides thread-safe concurrent access to two separate stores:
/// - State events indexed by repository identifier
/// - PR events indexed by event ID
///
/// Also manages a sync queue for background git data fetching:
/// - Tracks identifiers that need syncing with backoff/debouncing
/// - Supports both user-submitted events (3min delay) and sync-triggered (500ms delay)
#[derive(Clone)]
pub struct Purgatory {
    /// State events (kind 30618) indexed by repository identifier.
    /// Multiple state events can wait for the same identifier (different maintainers).
    state_events: Arc<DashMap<String, Vec<StatePurgatoryEntry>>>,

    /// PR events (kind 1617/1618) or placeholders indexed by event ID (hex string).
    /// Event ID is from the 'e' tag in the PR event itself.
    pr_events: Arc<DashMap<String, PrPurgatoryEntry>>,

    /// Sync queue for background git data fetching.
    /// Maps repository identifier to sync queue entry with timing/backoff state.
    sync_queue: Arc<DashMap<String, SyncQueueEntry>>,

    git_data_path: PathBuf,
}

impl Purgatory {
    /// Create a new empty purgatory.
    pub fn new(git_data_path: impl Into<PathBuf>) -> Self {
        Self {
            state_events: Arc::new(DashMap::new()),
            pr_events: Arc::new(DashMap::new()),
            sync_queue: Arc::new(DashMap::new()),
            git_data_path: git_data_path.into(),
        }
    }

    /// Enqueue an identifier for background git data sync.
    ///
    /// This method is called when a state or PR event is added to purgatory.
    /// It uses debouncing to handle burst arrivals efficiently:
    /// - If the identifier is already queued, resets attempt_count and updates
    ///   next_attempt if the new delay would be sooner
    /// - If not queued, creates a new entry with the given delay
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to sync
    /// * `delay` - How long to wait before the first sync attempt
    pub fn enqueue_sync(&self, identifier: &str, delay: Duration) {
        self.sync_queue
            .entry(identifier.to_string())
            .and_modify(|entry| {
                // Reset attempt count and potentially update next_attempt
                entry.on_new_event(delay);
                tracing::debug!(
                    identifier = %identifier,
                    "Updated existing sync queue entry"
                );
            })
            .or_insert_with(|| {
                tracing::debug!(
                    identifier = %identifier,
                    delay_secs = delay.as_secs(),
                    "Added new sync queue entry"
                );
                SyncQueueEntry::new(delay)
            });
    }

    /// Enqueue an identifier for sync with the default delay (3 minutes).
    ///
    /// Used for user-submitted events where we expect a git push to follow.
    pub fn enqueue_sync_default(&self, identifier: &str) {
        self.enqueue_sync(identifier, DEFAULT_SYNC_DELAY);
    }

    /// Enqueue an identifier for immediate sync (500ms delay).
    ///
    /// Used for sync-triggered events (e.g., from negentropy) where we want
    /// to batch burst arrivals but start syncing quickly.
    pub fn enqueue_sync_immediate(&self, identifier: &str) {
        self.enqueue_sync(identifier, IMMEDIATE_SYNC_DELAY);
    }

    /// Check if there are pending events for an identifier.
    ///
    /// Returns true if purgatory has state events or PR events for this identifier.
    /// This is used by the sync loop to determine if an identifier should remain
    /// in the sync queue.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to check
    pub fn has_pending_events(&self, identifier: &str) -> bool {
        // Check state events
        if self
            .state_events
            .get(identifier)
            .map_or(false, |entries| !entries.is_empty())
        {
            return true;
        }

        // Check PR events - need to scan all entries since they're indexed by event_id
        // PR events reference repositories via `a` tags with format `30617:<owner_pubkey>:<identifier>`
        for entry in self.pr_events.iter() {
            if let Some(ref event) = entry.value().event {
                if Self::event_references_identifier(event, identifier) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if an event references a specific repository identifier.
    ///
    /// Looks for `a` tags with format `30617:<owner_pubkey>:<identifier>`.
    fn event_references_identifier(event: &Event, identifier: &str) -> bool {
        for tag in event.tags.iter() {
            let tag_vec = tag.clone().to_vec();
            if tag_vec.len() >= 2 && tag_vec[0] == "a" && tag_vec[1].starts_with("30617:") {
                // Format: 30617:<owner_pubkey>:<identifier>
                let parts: Vec<&str> = tag_vec[1].split(':').collect();
                if parts.len() >= 3 && parts[2] == identifier {
                    return true;
                }
            }
        }
        false
    }

    /// Get a reference to the sync queue (for the sync loop).
    pub fn sync_queue(&self) -> &Arc<DashMap<String, SyncQueueEntry>> {
        &self.sync_queue
    }

    /// Remove an identifier from the sync queue.
    ///
    /// Called when sync completes or the identifier no longer has pending events.
    pub fn remove_from_sync_queue(&self, identifier: &str) {
        self.sync_queue.remove(identifier);
    }

    /// Add a state event to purgatory.
    ///
    /// The event will expire after the default duration unless matched with git data.
    /// Multiple state events for the same identifier are allowed (from different authors).
    ///
    /// Automatically enqueues the identifier for background sync with the default delay
    /// (3 minutes), giving time for a git push to arrive after the nostr event.
    /// For sync-triggered events, the SyncManager calls `enqueue_sync_immediate` separately
    /// to override this delay.
    ///
    /// # Arguments
    /// * `event` - The state event (kind 30618) to hold
    /// * `identifier` - The repository identifier from the 'd' tag
    /// * `author` - The event author's public key
    pub fn add_state(&self, event: Event, identifier: String, author: PublicKey) {
        let now = Instant::now();
        let entry = StatePurgatoryEntry {
            event,
            identifier: identifier.clone(),
            author,
            created_at: now,
            expires_at: now + DEFAULT_EXPIRY,
        };

        self.state_events
            .entry(identifier.clone())
            .or_default()
            .push(entry);

        // Enqueue for background sync with default delay
        // (SyncManager will call enqueue_sync_immediate for sync-triggered events)
        self.enqueue_sync_default(&identifier);
    }

    /// Add a PR event to purgatory.
    ///
    /// The event will expire after the default duration unless matched with git data.
    ///
    /// Automatically enqueues the referenced repository identifier for background sync
    /// with the default delay (3 minutes), giving time for a git push to arrive.
    ///
    /// # Arguments
    /// * `event` - The PR event (kind 1617/1618) to hold
    /// * `event_id` - The event ID (hex string) from the 'e' tag
    /// * `commit` - The commit SHA from the 'c' tag
    pub fn add_pr(&self, event: Event, event_id: String, commit: String) {
        // Extract identifier from the event's `a` tag for sync enqueueing
        let identifier = crate::git::sync::extract_identifier_from_pr_event(&event);

        let now = Instant::now();
        let entry = PrPurgatoryEntry {
            event: Some(event),
            commit,
            created_at: now,
            expires_at: now + DEFAULT_EXPIRY,
        };

        self.pr_events.insert(event_id, entry);

        // Enqueue the identifier for background sync if we could extract it
        if let Some(id) = identifier {
            self.enqueue_sync_default(&id);
        }
    }

    /// Add a PR placeholder (git data arrived before PR event).
    ///
    /// Creates a placeholder entry waiting for the corresponding PR event.
    ///
    /// # Arguments
    /// * `event_id` - The expected event ID (from git ref name)
    /// * `commit` - The commit SHA that was pushed
    pub fn add_pr_placeholder(&self, event_id: String, commit: String) {
        let now = Instant::now();
        let entry = PrPurgatoryEntry {
            event: None, // Placeholder - no event yet
            commit,
            created_at: now,
            expires_at: now + DEFAULT_EXPIRY,
        };

        self.pr_events.insert(event_id, entry);
    }

    /// Find state events waiting for a specific repository identifier.
    ///
    /// Returns all state events (from all maintainers) waiting for git data
    /// matching this identifier.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to search for
    ///
    /// # Returns
    /// Vector of state events waiting for this identifier, or empty vec if none found
    pub fn find_state(&self, identifier: &str) -> Vec<StatePurgatoryEntry> {
        self.state_events
            .get(identifier)
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }

    /// Find a PR event or placeholder by event ID.
    ///
    /// # Arguments
    /// * `event_id` - The event ID to search for
    ///
    /// # Returns
    /// The PR entry if found, None otherwise
    pub fn find_pr(&self, event_id: &str) -> Option<PrPurgatoryEntry> {
        self.pr_events.get(event_id).map(|entry| entry.clone())
    }

    /// Find a PR placeholder specifically (git-data-first scenario).
    ///
    /// Returns the commit SHA only if a placeholder exists (entry with no event).
    /// Used to distinguish placeholders from actual PR events.
    ///
    /// # Arguments
    /// * `event_id` - The event ID to search for
    ///
    /// # Returns
    /// Some(commit_sha) if a placeholder exists, None if no entry or entry has an event
    pub fn find_pr_placeholder(&self, event_id: &str) -> Option<String> {
        self.pr_events.get(event_id).and_then(|entry| {
            if entry.event.is_none() {
                Some(entry.commit.clone())
            } else {
                None
            }
        })
    }

    /// Find all PR events for a specific repository identifier.
    ///
    /// PR events reference repositories via `a` tags with format `30617:<owner_pubkey>:<identifier>`.
    /// This function scans all PR entries and returns those that reference the given identifier.
    ///
    /// Note: This is a linear scan since PR events are indexed by event_id, not by identifier.
    /// For repositories with many PR events, this could be optimized with a secondary index.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to search for
    ///
    /// # Returns
    /// Vector of PR purgatory entries that reference this identifier
    pub fn find_prs_for_identifier(&self, identifier: &str) -> Vec<PrPurgatoryEntry> {
        self.pr_events
            .iter()
            .filter(|entry| {
                if let Some(ref event) = entry.value().event {
                    Self::event_references_identifier(event, identifier)
                } else {
                    false
                }
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Remove a state event from purgatory.
    ///
    /// Removes all entries for the given identifier.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to remove
    pub fn remove_state(&self, identifier: &str) {
        self.state_events.remove(identifier);
    }

    /// Remove a specific state event by comparing the full event.
    ///
    /// This allows removing a single state event while leaving others
    /// for the same identifier intact.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier
    /// * `event_id` - The specific event ID to remove
    pub fn remove_state_event(&self, identifier: &str, event_id: &EventId) {
        if let Some(mut entries) = self.state_events.get_mut(identifier) {
            entries.retain(|entry| entry.event.id != *event_id);
            if entries.is_empty() {
                drop(entries); // Release lock before removal
                self.state_events.remove(identifier);
            }
        }
    }

    /// Find state events that could be satisfied by ref updates.
    ///
    /// Returns state events waiting for this identifier where applying the
    /// ref updates to local state results in exactly the declared state.
    /// Uses late-binding ref extraction at git push time.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to search for
    /// * `pushed_updates` - Ref updates in the current push operation
    /// * `local_refs` - Refs already existing locally (ref_name -> SHA)
    ///
    /// # Returns
    /// Vector of events that can be satisfied by the push
    pub fn find_matching_states(
        &self,
        identifier: &str,
        pushed_updates: &[RefUpdate],
        local_refs: &std::collections::HashMap<String, String>,
    ) -> Vec<Event> {
        self.state_events
            .get(identifier)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| {
                        helpers::can_satisfy_state(&entry.event, pushed_updates, local_refs)
                    })
                    .map(|entry| entry.event.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Extend expiry for state events about to be processed.
    ///
    /// Ensures entries have at least `duration` remaining on their timer.
    /// Sets expiry to max(current_expiry, now + duration).
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier
    /// * `event_ids` - Event IDs to extend expiry for
    /// * `duration` - Minimum duration to guarantee from now
    pub fn extend_expiry(&self, identifier: &str, event_ids: &[EventId], duration: Duration) {
        if let Some(mut entries) = self.state_events.get_mut(identifier) {
            let now = Instant::now();
            let new_expiry = now + duration;

            for entry in entries.iter_mut() {
                if event_ids.contains(&entry.event.id) {
                    // Set to max of current expiry and new expiry
                    if entry.expires_at < new_expiry {
                        entry.expires_at = new_expiry;
                    }
                }
            }
        }
    }

    /// Remove a PR event or placeholder from purgatory.
    ///
    /// # Arguments
    /// * `event_id` - The event ID to remove
    pub fn remove_pr(&self, event_id: &str) {
        self.pr_events.remove(event_id);
    }

    /// Get all event IDs currently stored in purgatory.
    ///
    /// Returns a HashSet of all event IDs for both state events and PR events
    /// held in purgatory. Useful for negentropy sync to avoid fetching events
    /// that are already in purgatory awaiting git data.
    ///
    /// # Returns
    /// HashSet of event IDs (as EventId) for all events in purgatory
    pub fn event_ids(&self) -> HashSet<EventId> {
        let mut ids = HashSet::new();

        // Collect state event IDs
        for entry in self.state_events.iter() {
            for state_entry in entry.value().iter() {
                ids.insert(state_entry.event.id);
            }
        }

        // Collect PR event IDs (only actual events, not placeholders)
        for entry in self.pr_events.iter() {
            if let Some(ref event) = entry.value().event {
                ids.insert(event.id);
            }
        }

        ids
    }

    /// Get all PR placeholder event IDs (git-data-first entries without events).
    ///
    /// Returns event IDs for entries where git data arrived before the PR event.
    /// These correspond to `refs/nostr/<event-id>` refs that should be cleaned up
    /// on shutdown since they don't have corresponding events.
    ///
    /// # Returns
    /// Vector of event IDs (hex strings) for placeholder entries
    pub fn get_placeholder_event_ids(&self) -> Vec<String> {
        self.pr_events
            .iter()
            .filter_map(|entry| {
                if entry.value().event.is_none() {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Remove expired entries from purgatory.
    ///
    /// Should be called periodically (every 60 seconds) by background task to clean up
    /// entries that have exceeded their expiry deadline.
    ///
    /// # Returns
    /// Tuple of (num_state_removed, num_pr_removed)
    pub fn cleanup(&self) -> (usize, usize) {
        let now = Instant::now();
        let mut state_removed = 0;

        // Remove expired state events
        self.state_events.retain(|_, entries| {
            let original_len = entries.len();
            entries.retain(|entry| entry.expires_at > now);
            state_removed += original_len - entries.len();
            !entries.is_empty()
        });

        // Remove expired PR events
        let expired_prs: Vec<String> = self
            .pr_events
            .iter()
            .filter(|entry| entry.value().expires_at <= now)
            .map(|entry| entry.key().clone())
            .collect();

        let pr_removed = expired_prs.len();
        for event_id in expired_prs {
            self.pr_events.remove(&event_id);
        }

        (state_removed, pr_removed)
    }

    /// Remove expired entries from purgatory (legacy method).
    ///
    /// # Returns
    /// Total number of entries removed (state + PR events)
    #[deprecated(since = "0.1.0", note = "Use cleanup() instead for separate counts")]
    pub fn remove_expired(&self) -> usize {
        let (state, pr) = self.cleanup();
        state + pr
    }

    /// Get current count of entries in purgatory.
    ///
    /// # Returns
    /// Tuple of (state_event_count, pr_event_count)
    pub fn count(&self) -> (usize, usize) {
        let state_count: usize = self.state_events.iter().map(|e| e.value().len()).sum();
        let pr_count = self.pr_events.len();
        (state_count, pr_count)
    }

    /// Clear all entries from purgatory (for testing).
    #[cfg(test)]
    pub fn clear(&self) {
        self.state_events.clear();
        self.pr_events.clear();
        self.sync_queue.clear();
    }

    /// Get the current size of the sync queue (for testing/metrics).
    pub fn sync_queue_size(&self) -> usize {
        self.sync_queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_purgatory_creation() {
        let purgatory = Purgatory::new(PathBuf::new());
        let (state_count, pr_count) = purgatory.count();
        assert_eq!(state_count, 0);
        assert_eq!(pr_count, 0);
    }

    #[test]
    fn test_purgatory_count() {
        let purgatory = Purgatory::new(PathBuf::new());

        // Add some test data
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test")
            .sign_with_keys(&keys)
            .unwrap();

        purgatory.add_state(event.clone(), "test-repo".to_string(), keys.public_key());
        purgatory.add_pr(event, "test-event-id".to_string(), "abc123".to_string());

        let (state_count, pr_count) = purgatory.count();
        assert_eq!(state_count, 1);
        assert_eq!(pr_count, 1);
    }

    #[test]
    fn test_enqueue_sync_debounces_rapid_calls() {
        let purgatory = Purgatory::new(PathBuf::new());

        // First call - creates entry
        purgatory.enqueue_sync("test-repo", Duration::from_secs(60));
        assert_eq!(purgatory.sync_queue_size(), 1);

        // Simulate some sync attempts
        if let Some(mut entry) = purgatory.sync_queue.get_mut("test-repo") {
            entry.attempt_count = 3;
            entry.next_attempt = Instant::now() + Duration::from_secs(120);
        }

        // Second call with shorter delay - should reset attempt_count and update next_attempt
        purgatory.enqueue_sync("test-repo", Duration::from_secs(10));

        // Should still be only one entry (debounced)
        assert_eq!(purgatory.sync_queue_size(), 1);

        // Attempt count should be reset
        let entry = purgatory.sync_queue.get("test-repo").unwrap();
        assert_eq!(entry.attempt_count, 0, "attempt_count should be reset to 0");

        // next_attempt should be updated to the sooner time (within tolerance)
        let expected_max = Instant::now() + Duration::from_secs(10) + Duration::from_millis(100);
        assert!(
            entry.next_attempt <= expected_max,
            "next_attempt should be updated to sooner time"
        );
    }

    #[test]
    fn test_has_pending_events_with_state_events() {
        let purgatory = Purgatory::new(PathBuf::new());
        let keys = Keys::generate();

        // No events initially
        assert!(!purgatory.has_pending_events("test-repo"));

        // Add a state event
        let event = EventBuilder::text_note("state")
            .sign_with_keys(&keys)
            .unwrap();
        purgatory.add_state(event, "test-repo".to_string(), keys.public_key());

        // Now should have pending events
        assert!(purgatory.has_pending_events("test-repo"));

        // Different identifier should not have pending events
        assert!(!purgatory.has_pending_events("other-repo"));
    }

    #[test]
    fn test_has_pending_events_with_pr_events() {
        use nostr_sdk::{Kind, Tag, TagKind};

        let purgatory = Purgatory::new(PathBuf::new());
        let keys = Keys::generate();

        // No events initially
        assert!(!purgatory.has_pending_events("test-repo"));

        // Add a PR event with `a` tag referencing the repository
        let tags = vec![Tag::custom(
            TagKind::Custom("a".into()),
            vec!["30617:abc123def456:test-repo".to_string()],
        )];

        let event = EventBuilder::new(Kind::from(1618), "PR content")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();

        purgatory.add_pr(event, "pr-event-id".to_string(), "commit123".to_string());

        // Now should have pending events for test-repo
        assert!(purgatory.has_pending_events("test-repo"));

        // Different identifier should not have pending events
        assert!(!purgatory.has_pending_events("other-repo"));
    }

    #[test]
    fn test_remove_from_sync_queue() {
        let purgatory = Purgatory::new(PathBuf::new());

        purgatory.enqueue_sync("repo-1", Duration::from_secs(60));
        purgatory.enqueue_sync("repo-2", Duration::from_secs(60));
        assert_eq!(purgatory.sync_queue_size(), 2);

        purgatory.remove_from_sync_queue("repo-1");
        assert_eq!(purgatory.sync_queue_size(), 1);

        // repo-1 should be gone
        assert!(purgatory.sync_queue.get("repo-1").is_none());
        // repo-2 should still be there
        assert!(purgatory.sync_queue.get("repo-2").is_some());
    }

    #[test]
    fn test_enqueue_sync_default_and_immediate() {
        let purgatory = Purgatory::new(PathBuf::new());

        // Test default delay (3 minutes)
        purgatory.enqueue_sync_default("repo-default");
        let entry = purgatory.sync_queue.get("repo-default").unwrap();
        let expected_min = Instant::now() + Duration::from_secs(170); // ~3min minus tolerance
        let expected_max = Instant::now() + Duration::from_secs(190); // ~3min plus tolerance
        assert!(
            entry.next_attempt >= expected_min && entry.next_attempt <= expected_max,
            "Default delay should be ~180 seconds"
        );
        drop(entry);

        // Test immediate delay (500ms)
        purgatory.enqueue_sync_immediate("repo-immediate");
        let entry = purgatory.sync_queue.get("repo-immediate").unwrap();
        let expected_max = Instant::now() + Duration::from_millis(600);
        assert!(
            entry.next_attempt <= expected_max,
            "Immediate delay should be ~500ms"
        );
    }
}

#[test]
fn test_pr_event_vs_placeholder() {
    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();
    let event = EventBuilder::text_note("test PR")
        .sign_with_keys(&keys)
        .unwrap();

    // Add a PR event with actual event
    purgatory.add_pr(
        event.clone(),
        "event-id-1".to_string(),
        "commit-abc".to_string(),
    );

    // Add a placeholder (no event)
    purgatory.add_pr_placeholder("event-id-2".to_string(), "commit-def".to_string());

    // find_pr should find both
    assert!(purgatory.find_pr("event-id-1").is_some());
    assert!(purgatory.find_pr("event-id-2").is_some());

    // find_pr_placeholder should only find the placeholder
    assert!(purgatory.find_pr_placeholder("event-id-1").is_none());
    assert_eq!(
        purgatory.find_pr_placeholder("event-id-2"),
        Some("commit-def".to_string())
    );
}

#[test]
fn test_pr_placeholder_creation_and_retrieval() {
    let purgatory = Purgatory::new(PathBuf::new());

    // Add a placeholder
    purgatory.add_pr_placeholder("placeholder-id".to_string(), "commit-123".to_string());

    // Should be findable by find_pr
    let entry = purgatory.find_pr("placeholder-id");
    assert!(entry.is_some());
    let entry = entry.unwrap();
    assert!(entry.event.is_none()); // No event yet
    assert_eq!(entry.commit, "commit-123");

    // Should be findable by find_pr_placeholder
    let commit = purgatory.find_pr_placeholder("placeholder-id");
    assert_eq!(commit, Some("commit-123".to_string()));
}

#[test]
fn test_cleanup_removes_expired_entries() {
    use std::time::Duration;

    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    // Create events
    let state_event = EventBuilder::text_note("state event")
        .sign_with_keys(&keys)
        .unwrap();
    let pr_event = EventBuilder::text_note("pr event")
        .sign_with_keys(&keys)
        .unwrap();

    // Add entries to purgatory
    purgatory.add_state(
        state_event.clone(),
        "test-repo".to_string(),
        keys.public_key(),
    );
    purgatory.add_pr(pr_event, "pr-123".to_string(), "commit-abc".to_string());
    purgatory.add_pr_placeholder("pr-456".to_string(), "commit-def".to_string());

    // Verify entries are there
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 1);
    assert_eq!(pr_count, 2);

    // Manually expire entries by modifying their expiry time
    // (This is a bit hacky but needed for testing without waiting 30 minutes)
    if let Some(mut entries) = purgatory.state_events.get_mut("test-repo") {
        for entry in entries.iter_mut() {
            entry.expires_at = Instant::now() - Duration::from_secs(1);
        }
    }

    // Expire PR events
    for mut entry in purgatory.pr_events.iter_mut() {
        entry.value_mut().expires_at = Instant::now() - Duration::from_secs(1);
    }

    // Run cleanup
    let (state_removed, pr_removed) = purgatory.cleanup();

    // Verify counts
    assert_eq!(state_removed, 1);
    assert_eq!(pr_removed, 2);

    // Verify entries are gone
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 0);
    assert_eq!(pr_count, 0);
}

#[test]
fn test_cleanup_preserves_non_expired_entries() {
    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    let state_event = EventBuilder::text_note("state event")
        .sign_with_keys(&keys)
        .unwrap();
    let pr_event = EventBuilder::text_note("pr event")
        .sign_with_keys(&keys)
        .unwrap();

    // Add fresh entries
    purgatory.add_state(state_event, "test-repo".to_string(), keys.public_key());
    purgatory.add_pr(pr_event, "pr-123".to_string(), "commit-abc".to_string());

    // Run cleanup
    let (state_removed, pr_removed) = purgatory.cleanup();

    // Nothing should be removed
    assert_eq!(state_removed, 0);
    assert_eq!(pr_removed, 0);

    // Verify entries are still there
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 1);
    assert_eq!(pr_count, 1);
}

#[test]
fn test_cleanup_mixed_expired_and_fresh() {
    use std::time::Duration;

    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    // Add multiple state events for same repo
    let event1 = EventBuilder::text_note("event1")
        .sign_with_keys(&keys)
        .unwrap();
    let event2 = EventBuilder::text_note("event2")
        .sign_with_keys(&keys)
        .unwrap();

    purgatory.add_state(event1, "test-repo".to_string(), keys.public_key());
    purgatory.add_state(event2, "test-repo".to_string(), keys.public_key());

    // Expire only the first one
    if let Some(mut entries) = purgatory.state_events.get_mut("test-repo") {
        if let Some(entry) = entries.get_mut(0) {
            entry.expires_at = Instant::now() - Duration::from_secs(1);
        }
    }

    // Add PR events
    let pr1 = EventBuilder::text_note("pr1")
        .sign_with_keys(&keys)
        .unwrap();
    let pr2 = EventBuilder::text_note("pr2")
        .sign_with_keys(&keys)
        .unwrap();

    purgatory.add_pr(pr1, "pr-1".to_string(), "commit-1".to_string());
    purgatory.add_pr(pr2, "pr-2".to_string(), "commit-2".to_string());

    // Expire only first PR
    if let Some(mut entry) = purgatory.pr_events.get_mut("pr-1") {
        entry.expires_at = Instant::now() - Duration::from_secs(1);
    }

    // Run cleanup
    let (state_removed, pr_removed) = purgatory.cleanup();

    // One of each should be removed
    assert_eq!(state_removed, 1);
    assert_eq!(pr_removed, 1);

    // Verify remaining counts
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 1); // One state event remains
    assert_eq!(pr_count, 1); // One PR event remains
}

#[test]
fn test_remove_expired_legacy_method() {
    use std::time::Duration;

    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    let state_event = EventBuilder::text_note("state")
        .sign_with_keys(&keys)
        .unwrap();
    let pr_event = EventBuilder::text_note("pr").sign_with_keys(&keys).unwrap();

    purgatory.add_state(state_event, "repo".to_string(), keys.public_key());
    purgatory.add_pr(pr_event, "pr-id".to_string(), "commit".to_string());

    // Expire both
    if let Some(mut entries) = purgatory.state_events.get_mut("repo") {
        for entry in entries.iter_mut() {
            entry.expires_at = Instant::now() - Duration::from_secs(1);
        }
    }
    for mut entry in purgatory.pr_events.iter_mut() {
        entry.value_mut().expires_at = Instant::now() - Duration::from_secs(1);
    }

    // Test legacy method returns total
    #[allow(deprecated)]
    let total = purgatory.remove_expired();
    assert_eq!(total, 2); // 1 state + 1 PR
}
