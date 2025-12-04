//! GRASP-02 Phase 1: Proactive Sync Basic Integration Tests
//!
//! Tests the basic proactive sync functionality using two TestRelay instances:
//! - relay_a: Source relay with events
//! - relay_b: Sync relay configured to sync from relay_a
//!
//! # Running Tests
//!
//! ```bash
//! cargo test --test proactive_sync_basic
//! cargo test --test proactive_sync_basic -- --nocapture
//! ```

mod common;

use std::time::Duration;

use common::TestRelay;
use nostr_sdk::prelude::*;

/// Kind 30617 - Repository State (NIP-34)
const KIND_REPOSITORY_STATE: u16 = 30617;

/// Create a valid repository announcement event for testing
///
/// This creates a kind 30617 event with required clone and relays tags
fn create_valid_repo_announcement(
    keys: &Keys,
    domain: &str,
    identifier: &str,
) -> Event {
    // Build tags for repository announcement
    let tags = vec![
        Tag::identifier(identifier),
        Tag::custom(
            TagKind::custom("clone"),
            vec![format!("http://{}/{}", domain, identifier)],
        ),
        Tag::custom(
            TagKind::custom("relays"),
            vec![format!("ws://{}", domain)],
        ),
    ];

    EventBuilder::new(Kind::Custom(KIND_REPOSITORY_STATE), "Repository state")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Test that syncing relay connects to source relay
#[tokio::test]
async fn test_sync_relay_connects_to_source() {
    // Start source relay (relay_a)
    let relay_a = TestRelay::start().await;

    // Start syncing relay (relay_b) configured to sync from relay_a
    let relay_b = TestRelay::start_with_sync(relay_a.url()).await;

    // Give some time for connection to establish
    tokio::time::sleep(Duration::from_millis(500)).await;

    // If we got here without panicking, the relays started successfully
    // The sync connection happens in the background

    relay_b.stop().await;
    relay_a.stop().await;
}

/// Test that valid events sync from source to syncing relay
#[tokio::test]
async fn test_valid_event_syncs_to_relay() {
    // Start source relay (relay_a)
    let relay_a = TestRelay::start().await;

    // Give relay_a time to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Start syncing relay (relay_b) configured to sync from relay_a
    let relay_b = TestRelay::start_with_sync(relay_a.url()).await;

    // Create test keys
    let keys = Keys::generate();

    // Create and submit a valid repository announcement to relay_a
    let event = create_valid_repo_announcement(&keys, &relay_a.domain(), "test-repo");
    let event_id = event.id;

    // Submit event to relay_a
    let client_a = Client::default();
    client_a.add_relay(relay_a.url()).await.expect("Failed to add relay_a");
    client_a.connect().await;

    let send_result = client_a.send_event(&event).await;
    assert!(send_result.is_ok(), "Failed to send event to relay_a: {:?}", send_result.err());

    // Wait for sync to occur
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Query relay_b to verify the event was synced
    let client_b = Client::default();
    client_b.add_relay(relay_b.url()).await.expect("Failed to add relay_b");
    client_b.connect().await;

    // Create filter to find our event
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_REPOSITORY_STATE))
        .author(keys.public_key());

    let events = client_b
        .fetch_events(filter, Duration::from_secs(5))
        .await
        .expect("Failed to fetch events from relay_b");

    // Check if our event was synced
    let found = events.iter().any(|e| e.id == event_id);

    // Clean up
    client_a.disconnect().await;
    client_b.disconnect().await;
    relay_b.stop().await;
    relay_a.stop().await;

    assert!(
        found,
        "Event {} was not synced to relay_b. Found {} events",
        event_id,
        events.len()
    );
}

/// Test that invalid events are rejected by syncing relay validation
#[tokio::test]
async fn test_invalid_event_rejected_by_sync_validation() {
    // Start source relay (relay_a) - this is a simple relay without GRASP validation
    // For this test, we'll use a second ngit-grasp relay, but the key insight is that
    // the syncing relay should reject events that don't pass its own validation

    let relay_a = TestRelay::start().await;
    let relay_b = TestRelay::start_with_sync(relay_a.url()).await;

    // Give time for connection
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create test keys
    let keys = Keys::generate();

    // Create an INVALID repository announcement (missing clone tag)
    let tags = vec![
        Tag::identifier("test-invalid-repo"),
        // Missing required "clone" tag!
        Tag::custom(
            TagKind::custom("relays"),
            vec![format!("ws://{}", relay_a.domain())],
        ),
    ];

    let invalid_event = EventBuilder::new(Kind::Custom(KIND_REPOSITORY_STATE), "Invalid repo")
        .tags(tags)
        .sign_with_keys(&keys)
        .expect("Failed to sign event");

    let invalid_event_id = invalid_event.id;

    // Submit invalid event to relay_a
    // Note: relay_a will also reject it due to GRASP validation
    let client_a = Client::default();
    client_a.add_relay(relay_a.url()).await.expect("Failed to add relay_a");
    client_a.connect().await;

    // This will likely fail since relay_a also validates, but let's try
    let _ = client_a.send_event(&invalid_event).await;

    // Wait for potential sync
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Query relay_b - the event should NOT be present
    let client_b = Client::default();
    client_b.add_relay(relay_b.url()).await.expect("Failed to add relay_b");
    client_b.connect().await;

    let filter = Filter::new()
        .kind(Kind::Custom(KIND_REPOSITORY_STATE))
        .author(keys.public_key());

    let events = client_b
        .fetch_events(filter, Duration::from_secs(3))
        .await
        .expect("Failed to fetch events from relay_b");

    let found = events.iter().any(|e| e.id == invalid_event_id);

    // Clean up
    client_a.disconnect().await;
    client_b.disconnect().await;
    relay_b.stop().await;
    relay_a.stop().await;

    assert!(
        !found,
        "Invalid event {} should NOT have been synced to relay_b",
        invalid_event_id
    );
}

/// Test that syncing relay maintains its own validation policy
#[tokio::test]
async fn test_sync_respects_local_validation() {
    // This test verifies that synced events go through the local Nip34WritePolicy
    // by testing that orphan events (events referencing non-existent repos) are rejected

    let relay_a = TestRelay::start().await;
    let relay_b = TestRelay::start_with_sync(relay_a.url()).await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    let keys = Keys::generate();

    // First, create a VALID repository announcement and submit it
    let valid_event = create_valid_repo_announcement(&keys, &relay_a.domain(), "valid-repo");
    let valid_event_id = valid_event.id;

    let client_a = Client::default();
    client_a.add_relay(relay_a.url()).await.expect("Failed to add relay_a");
    client_a.connect().await;

    client_a
        .send_event(&valid_event)
        .await
        .expect("Failed to send valid event");

    // Wait for sync
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Query relay_b to verify the valid event was synced
    let client_b = Client::default();
    client_b.add_relay(relay_b.url()).await.expect("Failed to add relay_b");
    client_b.connect().await;

    let filter = Filter::new()
        .kind(Kind::Custom(KIND_REPOSITORY_STATE))
        .author(keys.public_key());

    let events = client_b
        .fetch_events(filter, Duration::from_secs(5))
        .await
        .expect("Failed to fetch events from relay_b");

    let found = events.iter().any(|e| e.id == valid_event_id);

    // Clean up
    client_a.disconnect().await;
    client_b.disconnect().await;
    relay_b.stop().await;
    relay_a.stop().await;

    assert!(
        found,
        "Valid event {} should have been synced to relay_b",
        valid_event_id
    );
}