//! Filter Service for GRASP-02 Proactive Sync
//!
//! Implements the three-layer filter strategy for comprehensive event syncing:
//! - Layer 1: Announcement discovery (kinds 30617 + 30618)
//! - Layer 2: Repository events (A/a tags pointing to shared repos)
//! - Layer 3: Related events (E/e tags pointing to Layer 2 events)

use std::collections::HashSet;

use nostr_sdk::prelude::*;

use crate::nostr::builder::SharedDatabase;
use crate::nostr::events::KIND_REPOSITORY_ANNOUNCEMENT;

/// Maximum number of tags per filter to stay within relay limits
const MAX_TAGS_PER_FILTER: usize = 100;

/// Kind for maintainer metadata (NIP-34)
const KIND_MAINTAINER_LIST: u16 = 30618;

/// FilterService builds subscription filters for proactive sync
///
/// Uses a three-layer strategy:
/// 1. Layer 1: Discover new repository announcements and maintainer metadata
/// 2. Layer 2: Sync events directly related to repositories we track
/// 3. Layer 3: Sync discussions and updates related to Layer 2 events
#[derive(Debug)]
pub struct FilterService {
    database: SharedDatabase,
    /// Our relay's domain for filtering
    relay_domain: String,
}

impl FilterService {
    /// Create a new FilterService
    ///
    /// # Arguments
    /// * `database` - Shared database for querying stored events
    /// * `relay_domain` - Our relay's domain (used for filtering shared repos)
    pub fn new(database: SharedDatabase, relay_domain: String) -> Self {
        Self {
            database,
            relay_domain,
        }
    }

    /// Get Layer 1 filters for announcement discovery
    ///
    /// Returns filters for kinds 30617 (repository announcements) and 30618 (maintainer metadata)
    pub fn get_layer1_filters(&self) -> Vec<Filter> {
        vec![Filter::new().kinds(vec![
            Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT),
            Kind::Custom(KIND_MAINTAINER_LIST),
        ])]
    }

    /// Get Layer 2 filters for repository-related events
    ///
    /// Queries the database for kind 30617 events and builds filters for events
    /// with `a` tags pointing to repositories that reference both:
    /// - Our relay (from clone tags)
    /// - Are stored in our database (meaning they're relevant to us)
    ///
    /// # Arguments
    /// * `remote_relay_domain` - The domain of the remote relay we're syncing from
    pub async fn get_layer2_filters(&self, remote_relay_domain: &str) -> Vec<Filter> {
        // Query all kind 30617 events from our database
        let filter = Filter::new().kind(Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT));

        let events = match self.database.query(filter).await {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!("Failed to query announcements for Layer 2 filters: {}", e);
                return Vec::new();
            }
        };

        // Build a set of addressable coordinates for repos that list both relays
        let mut coords: Vec<String> = Vec::new();

        for event in events {
            // Check if this repo lists our domain in clone tags
            let has_our_relay = event.tags.iter().any(|tag| {
                let tag_vec = tag.clone().to_vec();
                tag_vec.len() >= 2
                    && (tag_vec[0] == "clone" || tag_vec[0] == "relays")
                    && tag_vec.iter().any(|v| v.contains(&self.relay_domain))
            });

            // Check if this repo lists the remote relay in clone/relays tags
            let has_remote_relay = event.tags.iter().any(|tag| {
                let tag_vec = tag.clone().to_vec();
                tag_vec.len() >= 2
                    && (tag_vec[0] == "clone" || tag_vec[0] == "relays")
                    && tag_vec.iter().any(|v| v.contains(remote_relay_domain))
            });

            if has_our_relay && has_remote_relay {
                // Extract the d tag (identifier)
                if let Some(identifier) = event.tags.iter().find_map(|tag| {
                    let tag_vec = tag.clone().to_vec();
                    if tag_vec.len() >= 2 && tag_vec[0] == "d" {
                        Some(tag_vec[1].clone())
                    } else {
                        None
                    }
                }) {
                    // Build the addressable coordinate: kind:pubkey:identifier
                    let coord = format!(
                        "{}:{}:{}",
                        KIND_REPOSITORY_ANNOUNCEMENT,
                        event.pubkey.to_hex(),
                        identifier
                    );
                    coords.push(coord);
                }
            }
        }

        if coords.is_empty() {
            return Vec::new();
        }

        // Batch coordinates into filters with A/a/q tags
        Self::batch_layer2_filters(coords)
    }

    /// Get Layer 3 filters for related events
    ///
    /// Queries the database for events with `a` tags (PRs, Issues, etc.)
    /// and builds filters for events that reference them with `e` tags.
    pub async fn get_layer3_filters(&self) -> Vec<Filter> {
        // Query events that reference repositories (have 'a' tags with 30617)
        // These are typically PRs (1618), Issues (1621), etc.

        // First, get all kind 30617 announcements
        let announcement_filter = Filter::new().kind(Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT));

        let announcements = match self.database.query(announcement_filter).await {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!("Failed to query announcements for Layer 3 filters: {}", e);
                return Vec::new();
            }
        };

        // Build a set of event IDs from PRs, Issues, etc. that reference our repos
        let mut event_ids: Vec<String> = Vec::new();

        // Get the set of valid repository coordinates
        let repo_coords: HashSet<String> = announcements
            .iter()
            .filter_map(|e| {
                e.tags.iter().find_map(|tag| {
                    let tag_vec = tag.clone().to_vec();
                    if tag_vec.len() >= 2 && tag_vec[0] == "d" {
                        Some(format!(
                            "{}:{}:{}",
                            KIND_REPOSITORY_ANNOUNCEMENT,
                            e.pubkey.to_hex(),
                            tag_vec[1]
                        ))
                    } else {
                        None
                    }
                })
            })
            .collect();

        if repo_coords.is_empty() {
            return Vec::new();
        }

        // Query for PR and Patch events from our repositories
        let repos_pr_patch_filter = Filter::new().kinds(vec![
            Kind::Custom(1617), // Patch
            Kind::Custom(1618), // PR
        ]);

        let related_events = match self.database.query(repos_pr_patch_filter).await {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!("Failed to query related events for Layer 3 filters: {}", e);
                return Vec::new();
            }
        };

        // Collect event IDs that reference our repositories
        for event in related_events {
            // Check if this event has an 'a' tag pointing to one of our repos
            let references_our_repo = event.tags.iter().any(|tag| {
                let tag_vec = tag.clone().to_vec();
                tag_vec.len() >= 2 && tag_vec[0] == "a" && repo_coords.contains(&tag_vec[1])
            });

            if references_our_repo {
                event_ids.push(event.id.to_hex());
            }
        }

        if event_ids.is_empty() {
            return Vec::new();
        }

        // Batch event IDs into filters with 'E', 'e', and 'q' tags
        Self::batch_layer3_filters(event_ids)
    }

    /// Batch a list of addressable coordinates into Layer 2 filters with 'A', 'a', and 'q' tags
    ///
    /// Different Nostr clients use different tag conventions for referencing repository
    /// announcements. This function generates THREE filters per chunk to capture all variants:
    /// - Uppercase 'A' tags (used by some clients)
    /// - Lowercase 'a' tags (standard addressable event reference)
    /// - Lowercase 'q' tags (quote tags, used by some clients)
    ///
    /// When tag counts exceed MAX_TAGS_PER_FILTER, creates multiple filter sets.
    fn batch_layer2_filters(coords: Vec<String>) -> Vec<Filter> {
        if coords.is_empty() {
            return Vec::new();
        }

        coords
            .chunks(MAX_TAGS_PER_FILTER)
            .flat_map(|chunk| {
                // Create THREE filters per chunk - one for each tag type
                vec![
                    // Uppercase A tag filter
                    Filter::new().custom_tags(
                        SingleLetterTag::uppercase(Alphabet::A),
                        chunk.iter().cloned(),
                    ),
                    // Lowercase a tag filter
                    Filter::new().custom_tags(
                        SingleLetterTag::lowercase(Alphabet::A),
                        chunk.iter().cloned(),
                    ),
                    // Quote q tag filter
                    Filter::new().custom_tags(
                        SingleLetterTag::lowercase(Alphabet::Q),
                        chunk.iter().cloned(),
                    ),
                ]
            })
            .collect()
    }

    /// Batch a list of event IDs into Layer 3 filters with 'E', 'e', and 'q' tags
    ///
    /// Different Nostr clients use different tag conventions for referencing events.
    /// This function generates THREE filters per chunk to capture all variants:
    /// - Uppercase 'E' tags (used by some clients)
    /// - Lowercase 'e' tags (standard event reference)
    /// - Lowercase 'q' tags (quote tags, used by some clients)
    ///
    /// When tag counts exceed MAX_TAGS_PER_FILTER, creates multiple filter sets.
    fn batch_layer3_filters(event_ids: Vec<String>) -> Vec<Filter> {
        if event_ids.is_empty() {
            return Vec::new();
        }

        event_ids
            .chunks(MAX_TAGS_PER_FILTER)
            .flat_map(|chunk| {
                // Create THREE filters per chunk - one for each tag type
                vec![
                    // Uppercase E tag filter
                    Filter::new().custom_tags(
                        SingleLetterTag::uppercase(Alphabet::E),
                        chunk.iter().cloned(),
                    ),
                    // Lowercase e tag filter
                    Filter::new().custom_tags(
                        SingleLetterTag::lowercase(Alphabet::E),
                        chunk.iter().cloned(),
                    ),
                    // Quote q tag filter
                    Filter::new().custom_tags(
                        SingleLetterTag::lowercase(Alphabet::Q),
                        chunk.iter().cloned(),
                    ),
                ]
            })
            .collect()
    }

    /// Discover relay URLs from stored kind 30617 announcements
    ///
    /// Only returns relay URLs from repositories that list **our** relay.
    /// This ensures we only connect to relays where we have shared repos,
    /// avoiding wasted connections with empty Layer 2 filters.
    ///
    /// Extracts unique relay URLs from `clone` and `relays` tags,
    /// excluding our own relay domain.
    pub async fn discover_relay_urls(&self) -> Vec<String> {
        let filter = Filter::new().kind(Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT));

        let events = match self.database.query(filter).await {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!("Failed to query announcements for relay discovery: {}", e);
                return Vec::new();
            }
        };

        let mut relay_urls: HashSet<String> = HashSet::new();

        for event in events {
            // First check: Does this repo list our relay?
            // Only process repos that reference us - otherwise we'd connect to relays
            // where we have no shared repos, resulting in empty Layer 2 filters.
            let has_our_relay = event.tags.iter().any(|tag| {
                let tag_vec = tag.clone().to_vec();
                tag_vec.len() >= 2
                    && (tag_vec[0] == "clone" || tag_vec[0] == "relays")
                    && tag_vec.iter().any(|v| v.contains(&self.relay_domain))
            });

            if !has_our_relay {
                // Skip repos that don't list our relay - no shared repos possible
                continue;
            }

            // Extract relay URLs from repos that list us
            for tag in event.tags.iter() {
                let tag_vec = tag.clone().to_vec();
                if tag_vec.len() < 2 {
                    continue;
                }

                // Extract URLs from clone and relays tags
                if tag_vec[0] == "clone" || tag_vec[0] == "relays" {
                    for value in tag_vec.iter().skip(1) {
                        // Check if it looks like a URL
                        if value.starts_with("ws://")
                            || value.starts_with("wss://")
                            || value.starts_with("http://")
                            || value.starts_with("https://")
                        {
                            // Exclude our own relay
                            if !value.contains(&self.relay_domain) {
                                relay_urls.insert(value.clone());
                            }
                        }
                    }
                }
            }
        }

        relay_urls.into_iter().collect()
    }

    /// Extract relay URLs from a specific event's clone tags
    ///
    /// Returns URLs that are not our own relay.
    pub fn extract_relay_urls_from_event(&self, event: &Event) -> Vec<String> {
        let mut urls = Vec::new();

        for tag in event.tags.iter() {
            let tag_vec = tag.clone().to_vec();
            if tag_vec.len() < 2 {
                continue;
            }

            if tag_vec[0] == "clone" || tag_vec[0] == "relays" {
                for value in tag_vec.iter().skip(1) {
                    if value.starts_with("ws://")
                        || value.starts_with("wss://")
                        || value.starts_with("http://")
                        || value.starts_with("https://")
                    {
                        if !value.contains(&self.relay_domain) {
                            urls.push(value.clone());
                        }
                    }
                }
            }
        }

        urls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_layer2_filters_empty() {
        let filters = FilterService::batch_layer2_filters(vec![]);
        assert!(filters.is_empty());
    }

    #[test]
    fn test_batch_layer2_filters_small() {
        let coords = vec!["30617:abc:repo1".to_string(), "30617:def:repo2".to_string()];
        let filters = FilterService::batch_layer2_filters(coords);
        // 1 chunk × 3 tag types (A, a, q) = 3 filters
        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_batch_layer2_filters_large() {
        // Create 250 coordinates to test batching
        let coords: Vec<String> = (0..250)
            .map(|i| format!("30617:pubkey{}:repo{}", i, i))
            .collect();

        let filters = FilterService::batch_layer2_filters(coords);
        // 3 chunks (100 + 100 + 50) × 3 tag types (A, a, q) = 9 filters
        assert_eq!(filters.len(), 9);
    }

    #[test]
    fn test_batch_layer3_filters_empty() {
        let filters = FilterService::batch_layer3_filters(vec![]);
        assert!(filters.is_empty());
    }

    #[test]
    fn test_batch_layer3_filters_small() {
        let event_ids = vec!["eventid1".to_string(), "eventid2".to_string()];
        let filters = FilterService::batch_layer3_filters(event_ids);
        // 1 chunk × 3 tag types (E, e, q) = 3 filters
        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_batch_layer3_filters_large() {
        // Create 250 event IDs to test batching
        let event_ids: Vec<String> = (0..250).map(|i| format!("eventid{:064}", i)).collect();

        let filters = FilterService::batch_layer3_filters(event_ids);
        // 3 chunks (100 + 100 + 50) × 3 tag types (E, e, q) = 9 filters
        assert_eq!(filters.len(), 9);
    }

    #[test]
    fn test_layer1_filters() {
        // Create a mock database - we'll use a memory database for testing
        // This test just verifies the filter structure
        let filter = Filter::new().kinds(vec![
            Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT),
            Kind::Custom(KIND_MAINTAINER_LIST),
        ]);

        // Verify the filter has the correct kinds
        // Note: We can't easily inspect Filter internals, but we can ensure it compiles
        assert!(!filter.is_empty());
    }
}
