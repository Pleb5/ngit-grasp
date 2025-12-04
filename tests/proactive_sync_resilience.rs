//! Integration tests for GRASP-02 Phase 3: Resilience & Health Tracking
//!
//! Tests verify:
//! - Exponential backoff on connection failures (5s → 1h max)
//! - Dead relay detection after 24h of failures
//! - Successful connection resets to Healthy
//! - Dead relays retry minimally (once per day)
//! - Health state tracking is thread-safe

use std::time::{Duration, Instant};

use ngit_grasp::sync::health::{HealthState, RelayHealthTracker};

/// Test that a single failure transitions relay to Degraded state
#[test]
fn test_single_failure_causes_degraded_state() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Initial state should allow connection
    assert!(tracker.should_attempt_connection(url));

    // Record a failure
    tracker.record_failure(url);

    // Should be in degraded state
    assert_eq!(tracker.get_state(url), HealthState::Degraded);
    assert_eq!(tracker.get_failure_count(url), 1);
}

/// Test that successful connection resets to Healthy state
#[test]
fn test_success_resets_to_healthy() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Simulate multiple failures
    tracker.record_failure(url);
    tracker.record_failure(url);
    tracker.record_failure(url);

    assert_eq!(tracker.get_state(url), HealthState::Degraded);
    assert_eq!(tracker.get_failure_count(url), 3);

    // Success should reset everything
    tracker.record_success(url);

    assert_eq!(tracker.get_state(url), HealthState::Healthy);
    assert_eq!(tracker.get_failure_count(url), 0);
    assert!(tracker.should_attempt_connection(url));
}

/// Test that backoff increases exponentially
#[test]
fn test_exponential_backoff_calculation() {
    let max_backoff = 3600u64; // 1 hour

    // failure 1: 5s (5 * 2^0)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(1, max_backoff),
        Duration::from_secs(5)
    );

    // failure 2: 10s (5 * 2^1)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(2, max_backoff),
        Duration::from_secs(10)
    );

    // failure 3: 20s (5 * 2^2)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(3, max_backoff),
        Duration::from_secs(20)
    );

    // failure 4: 40s (5 * 2^3)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(4, max_backoff),
        Duration::from_secs(40)
    );

    // failure 5: 80s (5 * 2^4)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(5, max_backoff),
        Duration::from_secs(80)
    );

    // failure 6: 160s (5 * 2^5)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(6, max_backoff),
        Duration::from_secs(160)
    );

    // failure 7: 320s (5 * 2^6)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(7, max_backoff),
        Duration::from_secs(320)
    );

    // failure 8: 640s (5 * 2^7)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(8, max_backoff),
        Duration::from_secs(640)
    );

    // failure 9: 1280s (5 * 2^8)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(9, max_backoff),
        Duration::from_secs(1280)
    );

    // failure 10: 2560s (5 * 2^9)
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(10, max_backoff),
        Duration::from_secs(2560)
    );
}

/// Test that backoff is capped at max_backoff
#[test]
fn test_backoff_capped_at_maximum() {
    let max_backoff = 3600u64; // 1 hour

    // After many failures, should cap at max_backoff
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(15, max_backoff),
        Duration::from_secs(max_backoff)
    );

    assert_eq!(
        RelayHealthTracker::get_backoff_duration(20, max_backoff),
        Duration::from_secs(max_backoff)
    );

    assert_eq!(
        RelayHealthTracker::get_backoff_duration(100, max_backoff),
        Duration::from_secs(max_backoff)
    );
}

/// Test that custom max_backoff is respected
#[test]
fn test_custom_max_backoff() {
    let custom_max = 60u64; // 1 minute max

    // After several failures, should cap at custom max
    assert_eq!(
        RelayHealthTracker::get_backoff_duration(10, custom_max),
        Duration::from_secs(custom_max)
    );

    // Tracker with custom max should use it
    let tracker = RelayHealthTracker::with_max_backoff(custom_max);
    let url = "wss://test-relay.example.com";

    // Simulate many failures
    for _ in 0..20 {
        tracker.record_failure(url);
    }

    // Should still be degraded (not dead without 24h)
    assert_eq!(tracker.get_state(url), HealthState::Degraded);
}

/// Test that backoff blocks immediate reconnection
#[test]
fn test_backoff_blocks_immediate_reconnection() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // First connection attempt should be allowed
    assert!(tracker.should_attempt_connection(url));

    // Record a failure
    tracker.record_failure(url);

    // Immediately after failure, connection should be blocked (backoff active)
    assert!(!tracker.should_attempt_connection(url));

    // Should have remaining backoff
    let remaining = tracker.get_remaining_backoff(url);
    assert!(remaining.is_some());
    assert!(remaining.unwrap() > Duration::ZERO);
}

/// Test that multiple relays are tracked independently
#[test]
fn test_multiple_relays_independent() {
    let tracker = RelayHealthTracker::with_defaults();
    let url1 = "wss://relay1.example.com";
    let url2 = "wss://relay2.example.com";
    let url3 = "wss://relay3.example.com";

    // Fail relay1 multiple times
    tracker.record_failure(url1);
    tracker.record_failure(url1);
    tracker.record_failure(url1);

    // Succeed on relay2
    tracker.record_success(url2);

    // Fail relay3 once
    tracker.record_failure(url3);

    // Verify independent states
    assert_eq!(tracker.get_state(url1), HealthState::Degraded);
    assert_eq!(tracker.get_failure_count(url1), 3);

    assert_eq!(tracker.get_state(url2), HealthState::Healthy);
    assert_eq!(tracker.get_failure_count(url2), 0);

    assert_eq!(tracker.get_state(url3), HealthState::Degraded);
    assert_eq!(tracker.get_failure_count(url3), 1);
}

/// Test is_dead returns false for degraded relays
#[test]
fn test_is_dead_false_for_degraded() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Simulate failures
    for _ in 0..10 {
        tracker.record_failure(url);
    }

    // Should be degraded but not dead (24h hasn't passed)
    assert_eq!(tracker.get_state(url), HealthState::Degraded);
    assert!(!tracker.is_dead(url));
}

/// Test get_tracked_relays returns all tracked URLs
#[test]
fn test_get_tracked_relays() {
    let tracker = RelayHealthTracker::with_defaults();

    // Track multiple relays
    tracker.record_success("wss://relay1.example.com");
    tracker.record_failure("wss://relay2.example.com");
    tracker.record_success("wss://relay3.example.com");

    let tracked = tracker.get_tracked_relays();
    assert_eq!(tracked.len(), 3);
    assert!(tracked.contains(&"wss://relay1.example.com".to_string()));
    assert!(tracked.contains(&"wss://relay2.example.com".to_string()));
    assert!(tracked.contains(&"wss://relay3.example.com".to_string()));
}

/// Test get_health returns cloned health info
#[test]
fn test_get_health_returns_clone() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Record success
    tracker.record_success(url);

    // Get health info
    let health = tracker.get_health(url);
    assert!(health.is_some());

    let health = health.unwrap();
    assert_eq!(health.state, HealthState::Healthy);
    assert!(health.last_success_time.is_some());
    assert_eq!(health.consecutive_failures, 0);
}

/// Test get_health returns None for non-existent relay
#[test]
fn test_get_health_nonexistent() {
    let tracker = RelayHealthTracker::with_defaults();

    let health = tracker.get_health("wss://nonexistent.example.com");
    assert!(health.is_none());
}

/// Test that new relays default to allowing connection
#[test]
fn test_new_relay_allows_connection() {
    let tracker = RelayHealthTracker::with_defaults();

    // A never-seen relay should allow connection
    assert!(tracker.should_attempt_connection("wss://brand-new-relay.example.com"));
}

/// Test health state display
#[test]
fn test_health_state_display() {
    assert_eq!(HealthState::Healthy.to_string(), "healthy");
    assert_eq!(HealthState::Degraded.to_string(), "degraded");
    assert_eq!(HealthState::Dead.to_string(), "dead");
}

/// Test thread safety with concurrent access
#[tokio::test]
async fn test_concurrent_health_tracking() {
    use std::sync::Arc;

    let tracker = Arc::new(RelayHealthTracker::with_defaults());
    let url = "wss://concurrent-test-relay.example.com";

    // Spawn multiple tasks that access the tracker concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let tracker_clone = tracker.clone();
        let url_owned = url.to_string();
        let handle = tokio::spawn(async move {
            if i % 2 == 0 {
                tracker_clone.record_failure(&url_owned);
            } else {
                tracker_clone.record_success(&url_owned);
            }
            tracker_clone.get_state(&url_owned);
            tracker_clone.should_attempt_connection(&url_owned);
        });
        handles.push(handle);
    }

    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }

    // Tracker should still be usable
    let health = tracker.get_health(url);
    assert!(health.is_some());
}

/// Test that failure streak tracking works correctly
#[test]
fn test_failure_streak_tracking() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Build up a failure streak
    for i in 1..=5 {
        tracker.record_failure(url);
        assert_eq!(tracker.get_failure_count(url), i);
    }

    // Success should reset the streak
    tracker.record_success(url);
    assert_eq!(tracker.get_failure_count(url), 0);

    // Start a new streak
    tracker.record_failure(url);
    assert_eq!(tracker.get_failure_count(url), 1);
}

/// Test recovery from degraded state
#[test]
fn test_recovery_from_degraded() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Enter degraded state
    tracker.record_failure(url);
    assert_eq!(tracker.get_state(url), HealthState::Degraded);

    // Recover
    tracker.record_success(url);
    assert_eq!(tracker.get_state(url), HealthState::Healthy);
    assert!(tracker.should_attempt_connection(url));
    assert!(tracker.get_remaining_backoff(url).is_none());
}

/// Test that remaining backoff is None after success
#[test]
fn test_no_remaining_backoff_after_success() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Fail to set backoff
    tracker.record_failure(url);
    assert!(tracker.get_remaining_backoff(url).is_some());

    // Succeed to clear backoff
    tracker.record_success(url);
    assert!(tracker.get_remaining_backoff(url).is_none());
}

/// Integration test: simulate a realistic connection lifecycle
#[test]
fn test_realistic_connection_lifecycle() {
    let tracker = RelayHealthTracker::with_max_backoff(60); // 1 minute max for test
    let url = "wss://production-relay.example.com";

    // Initial connection succeeds
    tracker.record_success(url);
    assert_eq!(tracker.get_state(url), HealthState::Healthy);

    // Connection drops - first failure
    tracker.record_failure(url);
    assert_eq!(tracker.get_state(url), HealthState::Degraded);
    assert_eq!(tracker.get_failure_count(url), 1);

    // Second failure (retry failed)
    tracker.record_failure(url);
    assert_eq!(tracker.get_failure_count(url), 2);

    // Third failure
    tracker.record_failure(url);
    assert_eq!(tracker.get_failure_count(url), 3);

    // Connection finally succeeds
    tracker.record_success(url);
    assert_eq!(tracker.get_state(url), HealthState::Healthy);
    assert_eq!(tracker.get_failure_count(url), 0);
    assert!(tracker.should_attempt_connection(url));
}

/// Test backoff timing sequence
#[test]
fn test_backoff_timing_sequence() {
    // With default max of 3600s (1 hour), verify the progression
    let max = 3600u64;

    let expected = vec![
        (1, 5),     // 5s
        (2, 10),    // 10s
        (3, 20),    // 20s
        (4, 40),    // 40s
        (5, 80),    // 80s
        (6, 160),   // 160s (~2.7 min)
        (7, 320),   // 320s (~5.3 min)
        (8, 640),   // 640s (~10.7 min)
        (9, 1280),  // 1280s (~21.3 min)
        (10, 2560), // 2560s (~42.7 min)
        (11, 3600), // capped at 3600s (1 hour)
        (12, 3600), // still capped
    ];

    for (failures, expected_secs) in expected {
        assert_eq!(
            RelayHealthTracker::get_backoff_duration(failures, max),
            Duration::from_secs(expected_secs),
            "Failed for {} failures",
            failures
        );
    }
}

/// Test that health info timestamp tracking works
#[test]
fn test_timestamp_tracking() {
    let tracker = RelayHealthTracker::with_defaults();
    let url = "wss://test-relay.example.com";

    // Record initial success
    let before = Instant::now();
    tracker.record_success(url);
    let after = Instant::now();

    let health = tracker.get_health(url).unwrap();
    let success_time = health.last_success_time.unwrap();

    // Success time should be between before and after
    assert!(success_time >= before);
    assert!(success_time <= after);

    // Record failure
    let before_fail = Instant::now();
    tracker.record_failure(url);
    let after_fail = Instant::now();

    let health = tracker.get_health(url).unwrap();
    let failure_time = health.last_failure_time.unwrap();
    let first_failure = health.first_failure_time.unwrap();

    // Failure times should be between before and after
    assert!(failure_time >= before_fail);
    assert!(failure_time <= after_fail);
    assert!(first_failure >= before_fail);
    assert!(first_failure <= after_fail);
}