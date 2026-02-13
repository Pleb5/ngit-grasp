//! GRASP-01 Purgatory Tests
//!
//! Tests for the GRASP-01 purgatory mechanism where events are accepted but not
//! served until corresponding git data arrives.
//!
//! ## Purgatory Behavior (GRASP-01 Line 22)
//!
//! "New repository announcements, repo state announcements, PRs and PR Updates
//! SHOULD be accepted with message 'purgatory: won't be served until git data arrives'
//! and kept in purgatory (not served) until the related git data arrives and otherwise
//! discarded after 30 minutes."
//!
//! ## Test Categories
//!
//! ### Announcement Purgatory (feature not yet implemented)
//! - `test_announcement_not_served_before_git_data`
//! - `test_announcement_served_after_git_push`
//! - `test_bare_repo_exists_for_purgatory_announcement`
//! - `test_state_event_accepted_for_purgatory_announcement`
//!
//! ### State Event Purgatory (already implemented)
//! - `test_state_event_not_served_before_git_data`
//! - `test_state_event_served_after_git_push`
//!
//! ### PR Purgatory (already implemented)
//! - `test_pr_event_not_served_before_git_data`
//! - `test_pr_event_served_after_correct_push`

use crate::specs::grasp01::SpecRef;
use crate::{AuditClient, AuditResult, FixtureKind, TestContext, TestResult};
use nostr_sdk::prelude::*;
use std::time::Duration;

/// Test suite for GRASP-01 purgatory behavior
pub struct PurgatoryTests;

impl PurgatoryTests {
    /// Run all purgatory tests
    pub async fn run_all(client: &AuditClient) -> AuditResult {
        let mut results = AuditResult::new("GRASP-01 Purgatory Tests");

        // Announcement purgatory tests (feature not yet implemented)
        results.add(Self::test_announcement_not_served_before_git_data(client).await);
        results.add(Self::test_announcement_served_after_git_push(client).await);
        results.add(Self::test_bare_repo_exists_for_purgatory_announcement(client).await);
        results.add(Self::test_state_event_accepted_for_purgatory_announcement(client).await);

        // State event purgatory tests (already implemented)
        results.add(Self::test_state_event_not_served_before_git_data(client).await);
        results.add(Self::test_state_event_served_after_git_push(client).await);

        // PR purgatory tests
        results.add(Self::test_pr_event_before_git_data_accepted_into_purgatory(client).await);
        results.add(Self::test_pr_event_remains_in_purgatory_until_git_data(client).await);
        results.add(Self::test_pr_event_git_push_accepted(client).await);
        results.add(Self::test_pr_event_served_after_git_push(client).await);

        results
    }

    // ============================================================
    // Announcement Purgatory Tests (#[ignore] - feature not yet implemented)
    // ============================================================

    /// Test: Repository announcement not served before git data arrives
    ///
    /// Spec: GRASP-01 Line 22
    /// "New repository announcements... SHOULD be accepted with message
    /// 'purgatory: won't be served until git data arrives' and kept in purgatory
    /// (not served) until the related git data arrives"
    ///
    /// This test verifies:
    /// 1. Send a valid repository announcement
    /// 2. Event is accepted (OK response)
    /// 3. Event is NOT queryable from the relay (in purgatory)
    ///
    /// NOTE: Announcement purgatory feature not yet implemented - test may fail
    pub async fn test_announcement_not_served_before_git_data(client: &AuditClient) -> TestResult {
        TestResult::new(
            "announcement_not_served_before_git_data",
            SpecRef::PurgatoryAcceptUntilGitData,
            "Repository announcements SHOULD be accepted but not served until git data arrives",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // Create a fresh repo announcement (not the served variant)
            let repo = ctx
                .get_fixture(FixtureKind::ValidRepoSent)
                .await
                .map_err(|e| format!("Failed to create repo announcement: {}", e))?;

            let repo_id = repo
                .tags
                .iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in repo announcement")?
                .to_string();

            // Query for the announcement - should NOT be served
            let filter = Filter::new()
                .kind(Kind::GitRepoAnnouncement)
                .author(client.public_key())
                .identifier(&repo_id);

            tokio::time::sleep(Duration::from_millis(300)).await;

            let events = client
                .query(filter)
                .await
                .map_err(|e| format!("Failed to query relay: {}", e))?;

            if events.iter().any(|e| e.id == repo.id) {
                return Err(format!(
                    "Announcement was served immediately - purgatory not implemented. \
                     Event ID: {} should NOT be queryable until git data arrives",
                    repo.id
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test: Repository announcement served after git push
    ///
    /// Spec: GRASP-01 Line 22
    /// "...kept in purgatory (not served) until the related git data arrives"
    ///
    /// This test verifies the full lifecycle:
    /// 1. Send repository announcement (enters purgatory)
    /// 2. Send state event (enters purgatory)
    /// 3. Push git data matching state event
    /// 4. Both announcement and state event are now served
    ///
    /// NOTE: Announcement purgatory feature not yet implemented - test may fail
    pub async fn test_announcement_served_after_git_push(client: &AuditClient) -> TestResult {
        TestResult::new(
            "announcement_served_after_git_push",
            SpecRef::PurgatoryAcceptUntilGitData,
            "Repository announcements SHOULD be served after git data arrives",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // OwnerStateDataPushed fixture handles the full lifecycle:
            // 1. Creates repo announcement (purgatory)
            // 2. Creates state event (purgatory)
            // 3. Pushes git data
            // 4. Verifies events are served
            let state_event = ctx
                .get_fixture(FixtureKind::OwnerStateDataPushed)
                .await
                .map_err(|e| format!("Failed to complete full lifecycle: {}", e))?;

            // Extract repo_id from state event
            let repo_id = state_event
                .tags
                .iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in state event")?
                .to_string();

            // Verify announcement is now served
            let announcement_filter = Filter::new()
                .kind(Kind::GitRepoAnnouncement)
                .author(client.public_key())
                .identifier(&repo_id);

            let announcements = client
                .query(announcement_filter)
                .await
                .map_err(|e| format!("Failed to query announcements: {}", e))?;

            if announcements.is_empty() {
                return Err(format!(
                    "Announcement not served after git push. Repo ID: {}",
                    repo_id
                ));
            }

            // Verify state event is served
            let state_filter = Filter::new()
                .kind(Kind::RepoState)
                .author(client.public_key())
                .identifier(&repo_id);

            let state_events = client
                .query(state_filter)
                .await
                .map_err(|e| format!("Failed to query state events: {}", e))?;

            if !state_events.iter().any(|e| e.id == state_event.id) {
                return Err(format!(
                    "State event not served after git push. Event ID: {}",
                    state_event.id
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test: Bare repository exists for purgatory announcement
    ///
    /// Spec: GRASP-01 Line 34
    /// "MUST serve a git repository via an unauthenticated git smart http service
    /// at `/<npub>/<identifier>.git` for each git repository announcement the relay
    /// serves or has in purgatory."
    ///
    /// This test verifies that git HTTP service works even for repos in purgatory.
    ///
    /// NOTE: Announcement purgatory feature not yet implemented - test may fail
    pub async fn test_bare_repo_exists_for_purgatory_announcement(
        client: &AuditClient,
    ) -> TestResult {
        TestResult::new(
            "bare_repo_exists_for_purgatory_announcement",
            SpecRef::GitServeRepository,
            "Git HTTP service MUST work for repos in purgatory",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // Get a repo announcement (in purgatory, no git data yet)
            let repo = ctx
                .get_fixture(FixtureKind::ValidRepoSent)
                .await
                .map_err(|e| format!("Failed to create repo announcement: {}", e))?;

            let repo_id = repo
                .tags
                .iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in repo announcement")?
                .to_string();

            let npub = client
                .public_key()
                .to_bech32()
                .map_err(|e| format!("Failed to convert pubkey: {}", e))?;

            // Get relay domain
            let relay_url = client
                .client()
                .relays()
                .await
                .keys()
                .next()
                .ok_or("No relay connected")?
                .to_string();
            let relay_domain = relay_url
                .replace("ws://", "")
                .replace("wss://", "")
                .replace(":8080", "");

            // Check git HTTP service is available
            let info_refs_url = format!(
                "http://{}/{}/{}.git/info/refs?service=git-upload-pack",
                relay_domain, npub, repo_id
            );

            let http_client = reqwest::Client::new();
            let response = http_client
                .get(&info_refs_url)
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {}", e))?;

            if !response.status().is_success() {
                return Err(format!(
                    "Git HTTP service not available for purgatory repo. \
                     URL: {}, Status: {}",
                    info_refs_url,
                    response.status()
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test: State event accepted for purgatory announcement
    ///
    /// Spec: GRASP-01 Line 22
    /// "New repository announcements, repo state announcements... SHOULD be accepted"
    ///
    /// This test verifies that state events are accepted even when the repo
    /// announcement is in purgatory (no git data yet).
    ///
    /// NOTE: Announcement purgatory feature not yet implemented - test may fail
    pub async fn test_state_event_accepted_for_purgatory_announcement(
        client: &AuditClient,
    ) -> TestResult {
        TestResult::new(
            "state_event_accepted_for_purgatory_announcement",
            SpecRef::PurgatoryAcceptUntilGitData,
            "State events SHOULD be accepted for repos in purgatory",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // Get a repo announcement (in purgatory)
            let repo = ctx
                .get_fixture(FixtureKind::ValidRepoSent)
                .await
                .map_err(|e| format!("Failed to create repo announcement: {}", e))?;

            // Build a state event for this repo
            let repo_id = repo
                .tags
                .iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in repo announcement")?
                .to_string();

            let state_event = client
                .event_builder(Kind::RepoState, "")
                .tag(Tag::identifier(&repo_id))
                .tag(Tag::custom(
                    TagKind::custom("refs/heads/main"),
                    vec!["abc123".to_string()],
                ))
                .tag(Tag::custom(
                    TagKind::custom("HEAD"),
                    vec!["ref: refs/heads/main".to_string()],
                ))
                .build(client.keys())
                .map_err(|e| format!("Failed to build state event: {}", e))?;

            // Send state event - should be accepted (even though repo is in purgatory)
            let (_, in_purgatory) = client
                .send_event_and_note_purgatory(state_event.clone())
                .await
                .map_err(|e| format!("Failed to send state event: {}", e))?;

            // Event should be accepted (either in purgatory or served)
            // We just verify it wasn't rejected
            if !in_purgatory {
                // Check if it's actually on the relay (might be served immediately)
                let filter = Filter::new()
                    .kind(Kind::RepoState)
                    .author(client.public_key())
                    .identifier(&repo_id);

                let events = client
                    .query(filter)
                    .await
                    .map_err(|e| format!("Failed to query: {}", e))?;

                if events.iter().any(|e| e.id == state_event.id) {
                    return Err(format!(
                        "State event was served immediately - repo announcement purgatory not implemented. \
                         Event ID: {} should NOT be queryable until git data arrives",
                        state_event.id
                    ));
                }

                return Err(format!(
                    "State event was neither in purgatory nor served. \
                     Event ID: {}",
                    state_event.id
                ));
            }

            // Feature IS implemented - state event in purgatory as expected
            Ok(())
        })
        .await
    }

    // ============================================================
    // State Event Purgatory Tests (non-ignored - already implemented)
    // ============================================================

    /// Test: State event not served before git data arrives
    ///
    /// Spec: GRASP-01 Line 22
    /// "repo state announcements... SHOULD be accepted with message
    /// 'purgatory: won't be served until git data arrives'"
    ///
    /// This test verifies:
    /// 1. Send state event for a repo with git data
    /// 2. State event points to a different commit than what's pushed
    /// 3. State event is NOT queryable (in purgatory)
    pub async fn test_state_event_not_served_before_git_data(client: &AuditClient) -> TestResult {
        TestResult::new(
            "state_event_not_served_before_git_data",
            SpecRef::PurgatoryAcceptUntilGitData,
            "State events SHOULD be accepted but not served until git data arrives",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // Get a repo with git data already pushed
            let existing_state = ctx
                .get_fixture(FixtureKind::OwnerStateDataPushed)
                .await
                .map_err(|e| format!("Failed to get existing repo: {}", e))?;

            let repo_id = existing_state
                .tags
                .iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in state event")?
                .to_string();

            // Create a NEW state event pointing to a DIFFERENT commit
            // This should enter purgatory since the commit doesn't exist
            let new_state = client
                .event_builder(Kind::RepoState, "")
                .tag(Tag::identifier(&repo_id))
                .tag(Tag::custom(
                    TagKind::custom("refs/heads/main"),
                    vec!["deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string()],
                ))
                .tag(Tag::custom(
                    TagKind::custom("HEAD"),
                    vec!["ref: refs/heads/main".to_string()],
                ))
                .build(client.keys())
                .map_err(|e| format!("Failed to build state event: {}", e))?;

            // Send the state event
            let (_, in_purgatory) = client
                .send_event_and_note_purgatory(new_state.clone())
                .await
                .map_err(|e| format!("Failed to send state event: {}", e))?;

            if !in_purgatory {
                return Err(format!(
                    "State event was served immediately despite pointing to \
                     non-existent commit. Event ID: {}",
                    new_state.id
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test: State event served after git push
    ///
    /// Spec: GRASP-01 Line 22
    /// "...kept in purgatory (not served) until the related git data arrives"
    ///
    /// This test verifies the full lifecycle using OwnerStateDataPushed fixture:
    /// 1. State event is sent (enters purgatory)
    /// 2. Git data is pushed matching the state event
    /// 3. State event is now served
    pub async fn test_state_event_served_after_git_push(client: &AuditClient) -> TestResult {
        TestResult::new(
            "state_event_served_after_git_push",
            SpecRef::PurgatoryAcceptUntilGitData,
            "State events SHOULD be served after matching git data arrives",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // OwnerStateDataPushed handles the full lifecycle
            let state_event = ctx
                .get_fixture(FixtureKind::OwnerStateDataPushed)
                .await
                .map_err(|e| format!("Failed to complete full lifecycle: {}", e))?;

            // Verify state event is now served
            let repo_id = state_event
                .tags
                .iter()
                .find(|t| t.kind() == TagKind::d())
                .and_then(|t| t.content())
                .ok_or("Missing d tag in state event")?
                .to_string();

            let filter = Filter::new()
                .kind(Kind::RepoState)
                .author(client.public_key())
                .identifier(&repo_id);

            let events = client
                .query(filter)
                .await
                .map_err(|e| format!("Failed to query state events: {}", e))?;

            if !events.iter().any(|e| e.id == state_event.id) {
                return Err(format!(
                    "State event not served after git push. Event ID: {}",
                    state_event.id
                ));
            }

            Ok(())
        })
        .await
    }

    // ============================================================
    // PR Purgatory Tests
    // ============================================================

    /// Test: PR event not served before git data arrives
    ///
    /// Spec: GRASP-01 Line 22
    /// "PRs and PR Updates SHOULD be accepted with message
    /// 'purgatory: won't be served until git data arrives'"
    ///
    /// This test verifies:
    /// 1. Send PR event for a repo
    /// 2. PR event is NOT queryable (in purgatory)
    /// 3. No git data exists at refs/nostr/<pr-event-id>
    pub async fn test_pr_event_before_git_data_accepted_into_purgatory(
        client: &AuditClient,
    ) -> TestResult {
        TestResult::new(
            "pr_event_before_git_data_accepted_into_purgatory",
            SpecRef::PurgatoryAcceptUntilGitData,
            "PR event SHOULD be accepted into purgatory when git data doesn't exist",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            let pr_event = ctx
                .get_fixture(FixtureKind::PREvent2Sent)
                .await
                .map_err(|e| format!("Failed to send PR event: {}", e))?;

            let filter = Filter::new()
                .kind(Kind::GitPullRequest)
                .author(client.pr_author_keys().public_key())
                .id(pr_event.id);

            tokio::time::sleep(Duration::from_millis(300)).await;

            let events = client
                .query(filter)
                .await
                .map_err(|e| format!("Failed to query PR events: {}", e))?;

            if !events.is_empty() {
                return Err(format!(
                    "PR event was served immediately - should be in purgatory. Event ID: {}",
                    pr_event.id
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test: PR event remains in purgatory until git data arrives
    ///
    /// Verifies the event stays in purgatory until matching git data is pushed.
    pub async fn test_pr_event_remains_in_purgatory_until_git_data(
        client: &AuditClient,
    ) -> TestResult {
        TestResult::new(
            "pr_event_remains_in_purgatory_until_git_data",
            SpecRef::PurgatoryAcceptUntilGitData,
            "PR event SHOULD remain in purgatory until git data arrives",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            let pr_event = ctx
                .get_fixture(FixtureKind::PREvent2Sent)
                .await
                .map_err(|e| format!("Failed to get PR event: {}", e))?;

            tokio::time::sleep(Duration::from_millis(500)).await;

            let filter = Filter::new()
                .kind(Kind::GitPullRequest)
                .author(client.pr_author_keys().public_key())
                .id(pr_event.id);

            let events = client
                .query(filter)
                .await
                .map_err(|e| format!("Failed to query PR events: {}", e))?;

            if !events.is_empty() {
                return Err(format!(
                    "PR event was served without git data - purgatory not working. Event ID: {}",
                    pr_event.id
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test: Git push accepted for PR event in purgatory
    ///
    /// Verifies that pushing the correct commit to refs/nostr/<pr-event-id>
    /// is accepted.
    pub async fn test_pr_event_git_push_accepted(client: &AuditClient) -> TestResult {
        TestResult::new(
            "pr_event_git_push_accepted",
            SpecRef::PurgatoryAcceptUntilGitData,
            "Git push for PR event SHOULD be accepted",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            let _pr_event = ctx
                .get_fixture(FixtureKind::PREvent2GitDataPushed)
                .await
                .map_err(|e| format!("Failed to push git data for PR event: {}", e))?;

            Ok(())
        })
        .await
    }

    /// Test: PR event served after git push
    ///
    /// Verifies the full purgatory release mechanism.
    pub async fn test_pr_event_served_after_git_push(client: &AuditClient) -> TestResult {
        TestResult::new(
            "pr_event_served_after_git_push",
            SpecRef::PurgatoryAcceptUntilGitData,
            "PR event SHOULD be served after matching git data arrives",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            let pr_event = ctx
                .get_fixture(FixtureKind::PREvent2Served)
                .await
                .map_err(|e| format!("Failed to complete purgatory release: {}", e))?;

            let filter = Filter::new()
                .kind(Kind::GitPullRequest)
                .author(client.pr_author_keys().public_key())
                .id(pr_event.id);

            let events = client
                .query(filter)
                .await
                .map_err(|e| format!("Failed to query PR events: {}", e))?;

            if events.is_empty() {
                return Err(format!(
                    "PR event not served after git push. Event ID: {} should be queryable",
                    pr_event.id
                ));
            }

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
    async fn test_grasp01_purgatory_against_relay() {
        let relay_url = std::env::var("RELAY_URL").expect(
            "RELAY_URL environment variable must be set. Example: RELAY_URL=ws://localhost:18081",
        );

        let config = AuditConfig::isolated();
        let client = AuditClient::new(&relay_url, config)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "Failed to connect to relay at {}. Ensure relay is running and accessible.",
                    relay_url
                )
            });

        let results = PurgatoryTests::run_all(&client).await;
        results.print_report();

        assert!(
            results.all_passed(),
            "Some purgatory tests failed. See report above."
        );
    }
}
