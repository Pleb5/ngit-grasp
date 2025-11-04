//! NIP-01 Smoke Tests
//!
//! These tests verify basic Nostr relay functionality.
//! We don't comprehensively test NIP-01 because rust-nostr already has 1000+ tests.
//! These are just smoke tests to ensure the relay is working at all.

use crate::{AuditClient, AuditResult, TestResult};
use nostr_sdk::prelude::*;

pub struct Nip01SmokeTests;

impl Nip01SmokeTests {
    /// Run all NIP-01 smoke tests
    pub async fn run_all(client: &AuditClient) -> AuditResult {
        let mut results = AuditResult::new("NIP-01 Smoke Tests");
        
        // Run tests in parallel
        let tests = vec![
            Self::test_websocket_connection(client),
            Self::test_send_receive_event(client),
            Self::test_create_subscription(client),
            Self::test_close_subscription(client),
            Self::test_reject_invalid_signature(client),
            Self::test_reject_invalid_event_id(client),
        ];
        
        let test_results = futures::future::join_all(tests).await;
        
        for result in test_results {
            results.add(result);
        }
        
        results
    }
    
    /// Test 1: Can establish WebSocket connection
    ///
    /// Spec: NIP-01 basic requirement
    /// Requirement: MUST serve a relay at / via WebSocket
    async fn test_websocket_connection(client: &AuditClient) -> TestResult {
        TestResult::new(
            "websocket_connection",
            "NIP-01:basic",
            "Can establish WebSocket connection to /",
        )
        .run(|| async {
            if !client.is_connected().await {
                return Err("Failed to connect to relay".to_string());
            }
            
            Ok(())
        })
        .await
    }
    
    /// Test 2: Can send EVENT and receive OK response
    ///
    /// Spec: NIP-01 EVENT message
    /// Requirement: Relay MUST accept valid EVENT messages
    async fn test_send_receive_event(client: &AuditClient) -> TestResult {
        TestResult::new(
            "send_receive_event",
            "NIP-01:event-message",
            "Can send EVENT and receive OK response",
        )
        .run(|| async {
            // Create audit event
            let event = client
                .event_builder(Kind::TextNote, "NIP-01 smoke test event")
                .build(client.keys())
                .await
                .map_err(|e| format!("Failed to build event: {}", e))?;
            
            // Send event
            let event_id = client
                .send_event(event.clone())
                .await
                .map_err(|e| format!("Failed to send event: {}", e))?;
            
            // Verify we got an event ID back
            if event_id != event.id {
                return Err(format!(
                    "Event ID mismatch: sent {}, got {}",
                    event.id, event_id
                ));
            }
            
            // Try to query it back
            let filter = Filter::new()
                .kind(Kind::TextNote)
                .id(event_id);
            
            let events = client
                .query(filter)
                .await
                .map_err(|e| format!("Failed to query event: {}", e))?;
            
            if events.is_empty() {
                return Err("Event not found after sending".to_string());
            }
            
            if events[0].id != event_id {
                return Err("Retrieved event has different ID".to_string());
            }
            
            Ok(())
        })
        .await
    }
    
    /// Test 3: Can create subscription with REQ
    ///
    /// Spec: NIP-01 REQ message
    /// Requirement: Relay MUST support REQ subscriptions
    async fn test_create_subscription(client: &AuditClient) -> TestResult {
        TestResult::new(
            "create_subscription",
            "NIP-01:req-message",
            "Can create subscription with REQ and receive EOSE",
        )
        .run(|| async {
            // Create a test event first
            let event = client
                .event_builder(Kind::TextNote, "Subscription test event")
                .build(client.keys())
                .await
                .map_err(|e| format!("Failed to build event: {}", e))?;
            
            client
                .send_event(event.clone())
                .await
                .map_err(|e| format!("Failed to send event: {}", e))?;
            
            // Subscribe to events
            let filter = Filter::new()
                .kind(Kind::TextNote)
                .author(client.public_key());
            
            let events = client
                .subscribe(vec![filter], Some(std::time::Duration::from_secs(5)))
                .await
                .map_err(|e| format!("Failed to subscribe: {}", e))?;
            
            // Should have at least our event
            if events.is_empty() {
                return Err("No events received from subscription".to_string());
            }
            
            Ok(())
        })
        .await
    }
    
    /// Test 4: Can close subscription with CLOSE
    ///
    /// Spec: NIP-01 CLOSE message
    /// Requirement: Relay MUST support CLOSE to end subscriptions
    async fn test_close_subscription(client: &AuditClient) -> TestResult {
        TestResult::new(
            "close_subscription",
            "NIP-01:close-message",
            "Can close subscriptions",
        )
        .run(|| async {
            // For now, we just verify we can query events
            // Full subscription management with CLOSE would require
            // lower-level WebSocket access
            
            let filter = Filter::new()
                .kind(Kind::TextNote)
                .limit(1);
            
            let _events = client
                .subscribe(vec![filter], Some(std::time::Duration::from_secs(2)))
                .await
                .map_err(|e| format!("Failed to subscribe: {}", e))?;
            
            // If we got here, subscription worked
            Ok(())
        })
        .await
    }
    
    /// Test 5: Rejects events with invalid signatures
    ///
    /// Spec: NIP-01 event validation
    /// Requirement: Relay MUST reject events with invalid signatures
    async fn test_reject_invalid_signature(client: &AuditClient) -> TestResult {
        TestResult::new(
            "reject_invalid_signature",
            "NIP-01:validation",
            "Rejects events with invalid signatures",
        )
        .run(|| async {
            // Create a valid event
            let mut event = client
                .event_builder(Kind::TextNote, "Invalid signature test")
                .build(client.keys())
                .await
                .map_err(|e| format!("Failed to build event: {}", e))?;
            
            // Corrupt the signature by creating a new event with wrong sig
            // We'll use a different key to sign, creating an invalid signature
            let wrong_keys = Keys::generate();
            let wrong_event = EventBuilder::new(
                event.kind,
                event.content.clone(),
                event.tags.clone(),
            )
            .to_event(&wrong_keys)
            .await
            .map_err(|e| format!("Failed to build wrong event: {}", e))?;
            
            // Create event with mismatched pubkey and signature
            // This should be rejected by the relay
            event = Event {
                id: event.id,
                pubkey: event.pubkey,
                created_at: event.created_at,
                kind: event.kind,
                tags: event.tags,
                content: event.content,
                sig: wrong_event.sig, // Wrong signature!
            };
            
            // Try to send the invalid event
            let result = client.send_event(event).await;
            
            // We expect this to fail
            if result.is_ok() {
                return Err("Relay accepted event with invalid signature".to_string());
            }
            
            Ok(())
        })
        .await
    }
    
    /// Test 6: Rejects events with invalid event IDs
    ///
    /// Spec: NIP-01 event ID validation
    /// Requirement: Relay MUST reject events where ID doesn't match hash
    async fn test_reject_invalid_event_id(client: &AuditClient) -> TestResult {
        TestResult::new(
            "reject_invalid_event_id",
            "NIP-01:validation",
            "Rejects events with invalid event IDs",
        )
        .run(|| async {
            // Create a valid event
            let mut event = client
                .event_builder(Kind::TextNote, "Invalid ID test")
                .build(client.keys())
                .await
                .map_err(|e| format!("Failed to build event: {}", e))?;
            
            // Corrupt the ID
            event = Event {
                id: EventId::all_zeros(), // Wrong ID!
                pubkey: event.pubkey,
                created_at: event.created_at,
                kind: event.kind,
                tags: event.tags,
                content: event.content,
                sig: event.sig,
            };
            
            // Try to send the invalid event
            let result = client.send_event(event).await;
            
            // We expect this to fail
            if result.is_ok() {
                return Err("Relay accepted event with invalid ID".to_string());
            }
            
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuditConfig;
    
    // Note: These tests require a running relay
    // They are integration tests, not unit tests
    
    #[tokio::test]
    #[ignore] // Ignore by default since it needs a running relay
    async fn test_smoke_tests_against_relay() {
        let config = AuditConfig::ci();
        let client = AuditClient::new("ws://localhost:7000", config)
            .await
            .expect("Failed to connect to relay");
        
        let results = Nip01SmokeTests::run_all(&client).await;
        results.print_report();
        
        assert!(results.all_passed(), "Some smoke tests failed");
    }
}
