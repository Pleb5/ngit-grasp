# Sync Test Refactor Options

## Summary of Requirements

From your feedback:

- **Tag variations**: Test with ONE sync mode (live OR historic), but make it clear which
- **Discovery**: Needs both live AND historic examples (mechanism differs)
- **Duplication**: Compare approaches before deciding

---

## Chosen Approach: Unified Helper with Event Slices

A single helper function that handles both sync modes based on which event slices have content:

````rust
/// Result from sync test scenario
pub struct SyncTestResult {
    pub source: TestRelay,
    pub syncing: TestRelay,
    pub keys: Keys,
    pub repo_coord: String,
}

/// Run a sync test scenario with historic and/or live events.
///
/// # Arguments
/// - `historic_events` - Events loaded on source BEFORE syncing relay connects
/// - `live_events` - Events fed to source AFTER syncing relay connects
///
/// # Mode Detection
/// - If only `historic_events` has content → Historic sync test
/// - If only `live_events` has content → Live sync test
/// - Both can have content for mixed scenarios
///
/// # Example - Historic Sync
/// ```rust
/// let (result, events) = run_sync_test(&[announcement, issue], &[]).await;
/// // Source had events before syncing relay started
/// // Verify events synced to result.syncing
/// ```
///
/// # Example - Live Sync
/// ```rust
/// let (result, events) = run_sync_test(&[], &[issue, patch]).await;
/// // Events were added after connection established
/// // Verify events synced to result.syncing
/// ```
pub async fn run_sync_test(
    historic_events: &[Event],
    live_events: &[Event],
) -> SyncTestResult {
    // 1. Pre-allocate syncing relay port for announcement tags
    let syncing_port = TestRelay::find_free_port();
    let syncing_domain = format!("127.0.0.1:{}", syncing_port);

    // 2. Start source relay
    let source = TestRelay::start().await;

    // 3. Create keys and announcement listing both relays
    let keys = Keys::generate();
    let announcement = create_repo_announcement(
        &keys,
        &[&source.domain(), &syncing_domain],
        "test-repo",
    );

    // 4. Send announcement + historic events to source BEFORE syncing relay starts
    send_to_relay(&source, &announcement).await;
    for event in historic_events {
        send_to_relay(&source, event).await;
    }

    // 5. Start syncing relay (connects to source)
    let syncing = TestRelay::start_on_port_with_options(
        syncing_port,
        Some(source.url().into()),
        false,
    ).await;

    // 6. Wait for sync connection to establish
    wait_for_sync_connection(syncing.url(), 1, Duration::from_secs(5)).await.ok();

    // 7. Send live events AFTER connection established
    for event in live_events {
        send_to_relay(&source, event).await;
    }

    // 8. Allow sync to complete
    tokio::time::sleep(Duration::from_secs(2)).await;

    SyncTestResult {
        source,
        syncing,
        keys,
        repo_coord: repo_coord(&keys, "test-repo"),
    }
}
````

### Test Usage Examples

```rust
// Historic sync - events existed before connection
#[tokio::test]
async fn test_historic_layer2_issue_syncs() {
    let keys = Keys::generate();
    let repo_coord = repo_coord(&keys, "test-repo");
    let issue = build_layer2_issue_event(&keys, &repo_coord, "Historic Issue")?;

    let result = run_sync_test(&[issue.clone()], &[]).await;

    assert!(
        wait_for_event_on_relay(result.syncing.url(), issue.id).await,
        "Historic issue should sync"
    );
}

// Live sync - events arrive after connection
#[tokio::test]
async fn test_live_layer2_issue_syncs() {
    let keys = Keys::generate();
    let repo_coord = repo_coord(&keys, "test-repo");
    let issue = build_layer2_issue_event(&keys, &repo_coord, "Live Issue")?;

    let result = run_sync_test(&[], &[issue.clone()]).await;

    assert!(
        wait_for_event_on_relay(result.syncing.url(), issue.id).await,
        "Live issue should sync"
    );
}

// Discovery test - historic
#[tokio::test]
async fn test_discovery_historic_syncs_layer2() {
    let keys = Keys::generate();
    let repo_coord = repo_coord(&keys, "test-repo");
    let issue = build_layer2_issue_event(&keys, &repo_coord, "Discovered Issue")?;

    // Source has the issue before discovery
    let result = run_sync_test(&[issue.clone()], &[]).await;

    assert!(wait_for_event_on_relay(result.syncing.url(), issue.id).await);
}

// Discovery test - live
#[tokio::test]
async fn test_discovery_live_syncs_layer2() {
    // ... setup similar, events in second slice
}
```

### Why This Approach

1. **Single function, both modes** - No duplication of setup logic
2. **Clear distinction** - Function signature makes it obvious which is historic vs live
3. **No new dependencies** - Plain Rust
4. **Readable tests** - Test body just creates events and calls helper
5. **Flexible** - Can test mixed scenarios if needed

---

## Proposed Test File Structure

```
tests/sync/
├── mod.rs                    # Module declarations + overview doc
├── historic_sync.rs          # NEW: Historic sync tests (events exist before connection)
├── live_sync.rs              # REFACTORED: Live sync tests (events arrive after connection)
├── discovery.rs              # REFACTORED: Relay discovery (both modes)
├── tag_variations.rs         # REFACTORED: Tag type coverage (live sync only)
├── metrics.rs                # UNCHANGED: Prometheus metrics tests
└── catchup.rs                # UNCHANGED: Documentation only
```

### `tests/sync/historic_sync.rs` (NEW - renamed from bootstrap.rs)

```rust
//! Historic Sync Tests
//!
//! Tests for syncing events that exist on source relay BEFORE the syncing relay connects.
//! This is the bootstrap/startup sync path.

/// Historic sync: Layer 1 announcements sync on startup
/// run_sync_test with historic_events: [announcement]
#[tokio::test]
async fn test_historic_layer1_announcement_syncs() { ... }

/// Historic sync: Layer 2 issues sync on startup
/// run_sync_test with historic_events: [issue]
#[tokio::test]
async fn test_historic_layer2_issue_syncs() { ... }

/// Historic sync: Layer 3 comments sync after Layer 2 syncs
/// run_sync_test with historic_events: [issue, comment]
#[tokio::test]
async fn test_historic_layer3_comment_syncs() { ... }

/// Historic sync: Events not listing relay domain are rejected
/// run_sync_test with historic_events: [announcement_missing_domain]
/// Verify NOT synced
#[tokio::test]
async fn test_historic_rejects_events_not_listing_relay() { ... }

/// Historic sync works without NIP-77 negentropy (REQ+EOSE fallback)
/// run_sync_test with historic_events, negentropy disabled
#[tokio::test]
async fn test_historic_sync_without_negentropy() { ... }
```

### `tests/sync/live_sync.rs` (REFACTORED)

```rust
//! Live Sync Tests
//!
//! Tests for syncing events that arrive on source relay AFTER the syncing relay connects.
//! This is the real-time/subscription-based sync path.

/// Live sync: Layer 2 issues sync in real-time
/// run_sync_test with live_events: [issue]
#[tokio::test]
async fn test_live_layer2_issue_syncs() { ... }

/// Live sync: Layer 3 comments sync after Layer 2 syncs
/// run_sync_test with live_events: [issue, comment]
#[tokio::test]
async fn test_live_layer3_comment_syncs() { ... }

/// Live sync: Events arrive in chronological order
/// run_sync_test with live_events: [issue1, issue2, issue3]
#[tokio::test]
async fn test_live_sync_event_ordering() { ... }
```

### `tests/sync/discovery.rs` (REFACTORED)

```rust
//! Relay Discovery Tests
//!
//! Tests for discovering other relays from announcement events.
//! Discovery can happen via historic sync or live sync paths.

// === HISTORIC DISCOVERY ===
// Relay discovers another relay from an announcement that existed before connection

/// Historic discovery: Discovers relay from announcement, syncs Layer 2
/// 1. relay_a has announcement + issue
/// 2. relay_b starts with sync from relay_a
/// 3. relay_b syncs announcement, discovers other relays listed, syncs issue
#[tokio::test]
async fn test_historic_discovery_syncs_layer2() { ... }

/// Historic discovery: 3-relay recursive discovery chain
/// 1. relay_b has announcement listing relay_c
/// 2. relay_c has separate announcement
/// 3. relay_a starts syncing from relay_b
/// 4. relay_a gets announcement, discovers relay_c, syncs from relay_c
#[tokio::test]
async fn test_historic_recursive_discovery() { ... }

// === LIVE DISCOVERY ===
// Relay discovers another relay from an announcement that arrives after connection

/// Live discovery: Discovers relay from new announcement, syncs Layer 2
/// 1. Both relays running
/// 2. Announcement submitted to relay_a
/// 3. relay_b discovers relay_a from announcement, syncs Layer 2 events
#[tokio::test]
async fn test_live_discovery_syncs_layer2() { ... }

/// Live discovery: Multi-hop discovery with layer chain
/// Similar to recursive but with live submission of announcement
#[tokio::test]
async fn test_live_recursive_discovery() { ... }
```

### `tests/sync/tag_variations.rs` (REFACTORED - Live sync only)

```rust
//! Tag Variation Tests (Live Sync Mode)
//!
//! Tests that all valid tag types are correctly processed during sync.
//! Uses LIVE sync mode - tag parsing is mode-independent, so testing one mode is sufficient.

// === LAYER 2 TAG VARIATIONS ===

/// Layer 2 with lowercase 'a' tag (standard NIP-01)
#[tokio::test]
async fn test_layer2_lowercase_a_tag() { ... }

/// Layer 2 with uppercase 'A' tag (NIP-33 style)
#[tokio::test]
async fn test_layer2_uppercase_a_tag() { ... }

/// Layer 2 with 'q' quote tag (NIP-18 style)
#[tokio::test]
async fn test_layer2_q_tag() { ... }

// === LAYER 3 TAG VARIATIONS ===

/// Layer 3 with lowercase 'e' tag (standard NIP-01)
#[tokio::test]
async fn test_layer3_lowercase_e_tag() { ... }

/// Layer 3 with uppercase 'E' tag (NIP-22 comment)
#[tokio::test]
async fn test_layer3_uppercase_e_tag() { ... }

/// Layer 3 with 'q' quote tag (NIP-18 style)
#[tokio::test]
async fn test_layer3_q_tag() { ... }
```

### `tests/sync/mod.rs` (UPDATED)

```rust
//! Sync Integration Tests
//!
//! Tests for ngit-grasp's proactive sync functionality, organized by sync mode:
//!
//! ## Sync Modes
//!
//! - **Historic Sync** (`historic_sync.rs`): Events exist BEFORE syncing relay connects
//!   - Also called bootstrap/startup sync
//!   - Tests the REQ+EOSE or negentropy-based initial sync
//!
//! - **Live Sync** (`live_sync.rs`): Events arrive AFTER syncing relay connects
//!   - Also called real-time/subscription-based sync
//!   - Tests the event forwarding via active subscriptions
//!
//! ## Other Test Categories
//!
//! - **Discovery** (`discovery.rs`): Relay discovers other relays from announcements
//!   - Has both historic and live variants
//!
//! - **Tag Variations** (`tag_variations.rs`): All valid tag types work correctly
//!   - Uses live sync (tag parsing is mode-independent)
//!
//! - **Metrics** (`metrics.rs`): Prometheus metrics for sync operations
//!
//! - **Catchup** (`catchup.rs`): Documentation only (not integration-testable)

pub mod catchup;
pub mod discovery;
pub mod historic_sync;  // Renamed from bootstrap
pub mod live_sync;
pub mod metrics;
pub mod tag_variations;
```

---

## Test Count Summary

| File                              | Before | After  | Notes                    |
| --------------------------------- | ------ | ------ | ------------------------ |
| `historic_sync.rs` (bootstrap.rs) | 4      | 5      | Renamed, minor additions |
| `live_sync.rs`                    | 3      | 3      | Simplified using helper  |
| `discovery.rs`                    | 3      | 4      | Split into historic/live |
| `tag_variations.rs`               | 6      | 6      | Simplified using helper  |
| `metrics.rs`                      | 9      | 9      | Unchanged                |
| `catchup.rs`                      | 0      | 0      | Documentation only       |
| **Total**                         | **25** | **27** | +2 discovery tests       |

---

## Implementation Plan

### Context

- Two metrics tests are currently failing: `test_live_sync_event_count` and `test_multi_source_aggregate_counts`
- This may indicate implementation bugs in sync functionality
- Plan must account for distinguishing test bugs from implementation bugs

### Phase 1: Establish Baseline

**Goal:** Understand current state before making changes

1. Run `cargo test --test sync` and capture full output
2. Identify all passing vs failing tests
3. For each failing test, investigate whether:
   - Test logic is incorrect (test bug)
   - Implementation is broken (impl bug)
   - Test was never working (aspirational test)
4. Document findings in Known Issues section below

### Phase 2: Add Test Infrastructure

**Goal:** Add new helpers without breaking existing tests

1. Add `SyncTestResult` struct to sync_helpers.rs:

   ```rust
   pub struct SyncTestResult {
       pub source: TestRelay,
       pub syncing: TestRelay,
       pub keys: Keys,
       pub repo_coord: String,
   }
   ```

2. Add `run_sync_test(historic_events, live_events)` helper function

3. Add unit tests for the helper itself:

   - `test_run_sync_test_historic_mode` - verify events sent before connection
   - `test_run_sync_test_live_mode` - verify events sent after connection

4. Run `cargo test` to confirm no regressions

### Phase 3: Refactor Historic Sync Tests

**Goal:** Migrate bootstrap.rs → historic_sync.rs incrementally

1. Rename file: `bootstrap.rs` → `historic_sync.rs`
2. Update `mod.rs` to reference new module name
3. Refactor tests one at a time:
   - `test_bootstrap_syncs_existing_layer2_events` → `test_historic_layer2_issue_syncs`
   - `test_relay_replays_events_after_restart` → keep or remove (tests restart, not sync mode)
   - `test_announcement_not_listing_relay_is_not_synced` → `test_historic_rejects_unlisted_relay`
   - `test_history_sync_without_negentropy` → `test_historic_sync_without_negentropy`
4. Test after each refactor: `cargo test --test sync historic_sync`

### Phase 4: Refactor Live Sync Tests

**Goal:** Simplify live_sync.rs using run_sync_test helper

1. Refactor tests one at a time:
   - `test_live_sync_layer2_events` → use `run_sync_test(&[], &[issue])`
   - `test_live_sync_layer3_events` → use `run_sync_test(&[], &[issue, comment])`
   - `test_live_sync_event_ordering` → may need custom setup for ordering test
2. Test after each refactor: `cargo test --test sync live_sync`

### Phase 5: Refactor Discovery Tests

**Goal:** Split discovery.rs into historic and live sections

1. Add section comment: `// === HISTORIC DISCOVERY ===`
2. Refactor existing tests to use run_sync_test
3. Add new tests:
   - `test_historic_discovery_syncs_layer2`
   - `test_live_discovery_syncs_layer2`
4. Test after each change: `cargo test --test sync discovery`

### Phase 6: Refactor Tag Variations Tests

**Goal:** Simplify tag_variations.rs using run_sync_test (live mode)

1. Add header doc comment explaining live sync mode choice
2. Refactor each test to use `run_sync_test(&[], &[event])`
3. Test after all changes: `cargo test --test sync tag_variations`

### Phase 7: Final Verification and Cleanup

1. Update `mod.rs` documentation
2. Run full test suite: `cargo test --test sync`
3. Compare results to Phase 1 baseline:
   - New failures = regressions from refactor (must fix)
   - Same failures as baseline = pre-existing issues (document)
4. Delete `work/sync-test-refactor-options.md`

---

## Known Issues

_To be filled in during Phase 1_

### Failing Tests Before Refactor

| Test                                          | Status     | Root Cause | Action |
| --------------------------------------------- | ---------- | ---------- | ------ |
| `metrics::test_live_sync_event_count`         | ❌ Failing | TBD        | TBD    |
| `metrics::test_multi_source_aggregate_counts` | ❌ Failing | TBD        | TBD    |

### Implementation Notes

_Any discoveries about how sync actually works vs how tests expect it to work_

---
