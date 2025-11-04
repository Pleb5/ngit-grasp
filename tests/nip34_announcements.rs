//! NIP-34 Repository Announcements Integration Tests (GRASP-01)
//!
//! Tests the acceptance and validation of repository announcements (kind 30617)
//! and repository state announcements (kind 30618) according to GRASP-01.
//!
//! Reference: GRASP-01, Lines 9-20
//!
//! # Test Strategy
//!
//! - Uses TestRelay fixture for automatic relay lifecycle management
//! - Pure Rust, no shell scripts
//! - Tests run in parallel with isolated relay instances
//!
//! # Running Tests
//!
//! ```bash
//! # Run all NIP-34 announcement tests
//! cargo test --test nip34_announcements
//!
//! # Run specific test
//! cargo test --test nip34_announcements test_accepts_valid_announcement
//!
//! # With output
//! cargo test --test nip34_announcements -- --nocapture
//! ```

mod common;

use common::TestRelay;
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::{EventBuilder, Keys, Kind, Tag, TagKind};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const KIND_REPOSITORY_ANNOUNCEMENT: u16 = 30617;
const KIND_REPOSITORY_STATE: u16 = 30618;

/// Helper to connect to a test relay
async fn connect_to_relay(url: &str) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = connect_async(url)
        .await
        .expect("Failed to connect to relay");
    ws
}

/// Helper to create a repository announcement event
fn create_announcement(
    keys: &Keys,
    _domain: &str,
    identifier: &str,
    clone_urls: Vec<String>,
    relays: Vec<String>,
) -> nostr_sdk::Event {
    let mut tags = vec![Tag::custom(TagKind::d(), vec![identifier.to_string()])];

    for url in clone_urls {
        tags.push(Tag::custom(
            TagKind::Clone,
            vec![url],
        ));
    }

    for relay in relays {
        tags.push(Tag::custom(TagKind::Relays, vec![relay]));
    }

    EventBuilder::new(
        Kind::from(KIND_REPOSITORY_ANNOUNCEMENT),
        "Test repository description",
    )
    .tags(tags)
    .sign_with_keys(keys)
    .expect("Failed to sign event")
}

/// Helper to create a repository state event
fn create_state(keys: &Keys, identifier: &str, branches: Vec<(&str, &str)>) -> nostr_sdk::Event {
    let mut tags = vec![Tag::custom(TagKind::d(), vec![identifier.to_string()])];

    for (branch, commit) in branches {
        tags.push(Tag::custom(
            TagKind::Custom("ref".into()),
            vec![format!("refs/heads/{}", branch), commit.to_string()],
        ));
    }

    EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// GRASP-01, Line 9-10: MUST serve a NIP-01 compliant nostr relay at `/`
#[tokio::test]
async fn test_relay_accepts_connection() {
    let relay = TestRelay::start().await;
    
    // Try to connect
    let ws = connect_to_relay(relay.url()).await;
    
    drop(ws); // Clean disconnect
}

/// GRASP-01, Line 11: MUST accept repository announcements (kind 30617)
#[tokio::test]
async fn test_accepts_valid_announcement() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let mut ws = connect_to_relay(relay.url()).await;

    let event = create_announcement(
        &keys,
        &relay.domain(),
        "test-repo",
        vec![format!("https://{}/alice/test-repo.git", relay.domain())],
        vec![format!("wss://{}", relay.domain())],
    );

    // Send event
    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    // Read response
    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: Value = serde_json::from_str(&text).expect("Failed to parse response");
        
        // Should be ["OK", event_id, true, ""]
        assert_eq!(response[0], "OK");
        assert_eq!(response[1], event.id.to_hex());
        if response[2] != true {
            eprintln!("Event rejected: {}", response[3]);
        }
        assert_eq!(response[2], true, "Event should be accepted");
    } else {
        panic!("No response received");
    }
}

/// GRASP-01, Line 12-13: MUST reject announcements that do not list the service
/// in both `clone` and `relays` tags
#[tokio::test]
async fn test_rejects_announcement_without_clone() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    // Missing clone tag
    let event = create_announcement(
        &keys,
        &relay.domain(),
        "test-repo",
        vec![], // No clone URLs
        vec![format!("wss://{}", relay.domain())],
    );

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: Value = serde_json::from_str(&text).expect("Failed to parse");
        
        // Should be rejected
        assert_eq!(response[0], "OK");
        assert_eq!(response[1], event.id.to_hex());
        assert_eq!(response[2], false, "Event should be rejected");
        
        let message = response[3].as_str().unwrap();
        assert!(
            message.contains("clone") || message.contains("invalid"),
            "Error message should mention clone requirement: {}",
            message
        );
    } else {
        panic!("No response received");
    }
}

/// GRASP-01, Line 12-13: MUST reject announcements that do not list the service
/// in both `clone` and `relays` tags
#[tokio::test]
async fn test_rejects_announcement_without_relay() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    // Missing relay tag
    let event = create_announcement(
        &keys,
        &relay.domain(),
        "test-repo",
        vec![format!("https://{}/alice/test-repo.git", relay.domain())],
        vec![], // No relays
    );

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: Value = serde_json::from_str(&text).expect("Failed to parse");
        
        // Should be rejected
        assert_eq!(response[0], "OK");
        assert_eq!(response[1], event.id.to_hex());
        assert_eq!(response[2], false, "Event should be rejected");
        
        let message = response[3].as_str().unwrap();
        assert!(
            message.contains("relays") || message.contains("invalid"),
            "Error message should mention relay requirement: {}",
            message
        );
    } else {
        panic!("No response received");
    }
}

/// GRASP-01, Line 12-13: MUST reject announcements listing other services
#[tokio::test]
async fn test_rejects_announcement_for_other_service() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    // Lists different service
    let event = create_announcement(
        &keys,
        &relay.domain(),
        "test-repo",
        vec!["https://other-service.com/alice/test-repo.git".to_string()],
        vec!["wss://other-service.com".to_string()],
    );

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: Value = serde_json::from_str(&text).expect("Failed to parse");
        
        // Should be rejected
        assert_eq!(response[0], "OK");
        assert_eq!(response[1], event.id.to_hex());
        assert_eq!(response[2], false, "Event should be rejected");
    } else {
        panic!("No response received");
    }
}

/// GRASP-01, Line 11: MUST accept repository state announcements (kind 30618)
#[tokio::test]
async fn test_accepts_valid_state() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    let event = create_state(
        &keys,
        "test-repo",
        vec![("main", "a1b2c3d4e5f6789012345678901234567890abcd")],
    );

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: Value = serde_json::from_str(&text).expect("Failed to parse");
        
        // Should be accepted
        assert_eq!(response[0], "OK");
        assert_eq!(response[1], event.id.to_hex());
        assert_eq!(response[2], true, "State event should be accepted");
    } else {
        panic!("No response received");
    }
}

/// Test state event with multiple branches
#[tokio::test]
async fn test_accepts_state_with_multiple_branches() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    let event = create_state(
        &keys,
        "test-repo",
        vec![
            ("main", "a1b2c3d4e5f6789012345678901234567890abcd"),
            ("develop", "b2c3d4e5f6789012345678901234567890abcde"),
            ("feature-x", "c3d4e5f6789012345678901234567890abcdef1"),
        ],
    );

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: Value = serde_json::from_str(&text).expect("Failed to parse");
        
        assert_eq!(response[0], "OK");
        assert_eq!(response[2], true, "State event should be accepted");
    } else {
        panic!("No response received");
    }
}

/// Test state event without identifier should be rejected
#[tokio::test]
async fn test_rejects_state_without_identifier() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    // Create state without identifier
    let event = EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "")
        .sign_with_keys(&keys)
        .expect("Failed to sign event");

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: Value = serde_json::from_str(&text).expect("Failed to parse");
        
        // Should be rejected
        assert_eq!(response[0], "OK");
        assert_eq!(response[1], event.id.to_hex());
        assert_eq!(response[2], false, "Event should be rejected");
        
        let message = response[3].as_str().unwrap();
        assert!(
            message.contains("identifier") || message.contains("invalid"),
            "Error message should mention identifier requirement: {}",
            message
        );
    } else {
        panic!("No response received");
    }
}

/// Test querying for announcements
#[tokio::test]
async fn test_query_announcements() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    // Send an announcement
    let event = create_announcement(
        &keys,
        &relay.domain(),
        "query-test-repo",
        vec![format!("https://{}/alice/query-test-repo.git", relay.domain())],
        vec![format!("wss://{}", relay.domain())],
    );

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    // Wait for OK response
    if let Some(Ok(Message::Text(_))) = ws.next().await {
        // Got OK response
    }

    // Query for announcements
    let req = json!([
        "REQ",
        "test-sub",
        {
            "kinds": [KIND_REPOSITORY_ANNOUNCEMENT],
            "authors": [keys.public_key().to_hex()]
        }
    ]);

    ws.send(Message::Text(req.to_string()))
        .await
        .expect("Failed to send REQ");

    // Read responses
    let mut found_event = false;
    let mut got_eose = false;

    for _ in 0..10 {
        if let Some(Ok(Message::Text(text))) = ws.next().await {
            let response: Value = serde_json::from_str(&text).expect("Failed to parse");
            
            if response[0] == "EVENT" {
                assert_eq!(response[1], "test-sub");
                found_event = true;
            } else if response[0] == "EOSE" {
                assert_eq!(response[1], "test-sub");
                got_eose = true;
                break;
            }
        }
    }

    assert!(found_event, "Should have received the announcement");
    assert!(got_eose, "Should have received EOSE");
}

/// Test querying for state events
#[tokio::test]
async fn test_query_states() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    // Send a state event
    let event = create_state(
        &keys,
        "query-test-repo",
        vec![("main", "a1b2c3d4e5f6789012345678901234567890abcd")],
    );

    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    // Wait for OK response
    if let Some(Ok(Message::Text(_))) = ws.next().await {
        // Got OK response
    }

    // Query for states
    let req = json!([
        "REQ",
        "test-sub",
        {
            "kinds": [KIND_REPOSITORY_STATE],
            "authors": [keys.public_key().to_hex()]
        }
    ]);

    ws.send(Message::Text(req.to_string()))
        .await
        .expect("Failed to send REQ");

    // Read responses
    let mut found_event = false;
    let mut got_eose = false;

    for _ in 0..10 {
        if let Some(Ok(Message::Text(text))) = ws.next().await {
            let response: Value = serde_json::from_str(&text).expect("Failed to parse");
            
            if response[0] == "EVENT" {
                assert_eq!(response[1], "test-sub");
                found_event = true;
            } else if response[0] == "EOSE" {
                assert_eq!(response[1], "test-sub");
                got_eose = true;
                break;
            }
        }
    }

    assert!(found_event, "Should have received the state event");
    assert!(got_eose, "Should have received EOSE");
}

/// Test duplicate event handling
#[tokio::test]
async fn test_duplicate_announcement() {
    let relay = TestRelay::start().await;
    let keys = Keys::generate();

    let (mut ws, _) = connect_async(relay.url())
        .await
        .expect("Failed to connect");

    let event = create_announcement(
        &keys,
        &relay.domain(),
        "duplicate-test",
        vec![format!("https://{}/alice/duplicate-test.git", relay.domain())],
        vec![format!("wss://{}", relay.domain())],
    );

    // Send first time
    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response1: Value = serde_json::from_str(&text).expect("Failed to parse");
        assert_eq!(response1[2], true, "First send should succeed");
    }

    // Send second time (duplicate)
    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response2: Value = serde_json::from_str(&text).expect("Failed to parse");
        assert_eq!(response2[2], true, "Duplicate should be acknowledged");
        
        let message = response2[3].as_str().unwrap();
        assert!(
            message.contains("duplicate") || message.is_empty(),
            "Should indicate duplicate: {}",
            message
        );
    }
}
