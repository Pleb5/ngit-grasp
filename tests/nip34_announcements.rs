//! GRASP-01 Repository Event Acceptance Integration Tests
//!
//! Tests ngit-grasp relay's implementation of GRASP-01 repository event acceptance policy.
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
//! # Run all GRASP-01 tests
//! cargo test --test nip34_announcements
//!
//! # Run specific test
//! cargo test --test nip34_announcements test_reject_orphan_kind1
//!
//! # With output
//! cargo test --test nip34_announcements -- --nocapture
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
            let config = AuditConfig::isolated();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = specs::EventAcceptancePolicyTests::$test_name(&client).await;

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

// Generate isolated tests for all GRASP-01 event acceptance policy tests
isolated_test!(test_accept_valid_repo_announcement);
isolated_test!(test_reject_repo_announcement_missing_clone_tag);
isolated_test!(test_reject_repo_announcement_missing_relays_tag);
isolated_test!(test_accept_maintainer_announcement_without_service_listed);
isolated_test!(test_accept_issue_via_a_tag);
isolated_test!(test_accept_comment_via_capital_a_tag);
isolated_test!(test_accept_kind1_via_q_tag);
isolated_test!(test_accept_issue_quoting_issue_via_q);
isolated_test!(test_accept_comment_via_capital_e_tag);
isolated_test!(test_accept_kind1_via_e_tag);
isolated_test!(test_accept_kind1_referenced_in_issue);
isolated_test!(test_accept_comment_referenced_in_comment);
isolated_test!(test_accept_kind1_referenced_in_kind1);
isolated_test!(test_reject_orphan_issue);
isolated_test!(test_reject_orphan_kind1);
isolated_test!(test_reject_comment_quoting_other_repo);
