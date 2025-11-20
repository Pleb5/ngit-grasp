//! NIP-01 Compliance Integration Tests
//!
//! Tests ngit-grasp relay's NIP-01 compliance using grasp-audit library.
//! Avoids code duplication by delegating to grasp-audit's test suite.
//!
//! # Test Strategy
//!
//! - Uses TestRelay fixture for ngit-grasp relay lifecycle management
//! - Uses grasp-audit's Nip01SmokeTests for actual test logic
//! - Minimal duplication - single source of truth in grasp-audit
//!
//! # Running Tests
//!
//! ```bash
//! # Run all NIP-01 compliance tests
//! cargo test --test nip01_compliance
//!
//! # Run specific test
//! cargo test --test nip01_compliance test_nip01_smoke
//!
//! # With output
//! cargo test --test nip01_compliance -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::*;

/// Test NIP-01 smoke tests against ngit-grasp relay
///
/// This test runs all NIP-01 smoke tests from grasp-audit against
/// the ngit-grasp relay implementation.
///
/// Tests cover:
/// - WebSocket connection
/// - Event send/receive
/// - Subscriptions (REQ/CLOSE)
/// - Event validation (signature, ID)
#[tokio::test]
async fn test_nip01_smoke() {
    // Start test relay
    let relay = TestRelay::start().await;

    // Create audit client in CI mode (isolated testing)
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config)
        .await
        .expect("Failed to create audit client");

    // Run all NIP-01 smoke tests
    let results = specs::Nip01SmokeTests::run_all(&client).await;

    // Print detailed report
    results.print_report();

    // Stop relay
    relay.stop().await;

    // Assert all tests passed
    assert!(
        results.all_passed(),
        "NIP-01 smoke tests failed: {}/{} passed",
        results.passed_count(),
        results.total_count()
    );
}

/// Test that relay properly validates events
///
/// Critical security test - ensures relay validates:
/// - Event signatures
/// - Event IDs
/// - Other NIP-01 requirements
#[tokio::test]
async fn test_relay_validates_events() {
    let relay = TestRelay::start().await;
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config)
        .await
        .expect("Failed to create audit client");

    // Run smoke tests which include validation tests
    let results = specs::Nip01SmokeTests::run_all(&client).await;

    relay.stop().await;

    // Filter to validation tests
    let validation_tests: Vec<_> = results
        .results
        .iter()
        .filter(|t| t.name.contains("reject") || t.name.contains("invalid"))
        .collect();

    // Should have validation tests
    assert!(
        !validation_tests.is_empty(),
        "No validation tests found (these are critical for security)"
    );

    // All validation tests should pass
    for test in validation_tests {
        assert!(
            test.passed,
            "Validation test failed: {} - {}\nThis is a security issue!",
            test.name,
            test.error.as_deref().unwrap_or("unknown error")
        );
    }
}

/// Test relay lifecycle management
///
/// Verifies TestRelay fixture properly manages relay lifecycle
#[tokio::test]
async fn test_relay_lifecycle() {
    // Start relay
    let relay = TestRelay::start().await;
    let url = relay.url().to_string();

    // Verify we can connect
    let config = AuditConfig::ci();
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
    let config = AuditConfig::ci();

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
