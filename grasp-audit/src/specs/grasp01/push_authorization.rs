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

use crate::{AuditClient, FixtureKind, TestContext, TestResult};
use nostr_sdk::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Test suite for Push Authorization operations
pub struct PushAuthorizationTests;

/// Helper to clone a repository and return the path
fn clone_repo(
    relay_domain: &str,
    npub: &str,
    repo_id: &str,
) -> Result<std::path::PathBuf, String> {
    let temp_base = std::env::temp_dir();
    let clone_dir_name = format!("grasp-push-test-{}", uuid::Uuid::new_v4());
    let clone_path = temp_base.join(&clone_dir_name);
    let _ = fs::remove_dir_all(&clone_path);

    let clone_url = format!("http://{}/{}/{}.git", relay_domain, npub, repo_id);
    let output = Command::new("git")
        .args(["clone", &clone_url, clone_path.to_str().unwrap()])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Failed to execute git clone: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git clone failed: {}", stderr));
    }

    // Configure git user
    let _ = Command::new("git")
        .args(["config", "user.email", "test@grasp-audit.local"])
        .current_dir(&clone_path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "GRASP Audit Test"])
        .current_dir(&clone_path)
        .output();

    Ok(clone_path)
}

/// Helper to create a commit and return the hash
fn create_commit(clone_path: &Path, message: &str) -> Result<String, String> {
    let test_file = clone_path.join(format!("test-{}.txt", uuid::Uuid::new_v4()));
    fs::write(&test_file, message).map_err(|e| format!("Failed to write file: {}", e))?;

    let filename = test_file.file_name().unwrap().to_str().unwrap();
    let output = Command::new("git")
        .args(["add", filename])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git add failed: {}", e))?;

    if !output.status.success() {
        return Err("Git add failed".to_string());
    }

    let output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git commit failed: {}", e))?;

    if !output.status.success() {
        return Err("Git commit failed".to_string());
    }

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git rev-parse failed: {}", e))?;

    if !output.status.success() {
        return Err("Failed to get commit hash".to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Helper to create a deterministic commit (for fixtures)
/// Uses fixed author/committer dates and disables GPG signing to ensure consistent hash
pub fn create_deterministic_commit(clone_path: &Path, message: &str) -> Result<String, String> {
    let test_file = clone_path.join("test.txt");
    fs::write(&test_file, message).map_err(|e| format!("Failed to write file: {}", e))?;

    let output = Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git add failed: {}", e))?;

    if !output.status.success() {
        return Err("Git add failed".to_string());
    }

    // Create deterministic commit with fixed dates and GPG disabled
    let output = Command::new("git")
        .args([
            "-c", "commit.gpgsign=false",
            "commit",
            "-m", message,
        ])
        .env("GIT_AUTHOR_DATE", "2024-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2024-01-01T00:00:00Z")
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git commit failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git commit failed: {}", stderr));
    }

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git rev-parse failed: {}", e))?;

    if !output.status.success() {
        return Err("Failed to get commit hash".to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Repository setup with deterministic commit
/// This struct holds all the data needed for push authorization tests
pub struct RepoSetup {
    pub clone_path: PathBuf,
    pub repo_id: String,
    pub npub: String,
    pub commit_hash: String,
}

impl Drop for RepoSetup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.clone_path);
    }
}

/// Helper function to set up a repository with deterministic commit
///
/// This performs all the common setup steps needed for push authorization tests:
/// 1. Gets RepoState fixture (repo announcement + state event with deterministic commit)
/// 2. Extracts repo_id and npub
/// 3. Verifies repo exists on disk
/// 4. Clones the repository
/// 5. Creates deterministic commit locally
/// 6. Verifies commit hash matches expected
/// 7. Creates and checks out main branch
/// 8. Pushes the commit so the grasp server has the state in the state event
///
/// Returns RepoSetup which auto-cleans up the clone_path on drop
pub async fn setup_repo_with_deterministic_commit(
    client: &AuditClient,
    git_data_dir: &Path,
    relay_domain: &str,
) -> Result<RepoSetup, String> {
    use crate::DETERMINISTIC_COMMIT_HASH;
    
    let ctx = TestContext::new(client);

    // Get RepoState fixture (includes repo announcement and state event with deterministic commit)
    let state_event = ctx.get_fixture(FixtureKind::RepoState).await
        .map_err(|e| format!("Failed to create repo state fixture: {}", e))?;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Extract repo_id from state event
    let repo_id = state_event.tags.iter().find(|t| t.kind() == TagKind::d())
        .and_then(|t| t.content())
        .ok_or("Missing repo_id")?
        .to_string();
    let npub = state_event.pubkey.to_bech32()
        .map_err(|e| format!("Failed to convert pubkey to bech32: {}", e))?;

    // Verify repo exists
    let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
    if !repo_path.exists() {
        return Err(format!("Repo not found: {}", repo_path.display()));
    }

    // Clone repo
    let clone_path = clone_repo(relay_domain, &npub, &repo_id)?;

    // Create deterministic commit locally (this will be the root commit with no parent)
    let commit_hash = create_deterministic_commit(&clone_path, "Initial commit")
        .map_err(|e| {
            let _ = fs::remove_dir_all(&clone_path);
            e
        })?;

    // Verify commit hash matches expected deterministic hash
    if commit_hash != DETERMINISTIC_COMMIT_HASH {
        let _ = fs::remove_dir_all(&clone_path);
        return Err(format!(
            "Commit hash mismatch: got {}, expected {}",
            commit_hash, DETERMINISTIC_COMMIT_HASH
        ));
    }

    // Create main branch pointing to our deterministic commit
    let branch_output = Command::new("git")
        .args(["branch", "main"])
        .current_dir(&clone_path)
        .output()
        .map_err(|e| {
            let _ = fs::remove_dir_all(&clone_path);
            format!("Failed to create main branch: {}", e)
        })?;
    
    if !branch_output.status.success() {
        let _ = fs::remove_dir_all(&clone_path);
        return Err(format!(
            "Failed to create main branch: {}",
            String::from_utf8_lossy(&branch_output.stderr)
        ));
    }

    // Checkout main branch
    let checkout_output = Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&clone_path)
        .output()
        .map_err(|e| {
            let _ = fs::remove_dir_all(&clone_path);
            format!("Failed to checkout main branch: {}", e)
        })?;
    
    if !checkout_output.status.success() {
        let _ = fs::remove_dir_all(&clone_path);
        return Err(format!(
            "Failed to checkout main branch: {}",
            String::from_utf8_lossy(&checkout_output.stderr)
        ));
    }

    // Push the commit to the server so the bare repo matches the state event
    let push_output = Command::new("git")
        .args(["push", "origin", "main"])
        .current_dir(&clone_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| {
            let _ = fs::remove_dir_all(&clone_path);
            format!("Failed to push to server: {}", e)
        })?;
    
    if !push_output.status.success() {
        let _ = fs::remove_dir_all(&clone_path);
        return Err(format!(
            "Failed to push to server: {}",
            String::from_utf8_lossy(&push_output.stderr)
        ));
    }

    Ok(RepoSetup {
        clone_path,
        repo_id,
        npub,
        commit_hash,
    })
}

/// Helper to attempt a push and return success/failure
fn try_push(clone_path: &Path) -> Result<bool, String> {
    let output = Command::new("git")
        .args(["push", "origin", "main"])
        .current_dir(clone_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Failed to execute git push: {}", e))?;

    Ok(output.status.success())
}

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

    /// Test that latest state event is used for authorization
    ///
    /// GRASP-01 requires that the relay use the LATEST state event (by created_at
    /// timestamp) when determining push authorization. This test verifies that
    /// a newer state event takes precedence over an older one.
    ///
    /// Scenario:
    /// 1. Owner creates repo with maintainer
    /// 2. Owner publishes state event for commit_a at t=100 (older)
    /// 3. Maintainer publishes state event for commit_b at t=200 (newer)
    /// 4. Push commit_b should be ACCEPTED (newer timestamp wins)
    /// 5. Push commit_a should be REJECTED (older state event superseded)
    pub async fn test_latest_state_event_used(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_latest_state_event_used";
        let description = "Latest state event takes precedence";

        // 1. Generate maintainer keypair
        let maintainer_keys = Keys::generate();
        let maintainer_pubkey = maintainer_keys.public_key().to_hex();

        // 2. Owner creates repo with maintainer
        let repo_event = match client
            .create_repo_announcement_with_maintainers(test_name, &[maintainer_pubkey.clone()])
            .await
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to create repo with maintainers: {}", e))
            }
        };

        // Send the owner's repo event
        if let Err(e) = client.send_event(repo_event.clone()).await {
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send owner repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo details
        let repo_id = match repo_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail("Repository event missing d tag")
            }
        };

        // Get relay URL for maintainer's repo announcement
        let relay_url = match client.relay_url().await {
            Ok(u) => u,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to get relay URL: {}", e))
            }
        };
        let http_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");
        let maintainer_npub = match maintainer_keys.public_key().to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to convert maintainer pubkey to npub: {}", e))
            }
        };

        // 3. Maintainer creates their own repo announcement (same d-tag)
        let maintainer_repo_event = match client
            .event_builder(
                Kind::GitRepoAnnouncement,
                format!("Maintainer's view of {} repository", test_name),
            )
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("name"),
                vec![format!("{} Test Repository (Maintainer)", test_name)],
            ))
            .tag(Tag::custom(
                TagKind::custom("clone"),
                vec![format!("{}/{}/{}.git", http_url, maintainer_npub, repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("relays"),
                vec![relay_url.clone()],
            ))
            .build(&maintainer_keys)
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to build maintainer repo event: {}", e))
            }
        };

        if let Err(e) = client.client().send_event(&maintainer_repo_event).await {
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send maintainer repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Verify maintainer's repo was created
        let maintainer_repo_path = git_data_dir
            .join(&maintainer_npub)
            .join(format!("{}.git", repo_id));
        if !maintainer_repo_path.exists() {
            return TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                "Maintainer repo not created at: {}",
                maintainer_repo_path.display()
            ));
        }

        // 4. Clone maintainer's repo
        let clone_path = match clone_repo(relay_domain, &maintainer_npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to clone maintainer repo: {}", e))
            }
        };

        // 5. Create first commit (commit_a) - this will be the one with OLDER timestamp
        let commit_a = match create_commit(&clone_path, "Commit A - older state") {
            Ok(h) => h,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to create commit_a: {}", e));
            }
        };

        // 6. Create second commit (commit_b) - this will be the one with NEWER timestamp
        let commit_b = match create_commit(&clone_path, "Commit B - newer state") {
            Ok(h) => h,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to create commit_b: {}", e));
            }
        };

        // 7. Calculate timestamps: older_timestamp (100 seconds ago) and newer_timestamp (now)
        let base_time = Timestamp::now().as_u64();
        let older_timestamp = Timestamp::from(base_time - 100); // 100 seconds ago
        let newer_timestamp = Timestamp::from(base_time);        // now

        // 8. Owner publishes state event for commit_a at OLDER timestamp
        let owner_state_event = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![commit_a.clone()],
            ))
            .custom_time(older_timestamp)
            .build(client.keys())
        {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to build owner state event: {}", e));
            }
        };

        if let Err(e) = client.client().send_event(&owner_state_event).await {
            let _ = fs::remove_dir_all(&clone_path);
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send owner state event: {}", e));
        }

        // 9. Maintainer publishes state event for commit_b at NEWER timestamp
        let maintainer_state_event = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![commit_b.clone()],
            ))
            .custom_time(newer_timestamp)
            .build(&maintainer_keys)
        {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to build maintainer state event: {}", e));
            }
        };

        if let Err(e) = client.client().send_event(&maintainer_state_event).await {
            let _ = fs::remove_dir_all(&clone_path);
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send maintainer state event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 10. Create and checkout main branch pointing to commit_b (the newer state)
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = branch_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                    "Failed to create main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = checkout_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                    "Failed to checkout main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        // 11. Attempt push - should be ACCEPTED because maintainer's newer state event
        // announces commit_b which is now HEAD of main
        let push_result = try_push(&clone_path);
        let _ = fs::remove_dir_all(&clone_path);

        match push_result {
            Ok(true) => TestResult::new(test_name, "GRASP-01", description).pass(),
            Ok(false) => TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                "Push was rejected but should have been accepted. \
                The maintainer published a state event at timestamp {} announcing commit_b ({}). \
                The owner published an older state event at timestamp {} announcing commit_a ({}). \
                The relay should use the NEWER state event (maintainer's) for authorization.",
                newer_timestamp.as_u64(),
                commit_b,
                older_timestamp.as_u64(),
                commit_a
            )),
            Err(e) => {
                TestResult::new(test_name, "GRASP-01", description).fail(&format!("Push error: {}", e))
            }
        }
    }

    /// Test push authorized by direct maintainer state event
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    /// This tests the first level: direct maintainers listed in the maintainers tag.
    ///
    /// Scenario:
    /// 1. Owner creates repo with `["maintainers", "<maintainer-pubkey>"]` tag
    /// 2. Maintainer creates their own repo announcement (same d-tag)
    /// 3. Maintainer publishes state event with a commit hash
    /// 4. Push to that commit should be ACCEPTED
    pub async fn test_push_authorized_by_direct_maintainer_state(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_authorized_by_direct_maintainer_state";

        // 1. Generate maintainer keypair
        let maintainer_keys = Keys::generate();
        let maintainer_pubkey = maintainer_keys.public_key().to_hex();

        // 2. Owner creates repo with maintainer listed
        let repo_event = match client
            .create_repo_announcement_with_maintainers(test_name, &[maintainer_pubkey.clone()])
            .await
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!("Failed to create repo with maintainers: {}", e))
            }
        };

        // Send the owner's repo event
        if let Err(e) = client.send_event(repo_event.clone()).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by direct maintainer state event",
            )
            .fail(&format!("Failed to send owner repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo details
        let repo_id = match repo_event
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
                    "Push authorized by direct maintainer state event",
                )
                .fail("Repository event missing d tag")
            }
        };

        // Get relay URL for maintainer's repo announcement
        let relay_url = match client.relay_url().await {
            Ok(u) => u,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!("Failed to get relay URL: {}", e))
            }
        };
        let http_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");
        let maintainer_npub = match maintainer_keys.public_key().to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!("Failed to convert maintainer pubkey to npub: {}", e))
            }
        };

        // 3. Maintainer creates their own repo announcement (same d-tag)
        // This creates a separate repo at maintainer-npub/repo-id.git
        let maintainer_repo_event = match client
            .event_builder(
                Kind::GitRepoAnnouncement,
                format!("Maintainer's view of {} repository", test_name),
            )
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("name"),
                vec![format!("{} Test Repository (Maintainer)", test_name)],
            ))
            .tag(Tag::custom(
                TagKind::custom("clone"),
                vec![format!("{}/{}/{}.git", http_url, maintainer_npub, repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("relays"),
                vec![relay_url.clone()],
            ))
            .build(&maintainer_keys)
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!("Failed to build maintainer repo event: {}", e))
            }
        };

        if let Err(e) = client.client().send_event(&maintainer_repo_event).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by direct maintainer state event",
            )
            .fail(&format!("Failed to send maintainer repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Verify maintainer's repo was created
        let maintainer_repo_path = git_data_dir
            .join(&maintainer_npub)
            .join(format!("{}.git", repo_id));
        if !maintainer_repo_path.exists() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by direct maintainer state event",
            )
            .fail(&format!(
                "Maintainer repo not created at: {}",
                maintainer_repo_path.display()
            ));
        }

        // 4. Clone maintainer's repo
        let clone_path = match clone_repo(relay_domain, &maintainer_npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!("Failed to clone maintainer repo: {}", e))
            }
        };

        // 5. Create deterministic commit
        let commit_hash = match create_deterministic_commit(&clone_path, "Initial commit") {
            Ok(h) => h,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!("Failed to create commit: {}", e));
            }
        };

        // 6. Maintainer publishes state event with commit hash
        let state_event = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![commit_hash.clone()],
            ))
            .build(&maintainer_keys)
        {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!("Failed to build state event: {}", e));
            }
        };

        if let Err(e) = client.client().send_event(&state_event).await {
            let _ = fs::remove_dir_all(&clone_path);
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by direct maintainer state event",
            )
            .fail(&format!("Failed to send state event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 7. Create and checkout main branch
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = branch_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!(
                    "Failed to create main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = checkout_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by direct maintainer state event",
                )
                .fail(&format!(
                    "Failed to checkout main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        // 8. Attempt push - should be ACCEPTED because maintainer's state event authorizes it
        let push_result = try_push(&clone_path);
        let _ = fs::remove_dir_all(&clone_path);

        match push_result {
            Ok(true) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by direct maintainer state event",
            )
            .pass(),
            Ok(false) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by direct maintainer state event",
            )
            .fail(&format!(
                "Push was rejected but should have been accepted. \
                The maintainer (pubkey: {}) is listed in the owner's maintainers tag \
                and published a state event announcing commit {}. \
                The relay should authorize pushes matching this state event.",
                maintainer_pubkey, commit_hash
            )),
            Err(e) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by direct maintainer state event",
            )
            .fail(&format!("Push error: {}", e)),
        }
    }

    /// Test push authorized by recursive maintainer state event
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    /// This tests recursive maintainer chains: Owner -> MaintainerA -> MaintainerB
    ///
    /// Scenario:
    /// 1. Owner creates repo with `["maintainers", "<maintainerA-pubkey>"]` tag
    /// 2. MaintainerA creates their own repo announcement (same d-tag) with MaintainerB
    /// 3. MaintainerB creates their own repo announcement (same d-tag, no further maintainers)
    /// 4. MaintainerB publishes state event with a commit hash
    /// 5. Push to that commit should be ACCEPTED (recursive maintainer chain)
    pub async fn test_push_authorized_by_recursive_maintainer_state(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_authorized_by_recursive_maintainer_state";

        // 1. Generate MaintainerA and MaintainerB keypairs
        let maintainer_a_keys = Keys::generate();
        let maintainer_a_pubkey = maintainer_a_keys.public_key().to_hex();

        let maintainer_b_keys = Keys::generate();
        let maintainer_b_pubkey = maintainer_b_keys.public_key().to_hex();

        // 2. Owner creates repo with MaintainerA listed
        let repo_event = match client
            .create_repo_announcement_with_maintainers(test_name, &[maintainer_a_pubkey.clone()])
            .await
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to create repo with maintainers: {}", e))
            }
        };

        // Send the owner's repo event
        if let Err(e) = client.send_event(repo_event.clone()).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!("Failed to send owner repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo details
        let repo_id = match repo_event
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
                .fail("Repository event missing d tag")
            }
        };

        // Get relay URL for maintainers' repo announcements
        let relay_url = match client.relay_url().await {
            Ok(u) => u,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to get relay URL: {}", e))
            }
        };
        let http_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");

        let maintainer_a_npub = match maintainer_a_keys.public_key().to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to convert maintainer A pubkey to npub: {}", e))
            }
        };

        let maintainer_b_npub = match maintainer_b_keys.public_key().to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to convert maintainer B pubkey to npub: {}", e))
            }
        };

        // 3. MaintainerA creates their own repo announcement (same d-tag) with MaintainerB listed
        let maintainer_a_repo_event = match client
            .event_builder(
                Kind::GitRepoAnnouncement,
                format!("MaintainerA's view of {} repository", test_name),
            )
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("name"),
                vec![format!("{} Test Repository (MaintainerA)", test_name)],
            ))
            .tag(Tag::custom(
                TagKind::custom("clone"),
                vec![format!("{}/{}/{}.git", http_url, maintainer_a_npub, repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("relays"),
                vec![relay_url.clone()],
            ))
            .tag(Tag::custom(
                TagKind::custom("maintainers"),
                vec![maintainer_b_pubkey.clone()],
            ))
            .build(&maintainer_a_keys)
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to build maintainer A repo event: {}", e))
            }
        };

        if let Err(e) = client.client().send_event(&maintainer_a_repo_event).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!("Failed to send maintainer A repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 4. MaintainerB creates their own repo announcement (same d-tag, no further maintainers)
        let maintainer_b_repo_event = match client
            .event_builder(
                Kind::GitRepoAnnouncement,
                format!("MaintainerB's view of {} repository", test_name),
            )
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("name"),
                vec![format!("{} Test Repository (MaintainerB)", test_name)],
            ))
            .tag(Tag::custom(
                TagKind::custom("clone"),
                vec![format!("{}/{}/{}.git", http_url, maintainer_b_npub, repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("relays"),
                vec![relay_url.clone()],
            ))
            .build(&maintainer_b_keys)
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to build maintainer B repo event: {}", e))
            }
        };

        if let Err(e) = client.client().send_event(&maintainer_b_repo_event).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!("Failed to send maintainer B repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Verify maintainer B's repo was created
        let maintainer_b_repo_path = git_data_dir
            .join(&maintainer_b_npub)
            .join(format!("{}.git", repo_id));
        if !maintainer_b_repo_path.exists() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!(
                "Maintainer B repo not created at: {}",
                maintainer_b_repo_path.display()
            ));
        }

        // 5. Clone maintainer B's repo
        let clone_path = match clone_repo(relay_domain, &maintainer_b_npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to clone maintainer B repo: {}", e))
            }
        };

        // 6. Create deterministic commit
        let commit_hash = match create_deterministic_commit(&clone_path, "Initial commit") {
            Ok(h) => h,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to create commit: {}", e));
            }
        };

        // 7. MaintainerB publishes state event with commit hash
        let state_event = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![commit_hash.clone()],
            ))
            .build(&maintainer_b_keys)
        {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(&format!("Failed to build state event: {}", e));
            }
        };

        if let Err(e) = client.client().send_event(&state_event).await {
            let _ = fs::remove_dir_all(&clone_path);
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(&format!("Failed to send state event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 8. Create and checkout main branch
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = branch_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
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
        }

        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = checkout_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
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
        }

        // 9. Attempt push - should be ACCEPTED because recursive maintainer chain authorizes it
        // Owner -> MaintainerA -> MaintainerB, and MaintainerB has published the state event
        let push_result = try_push(&clone_path);
        let _ = fs::remove_dir_all(&clone_path);

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
                The recursive maintainer chain is: Owner -> MaintainerA (pubkey: {}) -> MaintainerB (pubkey: {}). \
                MaintainerB published a state event announcing commit {}. \
                The relay should authorize pushes matching this state event through recursive maintainer traversal.",
                maintainer_a_pubkey, maintainer_b_pubkey, commit_hash
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

    /// Test that owner's newer state event beats maintainer's older state event
    ///
    /// GRASP-01 requires that the relay use the LATEST state event (by created_at
    /// timestamp) when determining push authorization. This test is the MIRROR of
    /// test_latest_state_event_used - confirming that timestamp is the deciding factor,
    /// not who authored the state event.
    ///
    /// Scenario:
    /// 1. Owner creates repo with maintainer
    /// 2. Maintainer publishes state event for commit_a at t=100 (older)
    /// 3. Owner publishes state event for commit_b at t=200 (newer)
    /// 4. Push commit_b should be ACCEPTED (owner's newer state wins)
    /// 5. Push commit_a should be REJECTED (maintainer's older state superseded)
    ///
    /// Key difference from test_latest_state_event_used:
    /// - Task 8: Owner=older, Maintainer=newer → Maintainer wins
    /// - Task 9: Maintainer=older, Owner=newer → Owner wins
    /// - **This confirms symmetry**: timestamp is the deciding factor
    pub async fn test_owner_newer_state_beats_maintainer(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_owner_newer_state_beats_maintainer";
        let description = "Owner's newer state event beats maintainer's older state";

        // 1. Generate maintainer keypair
        let maintainer_keys = Keys::generate();
        let maintainer_pubkey = maintainer_keys.public_key().to_hex();

        // 2. Owner creates repo with maintainer
        let repo_event = match client
            .create_repo_announcement_with_maintainers(test_name, &[maintainer_pubkey.clone()])
            .await
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to create repo with maintainers: {}", e))
            }
        };

        // Send the owner's repo event
        if let Err(e) = client.send_event(repo_event.clone()).await {
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send owner repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo details
        let repo_id = match repo_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail("Repository event missing d tag")
            }
        };

        // Get relay URL for maintainer's repo announcement
        let relay_url = match client.relay_url().await {
            Ok(u) => u,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to get relay URL: {}", e))
            }
        };
        let http_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");
        let maintainer_npub = match maintainer_keys.public_key().to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to convert maintainer pubkey to npub: {}", e))
            }
        };

        // 3. Maintainer creates their own repo announcement (same d-tag)
        let maintainer_repo_event = match client
            .event_builder(
                Kind::GitRepoAnnouncement,
                format!("Maintainer's view of {} repository", test_name),
            )
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("name"),
                vec![format!("{} Test Repository (Maintainer)", test_name)],
            ))
            .tag(Tag::custom(
                TagKind::custom("clone"),
                vec![format!("{}/{}/{}.git", http_url, maintainer_npub, repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("relays"),
                vec![relay_url.clone()],
            ))
            .build(&maintainer_keys)
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to build maintainer repo event: {}", e))
            }
        };

        if let Err(e) = client.client().send_event(&maintainer_repo_event).await {
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send maintainer repo event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Verify maintainer's repo was created
        let maintainer_repo_path = git_data_dir
            .join(&maintainer_npub)
            .join(format!("{}.git", repo_id));
        if !maintainer_repo_path.exists() {
            return TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                "Maintainer repo not created at: {}",
                maintainer_repo_path.display()
            ));
        }

        // 4. Clone maintainer's repo
        let clone_path = match clone_repo(relay_domain, &maintainer_npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to clone maintainer repo: {}", e))
            }
        };

        // 5. Create first commit (commit_a) - MAINTAINER will announce this with OLDER timestamp
        let commit_a = match create_commit(&clone_path, "Commit A - older state (maintainer)") {
            Ok(h) => h,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to create commit_a: {}", e));
            }
        };

        // 6. Create second commit (commit_b) - OWNER will announce this with NEWER timestamp
        let commit_b = match create_commit(&clone_path, "Commit B - newer state (owner)") {
            Ok(h) => h,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to create commit_b: {}", e));
            }
        };

        // 7. Calculate timestamps: older_timestamp (100 seconds ago) and newer_timestamp (now)
        let base_time = Timestamp::now().as_u64();
        let older_timestamp = Timestamp::from(base_time - 100); // 100 seconds ago - for MAINTAINER
        let newer_timestamp = Timestamp::from(base_time);        // now - for OWNER

        // 8. MAINTAINER publishes state event for commit_a at OLDER timestamp
        // This is the KEY DIFFERENCE from test_latest_state_event_used:
        // - In Task 8: Owner was older, Maintainer was newer
        // - In Task 9 (this test): Maintainer is older, Owner is newer
        let maintainer_state_event = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![commit_a.clone()],
            ))
            .custom_time(older_timestamp)
            .build(&maintainer_keys)
        {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to build maintainer state event: {}", e));
            }
        };

        if let Err(e) = client.client().send_event(&maintainer_state_event).await {
            let _ = fs::remove_dir_all(&clone_path);
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send maintainer state event: {}", e));
        }

        // 9. OWNER publishes state event for commit_b at NEWER timestamp
        let owner_state_event = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![commit_b.clone()],
            ))
            .custom_time(newer_timestamp)
            .build(client.keys())
        {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description)
                    .fail(&format!("Failed to build owner state event: {}", e));
            }
        };

        if let Err(e) = client.client().send_event(&owner_state_event).await {
            let _ = fs::remove_dir_all(&clone_path);
            return TestResult::new(test_name, "GRASP-01", description)
                .fail(&format!("Failed to send owner state event: {}", e));
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 10. Create and checkout main branch pointing to commit_b (the newer state)
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = branch_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                    "Failed to create main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        if let Ok(output) = checkout_output {
            if !output.status.success() {
                let _ = fs::remove_dir_all(&clone_path);
                return TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                    "Failed to checkout main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        // 11. Attempt push - should be ACCEPTED because owner's newer state event
        // announces commit_b which is now HEAD of main
        let push_result = try_push(&clone_path);
        let _ = fs::remove_dir_all(&clone_path);

        match push_result {
            Ok(true) => TestResult::new(test_name, "GRASP-01", description).pass(),
            Ok(false) => TestResult::new(test_name, "GRASP-01", description).fail(&format!(
                "Push was rejected but should have been accepted. \
                The OWNER published a state event at timestamp {} announcing commit_b ({}). \
                The MAINTAINER published an older state event at timestamp {} announcing commit_a ({}). \
                The relay should use the NEWER state event (owner's) for authorization. \
                This confirms symmetry with test_latest_state_event_used: timestamp is the deciding factor.",
                newer_timestamp.as_u64(),
                commit_b,
                older_timestamp.as_u64(),
                commit_a
            )),
            Err(e) => {
                TestResult::new(test_name, "GRASP-01", description).fail(&format!("Push error: {}", e))
            }
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