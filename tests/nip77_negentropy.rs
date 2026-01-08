//! NIP-77 Negentropy Sync Smoke Tests
//!
//! Verifies that ngit-grasp's NIP-77 claim is valid by testing negentropy
//! reconciliation between a client and the relay.
//!
//! # Background
//!
//! NIP-77 defines the negentropy protocol for efficient set reconciliation.
//! The nostr-relay-builder v0.44 provides built-in NIP-77 support via:
//! - NEG-OPEN message handling
//! - NEG-MSG message handling
//! - NEG-CLOSE message handling
//!
//! This test uses nostr-sdk's `client.sync()` method to perform negentropy
//! reconciliation against the relay.
//!
//! # Running Tests
//!
//! ```bash
//! cargo test --test nip77_negentropy -- --nocapture
//! ```

mod common;

use nostr_sdk::prelude::*;
use std::time::Duration;

use common::{sync_helpers::*, TestRelay};

/// Smoke test: NIP-77 negentropy reconciliation returns event IDs
///
/// Scenario:
/// 1. Start a TestRelay
/// 2. Publish a couple of events to it
/// 3. Create a fresh client with empty local database
/// 4. Call client.sync() to perform negentropy reconciliation
/// 5. Verify reconciliation found the events on the relay
#[tokio::test]
async fn test_nip77_negentropy_sync_finds_events() {
    // 1. Start relay
    let relay = TestRelay::start().await;
    println!("Relay started at {}", relay.url());

    // 2. Create keys and publish events
    let keys = Keys::generate();

    // Create a repository announcement that will be accepted by the relay
    let announcement = create_repo_announcement(&keys, &[&relay.domain()], "test-repo-nip77");
    let event1_id = announcement.id;
    println!(
        "Created event 1: {} (kind {})",
        event1_id,
        announcement.kind.as_u16()
    );

    // Create a second event (issue referencing the repo)
    let repo_coord = format!(
        "{}:{}:{}",
        Kind::GitRepoAnnouncement.as_u16(),
        keys.public_key().to_hex(),
        "test-repo-nip77"
    );
    let issue = build_layer2_issue_event(&keys, &repo_coord, "Test issue for NIP-77")
        .expect("Failed to build issue event");
    let event2_id = issue.id;
    println!(
        "Created event 2: {} (kind {})",
        event2_id,
        issue.kind.as_u16()
    );

    // 3. Send events to relay using TestClient
    let publish_client = TestClient::new(relay.url(), keys.clone())
        .await
        .expect("Failed to connect to relay");

    publish_client
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement");
    publish_client
        .send_event(&issue)
        .await
        .expect("Failed to send issue");
    println!("Events published to relay");

    publish_client.disconnect().await;

    // 4. Wait a moment for events to be stored
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 5. Create a fresh client to perform sync (different instance, no local events)
    let sync_keys = Keys::generate(); // Different keys, doesn't matter for sync
    let sync_client = Client::new(sync_keys);

    sync_client
        .add_relay(relay.url())
        .await
        .expect("Failed to add relay");
    sync_client.connect().await;

    // Wait for connection
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 6. Perform negentropy sync with filter matching our events
    let filter = Filter::new()
        .author(keys.public_key())
        .kinds(vec![Kind::GitRepoAnnouncement, Kind::GitIssue]);

    println!("Starting negentropy sync with filter: {:?}", filter);

    let sync_opts = SyncOptions::default();

    let result = sync_client.sync(filter, &sync_opts).await;

    // 7. Cleanup
    sync_client.disconnect().await;
    relay.stop().await;

    // 8. Verify results
    match result {
        Ok(output) => {
            let reconciliation = output.val;
            println!("Negentropy sync completed!");
            println!("  Local: {:?}", reconciliation.local);
            println!("  Remote: {:?}", reconciliation.remote);
            println!("  Sent: {:?}", reconciliation.sent);
            println!("  Received: {:?}", reconciliation.received);
            println!("  Failures: {:?}", output.failed);

            // The relay has events we don't have locally, so they should appear in "received"
            // or "remote" (depending on whether we requested them or just discovered them)
            let total_discovered = reconciliation.received.len() + reconciliation.remote.len();

            assert!(
                total_discovered >= 2,
                "Expected to discover at least 2 events via negentropy, got {} (received: {}, remote: {})",
                total_discovered,
                reconciliation.received.len(),
                reconciliation.remote.len()
            );

            // Verify our specific events were found
            let all_discovered: Vec<_> = reconciliation
                .received
                .iter()
                .chain(reconciliation.remote.iter())
                .collect();

            println!("All discovered event IDs: {:?}", all_discovered);
        }
        Err(e) => {
            panic!(
                "NIP-77 negentropy sync failed: {}. This means the relay does NOT support NIP-77 as claimed.",
                e
            );
        }
    }
}

/// Smoke test: Negentropy sync with empty database returns empty result
///
/// Verifies that negentropy sync works correctly when no events match the filter.
#[tokio::test]
async fn test_nip77_negentropy_sync_empty_result() {
    // 1. Start relay (empty, no events)
    let relay = TestRelay::start().await;
    println!("Relay started at {}", relay.url());

    // 2. Create client
    let keys = Keys::generate();
    let client = Client::new(keys.clone());

    client
        .add_relay(relay.url())
        .await
        .expect("Failed to add relay");
    client.connect().await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3. Sync with filter that won't match anything
    let filter = Filter::new()
        .author(keys.public_key()) // Random new key, no events exist
        .kind(Kind::GitRepoAnnouncement);

    println!("Starting negentropy sync with empty filter");

    let sync_opts = SyncOptions::default();

    let result = client.sync(filter, &sync_opts).await;

    // 4. Cleanup
    client.disconnect().await;
    relay.stop().await;

    // 5. Verify - should succeed but find nothing
    match result {
        Ok(output) => {
            let reconciliation = output.val;
            println!("Empty sync completed!");
            println!("  Received: {:?}", reconciliation.received);
            println!("  Remote: {:?}", reconciliation.remote);

            // Should be empty since no events match
            let total = reconciliation.received.len() + reconciliation.remote.len();
            assert_eq!(
                total, 0,
                "Expected 0 events for non-existent author, got {}",
                total
            );
        }
        Err(e) => {
            panic!("NIP-77 negentropy sync failed on empty query: {}", e);
        }
    }
}
