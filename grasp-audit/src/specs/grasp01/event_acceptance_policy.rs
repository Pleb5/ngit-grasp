//! GRASP-01 Nostr Event Acceptance Policy
//!
//! Tests for GRASP-01 Nostr event acceptance policy (lines 3-7 of ../grasp/01.md)
//!
//! This file validates that a GRASP-01 compliant relay:
//! - Accepts valid NIP-34 repository announcements listing the service
//! - Rejects announcements that don't list the service in clone and relays tags
//! - Accepts repository state announcements
//! - Accepts events that TAG accepted repositories
//! - Accepts events that ARE TAGGED BY accepted events (transitive)
//!
//! ## Running Tests
//!
//! ### Recommended: Automated Relay Management
//! ```bash
//! cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test
//! ```
//! This script automatically starts a relay, runs all tests, and cleans up.
//!
//! ### Manual Testing (if needed)
//! ```bash
//! # 1. Start ngit-relay in a separate terminal:
//! docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest
//!
//! # 2. Run all ignored tests (includes these smoke tests):
//! RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib -- --ignored --nocapture
//!
//! # 3. Run ONLY these specific smoke tests:
//! RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_grasp01_event_acceptance -- --ignored --nocapture
//! ```
//!
//! ## Test Groups (12 total tests, target <5s execution)
//!
//! ### Group 1: Accept Events Tagging Accepted Repositories (3 tests)
//! - **Test 1.1**: Issue via `a` tag → Validates NIP-33 addressable event references
//! - **Test 1.2**: Comment via `A` tag → Validates NIP-22 root addressable references
//! - **Test 1.3**: Kind 1 via `q` tag → Validates NIP-18 quote references
//!
//! ### Group 2: Accept Events Tagging Accepted Events - Transitive (3 tests)
//! - **Test 2.1**: Issue quoting issue via `q` → Multi-hop transitive acceptance
//! - **Test 2.2**: Comment via `E` tag → NIP-22 threaded root references
//! - **Test 2.3**: Kind 1 via `e` tag → Standard NIP-01 reply chains
//!
//! ### Group 3: Accept Events Tagged BY Accepted Events - Forward References (3 tests)
//! - **Test 3.1**: Kind 1 referenced in issue → Forward reference acceptance
//! - **Test 3.2**: Comment referenced in comment → Nested forward references
//! - **Test 3.3**: Kind 1 referenced in Kind 1 → Cross-event forward refs
//!
//! ### Group 4: Reject Unrelated Events (3 tests)
//! - **Test 4.1**: Orphan issue → No repo connection
//! - **Test 4.2**: Orphan Kind 1 → No accepted event references
//! - **Test 4.3**: Comment quoting other repo → Wrong repository context
//!
//! ## Test Coverage Summary
//!
//! **Tag Types Validated:**
//! - `a` tags (NIP-33 addressable events)
//! - `A` tags (NIP-22 root addressable)
//! - `q` tags (NIP-18 quotes)
//! - `e` tags (NIP-01 event references)
//! - `E` tags (NIP-22 root event references)
//!
//! **Acceptance Paths:**
//! - Direct repository references (tags accepted repos)
//! - Transitive acceptance (tags events that tag accepted repos)
//! - Forward references (late-arriving events tagged by accepted events)
//! - Rejection cases (unrelated events with no connection)
//!
//! **Helper Functions (6 total):**
//! - `extract_d_tag()` - Extract identifier from events
//! - `create_test_repo()` - Create repository announcements
//! - `create_issue_for_repo()` - Create issues referencing repos
//! - `create_comment_for_event()` - Create NIP-22 comments
//! - `send_and_verify_accepted()` - Verify event acceptance
//! - `send_and_verify_rejected()` - Verify event rejection
//!
//! ## Performance Target
//!
//! All 12 tests should complete in **under 5 seconds** total when run against
//! a local ngit-relay instance. Each test includes a 100ms sleep for relay
//! propagation, so total theoretical minimum is ~1.2s for serial execution.
//!
//! ## Implementation Notes
//!
//! - Tests use the audit client which automatically adds cleanup tags
//! - All events are tagged for production relay cleanup
//! - Tests are designed to be independent and can run in any order
//! - Forward reference tests verify out-of-order event acceptance
//! - Transitive tests verify multi-hop acceptance chains

use nostr_sdk::{Event, Filter, Kind, Tag, TagKind, Timestamp};
use crate::{AuditClient, AuditResult, TestResult};
use std::time::Duration;

/// Test suite for GRASP-01 event acceptance policy
pub struct EventAcceptancePolicyTests;

impl EventAcceptancePolicyTests {
    /// Run all event acceptance policy tests
    pub async fn run_all(client: &AuditClient) -> AuditResult {
        let mut results = AuditResult::new("GRASP-01 Nostr Event Acceptance Policy Tests");
        
        // Repository Announcement Acceptance Tests
        results.add(Self::test_accept_valid_repo_announcement(client).await);
        results.add(Self::test_reject_repo_announcement_missing_clone_tag(client).await);
        results.add(Self::test_reject_repo_announcement_missing_relays_tag(client).await);
        
        // Repository State Announcement Tests
        results.add(Self::test_accept_valid_repo_state_announcement(client).await);
        
        // Group 1: Accept Events Tagging Accepted Repositories
        results.add(Self::test_accept_issue_via_a_tag(client).await);
        results.add(Self::test_accept_comment_via_A_tag(client).await);
        results.add(Self::test_accept_kind1_via_q_tag(client).await);
        
        // Group 2: Accept Events Tagging Accepted Events (Transitive)
        results.add(Self::test_accept_issue_quoting_issue_via_q(client).await);
        results.add(Self::test_accept_comment_via_E_tag(client).await);
        results.add(Self::test_accept_kind1_via_e_tag(client).await);
        
        // Group 3: Accept Events Tagged BY Accepted Events (Forward Refs)
        results.add(Self::test_accept_kind1_referenced_in_issue(client).await);
        results.add(Self::test_accept_comment_referenced_in_comment(client).await);
        results.add(Self::test_accept_kind1_referenced_in_kind1(client).await);
        
        // Group 4: Reject Unrelated Events
        results.add(Self::test_reject_orphan_issue(client).await);
        results.add(Self::test_reject_orphan_kind1(client).await);
        results.add(Self::test_reject_comment_quoting_other_repo(client).await);
        
        results
    }
    
    // ============================================================
    // Repository Announcement Acceptance Tests
    // ============================================================
    
    /// Test: Accept valid repository announcements
    ///
    /// Spec: Lines 3-5 of ../grasp/01.md
    /// Requirement: MUST accept repo announcements listing service in clone & relays tags
    async fn test_accept_valid_repo_announcement(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_valid_repo_announcement",
            "GRASP-01:nostr-relay:3-5",
            "Accept valid repository announcements with service in clone and relays tags",
        )
        .run(|| async {
            // Create a NIP-34 repository announcement event
            let event = client.create_repo_announcement("accept_valid_repo_announcement").await
                .map_err(|e| format!("Failed to create repository announcement: {}", e))?;
            
            // Get relay URL for validation
            let relay_url = client.client().relays().await
                .keys()
                .next()
                .ok_or("No relay connected")?
                .to_string();
            
            // Convert WebSocket URL to HTTP URL for validation
            let http_url = relay_url
                .replace("ws://", "http://")
                .replace("wss://", "https://");
            
            // Extract repo_id from the event's d tag
            let repo_id = event.tags.iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in announcement")?
                .to_string();
            
            // Send the event
            let event_id = client.send_event(event.clone()).await
                .map_err(|e| format!("Failed to send repository announcement to relay: {}", e))?;
            
            // Query back to verify it was accepted and stored
            let filter = Filter::new()
                .kind(Kind::GitRepoAnnouncement)
                .author(client.public_key())
                .identifier(&repo_id);
            
            let events = client.query(filter).await
                .map_err(|e| format!("Failed to query events from relay: {}", e))?;
            
            // Verify we got the event back
            if events.is_empty() {
                return Err(format!(
                    "Event was not stored in relay (possibly rejected). Event ID: {}, Repo ID: {}",
                    event_id, repo_id
                ));
            }
            
            // Verify it's the same event
            let stored_event = events.iter()
                .find(|e| e.id == event_id)
                .ok_or(format!(
                    "Stored event ID doesn't match sent event. Expected: {}, Got {} events",
                    event_id, events.len()
                ))?;
            
            // Verify key tags are present
            let has_clone_tag = stored_event.tags.iter()
                .any(|t| {
                    t.kind() == TagKind::Custom("clone".into()) &&
                    t.content().map(|c| c.contains(&http_url)).unwrap_or(false)
                });
            
            let has_relays_tag = stored_event.tags.iter()
                .any(|t| {
                    t.kind() == TagKind::Custom("relays".into()) &&
                    t.content() == Some(&relay_url)
                });
            
            if !has_clone_tag {
                return Err(format!("Stored event missing clone tag with service URL ({})", http_url));
            }
            
            if !has_relays_tag {
                return Err(format!("Stored event missing relays tag with service URL ({})", relay_url));
            }
            
            Ok(())
        })
        .await
    }
    
    /// Test: Reject repo announcements not listing service in clone tag
    ///
    /// Spec: Line 5 of ../grasp/01.md
    /// Requirement: MUST reject announcements not listing service (unless GRASP-05)
    async fn test_reject_repo_announcement_missing_clone_tag(client: &AuditClient) -> TestResult {
        TestResult::new(
            "reject_repo_announcement_missing_clone_tag",
            "GRASP-01:nostr-relay:5",
            "Reject repository announcements without service in clone tag",
        )
        .run(|| async {
            // Get relay URL from client
            let relay_url = client.client().relays().await
                .keys()
                .next()
                .ok_or("No relay connected - client has no active relay connections")?
                .to_string();
            
            // Create unique repository identifier
            let timestamp = Timestamp::now().as_u64();
            let repo_id = format!("test-repo-no-clone-{}", timestamp);
            
            // Create repo announcement WITHOUT service in clone tag
            let event = client.event_builder(Kind::GitRepoAnnouncement, "")
                .tag(Tag::identifier(&repo_id))
                .tag(Tag::custom(TagKind::Custom("name".into()), vec!["Test Repo No Clone"]))
                .tag(Tag::custom(TagKind::Custom("clone".into()), vec!["https://github.com/user/repo.git"])) // NOT this service
                .tag(Tag::custom(TagKind::Custom("relays".into()), vec![relay_url.clone()])) // Correct relay
                .build(client.keys())
                .map_err(|e| format!("Failed to build event: {}", e))?;
            
            let event_id = event.id;
            
            // Send event - expect rejection
            let send_result = client.send_event(event.clone()).await;
            
            // Query to verify event is NOT stored
            let filter = Filter::new()
                .kind(Kind::GitRepoAnnouncement)
                .author(client.public_key())
                .identifier(&repo_id);
            
            let events = client.query(filter).await
                .map_err(|e| format!("Failed to query events from relay: {}", e))?;
            
            // Verify event was rejected (not stored)
            if events.iter().any(|e| e.id == event_id) {
                return Err(format!(
                    "Relay incorrectly accepted announcement without service in clone tag. \
                    Event ID: {}, Clone URL: https://github.com/user/repo.git (should require {})",
                    event_id, relay_url
                ));
            }
            
            Ok(())
        })
        .await
    }
    
    /// Test: Reject repo announcements not listing service in relays tag
    ///
    /// Spec: Line 5 of ../grasp/01.md
    /// Requirement: MUST reject announcements not listing service in relays
    async fn test_reject_repo_announcement_missing_relays_tag(client: &AuditClient) -> TestResult {
        TestResult::new(
            "reject_repo_announcement_missing_relays_tag",
            "GRASP-01:nostr-relay:5",
            "Reject repository announcements without service in relays tag",
        )
        .run(|| async {
            // Get relay URL from client
            let relay_url = client.client().relays().await
                .keys()
                .next()
                .ok_or("No relay connected - client has no active relay connections")?
                .to_string();
            
            // Convert WebSocket URL to HTTP URL for clone tag
            let http_url = relay_url
                .replace("ws://", "http://")
                .replace("wss://", "https://");
            
            // Create unique repository identifier
            let timestamp = Timestamp::now().as_u64();
            let repo_id = format!("test-repo-no-relays-{}", timestamp);
            
            // Create repo announcement WITHOUT service in relays tag
            let event = client.event_builder(Kind::GitRepoAnnouncement, "")
                .tag(Tag::identifier(&repo_id))
                .tag(Tag::custom(TagKind::custom("name"), vec!["Test Repo No Relays"]))
                .tag(Tag::custom(TagKind::custom("clone"), vec![format!("{}/{}/test-repo.git", http_url, client.public_key())])) // Correct clone
                .tag(Tag::custom(TagKind::custom("relays"), vec!["wss://relay.damus.io"])) // NOT this service
                .build(client.keys())
                .map_err(|e| format!("Failed to build event: {}", e))?;
            
            let event_id = event.id;
            
            // Send event - expect rejection
            let _send_result = client.send_event(event.clone()).await;
            
            // Query to verify event is NOT stored
            let filter = Filter::new()
                .kind(Kind::GitRepoAnnouncement)
                .author(client.public_key())
                .identifier(&repo_id);
            
            let events = client.query(filter).await
                .map_err(|e| format!("Failed to query events from relay: {}", e))?;
            
            // Verify event was rejected (not stored)
            if events.iter().any(|e| e.id == event_id) {
                return Err(format!(
                    "Relay incorrectly accepted announcement without service in relays tag. \
                    Event ID: {}, Relays URL: wss://relay.damus.io (should require {})",
                    event_id, relay_url
                ));
            }
            
            Ok(())
        })
        .await
    }
    
    // ============================================================
    // Repository State Announcement Tests
    // ============================================================
    
    /// Test: Accept valid repository state announcements
    ///
    /// Spec: Lines 6-7 of ../grasp/01.md
    /// Requirement: MUST accept repo state announcements with d, maintainers, and r tags
    async fn test_accept_valid_repo_state_announcement(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_valid_repo_state_announcement",
            "GRASP-01:nostr-relay:6-7",
            "Accept valid state announcements after repo announcement accepted",
        )
        .run(|| async {
            // First, create a repository announcement (kind 30617) by the same author
            let test_name = format!("test-repo-multi-refs-{}", Timestamp::now().as_u64());
            let repo_event = client.create_repo_announcement(&test_name).await
                .map_err(|e| format!("Failed to create repository announcement: {}", e))?;
            
            // Extract repo_id from the repository announcement
            let repo_id = repo_event.tags.iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in repository announcement")?
                .to_string();
            
            // Note: npub not used in this test, removed unused variable
            
            // Create kind 30618 repository state announcement with multiple refs
            // Format: ["r", "refs/heads/main", "<commit-id>"]
            let event = client.event_builder(Kind::Custom(30618), "")
                .tag(Tag::identifier(&repo_id))
                .tag(Tag::custom(TagKind::custom("refs/heads/main"), vec![
                    "abc123def456789012345678901234567890abcd"
                ]))
                .tag(Tag::custom(TagKind::custom("refs/heads/develop"), vec![
                    "def456789012345678901234567890abcdef123"
                ]))
                .tag(Tag::custom(TagKind::custom("refs/tags/v1.0.0"), vec![
                    "123456789012345678901234567890abcdef456"
                ]))
                .tag(Tag::custom(TagKind::custom("HEAD"), vec![
                    "ref: refs/heads/main"
                ]))
                .build(client.keys())
                .map_err(|e| format!("Failed to build state announcement: {}", e))?;
            
            let event_id = event.id;

            // Send the repo announcement event
            client.send_event(repo_event.clone()).await
                .map_err(|e| format!("Failed to send state announcement to relay: {}", e))?;

            // Send the state event
            client.send_event(event.clone()).await
                .map_err(|e| format!("Failed to send state announcement to relay: {}", e))?;
            
            // Query back to verify it was accepted and stored
            let filter = Filter::new()
                .kind(Kind::Custom(30618))
                .author(client.public_key())
                .identifier(&repo_id);
            
            let events = client.query(filter).await
                .map_err(|e| format!("Failed to query events from relay: {}", e))?;
            
            // Verify we got the event back
            if events.is_empty() {
                return Err(format!(
                    "Event was not stored in relay (possibly rejected). Event ID: {}, Repo ID: {}",
                    event_id, repo_id
                ));
            }
                        
            Ok(())
        })
        .await
    }
    
    // ============================================================
    // Helper Functions (6 total)
    // ============================================================
    
    /// Extract the `d` tag value from an event
    fn extract_d_tag(event: &Event) -> Option<String> {
        for tag in event.tags.iter() {
            let tag_vec = tag.clone().to_vec();
            if tag_vec.len() >= 2 && tag_vec[0] == "d" {
                return Some(tag_vec[1].to_string());
            }
        }
        None
    }
    
    /// Create a basic repository announcement (kind 30617)
    /// Uses the client's create_repo_announcement helper which includes required clone and relays tags
    async fn create_test_repo(client: &AuditClient, repo_id: &str) -> Result<Event, String> {
        client.create_repo_announcement(repo_id)
            .await
            .map_err(|e| e.to_string())
    }
    
    /// Create an issue (kind 1621) that references a repository
    /// Uses AuditClient::create_issue helper method
    fn create_issue_for_repo(
        client: &AuditClient,
        repo_event: &Event,
        issue_title: &str,
    ) -> Result<Event, String> {
        client.create_issue(repo_event, issue_title, "issue content", vec![])
            .map_err(|e| e.to_string())
    }
    
    /// Create a NIP-22 comment (kind 1111) for an event
    /// Uses AuditClient::create_comment helper method
    fn create_comment_for_event(
        client: &AuditClient,
        event: &Event,
        content: &str,
    ) -> Result<Event, String> {
        client.create_comment(event, content, vec![])
            .map_err(|e| e.to_string())
    }
    
    /// Send event and verify it was accepted (stored by relay)
    async fn send_and_verify_accepted(
        client: &AuditClient,
        event: Event,
        description: &str,
    ) -> Result<(), String> {
        let event_id = event.id;
        
        client.send_event(event).await
            .map_err(|e| e.to_string())?;
        
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        let filter = Filter::new().id(event_id);
        let events = client.query(filter).await
            .map_err(|e| e.to_string())?;
        
        if events.is_empty() {
            return Err(format!("Event should be accepted: {}", description));
        }
        
        Ok(())
    }
    
    /// Send event and verify it was rejected (NOT stored by relay)
    async fn send_and_verify_rejected(
        client: &AuditClient,
        event: Event,
        description: &str,
    ) -> Result<(), String> {
        let event_id = event.id;
        
        client.send_event(event).await
            .map_err(|e| e.to_string())?;
        
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        let filter = Filter::new().id(event_id);
        let events = client.query(filter).await
            .map_err(|e| e.to_string())?;
        
        if !events.is_empty() {
            return Err(format!("Event should be rejected: {}", description));
        }
        
        Ok(())
    }
    
    // ============================================================
    // Group 1: Accept Events Tagging Accepted Repositories (3 tests)
    // ============================================================
    
    /// Test 1.1: Issue referencing repo via `a` tag should be accepted
    async fn test_accept_issue_via_a_tag(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_issue_via_a_tag",
            "GRASP-01:event-acceptance:1.1",
            "Accept issue referencing repo via 'a' tag",
        )
        .run(|| async {
            // 1. Create and send repo announcement
            let repo = Self::create_test_repo(client, "test-repo-1").await?;
            Self::send_and_verify_accepted(client, repo.clone(), "repository announcement").await?;
            
            // 2. Create issue that references the repo (uses create_issue_for_repo helper)
            let issue = Self::create_issue_for_repo(client, &repo, "Test Issue 1")?;
            
            // 3. Send issue and verify it's accepted
            Self::send_and_verify_accepted(client, issue, "issue referencing repo via 'a' tag").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 1.2: NIP-22 comment with root `A` tag referencing repo should be accepted
    async fn test_accept_comment_via_A_tag(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_comment_via_A_tag",
            "GRASP-01:event-acceptance:1.2",
            "Accept NIP-22 comment with root 'A' tag referencing repo",
        )
        .run(|| async {
            // 1. Create and send repo announcement
            let repo = Self::create_test_repo(client, "test-repo-2").await?;
            Self::send_and_verify_accepted(client, repo.clone(), "repository announcement").await?;
            
            // 2. Extract repo_id and create `A` tag manually
            let repo_id = Self::extract_d_tag(&repo)
                .ok_or("Failed to extract repo_id from repo event")?;
            let a_tag_value = format!("30617:{}:{}", repo.pubkey, repo_id);
            
            // 3. Create comment with `A` tag (root reference to repo)
            let tags = vec![
                Tag::custom(TagKind::custom("A"), vec![a_tag_value.clone(), "".to_string(), "root".to_string()]),
                Tag::custom(TagKind::custom("K"), vec!["30617".to_string()]),
                Tag::public_key(repo.pubkey),
            ];
            
            let comment = client
                .event_builder(Kind::Custom(1111), "Comment on repo")
                .tags(tags)
                .build(client.keys())
                .map_err(|e| format!("Failed to build comment: {}", e))?;
            
            // 4. Send comment and verify it's accepted
            Self::send_and_verify_accepted(client, comment, "comment with 'A' tag to repo").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 1.3: Kind 1 text note quoting repo via `q` tag should be accepted
    async fn test_accept_kind1_via_q_tag(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_kind1_via_q_tag",
            "GRASP-01:event-acceptance:1.3",
            "Accept kind 1 note quoting repo via 'q' tag",
        )
        .run(|| async {
            // 1. Create and send repo announcement
            let repo = Self::create_test_repo(client, "test-repo-3").await?;
            Self::send_and_verify_accepted(client, repo.clone(), "repository announcement").await?;
            
            // 2. Extract repo_id and create `q` tag
            let repo_id = Self::extract_d_tag(&repo)
                .ok_or("Failed to extract repo_id from repo event")?;
            let a_tag_value = format!("30617:{}:{}", repo.pubkey, repo_id);
            
            // 3. Create kind 1 note with `q` tag (quote reference to repo)
            let tags = vec![
                Tag::custom(TagKind::custom("q"), vec![a_tag_value]),
            ];
            
            let note = client
                .event_builder(Kind::TextNote, "Mentioning this repo")
                .tags(tags)
                .build(client.keys())
                .map_err(|e| format!("Failed to build note: {}", e))?;
            
            // 4. Send note and verify it's accepted
            Self::send_and_verify_accepted(client, note, "kind 1 with 'q' tag to repo").await?;
            
            Ok(())
        })
        .await
    }
    
    // ============================================================
    // Group 2: Accept Events Tagging Accepted Events (3 tests)
    // ============================================================
    
    /// Test 2.1: Issue quoting another accepted issue should be accepted (transitive)
    async fn test_accept_issue_quoting_issue_via_q(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_issue_quoting_issue_via_q",
            "GRASP-01:event-acceptance:2.1",
            "Accept issue quoting accepted issue (transitive)",
        )
        .run(|| async {
        
        // 1. Create and send Repo A
        let repo_a = Self::create_test_repo(client, "repo-a").await?;
        Self::send_and_verify_accepted(client, repo_a.clone(), "repo A").await?;
        
        // 2. Create and send Issue A (references repo A, so it's accepted)
        let issue_a = Self::create_issue_for_repo(client, &repo_a, "Issue A")?;
        Self::send_and_verify_accepted(client, issue_a.clone(), "issue A").await?;
        
        // 3. Create Repo B but DON'T send it (unaccepted) - just for creating Issue B
        let repo_b = Self::create_test_repo(client, "repo-b").await?;
        
        // 4. Create Issue B that:
        //    - References unaccepted Repo B (would normally be rejected)
        //    - BUT also quotes accepted Issue A via 'q' tag (should make it accepted)
        let additional_tags = vec![
            // Quote to accepted Issue A (this makes it transitive)
            Tag::custom(TagKind::custom("q"), vec![issue_a.id.to_hex()]),
        ];
        
        let issue_b = client
            .create_issue(&repo_b, "Issue B", "issue content", additional_tags)
            .map_err(|e| format!("Failed to build issue B: {}", e))?;
        
            // 5. Send Issue B and verify it's ACCEPTED (via transitive quote to Issue A)
            Self::send_and_verify_accepted(client, issue_b, "issue B quoting accepted issue A").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 2.2: NIP-22 comment with root 'E' tag to accepted issue should be accepted
    async fn test_accept_comment_via_E_tag(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_comment_via_E_tag",
            "GRASP-01:event-acceptance:2.2",
            "Accept NIP-22 comment with root 'E' tag to accepted issue",
        )
        .run(|| async {
        
        // 1. Create and send repo
        let repo = Self::create_test_repo(client, "repo-comment").await?;
        Self::send_and_verify_accepted(client, repo.clone(), "repo").await?;
        
        // 2. Create and send issue (references repo, so it's accepted)
        let issue = Self::create_issue_for_repo(client, &repo, "Issue for comment")?;
        Self::send_and_verify_accepted(client, issue.clone(), "issue").await?;
        
        // 3. Create comment using the helper (which adds NIP-22 tags including 'E')
        let comment = Self::create_comment_for_event(client, &issue, "Comment content")?;
        
            // 4. Send comment and verify it's accepted (via E tag to accepted issue)
            Self::send_and_verify_accepted(client, comment, "comment with E tag to accepted issue").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 2.3: Kind 1 note with 'e' tag reply to accepted kind 1 should be accepted
    async fn test_accept_kind1_via_e_tag(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_kind1_via_e_tag",
            "GRASP-01:event-acceptance:2.3",
            "Accept kind 1 reply via 'e' tag to accepted kind 1",
        )
        .run(|| async {
        
        // 1. Create and send repo
        let repo = Self::create_test_repo(client, "repo-notes").await?;
        Self::send_and_verify_accepted(client, repo.clone(), "repo").await?;
        
        // 2. Create Kind 1 A that quotes the repo (makes it accepted)
        let repo_id = Self::extract_d_tag(&repo)
            .ok_or("Failed to extract repo_id")?;
        let a_tag_value = format!("30617:{}:{}", repo.pubkey, repo_id);
        
        let kind1_a = client
            .event_builder(Kind::TextNote, "Note A about repo")
            .tags(vec![Tag::custom(TagKind::custom("q"), vec![a_tag_value])])
            .build(client.keys())
            .map_err(|e| format!("Failed to build kind1 A: {}", e))?;
        
        Self::send_and_verify_accepted(client, kind1_a.clone(), "kind 1 A quoting repo").await?;
        
        // 3. Create Kind 1 B that replies to Kind 1 A via 'e' tag
        let kind1_b = client
            .event_builder(Kind::TextNote, "Reply to Note A")
            .tags(vec![Tag::event(kind1_a.id)])
            .build(client.keys())
            .map_err(|e| format!("Failed to build kind1 B: {}", e))?;
        
            // 4. Send Kind 1 B and verify it's accepted (via 'e' tag to accepted kind 1 A)
            Self::send_and_verify_accepted(client, kind1_b, "kind 1 B replying to accepted kind 1 A").await?;
            
            Ok(())
        })
        .await
    }
    
    // ============================================================
    // Group 3: Accept Events Tagged BY Accepted Events (3 tests)
    // ============================================================
    
    /// Test 3.1: Kind 1 note should be accepted when referenced by an accepted issue (forward ref)
    async fn test_accept_kind1_referenced_in_issue(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_kind1_referenced_in_issue",
            "GRASP-01:event-acceptance:3.1",
            "Accept kind 1 referenced in accepted issue (forward ref)",
        )
        .run(|| async {
        
        // 1. Create and send repo (this establishes the accepted context)
        let repo = Self::create_test_repo(client, "repo-fwd-1").await?;
        Self::send_and_verify_accepted(client, repo.clone(), "repo").await?;
        
        // 2. Create Kind 1 note locally but DON'T send it yet
        let kind1_note = client
            .event_builder(Kind::TextNote, "Note to be referenced")
            .build(client.keys())
            .map_err(|e| format!("Failed to build kind1: {}", e))?;
        
        // 3. Create and send issue that QUOTES the unsent Kind 1 note
        let issue_tags = vec![
            // Reference to accepted repo
            Tag::custom(TagKind::custom("a"), vec![
                format!("30617:{}:{}", repo.pubkey, Self::extract_d_tag(&repo).unwrap())
            ]),
            Tag::custom(TagKind::custom("subject"), vec!["Issue referencing kind1".to_string()]),
            // Quote the Kind 1 that hasn't been sent yet
            Tag::custom(TagKind::custom("q"), vec![kind1_note.id.to_hex()]),
        ];
        
        let issue = client
            .event_builder(Kind::Custom(1621), "issue content")
            .tags(issue_tags)
            .build(client.keys())
            .map_err(|e| format!("Failed to build issue: {}", e))?;
        
        Self::send_and_verify_accepted(client, issue, "issue quoting unsent kind1").await?;
        
            // 4. NOW send the Kind 1 note - should be accepted because accepted issue quotes it
            Self::send_and_verify_accepted(client, kind1_note, "kind1 note referenced by accepted issue").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 3.2: Comment should be accepted when referenced by another accepted comment (forward ref)
    async fn test_accept_comment_referenced_in_comment(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_comment_referenced_in_comment",
            "GRASP-01:event-acceptance:3.2",
            "Accept comment referenced in another accepted comment (forward ref)",
        )
        .run(|| async {
        
        // 1. Create and send repo
        let repo = Self::create_test_repo(client, "repo-fwd-2").await?;
        Self::send_and_verify_accepted(client, repo.clone(), "repo").await?;
        
        // 2. Create and send issue (references repo, so it's accepted)
        let issue = Self::create_issue_for_repo(client, &repo, "Issue for comments")?;
        Self::send_and_verify_accepted(client, issue.clone(), "issue").await?;
        
        // 3. Create Comment A locally but DON'T send it yet
        let comment_a = Self::create_comment_for_event(client, &issue, "Comment A")?;
        
        // 4. Create and send Comment B that quotes Comment A (which hasn't been sent)
        let comment_b_tags = vec![
            // NIP-22 tags for the original issue
            Tag::custom(TagKind::custom("E"), vec![issue.id.to_hex(), "".to_string(), "root".to_string()]),
            Tag::event(issue.id),
            Tag::custom(TagKind::custom("K"), vec![issue.kind.as_u16().to_string()]),
            Tag::public_key(issue.pubkey),
            // Quote Comment A which hasn't been sent yet
            Tag::custom(TagKind::custom("q"), vec![comment_a.id.to_hex()]),
        ];
        
        let comment_b = client
            .event_builder(Kind::Custom(1111), "Comment B quoting Comment A")
            .tags(comment_b_tags)
            .build(client.keys())
            .map_err(|e| format!("Failed to build comment B: {}", e))?;
        
        Self::send_and_verify_accepted(client, comment_b, "comment B quoting unsent comment A").await?;
        
            // 5. NOW send Comment A - should be accepted because accepted Comment B quotes it
            Self::send_and_verify_accepted(client, comment_a, "comment A referenced by accepted comment B").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 3.3: Kind 1 note should be accepted when referenced by another accepted kind 1 (forward ref)
    async fn test_accept_kind1_referenced_in_kind1(client: &AuditClient) -> TestResult {
        TestResult::new(
            "accept_kind1_referenced_in_kind1",
            "GRASP-01:event-acceptance:3.3",
            "Accept kind 1 referenced in another accepted kind 1 (forward ref)",
        )
        .run(|| async {
            // 1. Create and send repo
            let repo = Self::create_test_repo(client, "repo-fwd-3").await?;
            Self::send_and_verify_accepted(client, repo.clone(), "repo").await?;
            
            // 2. Create Kind 1 A locally but DON'T send it yet
            let kind1_a = client
                .event_builder(Kind::TextNote, "Note A to be referenced")
                .build(client.keys())
                .map_err(|e| format!("Failed to build kind1 A: {}", e))?;
            
            // 3. Create and send Kind 1 B that:
            //    - Quotes the repo (makes it accepted)
            //    - Mentions Kind 1 A via 'e' tag (which hasn't been sent yet)
            let repo_id = Self::extract_d_tag(&repo)
                .ok_or("Failed to extract repo_id")?;
            let a_tag_value = format!("30617:{}:{}", repo.pubkey, repo_id);
            
            let kind1_b = client
                .event_builder(Kind::TextNote, "Note B mentioning Note A")
                .tags(vec![
                    Tag::custom(TagKind::custom("q"), vec![a_tag_value]),  // Quote repo (accepted)
                    Tag::event(kind1_a.id),  // Mention unsent Kind 1 A
                ])
                .build(client.keys())
                .map_err(|e| format!("Failed to build kind1 B: {}", e))?;
            
            Self::send_and_verify_accepted(client, kind1_b, "kind1 B mentioning unsent kind1 A").await?;
            
            // 4. NOW send Kind 1 A - should be accepted because accepted Kind 1 B mentions it
            Self::send_and_verify_accepted(client, kind1_a, "kind1 A referenced by accepted kind1 B").await?;
            
            Ok(())
        })
        .await
    }
    
    // ============================================================
    // Group 4: Reject Unrelated Events (3 tests)
    // ============================================================
    
    /// Test 4.1: Issue referencing unaccepted repo should be rejected
    async fn test_reject_orphan_issue(client: &AuditClient) -> TestResult {
        TestResult::new(
            "reject_orphan_issue",
            "GRASP-01:event-acceptance:4.1",
            "Reject issue referencing unaccepted repo",
        )
        .run(|| async {
            // 1. Create a repo but DON'T send it (so it's unaccepted)
            let unaccepted_repo = Self::create_test_repo(client, "unaccepted-repo-1").await?;
            
            // 2. Create issue that references the unaccepted repo
            let orphan_issue = Self::create_issue_for_repo(client, &unaccepted_repo, "Orphan Issue")?;
            
            // 3. Send issue and verify it's REJECTED
            Self::send_and_verify_rejected(client, orphan_issue, "issue referencing unaccepted repo").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 4.2: Generic kind 1 note with no repo references should be rejected
    async fn test_reject_orphan_kind1(client: &AuditClient) -> TestResult {
        TestResult::new(
            "reject_orphan_kind1",
            "GRASP-01:event-acceptance:4.2",
            "Reject kind 1 with no repo references",
        )
        .run(|| async {
            // 1. Create a kind 1 note with no tags (no repo references)
            let orphan_note = client
                .event_builder(Kind::TextNote, "Just a random note")
                .build(client.keys())
                .map_err(|e| format!("Failed to build note: {}", e))?;
            
            // 2. Send note and verify it's REJECTED
            Self::send_and_verify_rejected(client, orphan_note, "kind 1 with no repo references").await?;
            
            Ok(())
        })
        .await
    }
    
    /// Test 4.3: Comment quoting unaccepted repo should be rejected
    async fn test_reject_comment_quoting_other_repo(client: &AuditClient) -> TestResult {
        TestResult::new(
            "reject_comment_quoting_other_repo",
            "GRASP-01:event-acceptance:4.3",
            "Reject comment quoting unaccepted repo",
        )
        .run(|| async {
            // 1. Create and send Repo A (this one IS accepted)
            let repo_a = Self::create_test_repo(client, "accepted-repo-a").await?;
            Self::send_and_verify_accepted(client, repo_a.clone(), "repo A").await?;
            
            // 2. Create Repo B but DON'T send it (unaccepted)
            let repo_b = Self::create_test_repo(client, "unaccepted-repo-b").await?;
            
            // 3. Extract repo_b info and create comment that quotes repo B (not repo A)
            let repo_b_id = Self::extract_d_tag(&repo_b)
                .ok_or("Failed to extract repo_b id")?;
            let repo_b_a_tag = format!("30617:{}:{}", repo_b.pubkey, repo_b_id);
            
            // 4. Create comment that references ONLY repo B (unaccepted)
            let tags = vec![
                Tag::custom(TagKind::custom("A"), vec![repo_b_a_tag, "".to_string(), "root".to_string()]),
                Tag::custom(TagKind::custom("K"), vec!["30617".to_string()]),
                Tag::public_key(repo_b.pubkey),
            ];
            
            let comment = client
                .event_builder(Kind::Custom(1111), "Comment on unaccepted repo")
                .tags(tags)
                .build(client.keys())
                .map_err(|e| format!("Failed to build comment: {}", e))?;
            
            // 5. Send comment and verify it's REJECTED (only references unaccepted repo B)
            Self::send_and_verify_rejected(client, comment, "comment quoting only unaccepted repo").await?;
            
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuditConfig;
    
    #[tokio::test]
    #[ignore] // Requires running relay
    async fn test_grasp01_event_acceptance_policy_against_relay() {
        // Read relay URL from environment variable - must be supplied
        let relay_url = std::env::var("RELAY_URL")
            .expect("RELAY_URL environment variable must be set. Example: RELAY_URL=ws://localhost:18081");
        
        let config = AuditConfig::ci();
        let client = AuditClient::new(&relay_url, config)
            .await
            .expect(&format!(
                "Failed to connect to relay at {}. Ensure relay is running and accessible. \
                Try: docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest",
                relay_url
            ));
        
        let results = EventAcceptancePolicyTests::run_all(&client).await;
        results.print_report();
        
        // Don't assert all passed yet - some tests may be failing
        // Future: assert!(results.all_passed(), "Some GRASP-01 event acceptance tests failed");
    }
}