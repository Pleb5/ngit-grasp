//! GRASP-01 Push Authorization Tests
//!
//! Tests that verify push authorization works correctly according to GRASP-01:
//! "MUST accept pushes via this service that match the latest repo state announcement
//! on the relay, respecting the recursive maintainer set."
//!
//! ## Test Coverage
//!
//! - Push authorized when state event matches commit being pushed
//! - Push rejected when no state event exists
//! - Push rejected when state event has different commit
//!
//! ## Running Tests
//!
//! ```bash
//! cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test
//! ```

use crate::{
    clone_repo, create_commit, setup_repo_for_maintainer, setup_repo_for_recursive_maintainer,
    setup_repo_with_deterministic_commit, try_push, AuditClient, FixtureKind, TestContext,
    TestResult,
};
use nostr_sdk::prelude::*;
use std::fs;
use std::path::Path;

/// Test suite for Push Authorization operations
pub struct PushAuthorizationTests;

impl PushAuthorizationTests {
    /// Test that push is authorized when state event matches the commit
    ///
    /// GRASP-01: "MUST accept pushes via this service that match the latest
    /// repo state announcement on the relay"
    pub async fn test_push_authorized_by_owner_state(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_authorized_by_owner_state";

        // this setup is exactly what we are testing
        match setup_repo_with_deterministic_commit(client, git_data_dir, relay_domain).await {
            Ok(_) => {
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state").pass()
            },
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Failed: {}", e))
            }
        };
    }

    /// Test that push is rejected when no state event exists
    pub async fn test_push_rejected_without_state_event(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_rejected_without_state_event";
        let ctx = TestContext::new(client);

        // Create repository (no state event)
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected without state event")
                    .fail(&format!("Failed to create repo: {}", e))
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let repo_id = repo.tags.iter().find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content()).unwrap().to_string();
        let npub = repo.pubkey.to_bech32().unwrap();

        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
        if !repo_path.exists() {
            return TestResult::new(test_name, "GRASP-01", "Push rejected without state event")
                .fail(&format!("Repo not found: {}", repo_path.display()));
        }

        // Clone and create commit
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => return TestResult::new(test_name, "GRASP-01", "Push rejected without state event").fail(&e),
        };
        let cleanup = || { let _ = fs::remove_dir_all(&clone_path); };

        if let Err(e) = create_commit(&clone_path, "Unauthorized commit") {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Push rejected without state event").fail(&e);
        }

        // Do NOT publish state event - push should be rejected
        let push_result = try_push(&clone_path);
        cleanup();

        match push_result {
            Ok(false) => TestResult::new(test_name, "GRASP-01", "Push rejected without state event").pass(),
            Ok(true) => TestResult::new(test_name, "GRASP-01", "Push rejected without state event").fail("Push accepted but should be rejected"),
            Err(e) => TestResult::new(test_name, "GRASP-01", "Push rejected without state event").fail(&e),
        }
    }

    /// Test that push is rejected when commit doesn't match state event
    ///
    /// This test verifies that the relay enforces state event authorization.
    /// The state event (from fixture) points to the deterministic commit which is
    /// already on the server. We create a new commit locally and try to push it.
    /// The push should be rejected because the new commit doesn't match what the
    /// state event announces.
    pub async fn test_push_rejected_wrong_commit(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_rejected_wrong_commit";

        // Set up repository with deterministic commit
        // This creates a state event pointing to DETERMINISTIC_COMMIT_HASH and pushes that commit
        let setup = match setup_repo_with_deterministic_commit(client, git_data_dir, relay_domain).await {
            Ok(s) => s,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Setup failed: {}", e))
            }
        };

        // Create a new commit locally - this is NOT announced in any state event
        let new_commit = match create_commit(&setup.clone_path, "Unauthorized commit") {
            Ok(h) => h,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to create commit: {}", e))
            }
        };

        // Try to push the new commit
        // This should be REJECTED because:
        // - The state event still points to the deterministic commit (setup.commit_hash)
        // - We're trying to push new_commit which is different
        // - The relay MUST reject pushes that don't match the announced state
        let push_result = try_push(&setup.clone_path);

        match push_result {
            Ok(false) => TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event").pass(),
            Ok(true) => TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                .fail(&format!(
                    "Push accepted but should be rejected. State event points to {}, but pushed {}",
                    setup.commit_hash, new_commit
                )),
            Err(e) => TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event").fail(&e),
        }
    }

    /// Test push authorized by maintainer state event only (no announcement)
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    /// This tests that a maintainer can authorize pushes with ONLY a state event,
    /// without publishing their own repo announcement. The maintainer is still
    /// listed in the owner's announcement, so they're a valid maintainer.
    ///
    /// Scenario:
    /// 1. Owner's repo announcement lists maintainer in maintainers tag
    /// 2. Maintainer publishes ONLY a state event (no announcement)
    /// 3. setup_repo_for_maintainer() clones, creates maintainer commit, verifies hash, pushes
    /// 4. The push should be ACCEPTED because maintainer's state event authorizes it
    pub async fn test_push_authorized_by_maintainer_state_only(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_authorized_by_maintainer_state_only";

        // Use setup_repo_for_maintainer which publishes ONLY the state event, no announcement
        match setup_repo_for_maintainer(client, git_data_dir, relay_domain).await {
            Ok(_setup) => {
                // Push succeeded in setup - this means the relay accepted the push
                // authorized by the maintainer's state event alone
                TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .pass()
            }
            Err(e) => {
                // Check if this was specifically a push rejection
                if e.contains("Failed to push") {
                    TestResult::new(
                        test_name,
                        "GRASP-01",
                        "Push authorized by maintainer state event only (no announcement)",
                    )
                    .fail(&format!(
                        "Push was rejected but should have been accepted. \
                        The maintainer published a state event with a commit hash, \
                        and even without a separate announcement, the relay should \
                        authorize pushes matching this state event since the maintainer \
                        is listed in the owner's announcement. \
                        Error: {}",
                        e
                    ))
                } else {
                    // Some other error during setup
                    TestResult::new(
                        test_name,
                        "GRASP-01",
                        "Push authorized by maintainer state event only (no announcement)",
                    )
                    .fail(&format!("Setup failed: {}", e))
                }
            }
        }
    }

    /// Test push authorized by recursive maintainer state event
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    /// This tests recursive maintainer chains: Owner -> MaintainerA -> MaintainerB
    ///
    /// Scenario:
    /// 1. RecursiveMaintainerRepoAndState fixture creates:
    ///    - Repo announcement signed by recursive_maintainer keys
    ///    - Lists main pubkey and maintainer pubkey in maintainers tag
    ///    - State event with RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH (2s in past)
    /// 2. setup_repo_for_recursive_maintainer() clones, creates recursive maintainer commit, verifies hash, pushes
    /// 3. The push should be ACCEPTED because recursive maintainer's state event authorizes it
    pub async fn test_push_authorized_by_recursive_maintainer_state(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_authorized_by_recursive_maintainer_state";

        // Use setup_repo_for_recursive_maintainer which leverages RecursiveMaintainerRepoAndState fixture
        // This does all the heavy lifting:
        // 1. Creates repo announcement signed by recursive maintainer keys
        // 2. Creates state event pointing to RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
        // 3. Clones the repo
        // 4. Creates the recursive maintainer deterministic commit locally
        // 5. Verifies commit hash matches expected
        // 6. Creates main branch, checks it out, and pushes
        match setup_repo_for_recursive_maintainer(client, git_data_dir, relay_domain).await {
            Ok(_setup) => {
                // Push succeeded in setup - this means the relay accepted the push
                // authorized by the recursive maintainer's state event
                TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .pass()
            }
            Err(e) => {
                // Check if this was specifically a push rejection
                if e.contains("Failed to push") {
                    TestResult::new(
                        test_name,
                        "GRASP-01",
                        "Push authorized by recursive maintainer state event",
                    )
                    .fail(&format!(
                        "Push was rejected but should have been accepted. \
                        The recursive maintainer published a state event with a commit hash, \
                        and the relay should authorize pushes matching this state event \
                        through recursive maintainer traversal. \
                        Error: {}",
                        e
                    ))
                } else {
                    // Some other error during setup
                    TestResult::new(
                        test_name,
                        "GRASP-01",
                        "Push authorized by recursive maintainer state event",
                    )
                    .fail(&format!("Setup failed: {}", e))
                }
            }
        }
    }

    /// Test that non-maintainer state event is ignored
    ///
    /// This test verifies that the relay ignores state events from non-maintainers.
    /// We set up a valid repo, then create a rogue state event signed by a different
    /// keypair (not the repo maintainer) that announces a different commit. The push
    /// should be rejected because the rogue state event is not authorized.
    pub async fn test_non_maintainer_state_rejected(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_non_maintainer_state_rejected";

        // Set up repository with deterministic commit (signed by maintainer)
        let setup = match setup_repo_with_deterministic_commit(client, git_data_dir, relay_domain).await {
            Ok(s) => s,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Setup failed: {}", e))
            }
        };

        // Create a new commit locally that we want to push
        let new_commit = match create_commit(&setup.clone_path, "New commit to push") {
            Ok(h) => h,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to create commit: {}", e))
            }
        };

        // Create a rogue keypair (NOT the maintainer)
        let rogue_keys = Keys::generate();
        
        // Create a rogue state event announcing the new commit
        // This event has the correct repo_id but is signed by a non-maintainer
        let rogue_state = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&setup.repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![new_commit.clone()],
            ))
            .build(&rogue_keys)
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to build rogue state event: {}", e))
            }
        };

        // Send the rogue state event using the raw client to bypass AuditClient's key check
        if let Err(e) = client.client().send_event(&rogue_state).await {
            return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                .fail(&format!("Failed to send rogue state event: {}", e));
        }

        // Wait for event to propagate
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Try to push the new commit
        // This should be REJECTED because:
        // - The rogue state event announces new_commit
        // - But the rogue state event is NOT signed by the maintainer
        // - The relay should ignore the rogue state event
        // - The valid state event (from setup) still points to the deterministic commit
        // - Therefore pushing new_commit should fail
        let push_result = try_push(&setup.clone_path);

        match push_result {
            Ok(false) => TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored").pass(),
            Ok(true) => TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                .fail(&format!(
                    "Push accepted but should be rejected. A non-maintainer (pubkey: {}) published \
                    a state event announcing commit {}, but the push was accepted. The relay should \
                    only accept state events from maintainers (pubkey: {}).",
                    rogue_keys.public_key(),
                    new_commit,
                    client.public_key()
                )),
            Err(e) => TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored").fail(&e),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exists() {
        assert!(true);
    }
}