//! NIP-01 Compliance Integration Tests
//!
//! Tests ngit-grasp relay's NIP-01 compliance using grasp-audit library.
//! Uses isolated test pattern for complete test independence.
//!
//! # Test Strategy
//!
//! - Each test runs in complete isolation with its own fresh relay instance
//! - Uses macro to eliminate boilerplate while maintaining test isolation
//! - Calls individual test methods from grasp-audit for minimal duplication
//!
//! # Running Tests
//!
//! ```bash
//! # Run all NIP-01 compliance tests
//! cargo test --test nip01_compliance
//!
//! # Run specific test
//! cargo test --test nip01_compliance test_websocket_connection
//!
//! # With output
//! cargo test --test nip01_compliance -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::*;

/// Macro to generate isolated integration tests
///
/// Each test runs with its own fresh relay instance to ensure complete isolation.
/// This eliminates flakiness and ensures tests don't interfere with each other.
macro_rules! isolated_test {
    ($test_name:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start().await;
            let config = AuditConfig::isolated();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = specs::Nip01SmokeTests::$test_name(&client).await;

            relay.stop().await;

            assert!(
                result.passed,
                "{} failed: {}",
                stringify!($test_name),
                result.error.as_deref().unwrap_or("unknown error")
            );
        }
    };
}

// Generate isolated tests for all NIP-01 smoke tests
isolated_test!(test_websocket_connection);
isolated_test!(test_send_receive_event);
isolated_test!(test_create_subscription);
isolated_test!(test_close_subscription);
isolated_test!(test_reject_invalid_signature);
isolated_test!(test_reject_invalid_event_id);

/// Test relay lifecycle management
///
/// Verifies TestRelay fixture properly manages relay lifecycle
#[tokio::test]
async fn test_relay_lifecycle() {
    // Start relay
    let relay = TestRelay::start().await;
    let url = relay.url().to_string();

    // Verify we can connect
    let config = AuditConfig::isolated();
    let client = AuditClient::new(&url, config)
        .await
        .expect("Failed to connect to relay");

    assert!(client.is_connected().await, "Client should be connected");

    // Stop relay
    relay.stop().await;
}

/// Test multiple relays can run in parallel
///
/// Ensures random port selection avoids conflicts
#[tokio::test]
async fn test_parallel_relays() {
    // Start two relays simultaneously
    let relay1 = TestRelay::start().await;
    let relay2 = TestRelay::start().await;

    // Should have different URLs (different ports)
    assert_ne!(
        relay1.url(),
        relay2.url(),
        "Relays should use different ports"
    );

    // Both should be connectable
    let config = AuditConfig::isolated();

    let client1 = AuditClient::new(relay1.url(), config.clone())
        .await
        .expect("Failed to connect to relay 1");

    let client2 = AuditClient::new(relay2.url(), config)
        .await
        .expect("Failed to connect to relay 2");

    assert!(client1.is_connected().await);
    assert!(client2.is_connected().await);

    // Clean up
    relay1.stop().await;
    relay2.stop().await;
}
