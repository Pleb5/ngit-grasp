//! Repository Creation Integration Tests
//!
//! Tests that verify bare Git repositories are created when repository announcements
//! are accepted by ngit-grasp relay.
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
//! # Run all repository creation tests
//! cargo test --test repository_creation
//!
//! # Run specific test
//! cargo test --test repository_creation test_bare_repo_created_on_announcement
//!
//! # With output
//! cargo test --test repository_creation -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::specs::grasp01::RepositoryCreationTests;
use grasp_audit::*;

/// Macro to generate isolated integration tests
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

            let result = RepositoryCreationTests::$test_name(&client, relay.git_data_dir()).await;

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

// Generate isolated tests for all repository creation tests
isolated_test!(test_bare_repo_created_on_announcement);
isolated_test!(test_repo_creation_idempotent);
isolated_test!(test_bare_repo_structure);
