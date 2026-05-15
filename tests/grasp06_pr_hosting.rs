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
// Test 1: Discovery gate — /prs/ MUST 404 when GRASP-06 is not advertised
// =============================================================================
//
// Spec: GRASP-06 06.md (audit-derived from NIP-11 capability semantics)
//
// Contract: if NIP-11 does NOT list "GRASP-06" in supported_grasps, the
// /prs/<npub>/<id>.git namespace MUST return 404. This lets clients rely on
// NIP-11 alone for capability discovery.
//
// Wired only as `no_grasp_06`: the test asserts the "off" contract. The
// matching "on" contract (NIP-11 advertises when feature is enabled) is a
// distinct invariant and lives in its own test below.

isolated_test_no_grasp_06!(
    test_prs_namespace_404_when_grasp06_not_advertised_no_grasp_06,
    PrsEndpointTests::test_prs_namespace_404_when_grasp06_not_advertised
);

// =============================================================================
// Test 2: NIP-11 MUST advertise GRASP-06 when the feature is enabled
// =============================================================================
//
// Spec: implementation plan Phase 9 (positive companion to test 1)
//
// Contract: a relay started with NGIT_GRASP06_ENABLE=true MUST include
// "GRASP-06" in its NIP-11 supported_grasps array. Without this, the
// capability is invisible to clients and /prs/ is effectively unreachable.
//
// Wired only as `with_grasp_06`. Pre-implementation this test FAILS (TDD
// red) — the relay binary ignores the env var and NIP-11 doesn't advertise.
// Once Phase 9 lands the test turns green and becomes the regression guard.

isolated_test_with_grasp_06!(
    test_nip11_advertises_grasp_06_when_enabled,
    Nip11Tests::test_nip11_advertises_grasp_06_when_enabled
);

// =============================================================================
// Test 3: Empty-bare-repo fetch for any well-formed /prs/<npub>/<id>.git
// =============================================================================
//
// Spec: GRASP-06 06.md line 13
//
// Contract: when GRASP-06 is advertised, a fetch against any well-formed
// /prs/<npub>/<id>.git path that has no accepted refs/nostr/<event-id> MUST
// respond as if serving an empty bare repository. In practice: `git clone`
// must succeed and the resulting working copy must have zero refs.
//
// Wired only as `with_grasp_06`. Pre-implementation NIP-11 doesn't advertise
// GRASP-06 even with the env var set, so the test takes the trivial-pass
// branch (precondition not met). Once the feature lands and NIP-11
// advertises, this exercises the real clone-and-verify-empty assertion.

isolated_test_with_grasp_06!(
    test_prs_fetch_unknown_path_serves_empty_repo_with_grasp_06,
    PrsEndpointTests::test_prs_fetch_unknown_path_serves_empty_repo
);
