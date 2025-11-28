//! Git Clone Integration Tests
//!
//! Tests that verify Git clone operations work correctly through the HTTP backend.
//!
//! # Test Strategy
//!
//! - Each test runs in complete isolation with its own fresh relay instance
//! - Uses macro to eliminate boilerplate while maintaining test isolation
//! - Calls individual test methods from grasp-audit for minimal duplication
//! - Automatic cleanup via TestRelay fixture (removes container and temp dirs)
//!
//! # Running Tests
//!
//! ```bash
//! # Run all git clone tests
//! cargo test --test git_clone
//!
//! # Run specific test
//! cargo test --test git_clone test_basic_git_clone
//!
//! # With output
//! cargo test --test git_clone -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::specs::grasp01::GitCloneTests;
use grasp_audit::*;

/// Macro to generate isolated integration tests with relay domain
///
/// Each test runs with its own fresh relay instance to ensure complete isolation.
/// This eliminates issues with leftover repositories and ensures clean state.
macro_rules! isolated_test {
    ($test_name:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start().await;
            let config = AuditConfig::ci();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = GitCloneTests::$test_name(
                &client,
                &relay.domain(),
            )
            .await;

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

// Generate isolated tests for all git clone tests
isolated_test!(test_basic_git_clone);
isolated_test!(test_clone_url_format);
isolated_test!(test_sha1_capabilities_advertised);