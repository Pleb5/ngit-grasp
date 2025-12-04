//! Subscription Manager for GRASP-02 Phase 4: Dynamic Subscriptions
//!
//! Manages dynamic subscription updates per connection, including:
//! - Tracking subscribed announcements and events
//! - Adding new subscriptions when announcements/PRs arrive
//! - Consolidating filters when count exceeds threshold
//! - Preventing duplicate subscriptions
//!
//! ## Dynamic Subscription Strategy
//!
//! Initial: Layer 1 (announcements)
//!   ↓ (announcement received)
//! Add: Layer 2 (events for that repo)
//!   ↓ (PR/Issue received)
//! Add: Layer 3 (events for that PR/Issue)
//!   ↓ (filter count > 150)
//! Consolidate: Back to Layer 1 only

use std::collections::HashSet;
use std::sync::Arc;

use nostr_sdk::prelude::*;

use super::filter::FilterService;

/// Maximum number of filters before consolidation is triggered
const CONSOLIDATION_THRESHOLD: usize = 150;

/// Kind 30617 - Repository Announcement (NIP-34)
const KIND_REPOSITORY_ANNOUNCEMENT: u16 = 30617;

/// Kind 30618 - Maintainer List (NIP-34)
const KIND_MAINTAINER_LIST: u16 = 30618;

/// Manages subscriptions for a single relay connection
///
/// Tracks which announcements and events have been subscribed to,
/// and handles dynamic subscription updates as new events arrive.
#[derive(Debug)]
pub struct SubscriptionManager {
    /// Event IDs of announcements we've subscribed to (for Layer 2)
    subscribed_announcements: HashSet<String>,
    /// Event IDs of PRs/Issues we've subscribed to (for Layer 3)
    subscribed_events: HashSet<String>,
    /// Whether we've consolidated back to Layer 1 only
    is_consolidated: bool,
    /// FilterService for building filters
    filter_service: Arc<FilterService>,
    /// Remote relay domain for Layer 2 filters
    remote_domain: String,
}

impl SubscriptionManager {
    /// Create a new SubscriptionManager
    ///
    /// # Arguments
    /// * `filter_service` - FilterService for building subscription filters
    /// * `remote_domain` - The domain of the remote relay we're syncing from
    pub fn new(filter_service: Arc<FilterService>, remote_domain: String) -> Self {
        Self {
            subscribed_announcements: HashSet::new(),
            subscribed_events: HashSet::new(),
            is_consolidated: false,
            filter_service,
            remote_domain,
        }
    }

    /// Add an announcement and return new filters to subscribe to
    ///
    /// When a new announcement (kind 30617/30618) arrives, this creates
    /// Layer 2 filters to subscribe to events for that repository.
    ///
    /// Returns `Some(filters)` if this is a new announcement, `None` if already subscribed.
    pub fn add_announcement(&mut self, event: &Event) -> Option<Vec<Filter>> {
        let event_id = event.id.to_hex();

        // Check if already subscribed or consolidated
        if self.is_consolidated || self.subscribed_announcements.contains(&event_id) {
            return None;
        }

        // Add to tracked announcements
        self.subscribed_announcements.insert(event_id);

        // Build Layer 2 filters for this announcement
        // Layer 2 filters target events with 'a' tags pointing to this repo
        let filters = self.build_layer2_filter_for_announcement(event);

        if filters.is_empty() {
            None
        } else {
            Some(filters)
        }
    }

    /// Add a PR/Issue/Patch event and return new filters to subscribe to
    ///
    /// When a new PR (kind 1617), Issue (kind 1621), or Patch (kind 1622) arrives,
    /// this creates Layer 3 filters to subscribe to related events.
    ///
    /// Returns `Some(filters)` if this is a new event, `None` if already subscribed.
    pub fn add_event(&mut self, event: &Event) -> Option<Vec<Filter>> {
        let event_id = event.id.to_hex();

        // Check if already subscribed or consolidated
        if self.is_consolidated || self.subscribed_events.contains(&event_id) {
            return None;
        }

        // Add to tracked events
        self.subscribed_events.insert(event_id.clone());

        // Build Layer 3 filter for this event
        // Layer 3 filters target events with 'e' tags pointing to this event
        let filter = Filter::new().custom_tag(
            SingleLetterTag::lowercase(Alphabet::E),
            event_id,
        );

        Some(vec![filter])
    }

    /// Check if consolidation is needed
    ///
    /// Returns true if the total filter count exceeds the threshold (150).
    pub fn should_consolidate(&self) -> bool {
        !self.is_consolidated && self.get_filter_count() > CONSOLIDATION_THRESHOLD
    }

    /// Consolidate all subscriptions back to Layer 1 only
    ///
    /// Clears all tracked announcements and events, marks as consolidated,
    /// and returns the Layer 1 filters to re-subscribe to.
    pub fn consolidate(&mut self) -> Vec<Filter> {
        tracing::info!(
            "Consolidating subscriptions: {} announcements, {} events -> Layer 1 only",
            self.subscribed_announcements.len(),
            self.subscribed_events.len()
        );

        // Clear tracked subscriptions
        self.subscribed_announcements.clear();
        self.subscribed_events.clear();
        self.is_consolidated = true;

        // Return Layer 1 filters
        self.filter_service.get_layer1_filters()
    }

    /// Get the total count of active filters
    ///
    /// Counts 1 filter per announcement (Layer 2) + 1 filter per event (Layer 3),
    /// plus the base Layer 1 filter count.
    pub fn get_filter_count(&self) -> usize {
        if self.is_consolidated {
            // When consolidated, we only have Layer 1 filters
            1
        } else {
            // Layer 1 (1) + Layer 2 (announcements) + Layer 3 (events)
            1 + self.subscribed_announcements.len() + self.subscribed_events.len()
        }
    }

    /// Check if an announcement has been subscribed to
    pub fn has_announcement(&self, event_id: &str) -> bool {
        self.subscribed_announcements.contains(event_id)
    }

    /// Check if an event has been subscribed to
    pub fn has_event(&self, event_id: &str) -> bool {
        self.subscribed_events.contains(event_id)
    }

    /// Check if subscriptions have been consolidated
    pub fn is_consolidated(&self) -> bool {
        self.is_consolidated
    }

    /// Get the count of subscribed announcements
    pub fn announcement_count(&self) -> usize {
        self.subscribed_announcements.len()
    }

    /// Get the count of subscribed events
    pub fn event_count(&self) -> usize {
        self.subscribed_events.len()
    }

    /// Build Layer 2 filter for a specific announcement event
    ///
    /// Creates a filter with an 'a' tag pointing to the announcement's coordinates.
    fn build_layer2_filter_for_announcement(&self, event: &Event) -> Vec<Filter> {
        // Extract the d tag (identifier) from the event
        let identifier = event.tags.iter().find_map(|tag| {
            let tag_vec = tag.clone().to_vec();
            if tag_vec.len() >= 2 && tag_vec[0] == "d" {
                Some(tag_vec[1].clone())
            } else {
                None
            }
        });

        let identifier = match identifier {
            Some(id) => id,
            None => {
                tracing::warn!(
                    "Announcement {} has no 'd' tag, cannot build Layer 2 filter",
                    event.id.to_hex()
                );
                return Vec::new();
            }
        };

        // Determine the kind for the coordinate
        let kind = event.kind.as_u16();
        if kind != KIND_REPOSITORY_ANNOUNCEMENT && kind != KIND_MAINTAINER_LIST {
            tracing::warn!(
                "Event {} is not an announcement (kind {}), cannot build Layer 2 filter",
                event.id.to_hex(),
                kind
            );
            return Vec::new();
        }

        // Build the addressable coordinate: kind:pubkey:identifier
        let coord = format!("{}:{}:{}", kind, event.pubkey.to_hex(), identifier);

        // Create filter with 'a' tag for this coordinate
        let filter = Filter::new().custom_tag(
            SingleLetterTag::lowercase(Alphabet::A),
            coord,
        );

        vec![filter]
    }

    /// Check if an event kind is an announcement kind
    pub fn is_announcement_kind(kind: u16) -> bool {
        kind == KIND_REPOSITORY_ANNOUNCEMENT || kind == KIND_MAINTAINER_LIST
    }

    /// Check if an event kind is a PR/Issue/Patch kind that should trigger Layer 3
    pub fn is_pr_issue_kind(kind: u16) -> bool {
        matches!(
            kind,
            1617 | // Patch proposal (NIP-34)
            1618 | // PR
            1619 | // PR Update
            1621 | // Issue
            1622   // Reply
        )
    }
}

#[cfg(test)]
mod tests {
    use super::SubscriptionManager;

    #[test]
    fn test_is_announcement_kind() {
        assert!(SubscriptionManager::is_announcement_kind(30617));
        assert!(SubscriptionManager::is_announcement_kind(30618));
        assert!(!SubscriptionManager::is_announcement_kind(1));
        assert!(!SubscriptionManager::is_announcement_kind(1617));
    }

    #[test]
    fn test_is_pr_issue_kind() {
        assert!(SubscriptionManager::is_pr_issue_kind(1617));
        assert!(SubscriptionManager::is_pr_issue_kind(1618));
        assert!(SubscriptionManager::is_pr_issue_kind(1619));
        assert!(SubscriptionManager::is_pr_issue_kind(1621));
        assert!(SubscriptionManager::is_pr_issue_kind(1622));
        assert!(!SubscriptionManager::is_pr_issue_kind(30617));
        assert!(!SubscriptionManager::is_pr_issue_kind(1));
    }
}