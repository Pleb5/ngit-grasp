//! GRASP-01 Repository Event Acceptance Integration Tests
//!
//! Tests ngit-grasp relay's implementation of GRASP-01 repository event acceptance policy.
//! Uses grasp-audit library to avoid code duplication.
//!
//! # Test Strategy
//!
//! - Uses TestRelay fixture for ngit-grasp relay lifecycle management
//! - Uses grasp-audit's EventAcceptancePolicyTests for actual test logic
//! - Minimal duplication - single source of truth in grasp-audit
//!
//! # Running Tests
//!
//! ```bash
//! # Run all GRASP-01 tests
//! cargo test --test nip34_announcements
//!
//! # Run specific test
//! cargo test --test nip34_announcements test_grasp01_event_acceptance
//!
//! # With output
//! cargo test --test nip34_announcements -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::*;

/// Test GRASP-01 event acceptance policy against ngit-grasp relay
///
/// This test runs all GRASP-01 event acceptance policy tests from grasp-audit
/// against the ngit-grasp relay implementation.
///
/// Tests cover:
/// - Repository announcement acceptance/rejection
/// - Repository state announcement acceptance
/// - Events tagging accepted repositories
/// - Transitive event acceptance (events tagging accepted events)
/// - Forward reference acceptance (events tagged by accepted events)
/// - Rejection of unrelated events
#[tokio::test]
async fn test_grasp01_event_acceptance() {
    // Start test relay
    let relay = TestRelay::start().await;

    // Create audit client in CI mode (isolated testing)
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config)
        .await
        .expect("Failed to create audit client");

    // Run all GRASP-01 event acceptance policy tests
    let results = specs::EventAcceptancePolicyTests::run_all(&client).await;

    // Print detailed report
    results.print_report();

    // Stop relay
    relay.stop().await;

    // Assert all tests passed
    assert!(
        results.all_passed(),
        "GRASP-01 event acceptance tests failed: {}/{} passed",
        results.passed_count(),
        results.total_count()
    );
}

/// Test that relay accepts valid repository announcements
///
/// Demonstrates running individual test categories from the suite
#[tokio::test]
async fn test_accepts_repository_announcements() {
    let relay = TestRelay::start().await;
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config)
        .await
        .expect("Failed to create audit client");

    // Run all tests
    let results = specs::EventAcceptancePolicyTests::run_all(&client).await;

    relay.stop().await;

    // Filter to only repository announcement tests
    let announcement_tests: Vec<_> = results
        .results
        .iter()
        .filter(|t| {
            t.spec_ref.contains("repo") || t.name.contains("announcement") || t.name.contains("state")
        })
        .collect();

    // Verify we have announcement tests
    assert!(
        !announcement_tests.is_empty(),
        "No repository announcement tests found"
    );

    // All should pass
    for test in announcement_tests {
        assert!(
            test.passed,
            "Repository test failed: {} - {}",
            test.name,
            test.error.as_deref().unwrap_or("unknown error")
        );
    }
}

/// Test that relay properly validates clone and relays tags
///
/// This is a critical security requirement for GRASP-01
#[tokio::test]
async fn test_validates_service_tags() {
    let relay = TestRelay::start().await;
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config)
        .await
        .expect("Failed to create audit client");

    let results = specs::EventAcceptancePolicyTests::run_all(&client).await;

    relay.stop().await;

    // Filter to rejection tests (these verify tag validation)
    let rejection_tests: Vec<_> = results
        .results
        .iter()
        .filter(|t| t.name.contains("reject"))
        .collect();

    // Should have rejection tests
    assert!(
        !rejection_tests.is_empty(),
        "No rejection tests found (these are critical for security)"
    );

    // All rejection tests should pass
    for test in rejection_tests {
        assert!(
            test.passed,
            "Rejection test failed: {} - {}\nThis is a security issue!",
            test.name,
            test.error.as_deref().unwrap_or("unknown error")
        );
    }
}
