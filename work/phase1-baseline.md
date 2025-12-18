# Phase 1: Sync Test Baseline

**Timestamp:** 2025-12-18T16:50:07Z (UTC)
**Git Commit:** (pre-refactoring baseline)

## Test Execution Command
```bash
cargo test --test sync
```

## Summary Statistics

- **Total Tests:** 40
- **Passed:** 38 (95%)
- **Failed:** 2 (5%)
- **Ignored:** 0
- **Filtered Out:** 0
- **Execution Time:** 8.05s

## Passing Tests (38)

### Common Module Tests (7)
- `common::relay::tests::test_find_free_port`
- `common::sync_helpers::tests::test_parse_empty_metrics`
- `common::sync_helpers::tests::test_parse_counter_with_labels`
- `common::sync_helpers::tests::test_parse_gauge_without_labels`
- `common::sync_helpers::tests::test_parse_metric_with_relay_url_label`
- `common::sync_helpers::tests::test_repo_coord_format`
- `common::sync_helpers::tests::test_build_layer3_comment_with_uppercase_e`

### Sync Helper Builder Tests (6)
- `common::sync_helpers::tests::test_build_layer3_comment_kind_1`
- `common::sync_helpers::tests::test_build_layer3_quote_with_q`
- `common::sync_helpers::tests::test_build_layer3_comment_kind_1111`
- `common::sync_helpers::tests::test_build_layer2_issue_event`
- `common::sync_helpers::tests::test_build_layer3_reply_with_e_tag`
- `common::sync_helpers::tests::test_build_layer2_issue_with_uppercase_a`
- `common::sync_helpers::tests::test_build_layer2_issue_with_q_tag`

### Metrics Tests (6 passing)
- `sync::metrics::test_metric_values_are_numeric`
- `sync::metrics::test_concurrent_metrics_requests`
- `sync::metrics::test_metrics_availability_during_sync`
- `sync::metrics::test_connection_failure_increments_counter`
- `sync::metrics::test_prometheus_format_valid`
- `sync::metrics::test_relay_connected_status`
- `sync::metrics::test_health_state_degrades_on_failure`
- `sync::metrics::test_startup_sync_event_count`

### Live Sync Tests (3)
- `sync::live_sync::test_live_sync_layer2_events`
- `sync::live_sync::test_live_sync_layer3_events`
- `sync::live_sync::test_live_sync_event_ordering`

### Bootstrap Tests (3)
- `sync::bootstrap::test_announcement_not_listing_relay_is_not_synced`
- `sync::bootstrap::test_history_sync_without_negentropy`
- `sync::bootstrap::test_bootstrap_syncs_existing_layer2_events`
- `sync::bootstrap::test_relay_replays_events_after_restart`

### Discovery Tests (3)
- `sync::discovery::test_layer2_discovery_with_chain`
- `sync::discovery::test_discovers_layer3_via_layer2`
- `sync::discovery::test_recursive_relay_discovery_syncs_announcement`

### Tag Variations Tests (6)
- `sync::tag_variations::test_layer2_sync_with_lowercase_a_tag`
- `sync::tag_variations::test_layer2_sync_with_q_tag`
- `sync::tag_variations::test_layer2_sync_with_uppercase_a_tag`
- `sync::tag_variations::test_layer3_sync_with_lowercase_e_tag`
- `sync::tag_variations::test_layer3_sync_with_q_tag`
- `sync::tag_variations::test_layer3_sync_with_uppercase_e_tag`

## Failing Tests (2)

### 1. sync::metrics::test_live_sync_event_count

**Location:** `tests/sync/metrics.rs:444`

**Error Type:** Assertion failure

**Details:**
```
assertion `left == right` failed: Should have 2 live events
  left: None
 right: Some(2)
```

**Root Cause:** Live event counting metric is not being populated. The metric parser is returning `None` when it should find a count of 2 live synced events.

**Output Sample:**
```
Live events synced: None
```

**Impact:** This suggests that the `sync_events_total{sync_type="live"}` metric either:
- Is not being incremented correctly during live sync
- Is using a different metric name/label than expected
- Is not being exposed in the metrics endpoint

---

### 2. sync::metrics::test_multi_source_aggregate_counts

**Location:** `tests/sync/metrics.rs:603`

**Error Type:** Assertion failure

**Details:**
```
assertion `left == right` failed: Should have 0 connected
  left: Some(1)
 right: Some(0)
```

**Root Cause:** After stopping a relay connection, the `sync_relays_connected_total` metric is not being decremented. The test expects 0 connected relays after calling stop, but the metric still shows 1.

**Output Sample:**
```
Tracked total: Some(1)
Connected total: Some(1)
After stop - Tracked total: Some(1)
After stop - Connected total: Some(1)
```

**Impact:** This indicates that relay disconnection is not properly updating the connection count metric. This could be:
- A lifecycle issue where the metric update happens asynchronously after the test assertion
- A bug where the disconnect handler doesn't decrement the counter
- A race condition in the test timing

---

## Analysis

### Test Health
The sync test suite is in relatively good shape with a 95% pass rate. The failures are both isolated to the metrics module and appear to be either timing/synchronization issues or metric collection bugs rather than fundamental sync logic problems.

### Pre-existing Issues
Both failing tests appear to be pre-existing issues unrelated to the planned refactoring work. They should be tracked separately and not conflated with any issues introduced during the refactor.

### Refactoring Risk Assessment
- **Low Risk Areas:** Bootstrap, discovery, live_sync, tag_variations modules are all passing
- **Medium Risk Area:** Metrics tests have 2 failures, but they're specific to metric collection, not sync functionality
- **Safe to Refactor:** The core sync logic tests are passing, so structural refactoring of test helpers and organization should not affect test outcomes

## Next Steps

This baseline will be used to:
1. Verify that refactoring doesn't introduce new failures
2. Distinguish pre-existing failures from regressions
3. Track if the refactoring inadvertently fixes the existing failures
4. Ensure that after refactoring, we still have 38 passing tests (or more if we fix the failing ones)

## Notes

- Both failures are in `tests/sync/metrics.rs`
- The failures appear to be metric collection/timing issues rather than sync logic bugs
- All functional sync tests (bootstrap, discovery, live_sync, tag_variations) are passing
- The refactoring should not affect these test results unless we accidentally change metric collection timing