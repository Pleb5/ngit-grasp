//! CORS Integration Tests
//!
//! Tests that verify CORS headers are correctly set on Git HTTP backend responses.
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
//! # Run all CORS tests
//! cargo test --test cors
//!
//! # Run specific test
//! cargo test --test cors test_cors_allow_origin
//!
//! # With output
//! cargo test --test cors -- --nocapture
//! ```

mod common;

use common::TestRelay;
use grasp_audit::specs::grasp01::CorsTests;
use grasp_audit::*;

/// Macro to generate isolated CORS integration tests with relay domain
///
/// Each test runs with its own fresh relay instance to ensure complete isolation.
macro_rules! isolated_cors_test {
    ($test_name:ident) => {
        #[tokio::test]
        async fn $test_name() {
            let relay = TestRelay::start().await;
            let config = AuditConfig::isolated();
            let client = AuditClient::new(relay.url(), config)
                .await
                .expect("Failed to create audit client");

            let result = CorsTests::$test_name(&client, &relay.domain()).await;

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

// Generate isolated tests for all CORS tests
isolated_cors_test!(test_cors_allow_origin);
isolated_cors_test!(test_cors_allow_methods);
isolated_cors_test!(test_cors_allow_headers);
isolated_cors_test!(test_cors_options_preflight);

isolated_cors_test!(test_cors_on_real_repo);

#[tokio::test]
async fn test_cors_allows_git_smart_http_preflight_headers() {
    let relay = TestRelay::start().await;
    let url = format!(
        "http://{}/npub1test/test.git/info/refs?service=git-upload-pack",
        relay.domain()
    );

    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &url)
        .header("Origin", "https://budabit.club")
        .header("Access-Control-Request-Method", "GET")
        .header(
            "Access-Control-Request-Headers",
            "git-protocol, authorization, accept, content-encoding, x-requested-with",
        )
        .send()
        .await
        .expect("Failed to send OPTIONS preflight request");

    let status = response.status();
    let allow_headers = response
        .headers()
        .get("Access-Control-Allow-Headers")
        .expect("Missing Access-Control-Allow-Headers")
        .to_str()
        .expect("Invalid Access-Control-Allow-Headers")
        .to_string();

    relay.stop().await;

    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);

    for expected in [
        "git-protocol",
        "authorization",
        "accept",
        "content-encoding",
        "x-requested-with",
    ] {
        assert!(
            allow_headers
                .split(',')
                .any(|header| header.trim().eq_ignore_ascii_case(expected)),
            "Expected Access-Control-Allow-Headers to include {expected}, got: {allow_headers}"
        );
    }
}
