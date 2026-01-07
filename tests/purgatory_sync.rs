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
    build_repo_coord, check_ref_at_commit, create_pr_event, create_repo_announcement,
    create_state_event, create_test_repo_with_commit, push_ref_to_relay, push_to_relay,
    verify_event_not_served, wait_for_event_served, wait_for_sync_connection, CommitVariant,
    TestRelay,
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

/// Test that a state event entering purgatory triggers remote git fetch
/// and is released once the git data is available.
///
/// Scenario:
/// 1. Start source relay with git repository containing test commit
/// 2. Start syncing relay that syncs from source
/// 3. Syncing relay syncs state event (goes to purgatory - no local git data)
/// 4. Wait for sync to fetch git data from source's clone URL
/// 5. Verify state event is released and served on syncing relay
#[tokio::test]
async fn test_state_event_syncs_from_remote() {
    // 1. Start source relay
    let source_relay = TestRelay::start().await;
    let keys = Keys::generate();
    let identifier = "state-sync-test-repo";

    // Pre-allocate syncing relay port so we can include it in announcement
    let syncing_port = TestRelay::find_free_port();
    let syncing_domain = format!("127.0.0.1:{}", syncing_port);

    // 2. Create test repository locally with deterministic commit
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let commit_hash = create_test_repo_with_commit(temp_dir.path(), CommitVariant::StateTest)
        .expect("Failed to create test repo");

    let npub = keys.public_key().to_bech32().expect("Failed to get npub");

    // 3. Create and send announcement listing BOTH relays
    // This ensures the syncing relay will accept the state event when it syncs
    let announcement = create_repo_announcement(
        &keys,
        &[&source_relay.domain(), &syncing_domain],
        identifier,
    );

    let source_client = Client::new(keys.clone());
    source_client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add source relay");
    source_client.connect().await;

    // Wait for connection
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send announcement to source relay
    source_client
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to source");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 4. Create and send state event BEFORE pushing
    // The state event goes to purgatory on source relay, which authorizes the push
    let clone_urls = [
        format!(
            "http://{}/{}/{}.git",
            source_relay.domain(),
            npub,
            identifier
        ),
        format!("http://{}/{}/{}.git", syncing_domain, npub, identifier),
    ];
    let relay_urls = [
        source_relay.url().to_string(),
        format!("ws://{}", syncing_domain),
    ];

    let state_event = create_state_event(
        &keys,
        identifier,
        &[("main", &commit_hash)],
        &[],
        &[&clone_urls[0], &clone_urls[1]],
        &[&relay_urls[0], &relay_urls[1]],
    )
    .expect("Failed to create state event");

    let state_event_id = state_event.id;

    // Send state event to source relay (goes to purgatory - no git data yet)
    source_client
        .send_event(&state_event)
        .await
        .expect("Failed to send state event to source");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 5. Push git data to source relay
    // The state event in purgatory authorizes this push
    push_to_relay(temp_dir.path(), &source_relay.domain(), &npub, identifier)
        .expect("Push to source should succeed");

    // After push, state event should be released from purgatory on source relay
    // Verify source relay is serving the state event
    wait_for_event_served(source_relay.url(), &state_event_id, Duration::from_secs(5))
        .await
        .expect("State event should be served on source relay after push");

    // 6. Start syncing relay (syncs from source)
    let syncing_relay = TestRelay::start_on_port_with_options(
        syncing_port,
        Some(source_relay.url().to_string()),
        false,
    )
    .await;

    // Wait for sync connection to establish
    wait_for_sync_connection(syncing_relay.url(), 1, Duration::from_secs(5))
        .await
        .expect("Sync connection should establish");

    // 7. Wait for state event to be released on syncing relay
    // The sync should:
    // a) Fetch the announcement and state event from source relay
    // b) Accept announcement (creates bare repo structure)
    // c) Put state event in purgatory (git data missing on syncing relay)
    // d) Fetch git data from source relay's clone URL
    // e) Release the state event from purgatory
    let found = wait_for_event_served(
        syncing_relay.url(),
        &state_event_id,
        Duration::from_secs(30), // Allow time for sync + git fetch
    )
    .await;

    assert!(
        found.is_ok(),
        "State event should be served after sync fetches git data: {:?}",
        found.err()
    );

    // 8. Verify refs are correct on syncing relay
    let ref_correct = check_ref_at_commit(
        &syncing_domain,
        &npub,
        identifier,
        "refs/heads/main",
        &commit_hash,
    )
    .await
    .expect("Failed to check ref");

    assert!(ref_correct, "main branch should point to correct commit");

    // Cleanup
    source_client.disconnect().await;
    syncing_relay.stop().await;
    source_relay.stop().await;
}

/// Test that a PR event entering purgatory triggers remote commit fetch
/// and is released once the commit is available.
///
/// Scenario:
/// 1. Start source relay with repository announcement
/// 2. Create PR event (goes to purgatory - no git data yet)
/// 3. Push commit to refs/nostr/<event-id> (authorized by PR event in purgatory)
/// 4. PR event gets released from purgatory on source relay
/// 5. Start syncing relay
/// 6. Syncing relay syncs PR event (goes to purgatory - no local git data)
/// 7. Syncing relay fetches commit from source's clone URL
/// 8. Verify PR event is released and refs/nostr/<event-id> created on syncing relay
#[tokio::test]
async fn test_pr_event_syncs_from_remote() {
    // 1. Start source relay
    let source_relay = TestRelay::start().await;
    let owner_keys = Keys::generate();
    let pr_author_keys = Keys::generate();
    let identifier = "pr-sync-test-repo";

    // Pre-allocate syncing relay port so we can include it in announcement
    let syncing_port = TestRelay::find_free_port();
    let syncing_domain = format!("127.0.0.1:{}", syncing_port);

    // 2. Create test repository locally with PR commit
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let commit_hash = create_test_repo_with_commit(temp_dir.path(), CommitVariant::PrTest)
        .expect("Failed to create test repo");

    let npub = owner_keys
        .public_key()
        .to_bech32()
        .expect("Failed to get npub");

    // 3. Create and send announcement listing BOTH relays
    // This ensures the syncing relay will accept the PR event when it syncs
    let announcement = create_repo_announcement(
        &owner_keys,
        &[&source_relay.domain(), &syncing_domain],
        identifier,
    );

    let source_client = Client::new(owner_keys.clone());
    source_client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add source relay");
    source_client.connect().await;

    // Wait for connection
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send announcement to source relay (creates bare repo)
    source_client
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to source");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 4. Create and send PR event BEFORE pushing
    // The PR event goes to purgatory on source relay, which authorizes the push
    let repo_coord = build_repo_coord(&owner_keys, identifier);

    let pr_event = create_pr_event(&pr_author_keys, &repo_coord, &commit_hash, "Test PR for sync")
        .expect("Failed to create PR event");

    let pr_event_id = pr_event.id;

    // Send PR event to source relay using PR author's client
    let pr_client = Client::new(pr_author_keys.clone());
    pr_client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add source relay for PR");
    pr_client.connect().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    pr_client
        .send_event(&pr_event)
        .await
        .expect("Failed to send PR event to source");

    // Small delay to ensure PR event is processed into purgatory
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 5. Push commit to refs/nostr/<event-id> on source relay
    // The PR event in purgatory authorizes this push
    let ref_name = format!("refs/nostr/{}", pr_event_id.to_hex());
    push_ref_to_relay(
        temp_dir.path(),
        &source_relay.domain(),
        &npub,
        identifier,
        &commit_hash,
        &ref_name,
    )
    .expect("Push to refs/nostr/<event-id> should succeed");

    // After push, PR event should be released from purgatory on source relay
    wait_for_event_served(source_relay.url(), &pr_event_id, Duration::from_secs(5))
        .await
        .expect("PR event should be served on source relay after push");

    // 6. Start syncing relay (syncs from source)
    let syncing_relay = TestRelay::start_on_port_with_options(
        syncing_port,
        Some(source_relay.url().to_string()),
        false,
    )
    .await;

    // Wait for sync connection to establish
    wait_for_sync_connection(syncing_relay.url(), 1, Duration::from_secs(5))
        .await
        .expect("Sync connection should establish");

    // 7. Wait for PR event to be released on syncing relay
    // The sync should:
    // a) Fetch the announcement and PR event from source relay
    // b) Accept announcement (creates bare repo structure)
    // c) Put PR event in purgatory (commit missing on syncing relay)
    // d) Fetch commit from source relay's clone URL
    // e) Release the PR event from purgatory
    // f) Create refs/nostr/<event-id> pointing to the commit
    let found = wait_for_event_served(
        syncing_relay.url(),
        &pr_event_id,
        Duration::from_secs(30), // Allow time for sync + git fetch
    )
    .await;

    assert!(
        found.is_ok(),
        "PR event should be served after sync fetches commit: {:?}",
        found.err()
    );

    // 8. Verify refs/nostr/<event-id> was created on syncing relay
    let ref_correct = check_ref_at_commit(
        &syncing_domain,
        &npub,
        identifier,
        &ref_name,
        &commit_hash,
    )
    .await
    .expect("Failed to check PR ref");

    assert!(
        ref_correct,
        "refs/nostr/<event-id> should point to PR commit"
    );

    // Cleanup
    source_client.disconnect().await;
    pr_client.disconnect().await;
    syncing_relay.stop().await;
    source_relay.stop().await;
}
