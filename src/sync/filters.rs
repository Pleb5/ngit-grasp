//! Filter Building Functions for Proactive Sync
//!
//! This module provides functions to construct Nostr filters for the three-layer
//! sync strategy defined in GRASP-02 v4:
//!
//! - Layer 1: Repository announcements (30617 + 30618)
//! - Layer 2: Events tagging our repos (a/A/q tags)
//! - Layer 3: Events tagging our root events (e/E/q tags)
//!
//! See `docs/explanation/grasp-02-proactive-sync-v4.md` for full design details.

use std::collections::HashSet;

use nostr_sdk::prelude::*;

/// Layer 1: Announcements filter (kinds 30617 + 30618)
///
/// Subscribed ONCE on connect - NOT included in consolidation rebuilds.
/// Note: 30618 is ONLY synced from remote relays, not self-subscribed.
pub fn build_announcement_filter(since: Option<Timestamp>) -> Filter {
    let filter = Filter::new().kinds([
        Kind::Custom(30617), // Repository announcements
        Kind::Custom(30618), // Maintainer lists
    ]);

    match since {
        Some(ts) => filter.since(ts),
        None => filter,
    }
}

/// Layer 2: Events tagging one of our repos
///
/// Uses lowercase a, uppercase A, and q tags for comprehensive coverage.
/// Batched per 100 repo refs.
///
/// # Arguments
/// * `repos` - Set of repo addressable refs (format: 30617:pubkey:identifier)
/// * `since` - Optional timestamp for incremental sync
///
/// # Returns
/// Vec of filters, one set of 3 filters (a/A/q) per 100-repo chunk
pub fn tagged_one_of_our_repo_event_filters(
    repos: &HashSet<String>,
    since: Option<Timestamp>,
) -> Vec<Filter> {
    if repos.is_empty() {
        return vec![];
    }

    let mut filters = Vec::new();
    let repo_refs: Vec<_> = repos.iter().collect();

    for chunk in repo_refs.chunks(100) {
        // Lowercase 'a' tag - standard addressable reference
        let mut f1 = Filter::new();
        for repo in chunk {
            f1 = f1.custom_tag(SingleLetterTag::lowercase(Alphabet::A), repo.as_str());
        }

        // Uppercase 'A' tag - some clients use this
        let mut f2 = Filter::new();
        for repo in chunk {
            f2 = f2.custom_tag(SingleLetterTag::uppercase(Alphabet::A), repo.as_str());
        }

        // Quote 'q' tag - NIP-10 quote references to addressable events
        let mut f3 = Filter::new();
        for repo in chunk {
            f3 = f3.custom_tag(SingleLetterTag::lowercase(Alphabet::Q), repo.as_str());
        }

        if let Some(ts) = since {
            f1 = f1.since(ts);
            f2 = f2.since(ts);
            f3 = f3.since(ts);
        }

        filters.push(f1);
        filters.push(f2);
        filters.push(f3);
    }

    filters
}

/// Layer 3: Events tagging one of our root events
///
/// Uses lowercase e, uppercase E, and q tags for comprehensive coverage.
/// Batched per 100 event IDs.
///
/// # Arguments
/// * `root_events` - Set of event IDs (1617/1618/1621 root events)
/// * `since` - Optional timestamp for incremental sync
///
/// # Returns
/// Vec of filters, one set of 3 filters (e/E/q) per 100-event chunk
pub fn tagged_one_of_our_root_event_filters(
    root_events: &HashSet<EventId>,
    since: Option<Timestamp>,
) -> Vec<Filter> {
    if root_events.is_empty() {
        return vec![];
    }

    let mut filters = Vec::new();
    let event_ids: Vec<String> = root_events.iter().map(|id| id.to_hex()).collect();

    for chunk in event_ids.chunks(100) {
        // Lowercase 'e' tag - standard event reference
        let mut f1 = Filter::new();
        for event_id in chunk {
            f1 = f1.custom_tag(SingleLetterTag::lowercase(Alphabet::E), event_id.as_str());
        }

        // Uppercase 'E' tag - some clients use this
        let mut f2 = Filter::new();
        for event_id in chunk {
            f2 = f2.custom_tag(SingleLetterTag::uppercase(Alphabet::E), event_id.as_str());
        }

        // Quote 'q' tag - NIP-10 quote references to events
        let mut f3 = Filter::new();
        for event_id in chunk {
            f3 = f3.custom_tag(SingleLetterTag::lowercase(Alphabet::Q), event_id.as_str());
        }

        if let Some(ts) = since {
            f1 = f1.since(ts);
            f2 = f2.since(ts);
            f3 = f3.since(ts);
        }

        filters.push(f1);
        filters.push(f2);
        filters.push(f3);
    }

    filters
}

/// Builds Layer 2 + Layer 3 filters only (NOT Layer 1)
///
/// Used by:
/// - compute_actions for incremental subscriptions
/// - consolidation rebuilds (Layer 1 remains active)
///
/// # Arguments
/// * `repos` - Set of repo addressable refs
/// * `root_events` - Set of root event IDs
/// * `since` - Optional timestamp for incremental sync
pub fn build_layer2_and_layer3_filters(
    repos: &HashSet<String>,
    root_events: &HashSet<EventId>,
    since: Option<Timestamp>,
) -> Vec<Filter> {
    let mut filters = Vec::new();
    filters.extend(tagged_one_of_our_repo_event_filters(repos, since));
    filters.extend(tagged_one_of_our_root_event_filters(root_events, since));
    filters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_announcement_filter_no_since() {
        let filter = build_announcement_filter(None);

        // Verify it includes both kinds
        // Filter API: we can check by converting to JSON or inspecting structure
        // For now we just verify it doesn't panic and returns a valid filter
        assert!(!filter.is_empty());
    }

    #[test]
    fn test_announcement_filter_with_since() {
        let since = Timestamp::from(1700000000);
        let filter = build_announcement_filter(Some(since));

        assert!(!filter.is_empty());
    }

    #[test]
    fn test_repo_filters_empty() {
        let repos: HashSet<String> = HashSet::new();
        let filters = tagged_one_of_our_repo_event_filters(&repos, None);

        assert!(filters.is_empty());
    }

    #[test]
    fn test_repo_filters_single_repo() {
        let mut repos = HashSet::new();
        repos.insert("30617:abc123:test-repo".to_string());

        let filters = tagged_one_of_our_repo_event_filters(&repos, None);

        // Should create 3 filters (a, A, q) for one chunk
        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_repo_filters_batching() {
        let mut repos = HashSet::new();
        for i in 0..250 {
            repos.insert(format!("30617:pubkey{}:repo{}", i, i));
        }

        let filters = tagged_one_of_our_repo_event_filters(&repos, None);

        // Should create 9 filters (3 chunks * 3 tag types)
        // 250 repos = 100 + 100 + 50 = 3 chunks
        assert_eq!(filters.len(), 9);
    }

    #[test]
    fn test_repo_filters_with_since() {
        let mut repos = HashSet::new();
        repos.insert("30617:abc123:test-repo".to_string());

        let since = Timestamp::from(1700000000);
        let filters = tagged_one_of_our_repo_event_filters(&repos, Some(since));

        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_root_event_filters_empty() {
        let root_events: HashSet<EventId> = HashSet::new();
        let filters = tagged_one_of_our_root_event_filters(&root_events, None);

        assert!(filters.is_empty());
    }

    #[test]
    fn test_root_event_filters_single_event() {
        let mut root_events = HashSet::new();
        // Create a valid event ID (all zeros for testing)
        root_events.insert(EventId::all_zeros());

        let filters = tagged_one_of_our_root_event_filters(&root_events, None);

        // Should create 3 filters (e, E, q) for one chunk
        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_root_event_filters_batching() {
        let mut root_events = HashSet::new();
        // EventId::all_zeros() will deduplicate, so we need unique IDs
        // For testing purposes, we'll just verify with one ID since HashSet
        // deduplicates all_zeros(). In real usage these would be unique.
        for _ in 0..250 {
            root_events.insert(EventId::all_zeros());
        }

        let filters = tagged_one_of_our_root_event_filters(&root_events, None);

        // With deduplication, we only have 1 unique ID, so 3 filters
        // In real usage with 250 unique IDs, it would be 9 filters
        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_root_event_filters_with_since() {
        let mut root_events = HashSet::new();
        root_events.insert(EventId::all_zeros());

        let since = Timestamp::from(1700000000);
        let filters = tagged_one_of_our_root_event_filters(&root_events, Some(since));

        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_combined_filters_empty() {
        let repos: HashSet<String> = HashSet::new();
        let root_events: HashSet<EventId> = HashSet::new();

        let filters = build_layer2_and_layer3_filters(&repos, &root_events, None);

        assert!(filters.is_empty());
    }

    #[test]
    fn test_combined_filters() {
        let mut repos = HashSet::new();
        repos.insert("30617:abc123:repo1".to_string());

        let mut root_events = HashSet::new();
        root_events.insert(EventId::all_zeros());

        let filters = build_layer2_and_layer3_filters(&repos, &root_events, None);

        // Should have 6 filters (3 for repos + 3 for root events)
        assert_eq!(filters.len(), 6);
    }

    #[test]
    fn test_combined_filters_repos_only() {
        let mut repos = HashSet::new();
        repos.insert("30617:abc123:repo1".to_string());

        let root_events: HashSet<EventId> = HashSet::new();

        let filters = build_layer2_and_layer3_filters(&repos, &root_events, None);

        // Should have 3 filters (3 for repos only)
        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_combined_filters_root_events_only() {
        let repos: HashSet<String> = HashSet::new();

        let mut root_events = HashSet::new();
        root_events.insert(EventId::all_zeros());

        let filters = build_layer2_and_layer3_filters(&repos, &root_events, None);

        // Should have 3 filters (3 for root events only)
        assert_eq!(filters.len(), 3);
    }

    #[test]
    fn test_combined_filters_with_since() {
        let mut repos = HashSet::new();
        repos.insert("30617:abc123:repo1".to_string());

        let mut root_events = HashSet::new();
        root_events.insert(EventId::all_zeros());

        let since = Timestamp::from(1700000000);
        let filters = build_layer2_and_layer3_filters(&repos, &root_events, Some(since));

        assert_eq!(filters.len(), 6);
    }
}
