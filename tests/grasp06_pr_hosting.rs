//! GRASP-06 contributor PR hosting integration tests
//!
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//!
//! Each test is driven by one of two macros depending on whether the relay
//! under test has GRASP-06 enabled:
//!
//! - [`isolated_test_no_grasp_06!`] — relay started with the default config
//!   (GRASP-06 disabled). Tests assert the "off" contract: NIP-11 must not
//!   advertise GRASP-06, and `/prs/*` must 404.
//! - [`isolated_test_with_grasp_06!`] — relay started with
//!   `NGIT_GRASP06_ENABLE=true`. Tests assert the "on" contract.
//!
//! ## TDD
//!
//! These tests are written before the implementation. They are expected to
//! pass against the current (no-GRASP-06) relay because the discovery-gate
//! contract is satisfied trivially when neither NIP-11 advertisement nor
//! `/prs/` routing exists. They become regression guards once the feature
//! lands — if a future change enables `/prs/` without updating NIP-11
//! (or vice versa), these tests will catch it.
//!
//! ## Running
//!
//! ```bash
//! cargo test --test grasp06_pr_hosting
//! cargo test --test grasp06_pr_hosting -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::*;

/// Generate an integration test that runs against a fresh relay started with
/// the default config (GRASP-06 disabled).
macro_rules! isolated_test_no_grasp_06 {
    ($test_name:ident, $spec_suite:ident :: $test_fn:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start().await;
            let config = AuditConfig::isolated();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = specs::$spec_suite::$test_fn(&client).await;

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

/// Generate an integration test that runs against a fresh relay started with
/// `NGIT_GRASP06_ENABLE=true`.
macro_rules! isolated_test_with_grasp_06 {
    ($test_name:ident, $spec_suite:ident :: $test_fn:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start_with_grasp_06_enabled().await;
            let config = AuditConfig::isolated();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = specs::$spec_suite::$test_fn(&client).await;

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

// =============================================================================
// Test 1: Discovery gate — NIP-11 advertisement controls /prs/ availability
// =============================================================================
//
// Spec: GRASP-06 06.md (audit-derived from NIP-11 capability semantics)
//
// Contract: if NIP-11 does NOT list "GRASP-06" in supported_grasps, the
// /prs/<npub>/<id>.git namespace MUST return 404. This is what allows clients
// to discover GRASP-06 support via NIP-11 alone.
//
// We exercise both relay configurations:
//
// - `no_grasp_06`: NIP-11 won't advertise GRASP-06 (feature off), so the test
//   asserts the 404 contract. This is the "negative" branch of the gate.
// - `with_grasp_06`: NIP-11 should advertise GRASP-06 (feature on), so the
//   test branches into trivial-pass. Once the implementation lands and NIP-11
//   correctly advertises, this test continues to pass. If a future bug ships
//   `/prs/` routing without NIP-11 advertisement (or vice versa), the
//   `no_grasp_06` variant fails.

isolated_test_no_grasp_06!(
    test_prs_namespace_404_when_grasp06_not_advertised_no_grasp_06,
    PrsEndpointTests::test_prs_namespace_404_when_grasp06_not_advertised
);

isolated_test_with_grasp_06!(
    test_prs_namespace_404_when_grasp06_not_advertised_with_grasp_06,
    PrsEndpointTests::test_prs_namespace_404_when_grasp06_not_advertised
);
