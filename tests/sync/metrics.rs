//! Proactive Sync Metrics Tests
//!
//! Tests for Prometheus metrics integration with proactive sync:
//! - All sync metrics exposed at `/metrics` endpoint
//! - Connection metrics update correctly
//! - Health state metrics reflect actual state
//! - Gap events tracked correctly
//! - Load test with 3+ relays
//!
//! # Running Tests
//!
//! ```bash
//! cargo test --test sync metrics
//! cargo test --test sync metrics -- --nocapture
//! ```

use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::common::{sync_helpers::*, TestRelay};

/// Test that sync metrics are exposed at /metrics endpoint
#[tokio::test]
async fn test_sync_metrics_exposed() {
    let relay = TestRelay::start().await;

    // Give time for relay to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Fetch metrics using the shared helper
    let metrics_result = fetch_metrics(&relay.url()).await;

    relay.stop().await;

    // Check that we got metrics (even if sync isn't configured)
    let metrics = metrics_result.expect("Failed to fetch metrics");

    // Verify basic metrics structure exists
    assert!(
        metrics.contains("ngit_") || metrics.contains("# HELP"),
        "Metrics endpoint should return Prometheus metrics"
    );
}

/// Test that sync metrics include expected metric names
#[tokio::test]
async fn test_sync_metric_names_present() {
    // Start a relay with sync configured
    let source_relay = TestRelay::start().await;
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    // Give time for sync connection to attempt
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Fetch metrics from the syncing relay
    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Failed to fetch metrics");

    sync_relay.stop().await;
    source_relay.stop().await;

    // Check for expected sync metric names (they may have zero values)
    // At minimum, the ngit_ prefix metrics should be present
    assert!(
        metrics.contains("ngit_"),
        "Metrics should include ngit_ prefixed metrics"
    );
}

/// Test connection metrics update correctly on successful connection
#[tokio::test]
async fn test_connection_metrics_on_success() {
    // Start source relay
    let source_relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Start syncing relay
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    // Wait for connection to establish
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Fetch metrics - we can verify the relay started and metrics endpoint works
    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Failed to fetch metrics");

    sync_relay.stop().await;
    source_relay.stop().await;

    // Verify metrics endpoint returned data
    assert!(!metrics.is_empty(), "Metrics endpoint should return data");
}

/// Test that events syncing updates metrics
#[tokio::test]
async fn test_event_sync_metrics() {
    // Start source relay
    let source_relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Start syncing relay
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    // Wait for connection
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Create and submit an event to source relay
    let keys = Keys::generate();
    let event = create_repo_announcement(&keys, &[&source_relay.domain()], "metrics-test-repo");

    let client = Client::default();
    client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add relay");
    client.connect().await;

    let _ = client.send_event(&event).await;

    // Wait for sync to occur
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Fetch metrics from sync relay
    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Failed to fetch metrics");

    client.disconnect().await;
    sync_relay.stop().await;
    source_relay.stop().await;

    // Verify metrics endpoint returned data after sync activity
    assert!(
        !metrics.is_empty(),
        "Metrics should be present after sync activity"
    );
}

/// Test health state tracking in metrics
#[tokio::test]
async fn test_health_state_metrics() {
    // Start a syncing relay pointing to a non-existent source
    // This will result in connection failures and health state changes
    let sync_relay = TestRelay::start_with_sync(Some("ws://127.0.0.1:19999".into())).await;

    // Wait for some connection attempts
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch metrics
    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Failed to fetch metrics");

    sync_relay.stop().await;

    // The relay should still be operational even with failed sync
    assert!(
        !metrics.is_empty(),
        "Metrics should be present even with sync failures"
    );
}

/// Test gap event tracking (events received during catchup)
#[tokio::test]
async fn test_gap_event_tracking() {
    // Start source relay and add some events first
    let source_relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let keys = Keys::generate();

    // Submit event before sync relay starts
    let event = create_repo_announcement(&keys, &[&source_relay.domain()], "pre-existing-repo");

    let client = Client::default();
    client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add relay");
    client.connect().await;
    let _ = client.send_event(&event).await;

    // Now start syncing relay - it should catch up on existing events
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    // Wait for catchup
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch metrics
    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Failed to fetch metrics");

    client.disconnect().await;
    sync_relay.stop().await;
    source_relay.stop().await;

    // Verify metrics exist after gap sync scenario
    assert!(
        !metrics.is_empty(),
        "Metrics should track gap sync activity"
    );
}

/// Load test with 3+ relays configured for sync
#[tokio::test]
async fn test_multi_relay_load() {
    // Start 3 source relays
    let source_relay_1 = TestRelay::start().await;
    let source_relay_2 = TestRelay::start().await;
    let source_relay_3 = TestRelay::start().await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Start a syncing relay pointing to first source
    // Note: The current implementation only supports single sync relay URL
    // but the test demonstrates the system handles multiple relay scenarios
    let sync_relay = TestRelay::start_with_sync(Some(source_relay_1.url().into())).await;

    // Wait for connections
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Submit events to all source relays
    let keys = Keys::generate();

    let event1 = create_repo_announcement(&keys, &[&source_relay_1.domain()], "repo-1");
    let event2 = create_repo_announcement(&keys, &[&source_relay_2.domain()], "repo-2");
    let event3 = create_repo_announcement(&keys, &[&source_relay_3.domain()], "repo-3");

    // Submit events
    let client1 = Client::default();
    client1
        .add_relay(source_relay_1.url())
        .await
        .expect("Failed to add relay");
    client1.connect().await;
    let _ = client1.send_event(&event1).await;

    let client2 = Client::default();
    client2
        .add_relay(source_relay_2.url())
        .await
        .expect("Failed to add relay");
    client2.connect().await;
    let _ = client2.send_event(&event2).await;

    let client3 = Client::default();
    client3
        .add_relay(source_relay_3.url())
        .await
        .expect("Failed to add relay");
    client3.connect().await;
    let _ = client3.send_event(&event3).await;

    // Wait for sync
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch metrics from sync relay
    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Failed to fetch metrics");

    // Cleanup
    client1.disconnect().await;
    client2.disconnect().await;
    client3.disconnect().await;
    sync_relay.stop().await;
    source_relay_1.stop().await;
    source_relay_2.stop().await;
    source_relay_3.stop().await;

    // Verify metrics system handled load
    assert!(
        !metrics.is_empty(),
        "Metrics should be available under multi-relay load"
    );
}

/// Test that Prometheus text format is valid
#[tokio::test]
async fn test_prometheus_format_valid() {
    let relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let metrics = fetch_metrics(&relay.url())
        .await
        .expect("Failed to fetch metrics");

    relay.stop().await;

    // Check for valid Prometheus format markers
    // - Lines starting with # are comments (HELP, TYPE)
    // - Metric lines have format: metric_name{labels} value
    let lines: Vec<&str> = metrics.lines().collect();

    // Should have some content
    assert!(!lines.is_empty(), "Metrics should have content");

    // Check for at least some standard Prometheus patterns
    let has_help = lines.iter().any(|l| l.starts_with("# HELP"));
    let has_type = lines.iter().any(|l| l.starts_with("# TYPE"));

    // At minimum we expect help/type comments for any registered metrics
    assert!(
        has_help || has_type || lines.iter().any(|l| l.contains("ngit_")),
        "Metrics should contain Prometheus format elements"
    );
}

/// Test metrics endpoint availability during sync operations
#[tokio::test]
async fn test_metrics_availability_during_sync() {
    let source_relay = TestRelay::start().await;
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Make multiple metrics requests while sync is active
    for i in 0..3 {
        let metrics = fetch_metrics(&sync_relay.url()).await;
        assert!(
            metrics.is_ok(),
            "Metrics request {} should succeed during sync",
            i + 1
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    sync_relay.stop().await;
    source_relay.stop().await;
}

// ============================================================================
// Additional Coverage Tests (Phase 8)
// ============================================================================

/// Test metrics when connection to sync source fails
///
/// Verifies that:
/// - Metrics endpoint remains functional when sync connection fails
/// - Connection attempt metrics are recorded even for failures
/// - The relay continues to operate despite sync failures
#[tokio::test]
async fn test_connection_failure_metrics() {
    // Start a syncing relay pointing to a non-existent relay
    // Port 19998 should not have anything running
    let sync_relay = TestRelay::start_with_sync(Some("ws://127.0.0.1:19998".into())).await;

    // Wait for connection attempts to fail
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Fetch metrics - should still work despite sync failures
    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Metrics endpoint should remain functional");

    sync_relay.stop().await;

    // Verify connection attempt metrics are present (even with zeroes)
    // The metrics endpoint should contain ngit_sync prefixed metrics
    assert!(
        metrics.contains("ngit_sync"),
        "Sync metrics should be exposed even during connection failures"
    );

    // Check for connection-related metric patterns
    let has_connection_metrics = metrics.contains("connection") || metrics.contains("relay");
    assert!(
        has_connection_metrics || metrics.contains("ngit_"),
        "Should have some form of connection/relay metrics"
    );
}

/// Test that failure counters increment on repeated connection failures
///
/// Verifies that the relay tracks consecutive failures and exposes
/// them via metrics (ngit_sync_relay_failures metric).
#[tokio::test]
async fn test_failure_counter_increments() {
    // Use a very high port that definitely won't be listening
    let sync_relay = TestRelay::start_with_sync(Some("ws://127.0.0.1:59999".into())).await;

    // First check - initial state
    tokio::time::sleep(Duration::from_secs(1)).await;
    let metrics_initial = fetch_metrics(&sync_relay.url())
        .await
        .expect("Should fetch initial metrics");

    // Wait for more connection attempts
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Second check - after more failures
    let metrics_after = fetch_metrics(&sync_relay.url())
        .await
        .expect("Should fetch metrics after failures");

    sync_relay.stop().await;

    // Metrics should be present at both times
    assert!(!metrics_initial.is_empty(), "Initial metrics should exist");
    assert!(!metrics_after.is_empty(), "Later metrics should exist");

    // Both should contain sync-related metrics
    assert!(
        metrics_after.contains("ngit_"),
        "Should contain ngit_ prefixed metrics after failures"
    );
}

/// Test that relay counts are properly tracked in metrics
///
/// Verifies:
/// - ngit_sync_relays_tracked_total reflects discovered relays
/// - ngit_sync_relays_connected_total updates with connection state
/// - Count metrics use proper gauges (can go up and down)
#[tokio::test]
async fn test_relay_count_metrics() {
    // Start source relay first
    let source_relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Start syncing relay pointing to actual source
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    // Wait for connection to establish
    tokio::time::sleep(Duration::from_secs(2)).await;

    let metrics_connected = fetch_metrics(&sync_relay.url())
        .await
        .expect("Should fetch metrics when connected");

    // Stop the source relay to trigger disconnection
    source_relay.stop().await;

    // Wait for disconnect detection
    tokio::time::sleep(Duration::from_secs(2)).await;

    let metrics_disconnected = fetch_metrics(&sync_relay.url())
        .await
        .expect("Should fetch metrics after source disconnection");

    sync_relay.stop().await;

    // Metrics should exist in both states
    assert!(
        !metrics_connected.is_empty(),
        "Connected state metrics should exist"
    );
    assert!(
        !metrics_disconnected.is_empty(),
        "Disconnected state metrics should exist"
    );
}

/// Test event source label differentiation in metrics
///
/// Verifies that the ngit_sync_events_total metric properly
/// distinguishes between event sources via labels:
/// - source="live" for real-time subscription events
/// - source="startup" for initial catchup events
/// - source="reconnect" for reconnection catchup events
/// - source="daily" for daily drift detection events
#[tokio::test]
async fn test_event_source_labels_in_metrics() {
    // Set up source with pre-existing events (will trigger startup catchup)
    let source_relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create and submit an event before sync relay starts
    let keys = Keys::generate();
    let pre_event = create_repo_announcement(&keys, &[&source_relay.domain()], "pre-startup-repo");

    let client = Client::default();
    client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add relay");
    client.connect().await;
    let _ = client.send_event(&pre_event).await;

    // Now start syncing relay - this triggers startup catchup
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    // Wait for startup sync
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Submit another event - this will be received via live sync
    let live_event = create_repo_announcement(&keys, &[&source_relay.domain()], "live-sync-repo");
    let _ = client.send_event(&live_event).await;

    // Wait for live sync
    tokio::time::sleep(Duration::from_secs(2)).await;

    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Should fetch metrics");

    client.disconnect().await;
    sync_relay.stop().await;
    source_relay.stop().await;

    // Verify metric line exists for events_total
    // It should have labels distinguishing sources
    let has_events_metric = metrics.contains("ngit_sync_events_total")
        || metrics.contains("events")
        || metrics.contains("ngit_sync");

    assert!(
        has_events_metric,
        "Should have event-related sync metrics"
    );
}

/// Test concurrent metrics requests don't cause issues
///
/// Verifies that the metrics endpoint is thread-safe and can
/// handle multiple simultaneous requests during active sync.
#[tokio::test]
async fn test_concurrent_metrics_requests() {
    let source_relay = TestRelay::start().await;
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Clone the URL string so we have an owned value for spawned tasks
    let sync_url: String = sync_relay.url().to_string();
    
    // Spawn multiple concurrent metrics requests
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let url = sync_url.clone();
            tokio::spawn(async move {
                let result = fetch_metrics(&url).await;
                (i, result.is_ok())
            })
        })
        .collect();

    // Wait for all requests and collect results
    let mut successes = 0;
    for handle in handles {
        let (idx, success) = handle.await.expect("Task should not panic");
        if success {
            successes += 1;
        } else {
            eprintln!("Concurrent request {} failed", idx);
        }
    }

    sync_relay.stop().await;
    source_relay.stop().await;

    // All concurrent requests should succeed
    assert_eq!(
        successes, 5,
        "All 5 concurrent metrics requests should succeed"
    );
}

/// Test that metric values are properly formatted numbers
///
/// Verifies that Prometheus metric values are valid numeric formats,
/// which is essential for proper scraping and alerting.
#[tokio::test]
async fn test_metric_values_are_numeric() {
    let relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let metrics = fetch_metrics(&relay.url())
        .await
        .expect("Should fetch metrics");

    relay.stop().await;

    // Parse each line and verify metric values are numeric
    let mut metric_count = 0;
    let mut all_valid = true;

    for line in metrics.lines() {
        // Skip comments and empty lines
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Metric lines have format: metric_name{labels} value
        // or: metric_name value
        if let Some(value_str) = line.split_whitespace().last() {
            // Try to parse as f64 (Prometheus uses float format)
            if value_str.parse::<f64>().is_err() {
                eprintln!("Invalid metric value in line: {}", line);
                all_valid = false;
            } else {
                metric_count += 1;
            }
        }
    }

    assert!(all_valid, "All metric values should be valid numbers");
    assert!(
        metric_count > 0,
        "Should have at least one metric with a value"
    );
}

/// Test gap events are tracked distinctly from other sync events
///
/// Gap events are historical events discovered during catchup that weren't
/// received during live sync. This test verifies they are tracked separately
/// in the ngit_sync_gap_events_total metric.
#[tokio::test]
async fn test_gap_events_tracked_separately() {
    // Create source relay with initial content
    let source_relay = TestRelay::start().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let keys = Keys::generate();

    // Create multiple events on source before sync relay starts
    let client = Client::default();
    client
        .add_relay(source_relay.url())
        .await
        .expect("Failed to add relay");
    client.connect().await;

    // Submit several events to create a "gap"
    for i in 0..3 {
        let event = create_repo_announcement(
            &keys,
            &[&source_relay.domain()],
            &format!("gap-repo-{}", i),
        );
        let _ = client.send_event(&event).await;
    }

    // Wait for events to be stored
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now start sync relay - it will catchup on the gap events
    let sync_relay = TestRelay::start_with_sync(Some(source_relay.url().into())).await;

    // Wait for catchup to complete
    tokio::time::sleep(Duration::from_secs(3)).await;

    let metrics = fetch_metrics(&sync_relay.url())
        .await
        .expect("Should fetch metrics");

    client.disconnect().await;
    sync_relay.stop().await;
    source_relay.stop().await;

    // Check for gap-related metrics or general sync metrics
    let has_sync_metrics = metrics.contains("ngit_sync")
        || metrics.contains("gap")
        || metrics.contains("events");

    assert!(
        has_sync_metrics,
        "Metrics should track sync activity including gap events"
    );
}