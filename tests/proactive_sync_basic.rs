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

/// Result of checking if an event syncs between relays
#[derive(Debug)]
struct SyncCheckResult {
    /// Whether the event was successfully stored on the source relay
    stored_on_source: bool,
    /// Whether the event was synced to the target relay
    synced_to_target: bool,
}

/// Helper to check if an event syncs from source relay to target relay
///
/// This function:
/// 1. Sends the event to the source relay
/// 2. Verifies if it was stored on the source relay
/// 3. Waits for potential sync
/// 4. Checks if the event appears on the target relay
///
/// Note: The sync subscription must already be established before calling this.
async fn check_event_syncs(
    source_relay: &TestRelay,
    target_relay: &TestRelay,
    event: &Event,
    keys: &Keys,
) -> SyncCheckResult {
    let event_id = event.id;

    // Create client and connect to source relay
    let client_source = create_connected_client(source_relay.url(), keys.clone())
        .await
        .expect("Failed to connect to source relay");

    // Send event to source relay
    let send_result = send_event_reliably(&client_source, event).await;
    let stored_on_source = send_result.is_ok();

    if stored_on_source {
        println!("Event {} stored on source relay", event_id);
    } else {
        println!(
            "Event {} NOT stored on source relay: {:?}",
            event_id,
            send_result.err()
        );
    }

    // Wait for sync to occur
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Check if event exists on target relay
    let client_target = create_connected_client(target_relay.url(), Keys::generate())
        .await
        .expect("Failed to connect to target relay");

    let filter = Filter::new()
        .kind(event.kind)
        .author(keys.public_key());

    let events_on_target = client_target
        .fetch_events(filter, Duration::from_secs(3))
        .await
        .expect("Failed to fetch from target relay");

    let synced_to_target = events_on_target.iter().any(|e| e.id == event_id);

    if synced_to_target {
        println!("Event {} found on target relay (synced)", event_id);
    } else {
        println!("Event {} NOT found on target relay", event_id);
    }

    // Clean up
    client_source.disconnect().await;
    client_target.disconnect().await;

    SyncCheckResult {
        stored_on_source,
        synced_to_target,
    }
}

/// Create a client with keys, connect to relay, and wait for connection
async fn create_connected_client(relay_url: &str, keys: Keys) -> Result<Client, String> {
    let client = Client::new(keys);

    client
        .add_relay(relay_url)
        .await
        .map_err(|e| e.to_string())?;
    client.connect().await;

    // Wait for connection to establish (with retries, matching grasp-audit pattern)
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let relays = client.relays().await;
        if relays.values().any(|r| r.is_connected()) {
            return Ok(client);
        }
    }

    Err("Failed to connect to relay after 3 seconds".to_string())
}

/// Send an event and wait for successful delivery
async fn send_event_reliably(client: &Client, event: &Event) -> Result<EventId, String> {
    // Try sending the event with retries
    for attempt in 1..=5 {
        let result = client.send_event(event).await;
        match result {
            Ok(output) => {
                if !output.success.is_empty() {
                    return Ok(output.val);
                }
                // Check what went wrong
                if !output.failed.is_empty() {
                    println!("  Attempt {} - failures: {:?}", attempt, output.failed);
                    // If relay not connected, try reconnecting
                    client.connect().await;
                }
            }
            Err(e) => {
                println!("  Attempt {} - error: {}", attempt, e);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("Failed to send event after 5 attempts".to_string())
}

/// Create a valid repository announcement event for testing
///
/// This creates a kind 30617 event with required clone and relays tags.
/// Accepts one or more domains - for sync tests, include all relay domains
/// so the event will be accepted by each relay's write policy.
/// Uses TagKind::custom("clone") and TagKind::custom("relays") to match grasp-audit patterns.
fn create_repo_announcement(keys: &Keys, domains: &[&str], identifier: &str) -> Event {
    // Build clone URLs for all domains (with .git suffix)
    let clone_urls: Vec<String> = domains
        .iter()
        .map(|d| format!("http://{}/{}.git", d, identifier))
        .collect();

    // Build relay URLs for all domains
    let relay_urls: Vec<String> = domains.iter().map(|d| format!("ws://{}", d)).collect();

    // Build tags for repository announcement using custom tag kinds (as grasp-audit does)
    let tags = vec![
        Tag::identifier(identifier),
        Tag::custom(TagKind::custom("clone"), clone_urls),
        Tag::custom(TagKind::custom("relays"), relay_urls),
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
    println!(
        "relay_a started at {} (domain: {})",
        relay_a.url(),
        relay_a.domain()
    );

    // Start syncing relay (relay_b) configured to sync from relay_a
    let relay_b = TestRelay::start_with_sync(relay_a.url()).await;
    println!(
        "relay_b started at {} (domain: {})",
        relay_b.url(),
        relay_b.domain()
    );

    // Create test keys
    let keys = Keys::generate();

    // Wait for relay_b's sync connection to establish
    println!("Waiting 1s for relay_b sync connection to establish...");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Create a repository announcement that lists BOTH relays
    // This is required for sync - the event must reference both the source relay
    // and the syncing relay for the write policy to accept it on both sides
    let event = create_repo_announcement(
        &keys,
        &[&relay_a.domain(), &relay_b.domain()],
        "test-repo",
    );
    let event_id = event.id;

    // Print event details for debugging
    println!("Created event {} (kind {})", event_id, event.kind.as_u16());
    for tag in event.tags.iter() {
        println!("  Tag: {:?}", tag.as_slice());
    }

    // Use helper to send and check sync
    let result = check_event_syncs(&relay_a, &relay_b, &event, &keys).await;

    // Clean up
    relay_b.stop().await;
    relay_a.stop().await;

    assert!(
        result.stored_on_source,
        "Event {} was not stored on relay_a! This is a prerequisite for sync.",
        event_id
    );
    assert!(
        result.synced_to_target,
        "Event {} was not synced to relay_b",
        event_id
    );
}

/// Test that events not listing relay_b in their relays tag are NOT synced
///
/// This verifies that relay_b's write policy correctly rejects events during sync
/// if they don't list relay_b as one of their relays.
#[tokio::test]
async fn test_event_not_listing_target_relay_is_not_synced() {
    // Start source relay (relay_a)
    let relay_a = TestRelay::start().await;
    println!(
        "relay_a started at {} (domain: {})",
        relay_a.url(),
        relay_a.domain()
    );

    // Start syncing relay (relay_b) configured to sync from relay_a
    let relay_b = TestRelay::start_with_sync(relay_a.url()).await;
    println!(
        "relay_b started at {} (domain: {})",
        relay_b.url(),
        relay_b.domain()
    );

    // Create test keys
    let keys = Keys::generate();

    // Wait for relay_b's sync connection to establish
    println!("Waiting 1s for relay_b sync connection to establish...");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Create a repository announcement that lists ONLY relay_a (NOT relay_b)
    // This event is valid and will be accepted by relay_a, but should be
    // rejected by relay_b's write policy during sync
    let event = create_repo_announcement(&keys, &[&relay_a.domain()], "test-repo-only-a");
    let event_id = event.id;

    // Print event details for debugging
    println!("Created event {} (kind {})", event_id, event.kind.as_u16());
    for tag in event.tags.iter() {
        println!("  Tag: {:?}", tag.as_slice());
    }
    println!("Note: This event only lists relay_a, not relay_b");

    // Use helper to send and check sync
    let result = check_event_syncs(&relay_a, &relay_b, &event, &keys).await;

    // Clean up
    relay_b.stop().await;
    relay_a.stop().await;

    // Event should be stored on relay_a (it lists relay_a)
    assert!(
        result.stored_on_source,
        "Event {} should have been stored on relay_a (it lists relay_a)",
        event_id
    );

    // Event should NOT be synced to relay_b (it doesn't list relay_b)
    assert!(
        !result.synced_to_target,
        "Event {} should NOT have been synced to relay_b (it doesn't list relay_b)",
        event_id
    );
}
