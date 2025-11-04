/// Integration tests for NIP-34 Repository Announcements (GRASP-01)
/// 
/// Tests the acceptance and validation of repository announcements (kind 30617)
/// and repository state announcements (kind 30618) according to GRASP-01.
///
/// Reference: GRASP-01, Lines 9-20

use futures_util::{SinkExt, StreamExt};
use nostr_sdk::{EventBuilder, Keys, Kind, Tag, TagKind};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

const RELAY_URL: &str = "ws://127.0.0.1:7000";
const DOMAIN: &str = "127.0.0.1:7000";

const KIND_REPOSITORY_ANNOUNCEMENT: u16 = 30617;
const KIND_REPOSITORY_STATE: u16 = 30618;

/// Helper to connect to the relay
async fn connect() -> WsStream {
    let (ws_stream, _) = connect_async(RELAY_URL)
        .await
        .expect("Failed to connect to relay");
    ws_stream
}

/// Helper to send an event and get the response
async fn send_event(ws: &mut WsStream, event: nostr_sdk::Event) -> Value {
    let event_msg = json!(["EVENT", event]);
    ws.send(Message::Text(event_msg.to_string()))
        .await
        .expect("Failed to send event");

    // Read response
    if let Some(Ok(Message::Text(text))) = ws.next().await {
        serde_json::from_str(&text).expect("Failed to parse response")
    } else {
        panic!("No response received");
    }
}

/// Helper to create a repository announcement event
fn create_announcement(
    keys: &Keys,
    identifier: &str,
    clone_urls: Vec<&str>,
    relays: Vec<&str>,
) -> nostr_sdk::Event {
    let mut tags = vec![Tag::custom(TagKind::D, vec![identifier.to_string()])];

    for url in clone_urls {
        tags.push(Tag::custom(
            TagKind::Custom("clone".into()),
            vec![url.to_string()],
        ));
    }

    for relay in relays {
        tags.push(Tag::custom(TagKind::Relays, vec![relay.to_string()]));
    }

    EventBuilder::new(
        Kind::from(KIND_REPOSITORY_ANNOUNCEMENT),
        "Test repository description",
        tags,
    )
    .sign_with_keys(keys)
    .expect("Failed to sign event")
}

/// Helper to create a repository state event
fn create_state(keys: &Keys, identifier: &str, branches: Vec<(&str, &str)>) -> nostr_sdk::Event {
    let mut tags = vec![Tag::custom(TagKind::D, vec![identifier.to_string()])];

    for (branch, commit) in branches {
        tags.push(Tag::custom(
            TagKind::Custom("ref".into()),
            vec![format!("refs/heads/{}", branch), commit.to_string()],
        ));
    }

    EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "", tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// GRASP-01, Line 9-10: MUST serve a NIP-01 compliant nostr relay at `/`
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_relay_accepts_connection() {
    let _ws = connect().await;
    // If we get here, connection succeeded
}

/// GRASP-01, Line 11: MUST accept repository announcements (kind 30617)
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_accepts_valid_announcement() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    let event = create_announcement(
        &keys,
        "test-repo",
        vec![&format!("https://{}/alice/test-repo.git", DOMAIN)],
        vec![&format!("wss://{}", DOMAIN)],
    );

    let response = send_event(&mut ws, event.clone()).await;

    // Should be ["OK", event_id, true, ""]
    assert_eq!(response[0], "OK");
    assert_eq!(response[1], event.id.to_hex());
    assert_eq!(response[2], true, "Event should be accepted");
}

/// GRASP-01, Line 12-13: MUST reject announcements that do not list the service
/// in both `clone` and `relays` tags
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_rejects_announcement_without_clone() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    // Missing clone tag
    let event = create_announcement(
        &keys,
        "test-repo",
        vec![], // No clone URLs
        vec![&format!("wss://{}", DOMAIN)],
    );

    let response = send_event(&mut ws, event.clone()).await;

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
}

/// GRASP-01, Line 12-13: MUST reject announcements that do not list the service
/// in both `clone` and `relays` tags
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_rejects_announcement_without_relay() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    // Missing relay tag
    let event = create_announcement(
        &keys,
        "test-repo",
        vec![&format!("https://{}/alice/test-repo.git", DOMAIN)],
        vec![], // No relays
    );

    let response = send_event(&mut ws, event.clone()).await;

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
}

/// GRASP-01, Line 12-13: MUST reject announcements listing other services
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_rejects_announcement_for_other_service() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    // Lists different service
    let event = create_announcement(
        &keys,
        "test-repo",
        vec!["https://other-service.com/alice/test-repo.git"],
        vec!["wss://other-service.com"],
    );

    let response = send_event(&mut ws, event.clone()).await;

    // Should be rejected
    assert_eq!(response[0], "OK");
    assert_eq!(response[1], event.id.to_hex());
    assert_eq!(response[2], false, "Event should be rejected");
}

/// GRASP-01, Line 11: MUST accept repository state announcements (kind 30618)
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_accepts_valid_state() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    let event = create_state(
        &keys,
        "test-repo",
        vec![("main", "a1b2c3d4e5f6789012345678901234567890abcd")],
    );

    let response = send_event(&mut ws, event.clone()).await;

    // Should be accepted
    assert_eq!(response[0], "OK");
    assert_eq!(response[1], event.id.to_hex());
    assert_eq!(response[2], true, "State event should be accepted");
}

/// Test state event with multiple branches
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_accepts_state_with_multiple_branches() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    let event = create_state(
        &keys,
        "test-repo",
        vec![
            ("main", "a1b2c3d4e5f6789012345678901234567890abcd"),
            ("develop", "b2c3d4e5f6789012345678901234567890abcde"),
            ("feature-x", "c3d4e5f6789012345678901234567890abcdef1"),
        ],
    );

    let response = send_event(&mut ws, event.clone()).await;

    assert_eq!(response[0], "OK");
    assert_eq!(response[2], true, "State event should be accepted");
}

/// Test state event without identifier should be rejected
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_rejects_state_without_identifier() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    // Create state without identifier
    let event = EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "", vec![])
        .sign_with_keys(&keys)
        .expect("Failed to sign event");

    let response = send_event(&mut ws, event.clone()).await;

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
}

/// Test querying for announcements
#[tokio::test]
#[ignore] // Requires relay to be running
async fn test_query_announcements() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    // Send an announcement
    let event = create_announcement(
        &keys,
        "query-test-repo",
        vec![&format!("https://{}/alice/query-test-repo.git", DOMAIN)],
        vec![&format!("wss://{}", DOMAIN)],
    );

    send_event(&mut ws, event.clone()).await;

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
#[ignore] // Requires relay to be running
async fn test_query_states() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    // Send a state event
    let event = create_state(
        &keys,
        "query-test-repo",
        vec![("main", "a1b2c3d4e5f6789012345678901234567890abcd")],
    );

    send_event(&mut ws, event.clone()).await;

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
#[ignore] // Requires relay to be running
async fn test_duplicate_announcement() {
    let mut ws = connect().await;
    let keys = Keys::generate();

    let event = create_announcement(
        &keys,
        "duplicate-test",
        vec![&format!("https://{}/alice/duplicate-test.git", DOMAIN)],
        vec![&format!("wss://{}", DOMAIN)],
    );

    // Send first time
    let response1 = send_event(&mut ws, event.clone()).await;
    assert_eq!(response1[2], true, "First send should succeed");

    // Send second time (duplicate)
    let response2 = send_event(&mut ws, event.clone()).await;
    assert_eq!(response2[2], true, "Duplicate should be acknowledged");
    
    let message = response2[3].as_str().unwrap();
    assert!(
        message.contains("duplicate") || message.is_empty(),
        "Should indicate duplicate: {}",
        message
    );
}
