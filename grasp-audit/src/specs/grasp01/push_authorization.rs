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
    clone_repo, create_commit, create_deterministic_commit, create_deterministic_commit_with_variant,
    try_push, AuditClient, CommitVariant, FixtureKind, TestContext, TestResult,
    DETERMINISTIC_COMMIT_HASH, MAINTAINER_DETERMINISTIC_COMMIT_HASH,
    RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH,
};
use nostr_sdk::prelude::*;
use std::fs;
use std::path::Path;

/// Test suite for Push Authorization operations
pub struct PushAuthorizationTests;

impl PushAuthorizationTests {
    /// Run all push authorization tests
    pub async fn run_all(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> crate::AuditResult {
        let mut results = crate::AuditResult::new("GRASP-01 Push Authorization Tests");

        results.add(Self::test_push_authorized_by_owner_state(client, git_data_dir, relay_domain).await);
        results.add(Self::test_push_rejected_without_state_event(client, git_data_dir, relay_domain).await);
        results.add(Self::test_push_rejected_wrong_commit(client, git_data_dir, relay_domain).await);
        results.add(Self::test_push_authorized_by_maintainer_state_only(client, git_data_dir, relay_domain).await);

        results
    }

    /// Test that push is authorized when state event matches the commit
    ///
    /// GRASP-01: "MUST accept pushes via this service that match the latest
    /// repo state announcement on the relay"
    ///
    /// ## Fixture-First Pattern
    ///
    /// 1. **Generate**: Create TestContext and get RepoState fixture
    ///    (repo announcement + state event pointing to deterministic commit)
    /// 2. **Send**: Clone repo, create deterministic commit locally, push to relay
    /// 3. **Verify**: Push should succeed because state event authorizes this commit
    pub async fn test_push_authorized_by_owner_state(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        use std::process::Command;

        let test_name = "test_push_authorized_by_owner_state";

        // ============================================================
        // Step 1: GENERATE - Create TestContext and get RepoState fixture
        // ============================================================
        let ctx = TestContext::new(client);

        let state_event = match ctx.get_fixture(FixtureKind::RepoState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Failed to create RepoState fixture: {}", e));
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo_id and npub from state event
        let repo_id = match state_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail("Missing repo_id in state event");
            }
        };

        let npub = match state_event.pubkey.to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // Verify repo exists on disk
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
        if !repo_path.exists() {
            return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                .fail(&format!("Repo not found: {}", repo_path.display()));
        }

        // ============================================================
        // Step 2: SEND - Clone repo, create deterministic commit, push
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Failed to clone repo: {}", e));
            }
        };

        // Cleanup helper
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // Create deterministic commit locally
        let commit_hash = match create_deterministic_commit(&clone_path, "Initial commit") {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Failed to create deterministic commit: {}", e));
            }
        };

        // Verify commit hash matches expected
        if commit_hash != DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                .fail(&format!(
                    "Commit hash mismatch: got {}, expected {}",
                    commit_hash, DETERMINISTIC_COMMIT_HASH
                ));
        }

        // Create main branch pointing to our deterministic commit
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        match branch_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!(
                        "Failed to create main branch: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // Checkout main branch
        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        match checkout_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!(
                        "Failed to checkout main branch: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // ============================================================
        // Step 3: VERIFY - Push should succeed because state event
        // authorizes this commit
        // ============================================================
        let push_result = try_push(&clone_path);
        cleanup();

        match push_result {
            Ok(true) => {
                TestResult::new(test_name, "GRASP-01", "Push authorized with matching state").pass()
            }
            Ok(false) => {
                TestResult::new(test_name, "GRASP-01", "Push authorized with matching state").fail(
                    &format!(
                        "Push was rejected but should have been accepted. \
                        The state event points to commit {} which matches the pushed commit.",
                        DETERMINISTIC_COMMIT_HASH
                    ),
                )
            }
            Err(e) => {
                TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                    .fail(&format!("Push error: {}", e))
            }
        }
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
    /// GRASP-01: "MUST accept pushes via this service that match the latest repo state announcement"
    /// (Conversely, MUST reject pushes that don't match)
    ///
    /// ## Fixture-First Pattern
    ///
    /// 1. **Generate**: Create TestContext and get RepoState fixture
    ///    (repo announcement + state event pointing to deterministic commit)
    /// 2. **Send**: Clone repo, create deterministic commit, push (establishes state on relay)
    /// 3. **Test**: Create a NEW commit locally, try to push
    /// 4. **Verify**: Push should be rejected because new commit doesn't match state event
    pub async fn test_push_rejected_wrong_commit(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        use std::process::Command;

        let test_name = "test_push_rejected_wrong_commit";

        // ============================================================
        // Step 1: GENERATE - Create TestContext and get RepoState fixture
        // ============================================================
        let ctx = TestContext::new(client);

        let state_event = match ctx.get_fixture(FixtureKind::RepoState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to create RepoState fixture: {}", e));
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo_id and npub from state event
        let repo_id = match state_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail("Missing repo_id in state event");
            }
        };

        let npub = match state_event.pubkey.to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // Verify repo exists on disk
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
        if !repo_path.exists() {
            return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                .fail(&format!("Repo not found: {}", repo_path.display()));
        }

        // ============================================================
        // Step 2: SEND - Clone repo, create deterministic commit, push
        // (establishes the state on the relay)
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to clone repo: {}", e));
            }
        };

        // Cleanup helper
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // Create deterministic commit locally
        let commit_hash = match create_deterministic_commit(&clone_path, "Initial commit") {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to create deterministic commit: {}", e));
            }
        };

        // Verify commit hash matches expected
        if commit_hash != DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                .fail(&format!(
                    "Commit hash mismatch: got {}, expected {}",
                    commit_hash, DETERMINISTIC_COMMIT_HASH
                ));
        }

        // Create main branch pointing to our deterministic commit
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        match branch_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!(
                        "Failed to create main branch: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // Checkout main branch
        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        match checkout_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!(
                        "Failed to checkout main branch: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // Push the deterministic commit to establish state on relay
        let push_output = Command::new("git")
            .args(["push", "origin", "main"])
            .current_dir(&clone_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output();

        match push_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to push initial commit: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!(
                        "Failed to push initial commit: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // ============================================================
        // Step 3: TEST - Create a NEW commit that is NOT announced
        // in any state event
        // ============================================================
        let new_commit = match create_commit(&clone_path, "Unauthorized commit") {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                    .fail(&format!("Failed to create commit: {}", e));
            }
        };

        // ============================================================
        // Step 4: VERIFY - Push should be rejected because new commit
        // doesn't match state event
        // ============================================================
        let push_result = try_push(&clone_path);
        cleanup();

        match push_result {
            Ok(false) => TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event").pass(),
            Ok(true) => TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                .fail(&format!(
                    "Push accepted but should be rejected. State event points to {}, but pushed {}",
                    DETERMINISTIC_COMMIT_HASH, new_commit
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
    /// ## Fixture-First Pattern
    ///
    /// 1. **Generate**: Create TestContext, get RepoState (owner) and MaintainerState fixtures
    /// 2. **Send**: Clone repo, create maintainer deterministic commit, push to relay
    /// 3. **Verify**: Push should succeed because maintainer's state event authorizes this commit
    ///
    /// Scenario:
    /// 1. Owner's repo announcement lists maintainer in maintainers tag
    /// 2. Maintainer publishes ONLY a state event (no announcement)
    /// 3. Clone, create maintainer commit, verify hash, push
    /// 4. The push should be ACCEPTED because maintainer's state event authorizes it
    pub async fn test_push_authorized_by_maintainer_state_only(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        use std::process::Command;

        let test_name = "test_push_authorized_by_maintainer_state_only";

        // ============================================================
        // Step 1: GENERATE - Create TestContext and get fixtures
        // ============================================================
        let ctx = TestContext::new(client);

        // Get RepoState fixture (owner's repo announcement + state event)
        let state_event = match ctx.get_fixture(FixtureKind::RepoState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&format!("Failed to create RepoState fixture: {}", e));
            }
        };

        // Get MaintainerState fixture (maintainer's state event ONLY - no announcement)
        // This tests that state-only authorization works without a maintainer announcement
        match ctx.get_fixture(FixtureKind::MaintainerState).await {
            Ok(_) => {}
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&format!("Failed to create MaintainerState fixture: {}", e));
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo_id and npub from owner's state event
        let repo_id = match state_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail("Missing repo_id in state event");
            }
        };

        let npub = match state_event.pubkey.to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // Verify repo exists on disk
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
        if !repo_path.exists() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by maintainer state event only (no announcement)",
            )
            .fail(&format!("Repo not found: {}", repo_path.display()));
        }

        // ============================================================
        // Step 2: SEND - Clone, create maintainer commit, push
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&e);
            }
        };
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // Create maintainer deterministic commit
        let commit_hash =
            match create_deterministic_commit_with_variant(&clone_path, CommitVariant::Maintainer) {
                Ok(h) => h,
                Err(e) => {
                    cleanup();
                    return TestResult::new(
                        test_name,
                        "GRASP-01",
                        "Push authorized by maintainer state event only (no announcement)",
                    )
                    .fail(&format!("Failed to create maintainer commit: {}", e));
                }
            };

        // Verify commit hash matches expected
        if commit_hash != MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by maintainer state event only (no announcement)",
            )
            .fail(&format!(
                "Maintainer commit hash mismatch: got {}, expected {}",
                commit_hash, MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
        }

        // Create main branch
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        match branch_output {
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&format!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&format!(
                    "Failed to create main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            _ => {}
        }

        // Checkout main branch
        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        match checkout_output {
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&format!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(&format!(
                    "Failed to checkout main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            _ => {}
        }

        // ============================================================
        // Step 3: VERIFY - Push should succeed because maintainer's
        // state event authorizes this commit
        // ============================================================
        let push_result = try_push(&clone_path);
        cleanup();

        match push_result {
            Ok(true) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by maintainer state event only (no announcement)",
            )
            .pass(),
            Ok(false) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by maintainer state event only (no announcement)",
            )
            .fail(&format!(
                "Push was rejected but should have been accepted. \
                The maintainer published a state event with commit {}, \
                and even without a separate announcement, the relay should \
                authorize pushes matching this state event since the maintainer \
                is listed in the owner's announcement.",
                MAINTAINER_DETERMINISTIC_COMMIT_HASH
            )),
            Err(e) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by maintainer state event only (no announcement)",
            )
            .fail(&format!("Push error: {}", e)),
        }
    }

    /// Test push authorized by recursive maintainer state event
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    /// This tests recursive maintainer chains: Owner -> MaintainerA -> MaintainerB
    ///
    /// ## Fixture-First Pattern
    ///
    /// 1. **Generate**: Create TestContext and get fixture chain:
    ///    - RepoState (owner's repo announcement + state event)
    ///    - MaintainerAnnouncement (maintainer lists recursive-maintainer)
    ///    - MaintainerState (maintainer's state event)
    ///    - RecursiveMaintainerRepoAndState (recursive maintainer's announcement + state)
    /// 2. **Send**: Clone repo, create recursive maintainer deterministic commit, push
    /// 3. **Verify**: Push should succeed because recursive maintainer's state event authorizes it
    ///
    /// The fixture chain establishes: Owner -> Maintainer -> RecursiveMaintainer
    /// Each level publishes announcements that authorize the next level.
    pub async fn test_push_authorized_by_recursive_maintainer_state(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        use std::process::Command;

        let test_name = "test_push_authorized_by_recursive_maintainer_state";

        // ============================================================
        // Step 1: GENERATE - Create TestContext and get fixture chain
        // ============================================================
        let ctx = TestContext::new(client);

        // Get RepoState fixture (owner's repo announcement + state event)
        let state_event = match ctx.get_fixture(FixtureKind::RepoState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to create RepoState fixture: {}", e));
            }
        };

        // Get MaintainerAnnouncement fixture (maintainer's repo announcement listing recursive maintainer)
        match ctx.get_fixture(FixtureKind::MaintainerAnnouncement).await {
            Ok(_) => {}
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to create MaintainerAnnouncement fixture: {}", e));
            }
        };

        // Get MaintainerState fixture (maintainer's state event)
        match ctx.get_fixture(FixtureKind::MaintainerState).await {
            Ok(_) => {}
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to create MaintainerState fixture: {}", e));
            }
        };

        // Get RecursiveMaintainerRepoAndState fixture (completes 3-level delegation chain)
        match ctx.get_fixture(FixtureKind::RecursiveMaintainerRepoAndState).await {
            Ok(_) => {}
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to create RecursiveMaintainerRepoAndState fixture: {}", e));
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo_id and npub from owner's state event
        let repo_id = match state_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail("Missing repo_id in state event");
            }
        };

        let npub = match state_event.pubkey.to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // Verify repo exists on disk
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
        if !repo_path.exists() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!("Repo not found: {}", repo_path.display()));
        }

        // ============================================================
        // Step 2: SEND - Clone, create recursive maintainer commit, push
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&e);
            }
        };
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // Create recursive maintainer deterministic commit
        let commit_hash =
            match create_deterministic_commit_with_variant(&clone_path, CommitVariant::RecursiveMaintainer) {
                Ok(h) => h,
                Err(e) => {
                    cleanup();
                    return TestResult::new(
                        test_name,
                        "GRASP-01",
                        "Push authorized by recursive maintainer state event",
                    )
                    .fail(&format!("Failed to create recursive maintainer commit: {}", e));
                }
            };

        // Verify commit hash matches expected
        if commit_hash != RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!(
                "Recursive maintainer commit hash mismatch: got {}, expected {}",
                commit_hash, RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
        }

        // Create main branch
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        match branch_output {
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!(
                    "Failed to create main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            _ => {}
        }

        // Checkout main branch
        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        match checkout_output {
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!(
                    "Failed to checkout main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            _ => {}
        }

        // ============================================================
        // Step 3: VERIFY - Push should succeed because recursive
        // maintainer's state event authorizes this commit
        // ============================================================
        let push_result = try_push(&clone_path);
        cleanup();

        match push_result {
            Ok(true) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .pass(),
            Ok(false) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!(
                "Push was rejected but should have been accepted. \
                The recursive maintainer published a state event with commit {}, \
                and the relay should authorize pushes matching this state event \
                through recursive maintainer traversal (Owner -> Maintainer -> RecursiveMaintainer).",
                RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
            )),
            Err(e) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!("Push error: {}", e)),
        }
    }

    /// Test that non-maintainer state event is ignored
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    /// (Conversely, state events from non-maintainers MUST be ignored)
    ///
    /// ## Fixture-First Pattern
    ///
    /// 1. **Generate**: Create TestContext and get RepoState fixture
    ///    (repo announcement + state event pointing to deterministic commit)
    /// 2. **Send**: Clone repo, create deterministic commit, push (establishes state on relay)
    /// 3. **Attack**: Create a rogue state event signed by a non-maintainer
    /// 4. **Test**: Create a new commit and try to push
    /// 5. **Verify**: Push should be rejected because rogue state event is ignored
    pub async fn test_non_maintainer_state_rejected(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        use std::process::Command;

        let test_name = "test_non_maintainer_state_rejected";

        // ============================================================
        // Step 1: GENERATE - Create TestContext and get RepoState fixture
        // ============================================================
        let ctx = TestContext::new(client);

        let state_event = match ctx.get_fixture(FixtureKind::RepoState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to create RepoState fixture: {}", e));
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo_id and npub from state event
        let repo_id = match state_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail("Missing repo_id in state event");
            }
        };

        let npub = match state_event.pubkey.to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // Verify repo exists on disk
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
        if !repo_path.exists() {
            return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                .fail(&format!("Repo not found: {}", repo_path.display()));
        }

        // ============================================================
        // Step 2: SEND - Clone repo, create deterministic commit, push
        // (establishes the state on the relay)
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to clone repo: {}", e));
            }
        };

        // Cleanup helper
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // Create deterministic commit locally
        let commit_hash = match create_deterministic_commit(&clone_path, "Initial commit") {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to create deterministic commit: {}", e));
            }
        };

        // Verify commit hash matches expected
        if commit_hash != DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                .fail(&format!(
                    "Commit hash mismatch: got {}, expected {}",
                    commit_hash, DETERMINISTIC_COMMIT_HASH
                ));
        }

        // Create main branch pointing to our deterministic commit
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        match branch_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!(
                        "Failed to create main branch: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // Checkout main branch
        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        match checkout_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!(
                        "Failed to checkout main branch: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // Push the deterministic commit to establish state on relay
        let push_output = Command::new("git")
            .args(["push", "origin", "main"])
            .current_dir(&clone_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output();

        match push_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to push initial commit: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!(
                        "Failed to push initial commit: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
            }
            _ => {}
        }

        // ============================================================
        // Step 3: ATTACK - Create a new commit and a rogue state event
        // from a non-maintainer
        // ============================================================
        let new_commit = match create_commit(&clone_path, "New commit to push") {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to create commit: {}", e));
            }
        };

        // Create a rogue keypair (NOT the maintainer)
        let rogue_keys = Keys::generate();
        
        // Create a rogue state event announcing the new commit
        // This event has the correct repo_id but is signed by a non-maintainer
        let rogue_state = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![new_commit.clone()],
            ))
            .build(&rogue_keys)
        {
            Ok(e) => e,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                    .fail(&format!("Failed to build rogue state event: {}", e));
            }
        };

        // Send the rogue state event using the raw client to bypass AuditClient's key check
        if let Err(e) = client.client().send_event(&rogue_state).await {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                .fail(&format!("Failed to send rogue state event: {}", e));
        }

        // Wait for event to propagate
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // ============================================================
        // Step 4 & 5: VERIFY - Push should be rejected because rogue
        // state event is ignored
        // ============================================================
        let push_result = try_push(&clone_path);
        cleanup();

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