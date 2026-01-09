//! Integration tests for GRASP-02 PR3: Maintainer Announcement Re-Processing
//!
//! Tests the two-tier rejected events index and immediate re-processing of
//! maintainer announcements when owner announcements are accepted.

use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::common::{sync_helpers::*, TestRelay};

/// Test that maintainer announcements are re-processed immediately when owner announcement accepted
///
/// Flow:
/// 1. relay_a: Maintainer sends announcement (gets rejected - doesn't list relay_b)
/// 2. relay_b: Owner sends announcement (lists relay_a + maintainer)
/// 3. relay_b syncs from relay_a, maintainer announcement enters rejected index
/// 4. relay_b processes owner announcement, invalidates and re-processes maintainer announcement
/// 5. Both announcements should be in relay_b's database
///
/// Expected time: <5 seconds (vs 24 hours without hot cache)
#[tokio::test]
async fn test_maintainer_announcement_reprocessed_immediately() {
    // Start relay_a (where maintainer announcement will be sent)
    let relay_a = TestRelay::start().await;
    println!("relay_a started at {}", relay_a.url());

    // Start relay_b with sync enabled (will sync from relay_a)
    let relay_b = TestRelay::start_with_sync(None).await;
    println!("relay_b started at {}", relay_b.url());

    // Create keys
    let owner_keys = Keys::generate();
    let maintainer_keys = Keys::generate();

    let identifier = "test-repo";

    let start = std::time::Instant::now();

    // Step 1: Send maintainer announcement to relay_a (will be rejected - doesn't list relay_b)
    let client_a = TestClient::new(relay_a.url(), maintainer_keys.clone())
        .await
        .expect("Failed to connect to relay_a");

    let maintainer_announcement =
        EventBuilder::new(Kind::GitRepoAnnouncement, "Maintainer's repository")
            .tags(vec![
                Tag::identifier(identifier),
                Tag::custom(
                    TagKind::custom("clone"),
                    vec![format!("https://{}/{}.git", relay_a.domain(), identifier)],
                ),
                Tag::custom(TagKind::custom("relays"), vec![relay_a.url().to_string()]),
            ])
            .sign_with_keys(&maintainer_keys)
            .unwrap();

    client_a.send_event(&maintainer_announcement).await.unwrap();
    println!("✓ Maintainer announcement sent to relay_a");

    // Step 2: Send owner announcement to relay_b (lists relay_a + maintainer)
    let client_b = TestClient::new(relay_b.url(), owner_keys.clone())
        .await
        .expect("Failed to connect to relay_b");

    let owner_announcement = EventBuilder::new(Kind::GitRepoAnnouncement, "Owner's repository")
        .tags(vec![
            Tag::identifier(identifier),
            Tag::custom(
                TagKind::custom("clone"),
                vec![format!("https://{}/{}.git", relay_b.domain(), identifier)],
            ),
            Tag::custom(
                TagKind::custom("relays"),
                vec![relay_a.url().to_string(), relay_b.url().to_string()],
            ),
            Tag::custom(
                TagKind::custom("maintainers"),
                vec![maintainer_keys.public_key().to_hex()],
            ),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    client_b.send_event(&owner_announcement).await.unwrap();
    println!("✓ Owner announcement sent to relay_b");

    // Step 3: Wait for sync and re-processing (relay_b discovers relay_a, syncs, re-processes)
    tokio::time::sleep(Duration::from_secs(3)).await;

    let elapsed = start.elapsed();

    // Step 4: Verify both announcements are in relay_b's database
    let owner_filter = Filter::new()
        .kind(Kind::GitRepoAnnouncement)
        .author(owner_keys.public_key())
        .identifier(identifier);

    let owner_found =
        wait_for_event_on_relay(relay_b.url(), owner_filter, Duration::from_secs(2)).await;
    assert!(owner_found, "Owner announcement should be in relay_b");

    let maintainer_filter = Filter::new()
        .kind(Kind::GitRepoAnnouncement)
        .author(maintainer_keys.public_key())
        .identifier(identifier);

    let maintainer_found =
        wait_for_event_on_relay(relay_b.url(), maintainer_filter, Duration::from_secs(2)).await;
    assert!(
        maintainer_found,
        "Maintainer announcement should be re-processed and accepted in relay_b"
    );

    // Step 5: Verify it happened quickly (not 24 hours!)
    assert!(
        elapsed.as_secs() < 10,
        "Re-processing should happen in <10 seconds, took {:?}",
        elapsed
    );

    println!("✅ Maintainer announcement re-processed in {:?}", elapsed);

    client_a.disconnect().await;
    client_b.disconnect().await;
    relay_a.stop().await;
    relay_b.stop().await;
}

/// Test that maintainer announcements NOT in hot cache are still prevented from re-fetching
///
/// Flow:
/// 1. Maintainer announcement arrives → Rejected (added to hot cache + cold index)
/// 2. Wait for hot cache to expire (2+ minutes)
/// 3. Owner announcement arrives → Invalidates cold index
/// 4. Maintainer announcement should NOT be re-fetched (cold index prevents)
/// 5. Only owner announcement should be in database
///
/// This test verifies the cold index prevents repeated downloads after hot cache expiry.
/// Note: This test is slow (2+ minutes) so we'll skip it in normal test runs.
#[tokio::test]
#[ignore] // Skip by default due to 2+ minute duration
async fn test_maintainer_announcement_cold_index_prevents_refetch() {
    let relay = TestRelay::start().await;

    // Create keys
    let owner_keys = Keys::generate();
    let maintainer_keys = Keys::generate();

    let identifier = "test-repo-cold";

    // Create client using TestClient helper
    let client = TestClient::new(relay.url(), maintainer_keys.clone())
        .await
        .expect("Failed to connect to relay");

    // Step 1: Send maintainer announcement (will be rejected - doesn't list our relay)
    let maintainer_announcement =
        EventBuilder::new(Kind::GitRepoAnnouncement, "Maintainer's repository")
            .tags(vec![
                Tag::identifier(identifier),
                Tag::custom(
                    TagKind::custom("clone"),
                    vec![format!("https://example.com/{}.git", identifier)],
                ),
                Tag::custom(
                    TagKind::custom("relays"),
                    vec!["wss://example.com".to_string()],
                ),
            ])
            .sign_with_keys(&maintainer_keys)
            .unwrap();

    // Send maintainer announcement - expect it to be rejected
    let _ = client.send_event(&maintainer_announcement).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Step 2: Wait for hot cache to expire (default: 120 seconds)
    println!("⏳ Waiting for hot cache to expire (120 seconds)...");
    tokio::time::sleep(Duration::from_secs(125)).await;

    // Step 3: Send owner announcement (lists maintainer)
    let owner_announcement = EventBuilder::new(Kind::GitRepoAnnouncement, "Owner's repository")
        .tags(vec![
            Tag::identifier(identifier),
            Tag::custom(
                TagKind::custom("clone"),
                vec![format!("https://{}/{}.git", relay.domain(), identifier)],
            ),
            Tag::custom(TagKind::custom("relays"), vec![relay.url().to_string()]),
            Tag::custom(
                TagKind::custom("maintainers"),
                vec![maintainer_keys.public_key().to_hex()],
            ),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    client.send_event(&owner_announcement).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Step 4: Verify only owner announcement is in database
    let owner_filter = Filter::new()
        .kind(Kind::GitRepoAnnouncement)
        .author(owner_keys.public_key())
        .identifier(identifier);

    let owner_found =
        wait_for_event_on_relay(relay.url(), owner_filter, Duration::from_secs(2)).await;
    assert!(owner_found, "Owner announcement should be accepted");

    let maintainer_filter = Filter::new()
        .kind(Kind::GitRepoAnnouncement)
        .author(maintainer_keys.public_key())
        .identifier(identifier);

    let maintainer_found =
        wait_for_event_on_relay(relay.url(), maintainer_filter, Duration::from_millis(500)).await;
    assert!(
        !maintainer_found,
        "Maintainer announcement should NOT be re-processed (hot cache expired)"
    );

    println!("✅ Cold index prevented re-fetch after hot cache expiry");

    client.disconnect().await;
    relay.stop().await;
}

/// Test multiple maintainers are all re-processed when owner announcement accepted
///
/// Flow:
/// 1. relay_a: Three maintainers send announcements (get rejected - don't list relay_b)
/// 2. relay_b: Owner sends announcement (lists relay_a + all three maintainers)
/// 3. relay_b syncs from relay_a, all maintainer announcements enter rejected index
/// 4. relay_b processes owner announcement, invalidates and re-processes all maintainer announcements
/// 5. All four announcements should be in relay_b's database
#[tokio::test]
async fn test_multiple_maintainers_all_reprocessed() {
    // Start relay_a (where maintainer announcements will be sent)
    let relay_a = TestRelay::start().await;
    println!("relay_a started at {}", relay_a.url());

    // Start relay_b with sync enabled (will sync from relay_a)
    let relay_b = TestRelay::start_with_sync(None).await;
    println!("relay_b started at {}", relay_b.url());

    // Create keys
    let owner_keys = Keys::generate();
    let maintainer1_keys = Keys::generate();
    let maintainer2_keys = Keys::generate();
    let maintainer3_keys = Keys::generate();

    let identifier = "multi-maintainer-repo";

    // Step 1: Send three maintainer announcements to relay_a
    let client_a = TestClient::new(relay_a.url(), maintainer1_keys.clone())
        .await
        .expect("Failed to connect to relay_a");

    for (idx, maintainer_keys) in [&maintainer1_keys, &maintainer2_keys, &maintainer3_keys]
        .iter()
        .enumerate()
    {
        let announcement = EventBuilder::new(
            Kind::GitRepoAnnouncement,
            format!("Maintainer {} repository", idx + 1),
        )
        .tags(vec![
            Tag::identifier(identifier),
            Tag::custom(
                TagKind::custom("clone"),
                vec![format!("https://{}/{}.git", relay_a.domain(), identifier)],
            ),
            Tag::custom(TagKind::custom("relays"), vec![relay_a.url().to_string()]),
        ])
        .sign_with_keys(maintainer_keys)
        .unwrap();

        client_a.send_event(&announcement).await.unwrap();
    }
    println!("✓ Three maintainer announcements sent to relay_a");

    // Step 2: Send owner announcement to relay_b (lists relay_a + all three maintainers)
    let client_b = TestClient::new(relay_b.url(), owner_keys.clone())
        .await
        .expect("Failed to connect to relay_b");

    let owner_announcement = EventBuilder::new(Kind::GitRepoAnnouncement, "Owner's repository")
        .tags(vec![
            Tag::identifier(identifier),
            Tag::custom(
                TagKind::custom("clone"),
                vec![format!("https://{}/{}.git", relay_b.domain(), identifier)],
            ),
            Tag::custom(
                TagKind::custom("relays"),
                vec![relay_a.url().to_string(), relay_b.url().to_string()],
            ),
            Tag::custom(
                TagKind::custom("maintainers"),
                vec![
                    maintainer1_keys.public_key().to_hex(),
                    maintainer2_keys.public_key().to_hex(),
                    maintainer3_keys.public_key().to_hex(),
                ],
            ),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    client_b.send_event(&owner_announcement).await.unwrap();
    println!("✓ Owner announcement sent to relay_b");

    // Step 3: Wait for sync and re-processing
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Step 4: Verify all four announcements are in relay_b's database
    for (name, keys) in [
        ("owner", &owner_keys),
        ("maintainer1", &maintainer1_keys),
        ("maintainer2", &maintainer2_keys),
        ("maintainer3", &maintainer3_keys),
    ] {
        let filter = Filter::new()
            .kind(Kind::GitRepoAnnouncement)
            .author(keys.public_key())
            .identifier(identifier);

        let found = wait_for_event_on_relay(relay_b.url(), filter, Duration::from_secs(2)).await;
        assert!(found, "{} announcement should be in relay_b", name);
    }

    println!("✅ All three maintainer announcements re-processed successfully");

    client_a.disconnect().await;
    client_b.disconnect().await;
    relay_a.stop().await;
    relay_b.stop().await;
}

/// Test that invalid maintainer public keys don't cause panics
///
/// Flow:
/// 1. Maintainer announcement arrives → Rejected
/// 2. Owner announcement arrives with INVALID maintainer hex → Should handle gracefully
/// 3. Owner announcement should still be accepted
/// 4. Maintainer announcement should NOT be re-processed (invalid pubkey)
#[tokio::test]
async fn test_invalid_maintainer_pubkey_handled_gracefully() {
    let relay = TestRelay::start().await;

    // Create keys
    let owner_keys = Keys::generate();
    let maintainer_keys = Keys::generate();

    let identifier = "invalid-maintainer-repo";

    // Create client using TestClient helper
    let client = TestClient::new(relay.url(), owner_keys.clone())
        .await
        .expect("Failed to connect to relay");

    // Step 1: Send maintainer announcement (will be rejected - doesn't list our relay)
    let maintainer_announcement =
        EventBuilder::new(Kind::GitRepoAnnouncement, "Maintainer's repository")
            .tags(vec![
                Tag::identifier(identifier),
                Tag::custom(
                    TagKind::custom("clone"),
                    vec![format!("https://example.com/{}.git", identifier)],
                ),
                Tag::custom(
                    TagKind::custom("relays"),
                    vec!["wss://example.com".to_string()],
                ),
            ])
            .sign_with_keys(&maintainer_keys)
            .unwrap();

    // Send maintainer announcement - expect it to be rejected
    let _ = client.send_event(&maintainer_announcement).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Step 2: Send owner announcement with INVALID maintainer hex
    let owner_announcement = EventBuilder::new(Kind::GitRepoAnnouncement, "Owner's repository")
        .tags(vec![
            Tag::identifier(identifier),
            Tag::custom(
                TagKind::custom("clone"),
                vec![format!("https://{}/{}.git", relay.domain(), identifier)],
            ),
            Tag::custom(TagKind::custom("relays"), vec![relay.url().to_string()]),
            Tag::custom(
                TagKind::custom("maintainers"),
                vec!["invalid-hex-not-a-pubkey".to_string()],
            ),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    client.send_event(&owner_announcement).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Step 3: Verify owner announcement accepted, maintainer not re-processed
    let owner_filter = Filter::new()
        .kind(Kind::GitRepoAnnouncement)
        .author(owner_keys.public_key())
        .identifier(identifier);

    let owner_found =
        wait_for_event_on_relay(relay.url(), owner_filter, Duration::from_secs(2)).await;
    assert!(
        owner_found,
        "Owner announcement should be accepted despite invalid maintainer"
    );

    let maintainer_filter = Filter::new()
        .kind(Kind::GitRepoAnnouncement)
        .author(maintainer_keys.public_key())
        .identifier(identifier);

    let maintainer_found =
        wait_for_event_on_relay(relay.url(), maintainer_filter, Duration::from_millis(500)).await;
    assert!(
        !maintainer_found,
        "Maintainer announcement should NOT be re-processed (invalid pubkey)"
    );

    println!("✅ Invalid maintainer pubkey handled gracefully without panic");

    client.disconnect().await;
    relay.stop().await;
}
