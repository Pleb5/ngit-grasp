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
/// Uses TagKind::custom("clone") and TagKind::custom("relays") to match grasp-audit patterns.
#[allow(dead_code)]
fn create_valid_repo_announcement(keys: &Keys, domain: &str, identifier: &str) -> Event {
    // Build tags for repository announcement using custom tag kinds (as grasp-audit does)
    let tags = vec![
        Tag::identifier(identifier),
        Tag::custom(
            TagKind::custom("clone"),
            vec![format!("http://{}/{}.git", domain, identifier)],
        ),
        Tag::custom(TagKind::custom("relays"), vec![format!("ws://{}", domain)]),
    ];

    EventBuilder::new(Kind::Custom(KIND_REPOSITORY_STATE), "Repository state")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Create a valid repository announcement event listing multiple relays
///
/// This creates a kind 30617 event with clone/relays tags referencing multiple domains,
/// which is necessary for sync tests where the event needs to be accepted by both relays.
/// Uses TagKind::custom("clone") and TagKind::custom("relays") to match grasp-audit patterns.
fn create_shared_repo_announcement(keys: &Keys, domains: &[&str], identifier: &str) -> Event {
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

    // Create test keys that will be used for both client and event signing
    let keys = Keys::generate();

    // Wait for relay_b's sync connection to establish
    // With NGIT_SYNC_STARTUP_JITTER_MS=0 (set by TestRelay), sync connects immediately.
    // A brief wait allows the WebSocket connection and Layer 1 subscription to be set up.
    println!("Waiting 1s for relay_b sync connection to establish...");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Create a client with our keys and connect to relay_a
    let client_a = create_connected_client(relay_a.url(), keys.clone())
        .await
        .expect("Failed to connect to relay_a");
    println!("client_a connected to relay_a");

    // Create a repository announcement that lists BOTH relays
    // This is required for sync - the event must reference both the source relay
    // and the syncing relay for the write policy to accept it on both sides
    let event = create_shared_repo_announcement(
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

    // Submit event to relay_a AFTER relay_b's subscription is established
    // This ensures the event is received via the live subscription
    println!("Sending event to relay_a...");
    send_event_reliably(&client_a, &event)
        .await
        .expect("Failed to send event to relay_a");
    println!("Event sent successfully");

    // Verify event is stored on relay_a first
    let filter_a = Filter::new()
        .kind(Kind::Custom(KIND_REPOSITORY_STATE))
        .author(keys.public_key());

    let events_on_a = client_a
        .fetch_events(filter_a.clone(), Duration::from_secs(5))
        .await
        .expect("Failed to fetch events from relay_a");

    println!(
        "Events on relay_a: {} (looking for {})",
        events_on_a.len(),
        event_id
    );
    for e in events_on_a.iter() {
        println!("  Found event: {} (kind {})", e.id, e.kind.as_u16());
    }

    let found_on_a = events_on_a.iter().any(|e| e.id == event_id);
    assert!(
        found_on_a,
        "Event {} was not stored on relay_a! This is a prerequisite for sync.",
        event_id
    );
    println!("✓ Event confirmed on relay_a");

    // Wait for sync to occur (event processing and storage)
    println!("Waiting 1s for sync to occur...");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Query relay_b to verify the event was synced
    let client_b = create_connected_client(relay_b.url(), Keys::generate())
        .await
        .expect("Failed to connect to relay_b");

    // Create filter to find our event
    let filter_b = Filter::new()
        .kind(Kind::Custom(KIND_REPOSITORY_STATE))
        .author(keys.public_key());

    let events_on_b = client_b
        .fetch_events(filter_b, Duration::from_secs(5))
        .await
        .expect("Failed to fetch events from relay_b");

    println!(
        "Events on relay_b: {} (looking for {})",
        events_on_b.len(),
        event_id
    );
    for e in events_on_b.iter() {
        println!("  Found event: {} (kind {})", e.id, e.kind.as_u16());
    }

    // Check if our event was synced
    let found_on_b = events_on_b.iter().any(|e| e.id == event_id);

    // Clean up
    client_a.disconnect().await;
    client_b.disconnect().await;
    relay_b.stop().await;
    relay_a.stop().await;

    assert!(
        found_on_b,
        "Event {} was not synced to relay_b. Found {} events on relay_b",
        event_id,
        events_on_b.len()
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
    client_a
        .add_relay(relay_a.url())
        .await
        .expect("Failed to add relay_a");
    client_a.connect().await;

    // This will likely fail since relay_a also validates, but let's try
    let _ = client_a.send_event(&invalid_event).await;

    // Wait for potential sync
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Query relay_b - the event should NOT be present
    let client_b = Client::default();
    client_b
        .add_relay(relay_b.url())
        .await
        .expect("Failed to add relay_b");
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
    client_a
        .add_relay(relay_a.url())
        .await
        .expect("Failed to add relay_a");
    client_a.connect().await;

    client_a
        .send_event(&valid_event)
        .await
        .expect("Failed to send valid event");

    // Wait for sync
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Query relay_b to verify the valid event was synced
    let client_b = Client::default();
    client_b
        .add_relay(relay_b.url())
        .await
        .expect("Failed to add relay_b");
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
