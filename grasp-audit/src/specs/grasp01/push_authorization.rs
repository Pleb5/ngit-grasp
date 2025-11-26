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

    /// Test recursive maintainer authorization
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    pub async fn test_recursive_maintainer_authorization(
        _client: &AuditClient,
        _git_data_dir: &Path,
        _relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_recursive_maintainer_authorization";

        // This test requires two separate clients (owner and maintainer)
        // For now, return not implemented
        TestResult::new(test_name, "GRASP-01", "Maintainer can authorize pushes")
            .fail("Not implemented: requires multiple client support")
    }

    /// Test that latest state event is used for authorization
    pub async fn test_latest_state_event_used(
        _client: &AuditClient,
        _git_data_dir: &Path,
        _relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_latest_state_event_used";

        // This test requires publishing multiple state events with timestamps
        // and verifying the latest one is used
        TestResult::new(test_name, "GRASP-01", "Latest state event takes precedence")
            .fail("Not implemented: requires timestamp manipulation")
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