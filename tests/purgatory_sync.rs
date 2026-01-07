//! Purgatory Sync Integration Tests
//!
//! Tests that verify purgatory sync behavior:
//! - Events entering purgatory are released when git data arrives
//! - Git push triggers unified processing
//! - Remote sync fetches git data and releases events
//!
//! # Test Strategy
//!
//! These tests verify the end-to-end purgatory flow:
//! 1. State/PR events go to purgatory when git data is missing
//! 2. Git push triggers `process_newly_available_git_data`
//! 3. Events are released from purgatory when git data becomes available
//!
//! # Running Tests
//!
//! ```bash
//! # Run all purgatory sync tests
//! cargo test --test purgatory_sync
//!
//! # Run specific test
//! cargo test --test purgatory_sync test_push_triggers_unified_processing
//!
//! # With output for debugging
//! cargo test --test purgatory_sync -- --nocapture
//! ```

mod common;

use common::{
    create_repo_announcement, create_state_event, create_test_repo_with_commit, push_to_relay,
    verify_event_not_served, wait_for_event_served, CommitVariant, TestRelay,
};
use nostr_sdk::prelude::*;
use std::time::Duration;

/// Test that a git push triggers `process_newly_available_git_data` and
/// releases state events from purgatory.
///
/// Scenario:
/// 1. Start relay
/// 2. Create and send repository announcement
/// 3. Create and send state event (goes to purgatory - no git data yet)
/// 4. Verify event is NOT served (in purgatory)
/// 5. Git push the required commit
/// 6. Verify event IS now served (released from purgatory)
#[tokio::test]
async fn test_push_triggers_unified_processing() {
    // 1. Start relay
    let relay = TestRelay::start().await;
    let keys = Keys::generate();
    let identifier = "push-test-repo";

    // 2. Create test repository locally with deterministic commit
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let commit_hash =
        create_test_repo_with_commit(temp_dir.path(), CommitVariant::StateTest)
            .expect("Failed to create test repo");

    // 3. Create and send announcement
    let announcement = create_repo_announcement(&keys, &[&relay.domain()], identifier);

    let client = Client::new(keys.clone());
    client
        .add_relay(relay.url())
        .await
        .expect("Failed to add relay");
    client.connect().await;

    // Wait for connection to be established
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send announcement
    client
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement");

    // Small delay to ensure announcement is processed
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 4. Create and send state event referencing the commit
    // The state event has refs that point to commits we haven't pushed yet
    let npub = keys.public_key().to_bech32().expect("Failed to get npub");
    let clone_url = format!("http://{}/{}/{}.git", relay.domain(), npub, identifier);

    let state_event = create_state_event(
        &keys,
        identifier,
        &[("main", &commit_hash)],
        &[],
        &[&clone_url],
        &[relay.url()],
    )
    .expect("Failed to create state event");

    let state_event_id = state_event.id;
    client
        .send_event(&state_event)
        .await
        .expect("Failed to send state event");

    // 5. Verify event is NOT served yet (in purgatory)
    // Give a moment for the event to be processed into purgatory
    tokio::time::sleep(Duration::from_millis(200)).await;

    verify_event_not_served(relay.url(), &state_event_id, Duration::from_secs(1))
        .await
        .expect("State event should NOT be served before git push (should be in purgatory)");

    // 6. Git push to relay (this should trigger process_newly_available_git_data)
    push_to_relay(temp_dir.path(), &relay.domain(), &npub, identifier)
        .expect("Git push should succeed");

    // 7. Verify event IS now served (released from purgatory)
    let found_event = wait_for_event_served(relay.url(), &state_event_id, Duration::from_secs(5))
        .await
        .expect("State event should be served after git push");

    // Verify it's the same event
    assert_eq!(found_event.id, state_event_id);

    // Cleanup
    client.disconnect().await;
    relay.stop().await;
}
