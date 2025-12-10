//! Catchup Sync Tests
//!
//! Tests for the catchup synchronization feature (Test 0).
//!
//! # Catchup Sync Overview
//!
//! Catchup sync refers to the ability of a relay to synchronize historical events
//! that were published while it was offline or unreachable. This is critical for
//! ensuring data consistency across the relay network.
//!
//! ## Expected Behavior
//!
//! When a relay comes back online after being offline:
//! 1. Detect gap in event history by comparing timestamps
//! 2. Query connected relays for events in the gap period
//! 3. Backfill Layer 2 events (kind 1618) from bootstrap relays
//! 4. Discover and sync Layer 3 events (kinds 1, 1111) referencing Layer 2 events
//! 5. Maintain chronological ordering during backfill
//!
//! ## Implementation Status
//!
//! ⚠ **NOT YET IMPLEMENTED** - Tests marked with `#[ignore]`
//!
//! These tests are ready to enable once catchup sync is implemented in the relay.
//!
//! ## See Also
//!
//! - Bootstrap sync: [`tests/sync/bootstrap.rs`](bootstrap.rs)
//! - Live sync: [`tests/sync/live_sync.rs`](live_sync.rs)
//! - Discovery sync: [`tests/sync/discovery.rs`](discovery.rs)

use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::common::{sync_helpers::*, TestRelay};

/// Create a valid repository announcement event for testing sync.
///
/// This creates a kind 30617 event with required clone and relays tags.
/// The event lists all provided domains so it will be accepted by each
/// relay's write policy.
///
/// # Arguments
/// * `keys` - Keys for signing
/// * `domains` - Slice of domain strings (e.g., "127.0.0.1:8080")
/// * `identifier` - Repository identifier (d-tag)
fn create_repo_announcement(keys: &Keys, domains: &[&str], identifier: &str) -> Event {
    // Build clone URLs for all domains (with .git suffix)
    let clone_urls: Vec<String> = domains
        .iter()
        .map(|d| format!("http://{}/{}.git", d, identifier))
        .collect();

    // Build relay URLs for all domains
    let relay_urls: Vec<String> = domains.iter().map(|d| format!("ws://{}", d)).collect();

    // Build tags for repository announcement
    let tags = vec![
        Tag::identifier(identifier),
        Tag::custom(TagKind::custom("clone"), clone_urls),
        Tag::custom(TagKind::custom("relays"), relay_urls),
    ];

    EventBuilder::new(Kind::Custom(KIND_REPOSITORY_STATE), "Repository state")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign repo announcement")
}

/// Test that relay performs catchup sync after being offline
///
/// # Scenario
///
/// 1. Start two relays (relay1, relay2) with discovery configured
/// 2. Publish several Layer 2 events to relay2
/// 3. Stop relay1 (simulating offline state)
/// 4. Publish more Layer 2 events to relay2 while relay1 is offline
/// 5. Restart relay1
/// 6. Verify relay1 catches up and syncs events it missed
///
/// # Expected Result
///
/// All events published while relay1 was offline should be synced
/// to relay1 after it comes back online, maintaining chronological order.
///
/// # TODO
///
/// - Implement catchup sync mechanism in relay
/// - Add timestamp-based gap detection
/// - Add backfill query generation
/// - Enable this test by removing `#[ignore]`
#[tokio::test]
#[ignore = "Catchup sync not yet implemented"]
async fn test_catchup_sync_after_relay_restart() {
    // NOTE: This is a skeleton implementation ready for when catchup sync is added

    // 1. Start two relays
    let relay1 = TestRelay::start().await;
    let relay2 = TestRelay::start().await;

    // 2. Set up discovery between relays via shared announcement
    let keys = Keys::generate();
    let identifier = "catchup-test-repo";

    // Create announcement listing both relays
    let domain1 = relay1.domain();
    let domain2 = relay2.domain();
    let announcement = create_repo_announcement(
        &keys,
        &[&domain1, &domain2],
        identifier,
    );

    // Publish announcement to both relays
    let client1 = TestClient::new(relay1.url(), keys.clone())
        .await
        .expect("Failed to connect to relay1");
    let client2 = TestClient::new(relay2.url(), keys.clone())
        .await
        .expect("Failed to connect to relay2");

    client1
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to relay1");
    client2
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to relay2");

    // Wait for discovery connections to establish
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 3. Publish initial Layer 2 event (while both relays are online)
    let repo_coord_str = repo_coord(&keys, identifier);
    let event1 = build_layer2_issue_event(&keys, &repo_coord_str, "Issue 1 - before offline")
        .expect("Failed to build event1");
    let event1_id = client2
        .send_event(&event1)
        .await
        .expect("Failed to send event1");

    // Verify initial sync works (baseline check)
    let synced = wait_for_event_on_relay(
        relay1.url(),
        Filter::new().id(event1_id),
        Duration::from_secs(5),
    )
    .await;
    assert!(synced, "Initial event should sync normally via live sync");

    // 4. Stop relay1 (simulating offline state)
    // Note: In a real implementation, we'd need a way to stop and restart a relay
    // For now, this skeleton demonstrates the intended test flow
    relay1.stop().await;

    // Small delay to ensure relay1 is fully stopped
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 5. Publish events while relay1 is offline
    let event2 = build_layer2_issue_event(&keys, &repo_coord_str, "Issue 2 - during offline")
        .expect("Failed to build event2");
    let event2_id = client2
        .send_event(&event2)
        .await
        .expect("Failed to send event2");

    let event3 = build_layer2_issue_event(&keys, &repo_coord_str, "Issue 3 - during offline")
        .expect("Failed to build event3");
    let event3_id = client2
        .send_event(&event3)
        .await
        .expect("Failed to send event3");

    // Give time for events to be stored in relay2
    tokio::time::sleep(Duration::from_secs(1)).await;

    // 6. Restart relay1
    // Note: TestRelay doesn't currently support restart, so we start a new instance
    // A real implementation would need persistent storage and relay restart capability
    let relay1_restarted = TestRelay::start().await;

    // Reconnect client to the new relay instance
    let client1_restarted = TestClient::new(relay1_restarted.url(), keys.clone())
        .await
        .expect("Failed to connect to restarted relay1");

    // Re-publish announcement to establish discovery
    let domain1_restarted = relay1_restarted.domain();
    let announcement_restarted = create_repo_announcement(
        &keys,
        &[&domain1_restarted, &domain2],
        identifier,
    );
    client1_restarted
        .send_event(&announcement_restarted)
        .await
        .expect("Failed to send announcement to restarted relay1");

    // 7. Wait for catchup sync to complete
    // This is where the catchup sync mechanism would kick in
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 8. Verify missed events were synced via catchup
    let event2_synced = wait_for_event_on_relay(
        relay1_restarted.url(),
        Filter::new().id(event2_id),
        Duration::from_secs(5),
    )
    .await;

    let event3_synced = wait_for_event_on_relay(
        relay1_restarted.url(),
        Filter::new().id(event3_id),
        Duration::from_secs(5),
    )
    .await;

    assert!(
        event2_synced,
        "Event 2 (missed while offline) should be synced via catchup"
    );
    assert!(
        event3_synced,
        "Event 3 (missed while offline) should be synced via catchup"
    );

    // 9. Cleanup
    client1_restarted.disconnect().await;
    client2.disconnect().await;
    relay1_restarted.stop().await;
    relay2.stop().await;
}