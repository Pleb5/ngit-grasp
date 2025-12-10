//! Discovery Sync Tests
//!
//! Tests for relay discovery from announcement events.
//! When a relay receives an announcement listing another relay,
//! it should discover and connect to that relay to sync events.
//!
//! # Tests
//! - Test 2: Direct Layer 3 discovery from Layer 2
//! - Test 3: Recursive multi-hop Layer 3 discovery

use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::common::{sync_helpers::*, TestRelay};

/// Kind 1617 - Patch event (NIP-34)
const KIND_PATCH: u16 = 1617;

/// Create an event referencing a repository coordinate via 'a' tag.
///
/// Used to create Layer 2 events like patches that reference a repository.
fn create_event_referencing_repo(keys: &Keys, repo_coord: &str, kind: u16, content: &str) -> Event {
    let tags = vec![Tag::custom(
        TagKind::custom("a"),
        vec![repo_coord.to_string()],
    )];

    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign event")
}

/// Test 2: Relay discovers another relay via announcement and syncs Layer 2 events
///
/// Scenario:
/// 1. relay_a has announcement + patch event (Layer 2)
/// 2. relay_b (sync enabled, NO bootstrap) receives the announcement directly
/// 3. relay_b discovers relay_a from the announcement's relays tag
/// 4. relay_b connects to relay_a and syncs the patch event
///
/// This tests dynamic relay discovery from direct submissions.
#[tokio::test]
async fn test_discovers_layer3_via_layer2() {
    // 1. Start relay_a (source) with the patch event
    let relay_a = TestRelay::start().await;
    println!(
        "relay_a started at {} (domain: {})",
        relay_a.url(),
        relay_a.domain()
    );

    // 2. Start relay_b: sync enabled but NO bootstrap relay - will discover relay_a
    let relay_b = TestRelay::start_with_sync(None).await;
    println!(
        "relay_b started at {} (domain: {})",
        relay_b.url(),
        relay_b.domain()
    );

    // 3. Create test keys
    let keys = Keys::generate();

    // 4. Create a repository announcement that lists BOTH relays
    let announcement = create_repo_announcement(
        &keys,
        &[&relay_a.domain(), &relay_b.domain()],
        "test-repo-discovery",
    );
    let announcement_id = announcement.id;

    println!(
        "Created announcement {} (kind {})",
        announcement_id,
        announcement.kind.as_u16()
    );
    for tag in announcement.tags.iter() {
        println!("  Tag: {:?}", tag.as_slice());
    }

    // 5. Build the repo coordinate for the 'a' tag in the patch
    let repo_coord = format!(
        "{}:{}:{}",
        KIND_REPOSITORY_STATE,
        keys.public_key().to_hex(),
        "test-repo-discovery"
    );

    // 6. Create a patch event (Layer 2) that references the announcement
    let patch = create_event_referencing_repo(&keys, &repo_coord, KIND_PATCH, "Test patch proposal");
    let patch_id = patch.id;

    println!("Created patch {} (kind {})", patch_id, patch.kind.as_u16());
    for tag in patch.tags.iter() {
        println!("  Tag: {:?}", tag.as_slice());
    }

    // 7. Send announcement and patch to relay_a ONLY
    let client_a = TestClient::new(relay_a.url(), keys.clone())
        .await
        .expect("Failed to connect to relay_a");

    client_a
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to relay_a");
    println!("Announcement sent to relay_a");

    client_a
        .send_event(&patch)
        .await
        .expect("Failed to send patch to relay_a");
    println!("Patch sent to relay_a");

    client_a.disconnect().await;

    // 8. Send announcement to relay_b directly (triggers discovery of relay_a)
    let client_b = TestClient::new(relay_b.url(), keys.clone())
        .await
        .expect("Failed to connect to relay_b");

    client_b
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to relay_b");
    println!("Announcement sent to relay_b (should trigger discovery of relay_a)");

    client_b.disconnect().await;

    // 9. Wait for relay_b to discover relay_a and sync the patch
    println!("Waiting 3s for relay_b to discover relay_a and sync patch...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 10. Verify patch was synced to relay_b
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_PATCH))
        .author(keys.public_key());

    let patch_synced = wait_for_event_on_relay(relay_b.url(), filter, Duration::from_secs(5)).await;

    if patch_synced {
        println!(
            "Patch {} found on relay_b (synced from discovered relay_a)",
            patch_id
        );
    } else {
        println!("Patch {} NOT found on relay_b", patch_id);
    }

    // 11. Cleanup
    relay_b.stop().await;
    relay_a.stop().await;

    assert!(
        patch_synced,
        "Patch {} should have been synced to relay_b from discovered relay_a",
        patch_id
    );
}

/// Test 3: Layer 2 discovery with full event chain
///
/// Scenario:
/// 1. relay_a has: announcement → issue (Layer 2)
/// 2. relay_b receives announcement directly
/// 3. relay_b discovers relay_a and syncs the issue (Layer 2)
///
/// This tests that Layer 2 events (issues/patches) are synced when their
/// parent repository is discovered. The chain is:
///   Layer 1 (30617): Repository announcement
///   Layer 2 (1618): Issue referencing repo
///
/// Note: Layer 3 (comments on issues) sync is tracked separately and may
/// be implemented in future phases. This test focuses on Layer 2 discovery.
#[tokio::test]
async fn test_layer2_discovery_with_chain() {
    // 1. Start relay_a (source) with the event chain
    let relay_a = TestRelay::start().await;
    println!(
        "relay_a started at {} (domain: {})",
        relay_a.url(),
        relay_a.domain()
    );

    // 2. Start relay_b: sync enabled but NO bootstrap relay
    let relay_b = TestRelay::start_with_sync(None).await;
    println!(
        "relay_b started at {} (domain: {})",
        relay_b.url(),
        relay_b.domain()
    );

    // 3. Create test keys
    let keys = Keys::generate();

    // 4. Create the event chain on relay_a:

    // Layer 1: Repository announcement
    let announcement = create_repo_announcement(
        &keys,
        &[&relay_a.domain(), &relay_b.domain()],
        "test-repo-chain",
    );
    let announcement_id = announcement.id;
    println!("Created announcement {} (Layer 1)", announcement_id);

    // Build repo coordinate for Layer 2 reference
    let repo_coord = repo_coord(&keys, "test-repo-chain");

    // Layer 2: Issue referencing the repo
    let issue = build_layer2_issue_event(&keys, &repo_coord, "Test issue for chain discovery")
        .expect("Failed to create issue");
    let issue_id = issue.id;
    println!("Created issue {} (Layer 2)", issue_id);

    // 5. Send all events to relay_a
    let client_a = TestClient::new(relay_a.url(), keys.clone())
        .await
        .expect("Failed to connect to relay_a");

    client_a
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement");
    client_a
        .send_event(&issue)
        .await
        .expect("Failed to send issue");

    println!("Events sent to relay_a");
    client_a.disconnect().await;

    // 6. Send only the announcement to relay_b (triggers discovery)
    let client_b = TestClient::new(relay_b.url(), keys.clone())
        .await
        .expect("Failed to connect to relay_b");

    client_b
        .send_event(&announcement)
        .await
        .expect("Failed to send announcement to relay_b");
    println!("Announcement sent to relay_b (should trigger discovery)");

    client_b.disconnect().await;

    // 7. Wait for sync
    println!("Waiting 3s for Layer 2 sync...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 8. Verify Layer 2 event synced to relay_b
    let issue_filter = Filter::new()
        .kind(Kind::Custom(KIND_ISSUE))
        .author(keys.public_key());
    let issue_synced = wait_for_event_on_relay(relay_b.url(), issue_filter, Duration::from_secs(5)).await;

    println!("Sync result:");
    println!("  Issue {} synced: {}", issue_id, issue_synced);

    // 9. Cleanup
    relay_b.stop().await;
    relay_a.stop().await;

    // 10. Assert Layer 2 event synced
    assert!(
        issue_synced,
        "Issue {} (Layer 2) should have synced to relay_b via discovery",
        issue_id
    );
}

/// Test 3: 3-relay recursive discovery - relay discovers third relay through bootstrap
///
/// Scenario:
/// ```text
///     relay_a (SUT)       relay_b (bootstrap)     relay_c (discovered)
///         │                     │                       │
///         │                     │ has announcement_x    │ has announcement_y
///         │                     │ listing A+B+C         │ listing A+C
///         │                     │                       │
///         ├────connect──────────►                       │
///         │◄───sync announcement_x───────────────────────
///         │                                             │
///         │    discovers relay_c from announcement_x    │
///         │                                             │
///         ├─────────────connect─────────────────────────►
///         │◄────────────sync announcement_y─────────────┘
/// ```
///
/// This tests that relay_a:
/// 1. Connects to relay_b (configured as bootstrap)
/// 2. Receives announcement_x which lists relay_c
/// 3. Discovers and connects to relay_c
/// 4. Syncs announcement_y from relay_c
///
#[tokio::test]
async fn test_recursive_relay_discovery_syncs_announcement() {
    // 1. Start all three relays
    
    // relay_b - will be the bootstrap relay, has announcement_x
    let relay_b = TestRelay::start().await;
    println!(
        "relay_b (bootstrap) started at {} (domain: {})",
        relay_b.url(),
        relay_b.domain()
    );

    // relay_c - will be discovered via announcement_x, has announcement_y
    let relay_c = TestRelay::start().await;
    println!(
        "relay_c (to be discovered) started at {} (domain: {})",
        relay_c.url(),
        relay_c.domain()
    );

    // relay_a - SUT, starts with relay_b as bootstrap
    let relay_a = TestRelay::start_with_sync(Some(relay_b.url().to_string())).await;
    println!(
        "relay_a (SUT) started at {} (domain: {})",
        relay_a.url(),
        relay_a.domain()
    );

    // 2. Create test keys (one for each announcement)
    let keys_x = Keys::generate();
    let keys_y = Keys::generate();

    // 3. Create announcement_x on relay_b (lists all three relays: A+B+C)
    let announcement_x = create_repo_announcement(
        &keys_x,
        &[&relay_a.domain(), &relay_b.domain(), &relay_c.domain()],
        "repo-x-all-relays",
    );
    let announcement_x_id = announcement_x.id;
    println!("Created announcement_x {} listing A+B+C", announcement_x_id);
    for tag in announcement_x.tags.iter() {
        println!("  Tag: {:?}", tag.as_slice());
    }

    // 4. Create announcement_y on relay_c (lists only A+C, NOT B)
    let announcement_y = create_repo_announcement(
        &keys_y,
        &[&relay_a.domain(), &relay_c.domain()],
        "repo-y-ac-only",
    );
    let announcement_y_id = announcement_y.id;
    println!("Created announcement_y {} listing A+C only", announcement_y_id);
    for tag in announcement_y.tags.iter() {
        println!("  Tag: {:?}", tag.as_slice());
    }

    // 5. Send announcement_x to relay_b only
    let client_b = TestClient::new(relay_b.url(), keys_x.clone())
        .await
        .expect("Failed to connect to relay_b");

    client_b
        .send_event(&announcement_x)
        .await
        .expect("Failed to send announcement_x to relay_b");
    println!("announcement_x sent to relay_b");

    client_b.disconnect().await;

    // 6. Send announcement_y to relay_c only
    let client_c = TestClient::new(relay_c.url(), keys_y.clone())
        .await
        .expect("Failed to connect to relay_c");

    client_c
        .send_event(&announcement_y)
        .await
        .expect("Failed to send announcement_y to relay_c");
    println!("announcement_y sent to relay_c");

    client_c.disconnect().await;

    // 7. Wait for relay_a to:
    //    - Sync from bootstrap relay_b (gets announcement_x)
    //    - Discover relay_c from announcement_x's relays tag
    //    - Connect to relay_c and sync announcement_y
    println!("Waiting 5s for recursive relay discovery...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 8. Verify announcement_x was synced to relay_a (from bootstrap relay_b)
    let filter_x = Filter::new()
        .kind(Kind::Custom(KIND_REPOSITORY_STATE))
        .author(keys_x.public_key());

    let announcement_x_synced =
        wait_for_event_on_relay(relay_a.url(), filter_x, Duration::from_secs(5)).await;

    println!(
        "announcement_x {} synced to relay_a: {}",
        announcement_x_id, announcement_x_synced
    );

    // 9. Verify announcement_y was synced to relay_a (from discovered relay_c)
    let filter_y = Filter::new()
        .kind(Kind::Custom(KIND_REPOSITORY_STATE))
        .author(keys_y.public_key());

    let announcement_y_synced =
        wait_for_event_on_relay(relay_a.url(), filter_y, Duration::from_secs(5)).await;

    println!(
        "announcement_y {} synced to relay_a: {}",
        announcement_y_id, announcement_y_synced
    );

    // 10. Cleanup
    relay_a.stop().await;
    relay_b.stop().await;
    relay_c.stop().await;

    // 11. Assertions
    assert!(
        announcement_x_synced,
        "announcement_x {} should have synced from bootstrap relay_b to relay_a",
        announcement_x_id
    );

    assert!(
        announcement_y_synced,
        "announcement_y {} should have synced from discovered relay_c to relay_a (recursive discovery)",
        announcement_y_id
    );
}