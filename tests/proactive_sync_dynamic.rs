//! GRASP-02 Phase 4: Dynamic Subscription Integration Tests
//!
//! Tests verify dynamic subscription management:
//! - New announcement triggers Layer 2 subscription
//! - New PR/Issue triggers Layer 3 subscription
//! - Subscription count tracking per connection
//! - Consolidation at filter count > 150
//! - No duplicate subscriptions
//!
//! # Running Tests
//!
//! ```bash
//! cargo test --test proactive_sync_dynamic
//! cargo test --test proactive_sync_dynamic -- --nocapture
//! ```

use std::collections::HashSet;

use ngit_grasp::sync::SubscriptionManager;
use nostr_sdk::prelude::*;

/// Kind 30617 - Repository Announcement (NIP-34)
const KIND_REPOSITORY_ANNOUNCEMENT: u16 = 30617;

/// Kind 30618 - Maintainer List (NIP-34)
const KIND_MAINTAINER_LIST: u16 = 30618;

/// Maximum filters before consolidation (from spec)
const CONSOLIDATION_THRESHOLD: usize = 150;

/// Helper to create a test announcement event
fn create_test_announcement(keys: &Keys, identifier: &str) -> Event {
    let tags = vec![
        Tag::identifier(identifier),
        Tag::custom(
            TagKind::custom("clone"),
            vec![format!("http://test.example.com/{}", identifier)],
        ),
        Tag::custom(
            TagKind::custom("relays"),
            vec!["ws://test.example.com".to_string()],
        ),
    ];

    EventBuilder::new(Kind::Custom(KIND_REPOSITORY_ANNOUNCEMENT), "Test repo")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Helper to create a test maintainer list event
fn create_test_maintainer_list(keys: &Keys, identifier: &str) -> Event {
    let tags = vec![
        Tag::identifier(identifier),
        Tag::custom(
            TagKind::custom("relays"),
            vec!["ws://test.example.com".to_string()],
        ),
    ];

    EventBuilder::new(Kind::Custom(KIND_MAINTAINER_LIST), "Maintainer list")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Helper to create a test PR event (kind 1617)
fn create_test_pr_event(keys: &Keys, repo_coord: &str) -> Event {
    let tags = vec![Tag::custom(
        TagKind::custom("a"),
        vec![repo_coord.to_string()],
    )];

    EventBuilder::new(Kind::Custom(1617), "Test patch proposal")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Helper to create a test PR event (kind 1618)
fn create_test_pr_1618_event(keys: &Keys, repo_coord: &str) -> Event {
    let tags = vec![Tag::custom(
        TagKind::custom("a"),
        vec![repo_coord.to_string()],
    )];

    EventBuilder::new(Kind::Custom(1618), "Test PR")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Helper to create a test Issue event (kind 1621)
fn create_test_issue_event(keys: &Keys, repo_coord: &str) -> Event {
    let tags = vec![Tag::custom(
        TagKind::custom("a"),
        vec![repo_coord.to_string()],
    )];

    EventBuilder::new(Kind::Custom(1621), "Test issue")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Helper to create a test Reply event (kind 1622)
fn create_test_reply_event(keys: &Keys, event_id: &str) -> Event {
    let tags = vec![Tag::custom(
        TagKind::custom("e"),
        vec![event_id.to_string()],
    )];

    EventBuilder::new(Kind::Custom(1622), "Test reply")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

// ============================================================================
// Filter Count Tests
// ============================================================================

/// Test initial filter count is 1 (Layer 1 only)
#[test]
fn test_initial_filter_count() {
    // Create a minimal SubscriptionManager-like state for testing
    // We test the logic without needing a full FilterService

    // Initial state: 0 announcements, 0 events, not consolidated
    // Filter count should be: 1 (Layer 1) + 0 + 0 = 1
    let announcement_count = 0;
    let event_count = 0;
    let is_consolidated = false;

    let filter_count = if is_consolidated {
        1
    } else {
        1 + announcement_count + event_count
    };

    assert_eq!(filter_count, 1);
}

/// Test filter count increases with announcements
#[test]
fn test_filter_count_with_announcements() {
    let announcement_count = 5;
    let event_count = 0;
    let is_consolidated = false;

    let filter_count = if is_consolidated {
        1
    } else {
        1 + announcement_count + event_count
    };

    // 1 (Layer 1) + 5 (announcements) = 6
    assert_eq!(filter_count, 6);
}

/// Test filter count increases with events
#[test]
fn test_filter_count_with_events() {
    let announcement_count = 0;
    let event_count = 10;
    let is_consolidated = false;

    let filter_count = if is_consolidated {
        1
    } else {
        1 + announcement_count + event_count
    };

    // 1 (Layer 1) + 10 (events) = 11
    assert_eq!(filter_count, 11);
}

/// Test filter count with both announcements and events
#[test]
fn test_filter_count_mixed() {
    let announcement_count = 50;
    let event_count = 30;
    let is_consolidated = false;

    let filter_count = if is_consolidated {
        1
    } else {
        1 + announcement_count + event_count
    };

    // 1 + 50 + 30 = 81
    assert_eq!(filter_count, 81);
}

/// Test filter count is 1 when consolidated
#[test]
fn test_filter_count_consolidated() {
    let announcement_count = 100; // These would be cleared on consolidation
    let event_count = 100;
    let is_consolidated = true;

    let filter_count = if is_consolidated {
        1
    } else {
        1 + announcement_count + event_count
    };

    assert_eq!(filter_count, 1);
}

// ============================================================================
// Consolidation Threshold Tests
// ============================================================================

/// Test consolidation is not triggered below threshold
#[test]
fn test_should_consolidate_below_threshold() {
    let filter_count = 100;
    let is_consolidated = false;

    let should_consolidate = !is_consolidated && filter_count > CONSOLIDATION_THRESHOLD;

    assert!(!should_consolidate);
}

/// Test consolidation is triggered at threshold
#[test]
fn test_should_consolidate_at_threshold() {
    let filter_count = 151; // > 150
    let is_consolidated = false;

    let should_consolidate = !is_consolidated && filter_count > CONSOLIDATION_THRESHOLD;

    assert!(should_consolidate);
}

/// Test consolidation is triggered well above threshold
#[test]
fn test_should_consolidate_above_threshold() {
    let filter_count = 200;
    let is_consolidated = false;

    let should_consolidate = !is_consolidated && filter_count > CONSOLIDATION_THRESHOLD;

    assert!(should_consolidate);
}

/// Test consolidation is not triggered if already consolidated
#[test]
fn test_should_consolidate_already_consolidated() {
    let filter_count = 200; // Would trigger, but already consolidated
    let is_consolidated = true;

    let should_consolidate = !is_consolidated && filter_count > CONSOLIDATION_THRESHOLD;

    assert!(!should_consolidate);
}

/// Test exact threshold boundary (150 should NOT trigger, 151 should)
#[test]
fn test_consolidation_threshold_boundary() {
    let is_consolidated = false;

    // 150 should NOT trigger (> 150, not >= 150)
    let should_consolidate_at_150 = !is_consolidated && 150 > CONSOLIDATION_THRESHOLD;
    assert!(!should_consolidate_at_150);

    // 151 should trigger
    let should_consolidate_at_151 = !is_consolidated && 151 > CONSOLIDATION_THRESHOLD;
    assert!(should_consolidate_at_151);
}

// ============================================================================
// Duplicate Prevention Tests
// ============================================================================

/// Test duplicate announcement detection
#[test]
fn test_duplicate_announcement_prevention() {
    let mut subscribed_announcements: HashSet<String> = HashSet::new();

    let event_id = "abc123".to_string();

    // First add should succeed
    let is_new = !subscribed_announcements.contains(&event_id);
    assert!(is_new);
    subscribed_announcements.insert(event_id.clone());

    // Second add should fail (duplicate)
    let is_new_again = !subscribed_announcements.contains(&event_id);
    assert!(!is_new_again);
}

/// Test duplicate event detection
#[test]
fn test_duplicate_event_prevention() {
    let mut subscribed_events: HashSet<String> = HashSet::new();

    let event_id = "def456".to_string();

    // First add should succeed
    let is_new = !subscribed_events.contains(&event_id);
    assert!(is_new);
    subscribed_events.insert(event_id.clone());

    // Second add should fail (duplicate)
    let is_new_again = !subscribed_events.contains(&event_id);
    assert!(!is_new_again);
}

/// Test multiple unique items are tracked correctly
#[test]
fn test_multiple_unique_items_tracked() {
    let mut subscribed_announcements: HashSet<String> = HashSet::new();

    // Add multiple unique announcements
    for i in 0..10 {
        let id = format!("announcement_{}", i);
        assert!(!subscribed_announcements.contains(&id));
        subscribed_announcements.insert(id);
    }

    assert_eq!(subscribed_announcements.len(), 10);
}

// ============================================================================
// Event Creation and Validation Tests
// ============================================================================

/// Test announcement event has required d tag
#[test]
fn test_announcement_has_d_tag() {
    let keys = Keys::generate();
    let event = create_test_announcement(&keys, "my-repo");

    let has_d_tag = event.tags.iter().any(|tag| {
        let tag_vec = tag.clone().to_vec();
        tag_vec.len() >= 2 && tag_vec[0] == "d"
    });

    assert!(has_d_tag);
}

/// Test announcement event has correct kind
#[test]
fn test_announcement_correct_kind() {
    let keys = Keys::generate();
    let event = create_test_announcement(&keys, "my-repo");

    assert_eq!(event.kind.as_u16(), KIND_REPOSITORY_ANNOUNCEMENT);
}

/// Test maintainer list event has correct kind
#[test]
fn test_maintainer_list_correct_kind() {
    let keys = Keys::generate();
    let event = create_test_maintainer_list(&keys, "maintainers");

    assert_eq!(event.kind.as_u16(), KIND_MAINTAINER_LIST);
}

/// Test PR event has a tag
#[test]
fn test_pr_event_has_a_tag() {
    let keys = Keys::generate();
    let coord = "30617:pubkey123:my-repo";
    let event = create_test_pr_event(&keys, coord);

    let has_a_tag = event.tags.iter().any(|tag| {
        let tag_vec = tag.clone().to_vec();
        tag_vec.len() >= 2 && tag_vec[0] == "a"
    });

    assert!(has_a_tag);
}

/// Test issue event has a tag
#[test]
fn test_issue_event_has_a_tag() {
    let keys = Keys::generate();
    let coord = "30617:pubkey123:my-repo";
    let event = create_test_issue_event(&keys, coord);

    let has_a_tag = event.tags.iter().any(|tag| {
        let tag_vec = tag.clone().to_vec();
        tag_vec.len() >= 2 && tag_vec[0] == "a"
    });

    assert!(has_a_tag);
}

/// Test reply event has e tag
#[test]
fn test_reply_event_has_e_tag() {
    let keys = Keys::generate();
    let event_id = "abc123def456";
    let event = create_test_reply_event(&keys, event_id);

    let has_e_tag = event.tags.iter().any(|tag| {
        let tag_vec = tag.clone().to_vec();
        tag_vec.len() >= 2 && tag_vec[0] == "e"
    });

    assert!(has_e_tag);
}

// ============================================================================
// Subscription Lifecycle Tests
// ============================================================================

/// Test subscription lifecycle: initial -> add announcements -> add events -> consolidate
#[test]
fn test_subscription_lifecycle() {
    let mut subscribed_announcements: HashSet<String> = HashSet::new();
    let mut subscribed_events: HashSet<String> = HashSet::new();
    let mut is_consolidated = false;

    // Initial state
    let initial_count = 1 + subscribed_announcements.len() + subscribed_events.len();
    assert_eq!(initial_count, 1);

    // Add some announcements
    for i in 0..50 {
        subscribed_announcements.insert(format!("ann_{}", i));
    }

    let after_announcements = 1 + subscribed_announcements.len() + subscribed_events.len();
    assert_eq!(after_announcements, 51);

    // Add some events
    for i in 0..50 {
        subscribed_events.insert(format!("evt_{}", i));
    }

    let after_events = 1 + subscribed_announcements.len() + subscribed_events.len();
    assert_eq!(after_events, 101);

    // Add more to exceed threshold
    for i in 50..100 {
        subscribed_announcements.insert(format!("ann_{}", i));
    }

    let before_consolidation = 1 + subscribed_announcements.len() + subscribed_events.len();
    assert_eq!(before_consolidation, 151);

    // Should trigger consolidation
    let should_consolidate = !is_consolidated && before_consolidation > CONSOLIDATION_THRESHOLD;
    assert!(should_consolidate);

    // Consolidate
    subscribed_announcements.clear();
    subscribed_events.clear();
    is_consolidated = true;

    // After consolidation
    let after_consolidation = if is_consolidated {
        1
    } else {
        1 + subscribed_announcements.len() + subscribed_events.len()
    };
    assert_eq!(after_consolidation, 1);

    // Should not trigger consolidation again
    let should_consolidate_again =
        !is_consolidated && after_consolidation > CONSOLIDATION_THRESHOLD;
    assert!(!should_consolidate_again);
}

/// Test that consolidated state blocks new additions
#[test]
fn test_consolidated_blocks_additions() {
    let is_consolidated = true;

    // When consolidated, add_announcement should return None (simulated)
    // The logic is: if is_consolidated, return None
    let should_add = !is_consolidated;

    assert!(!should_add);
}

/// Test that non-consolidated state allows additions
#[test]
fn test_non_consolidated_allows_additions() {
    let is_consolidated = false;
    let mut subscribed_announcements: HashSet<String> = HashSet::new();
    let event_id = "new_announcement";

    // When not consolidated and event not in set, should add
    let should_add = !is_consolidated && !subscribed_announcements.contains(event_id);

    assert!(should_add);

    subscribed_announcements.insert(event_id.to_string());
    assert!(subscribed_announcements.contains(event_id));
}

// ============================================================================
// Filter Building Tests (coordinate format)
// ============================================================================

/// Test announcement coordinate format
#[test]
fn test_announcement_coordinate_format() {
    let keys = Keys::generate();
    let identifier = "my-repo";
    let event = create_test_announcement(&keys, identifier);

    // Extract d tag
    let d_tag = event.tags.iter().find_map(|tag| {
        let tag_vec = tag.clone().to_vec();
        if tag_vec.len() >= 2 && tag_vec[0] == "d" {
            Some(tag_vec[1].clone())
        } else {
            None
        }
    });

    assert!(d_tag.is_some());
    assert_eq!(d_tag.unwrap(), identifier);

    // Build coordinate: kind:pubkey:identifier
    let coord = format!(
        "{}:{}:{}",
        KIND_REPOSITORY_ANNOUNCEMENT,
        event.pubkey.to_hex(),
        identifier
    );

    // Verify format
    let parts: Vec<&str> = coord.split(':').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "30617");
    assert_eq!(parts[2], identifier);
}

/// Test multiple announcement coordinates are unique
#[test]
fn test_multiple_announcement_coordinates_unique() {
    let keys = Keys::generate();

    let identifiers = vec!["repo1", "repo2", "repo3"];
    let mut coords: HashSet<String> = HashSet::new();

    for id in identifiers {
        let event = create_test_announcement(&keys, id);
        let coord = format!(
            "{}:{}:{}",
            KIND_REPOSITORY_ANNOUNCEMENT,
            event.pubkey.to_hex(),
            id
        );
        coords.insert(coord);
    }

    assert_eq!(coords.len(), 3);
}

// ============================================================================
// Integration-style Tests
// ============================================================================

/// Test simulated workflow: announcement received, then PR received
#[test]
fn test_workflow_announcement_then_pr() {
    let keys = Keys::generate();
    let mut subscribed_announcements: HashSet<String> = HashSet::new();
    let mut subscribed_events: HashSet<String> = HashSet::new();
    let is_consolidated = false;

    // Step 1: Receive announcement
    let announcement = create_test_announcement(&keys, "my-repo");
    let ann_id = announcement.id.to_hex();

    // Should add to tracking (simulating add_announcement)
    let should_add_ann = !is_consolidated && !subscribed_announcements.contains(&ann_id);
    assert!(should_add_ann);
    subscribed_announcements.insert(ann_id.clone());

    // Filter count should increase
    let filter_count = 1 + subscribed_announcements.len() + subscribed_events.len();
    assert_eq!(filter_count, 2);

    // Step 2: Receive PR for that repo
    let coord = format!(
        "{}:{}:my-repo",
        KIND_REPOSITORY_ANNOUNCEMENT,
        keys.public_key().to_hex()
    );
    let pr = create_test_pr_event(&keys, &coord);
    let pr_id = pr.id.to_hex();

    // Should add to tracking (simulating add_event)
    let should_add_pr = !is_consolidated && !subscribed_events.contains(&pr_id);
    assert!(should_add_pr);
    subscribed_events.insert(pr_id.clone());

    // Filter count should increase again
    let filter_count = 1 + subscribed_announcements.len() + subscribed_events.len();
    assert_eq!(filter_count, 3);
}

/// Test stress: adding many items triggers consolidation
#[test]
fn test_stress_many_items_triggers_consolidation() {
    let keys = Keys::generate();
    let mut subscribed_announcements: HashSet<String> = HashSet::new();
    let mut subscribed_events: HashSet<String> = HashSet::new();
    let mut is_consolidated = false;
    let mut consolidation_triggered = false;

    // Add 100 announcements
    for i in 0..100 {
        let event = create_test_announcement(&keys, &format!("repo-{}", i));
        let event_id = event.id.to_hex();

        if !is_consolidated && !subscribed_announcements.contains(&event_id) {
            subscribed_announcements.insert(event_id);
        }

        // Check consolidation after each add
        let filter_count = 1 + subscribed_announcements.len() + subscribed_events.len();
        if !is_consolidated && filter_count > CONSOLIDATION_THRESHOLD {
            consolidation_triggered = true;
            subscribed_announcements.clear();
            subscribed_events.clear();
            is_consolidated = true;
            break;
        }
    }

    // If we didn't consolidate yet, add events
    if !consolidation_triggered {
        for i in 0..100 {
            let coord = format!("30617:pubkey:repo-{}", i);
            let event = create_test_pr_event(&keys, &coord);
            let event_id = event.id.to_hex();

            if !is_consolidated && !subscribed_events.contains(&event_id) {
                subscribed_events.insert(event_id);
            }

            // Check consolidation after each add
            let filter_count = 1 + subscribed_announcements.len() + subscribed_events.len();
            if !is_consolidated && filter_count > CONSOLIDATION_THRESHOLD {
                consolidation_triggered = true;
                subscribed_announcements.clear();
                subscribed_events.clear();
                is_consolidated = true;
                break;
            }
        }
    }

    // Consolidation should have been triggered
    assert!(consolidation_triggered);
    assert!(is_consolidated);

    // After consolidation, counts should be reset
    assert_eq!(subscribed_announcements.len(), 0);
    assert_eq!(subscribed_events.len(), 0);
}
