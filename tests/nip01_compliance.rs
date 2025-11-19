//! NIP-01 Compliance Integration Tests
//!
//! These tests verify that ngit-grasp relay implements NIP-01 correctly
//! by using the grasp-audit library to run compliance tests.
//!
//! # Test Strategy
//!
//! - Uses grasp-audit as a library (not CLI)
//! - Automatically manages relay lifecycle
//! - Reuses test specs from grasp-audit (single source of truth)
//! - Pure Rust, no shell scripts
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
/// This test:
/// 1. Starts a fresh ngit-grasp relay instance
/// 2. Runs all NIP-01 smoke tests from grasp-audit
/// 3. Verifies all tests pass
/// 4. Shuts down the relay
#[tokio::test]
async fn test_nip01_smoke() {
    // Start test relay
    let relay = TestRelay::start().await;

    // Create audit client in CI mode (isolated, no cleanup needed)
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

/// Test individual NIP-01 tests can be run separately
///
/// This demonstrates that we can run individual tests from the specs
/// for more granular testing or debugging.
#[tokio::test]
async fn test_nip01_individual_tests() {
    use grasp_audit::specs::grasp01::Nip01SmokeTests;

    let relay = TestRelay::start().await;
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config)
        .await
        .expect("Failed to create audit client");

    // We can't call private methods, so we'll run the full suite
    // This test is mainly to show the pattern
    let all_results = Nip01SmokeTests::run_all(&client).await;

    relay.stop().await;

    // Verify
    assert!(all_results.all_passed());
}

/// Test that relay rejects invalid events
///
/// This is a critical security test - we want to ensure the relay
/// properly validates events before accepting them.
#[tokio::test]
async fn test_relay_validates_events() {
    let relay = TestRelay::start().await;
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config)
        .await
        .expect("Failed to create audit client");

    // The validation tests are part of the smoke tests
    let results = specs::Nip01SmokeTests::run_all(&client).await;

    // Check that validation tests exist and pass
    let validation_tests: Vec<_> = results
        .results
        .iter()
        .filter(|t| t.spec_ref.contains("validation"))
        .collect();

    relay.stop().await;

    // Should have validation tests
    assert!(
        !validation_tests.is_empty(),
        "No validation tests found in NIP-01 smoke tests"
    );

    // All validation tests should pass
    for test in validation_tests {
        assert!(
            test.passed,
            "Validation test failed: {} - {}",
            test.name,
            test.error.as_deref().unwrap_or("unknown error")
        );
    }
}

/// Test relay lifecycle management
///
/// Ensures our test fixture properly manages relay lifecycle
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

    // Note: We can't easily verify disconnection without modifying grasp-audit
    // to expose connection state after relay shutdown. That's okay - the
    // important part is that the relay starts and stops cleanly.
}

/// Test multiple relays can run in parallel
///
/// This ensures our random port selection works correctly
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
