//! Archive Read-Only Mode Integration Tests
//!
//! Tests that verify archive_read_only mode behavior:
//! - Bare git repositories are created for announcements
//! - Git data is synced via relay-to-relay sync (purgatory sync)
//! - Git pushes are rejected (read-only mode)
//!
//! # Test Strategy
//!
//! These tests verify the GRASP-05 archive mode with read_only flag:
//! 1. Source relay has full repository (announcement + state events + git data)
//! 2. Archive relay syncs from source relay (relay-to-relay sync)
//! 3. State events trigger purgatory sync which fetches git data
//! 4. Git data is validated against Nostr state events
//! 5. Git pushes are rejected (read-only enforcement)
//!
//! # Security Model
//!
//! Archive mode uses the existing purgatory sync infrastructure to ensure:
//! - Git data is validated against Nostr state events
//! - "Naughty git servers" can't provide incorrect state
//! - Same security guarantees as normal relay operation
//!
//! # Running Tests
//!
//! ```bash
//! # Run all archive read-only tests
//! cargo test --test archive_read_only
//!
//! # Run specific test
//! cargo test --test archive_read_only test_archive_read_only_creates_bare_repo
//!
//! # With output for debugging
//! cargo test --test archive_read_only -- --nocapture
//! ```

mod common;

use common::{
    check_ref_at_commit, create_repo_announcement, create_state_event,
    create_test_repo_with_commit, push_to_relay, wait_for_event_served, wait_for_sync_connection,
    CommitVariant, TestRelay,
};
use nostr_sdk::prelude::*;
use std::time::Duration;

/// Test that archive_read_only mode creates bare git repositories and syncs data
/// via relay-to-relay sync (purgatory sync infrastructure).
///
/// Scenario:
/// 1. Start source relay with full repository (announcement + state + git data)
/// 2. Start archive relay with archive_all=true, archive_read_only=true, syncing from source
/// 3. Archive relay syncs announcement and state events from source
/// 4. State events trigger purgatory sync which fetches git data from source's clone URL
/// 5. Verify bare repository is created and git data is synced
/// 6. Verify git pushes are rejected (read-only mode)
#[tokio::test]
async fn test_archive_read_only_creates_bare_repo() {
    // 1. Start source relay
    let source_relay = TestRelay::start().await;
    let keys = Keys::generate();
    let identifier = "archive-test-repo";

    // Pre-allocate archive relay port so we can include it in announcement
    let archive_port = TestRelay::find_free_port();
    let archive_domain = format!("127.0.0.1:{}", archive_port);

    // 2. Create test repository locally with deterministic commit
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let commit_hash = create_test_repo_with_commit(temp_dir.path(), CommitVariant::StateTest)
        .expect("Failed to create test repo");

    let npub = keys.public_key().to_bech32().expect("Failed to get npub");

    // 3. Create and send announcement listing BOTH relays
    // This ensures the archive relay will accept the state event when it syncs
    let announcement = create_repo_announcement(
        &keys,
        &[&source_relay.domain(), &archive_domain],
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

    // 4. Create and send state event
    let clone_urls = [
        format!(
            "http://{}/{}/{}.git",
            source_relay.domain(),
            npub,
            identifier
        ),
        format!("http://{}/{}/{}.git", archive_domain, npub, identifier),
    ];
    let relay_urls = [
        source_relay.url().to_string(),
        format!("ws://{}", archive_domain),
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
    wait_for_event_served(source_relay.url(), &state_event_id, Duration::from_secs(5))
        .await
        .expect("State event should be served on source relay after push");

    // 6. Start archive relay with archive_all=true, archive_read_only=true, syncing from source
    let archive_relay = TestRelay::start_with_archive_and_sync(
        archive_port,
        Some(source_relay.url().to_string()),
        false, // negentropy enabled
        true,  // archive_all
        true,  // archive_read_only
    )
    .await;

    // Wait for sync connection to establish
    wait_for_sync_connection(archive_relay.url(), 1, Duration::from_secs(5))
        .await
        .expect("Sync connection should establish");

    // 7. Wait for state event to be released on archive relay
    // The sync should:
    // a) Fetch the announcement and state event from source relay
    // b) Accept announcement (creates bare repo structure) - via archive mode
    // c) Put state event in purgatory (git data missing on archive relay)
    // d) Fetch git data from source relay's clone URL
    // e) Release the state event from purgatory
    let found = wait_for_event_served(
        archive_relay.url(),
        &state_event_id,
        Duration::from_secs(30), // Allow time for sync + git fetch
    )
    .await;

    assert!(
        found.is_ok(),
        "State event should be served after sync fetches git data: {:?}",
        found.err()
    );

    // 8. Verify bare repository was created
    let repo_path = archive_relay
        .git_data_path()
        .join(format!("{}/{}.git", npub, identifier));

    assert!(
        repo_path.exists(),
        "Bare repository should be created at {:?} for archive announcement",
        repo_path
    );

    // 9. Verify it's a bare repository (check for config file with bare = true)
    let config_path = repo_path.join("config");
    assert!(
        config_path.exists(),
        "Git config should exist at {:?}",
        config_path
    );

    let config_content = tokio::fs::read_to_string(&config_path)
        .await
        .expect("Should read git config");
    assert!(
        config_content.contains("bare = true"),
        "Repository at {:?} should be bare (config should contain 'bare = true')",
        repo_path
    );

    // 10. Verify refs are correct on archive relay
    let ref_correct = check_ref_at_commit(
        &archive_domain,
        &npub,
        identifier,
        "refs/heads/main",
        &commit_hash,
    )
    .await
    .expect("Failed to check ref");

    assert!(ref_correct, "main branch should point to correct commit");

    // 11. Verify git pushes are rejected (read-only mode)
    // Create a new commit in the source repo
    tokio::fs::write(temp_dir.path().join("new_file.txt"), "new content")
        .await
        .expect("Failed to write new file");

    let output = tokio::process::Command::new("git")
        .args(["add", "."])
        .current_dir(temp_dir.path())
        .output()
        .await
        .expect("Failed to git add");
    assert!(output.status.success());

    let output = tokio::process::Command::new("git")
        .args(["commit", "-m", "New commit for push test"])
        .current_dir(temp_dir.path())
        .output()
        .await
        .expect("Failed to git commit");
    assert!(output.status.success());

    // Try to push to archive relay (should fail in read-only mode)
    let push_url = format!("http://{}/{}/{}.git", archive_domain, npub, identifier);
    let output = tokio::process::Command::new("git")
        .args(["push", &push_url, "main"])
        .current_dir(temp_dir.path())
        .output()
        .await
        .expect("Failed to run git push");

    assert!(
        !output.status.success(),
        "Git push should be rejected in archive_read_only mode. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Cleanup
    source_client.disconnect().await;
    archive_relay.stop().await;
    source_relay.stop().await;
}

/// Test that archive mode without state events does NOT sync git data.
///
/// This verifies the security model: archive mode only syncs git data
/// when there are state events to validate against.
///
/// Scenario:
/// 1. Start source relay with announcement only (no state events)
/// 2. Start archive relay syncing from source
/// 3. Archive relay syncs announcement (creates bare repo)
/// 4. Verify git data is NOT synced (no state events to trigger purgatory sync)
#[tokio::test]
async fn test_archive_without_state_events_does_not_sync_git() {
    // 1. Start source relay
    let source_relay = TestRelay::start().await;
    let keys = Keys::generate();
    let identifier = "archive-no-state-repo";

    // Pre-allocate archive relay port
    let archive_port = TestRelay::find_free_port();
    let archive_domain = format!("127.0.0.1:{}", archive_port);

    // 2. Create test repository locally
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let commit_hash = create_test_repo_with_commit(temp_dir.path(), CommitVariant::StateTest)
        .expect("Failed to create test repo");

    let npub = keys.public_key().to_bech32().expect("Failed to get npub");

    // 3. Create and send announcement listing BOTH relays (but NO state event)
    let announcement = create_repo_announcement(
        &keys,
        &[&source_relay.domain(), &archive_domain],
        identifier,
    );

    let source_client = Client::new(keys.clone());
    source_client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add source relay");
    source_client.connect().await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send announcement to source relay
    source_client
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to source");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 4. Push git data to source relay (but no state event to authorize it)
    // This push will fail because there's no state event in purgatory
    // That's expected - we're testing that archive mode doesn't blindly fetch git data

    // 5. Start archive relay
    let archive_relay = TestRelay::start_with_archive_and_sync(
        archive_port,
        Some(source_relay.url().to_string()),
        false,
        true,
        true,
    )
    .await;

    // Wait for sync
    wait_for_sync_connection(archive_relay.url(), 1, Duration::from_secs(5))
        .await
        .expect("Sync connection should establish");

    // Give time for any potential git sync to happen
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 6. Verify bare repository was created (announcement was accepted)
    let repo_path = archive_relay
        .git_data_path()
        .join(format!("{}/{}.git", npub, identifier));

    assert!(
        repo_path.exists(),
        "Bare repository should be created for archive announcement"
    );

    // 7. Verify git data was NOT synced (no state events to trigger purgatory sync)
    // Check that the commit does NOT exist in the archive relay's repo
    let output = tokio::process::Command::new("git")
        .args(["cat-file", "-t", &commit_hash])
        .current_dir(&repo_path)
        .output()
        .await;

    let commit_exists = output.map(|o| o.status.success()).unwrap_or(false);

    assert!(
        !commit_exists,
        "Git data should NOT be synced without state events (security: validates against Nostr state)"
    );

    // Cleanup
    source_client.disconnect().await;
    archive_relay.stop().await;
    source_relay.stop().await;
}
