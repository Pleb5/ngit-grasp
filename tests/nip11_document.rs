//! GRASP-01 NIP-11 Document Integration Tests
//!
//! Tests ngit-grasp relay's implementation of GRASP-01 NIP-11 relay information requirements.
//! Uses grasp-audit library to avoid code duplication.
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
//! # Run all GRASP-01 NIP-11 tests
//! cargo test --test nip11_document
//!
//! # Run specific test
//! cargo test --test nip11_document test_nip11_document_exists
//!
//! # With output
//! cargo test --test nip11_document -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::*;

/// Macro to generate isolated integration tests
///
/// Each test runs with its own fresh relay instance to ensure complete isolation.
/// This eliminates rate-limiting issues and ensures tests don't interfere with each other.
macro_rules! isolated_test {
    ($test_name:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start().await;
            let config = AuditConfig::ci();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = specs::Nip11DocumentTests::$test_name(&client).await;

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

// Generate isolated tests for all GRASP-01 NIP-11 document tests
isolated_test!(test_nip11_document_exists);
isolated_test!(test_nip11_supported_grasps_field);
isolated_test!(test_nip11_repo_acceptance_criteria_field);
isolated_test!(test_nip11_curation_field);
