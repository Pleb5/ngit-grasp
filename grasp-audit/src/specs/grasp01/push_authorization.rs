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

/// Expected hash for PR test deterministic commit
///
/// This hash is produced by creating a commit with:
/// - File: test.txt containing "PR test deterministic commit"
/// - Message: "PR test deterministic commit"
/// - Author: "GRASP Audit Test <test@grasp-audit.local>"
/// - Author date: 2024-01-01T00:00:00Z
/// - Committer date: 2024-01-01T00:00:00Z
/// - GPG signing: disabled
/// - Parent: none (root commit)
///
/// Run `test_pr_test_commit_hash_discovery` to discover/verify this value.
#[allow(dead_code)]
const PR_TEST_COMMIT_HASH: &str = "5d40fb1555a0c28bf4d650515a73aaa54d4d9bfb";

use crate::{
    clone_repo, create_commit, create_deterministic_commit,
    create_deterministic_commit_with_variant, try_push, try_push_to_ref, AuditClient,
    CommitVariant, FixtureKind, TestContext, TestResult, DETERMINISTIC_COMMIT_HASH,
    MAINTAINER_DETERMINISTIC_COMMIT_HASH, RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH,
};
use nostr_sdk::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ============================================================
// PR Event Test Helper Functions
// ============================================================

/// Creates a deterministic PR test commit in the specified repository.
/// Returns the commit hash which should match PR_TEST_COMMIT_HASH.
///
/// This function handles:
/// 1. Creating an orphan branch (removes all history)
/// 2. Clearing staged files
/// 3. Creating deterministic commit using PRTestCommit variant
/// 4. Replacing main branch with the orphan branch
/// 5. Verifying the commit hash matches expected value
///
/// # Arguments
/// * `clone_path` - Path to the cloned repository
///
/// # Returns
/// * `Ok(String)` - The commit hash (should match PR_TEST_COMMIT_HASH)
/// * `Err(String)` - Error message if commit creation failed
fn create_pr_test_commit(clone_path: &Path) -> Result<String, String> {
    // Step 1: Clean up any tracked files in the working directory
    // This ensures we start with a clean slate
    let _ = Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(clone_path)
        .output();

    // Step 2: Create orphan branch (removes all history)
    let output = Command::new("git")
        .args(["checkout", "--orphan", "pr-test-branch"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Failed to execute git checkout --orphan: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "git checkout --orphan failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Step 3: Remove ALL files from the index (staging area)
    let output = Command::new("git")
        .args(["rm", "-rf", "--cached", "."])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Failed to execute git rm: {}", e))?;

    // Note: git rm may return error if there are no files to remove, that's OK
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Ignore "did not match any files" errors
        if !stderr.contains("did not match any files") {
            return Err(format!("git rm -rf --cached . failed: {}", stderr));
        }
    }

    // Step 4: Remove ALL files from working directory (except .git)
    // This ensures only test.txt will be in the commit
    for entry in fs::read_dir(clone_path).map_err(|e| format!("Failed to read dir: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name != ".git" {
            if path.is_dir() {
                fs::remove_dir_all(&path)
                    .map_err(|e| format!("Failed to remove dir {}: {}", path.display(), e))?;
            } else {
                fs::remove_file(&path)
                    .map_err(|e| format!("Failed to remove file {}: {}", path.display(), e))?;
            }
        }
    }

    // Step 5: Create deterministic commit using existing function
    let commit_hash =
        create_deterministic_commit_with_variant(clone_path, CommitVariant::PRTestCommit)?;

    // Step 6: Verify this is actually a root commit (no parent)
    let output = Command::new("git")
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Failed to check root commit: {}", e))?;

    let root_commits = String::from_utf8_lossy(&output.stdout);
    if !root_commits.trim().contains(&commit_hash) {
        return Err(format!(
            "Commit {} is not a root commit (has parent). Root commits: {}",
            commit_hash,
            root_commits.trim()
        ));
    }

    // Step 7: Replace main branch with our new orphan branch
    let _ = Command::new("git")
        .args(["branch", "-D", "main"])
        .current_dir(clone_path)
        .output();

    let output = Command::new("git")
        .args(["branch", "-m", "main"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Failed to rename branch: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "Failed to rename branch to main: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Step 8: Verify commit hash matches expected
    if commit_hash != PR_TEST_COMMIT_HASH {
        // Debug: Show what's in the commit
        let tree_output = Command::new("git")
            .args(["ls-tree", "-r", "HEAD"])
            .current_dir(clone_path)
            .output();
        let tree_info = tree_output
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_else(|_| "Failed to get tree".to_string());

        let cat_output = Command::new("git")
            .args(["cat-file", "-p", "HEAD"])
            .current_dir(clone_path)
            .output();
        let commit_info = cat_output
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_else(|_| "Failed to get commit".to_string());

        return Err(format!(
            "PR test commit hash mismatch: got {}, expected {}\nTree contents:\n{}\nCommit info:\n{}",
            commit_hash, PR_TEST_COMMIT_HASH, tree_info, commit_info
        ));
    }

    Ok(commit_hash)
}

/// Sets up a complete PR test repository with deterministic commit.
/// Returns: (clone_path, pr_event_id, repo_id, owner_npub)
///
/// This function handles the complete setup for PR event tests:
/// 1. Gets RepoAnnouncement and PREvent fixtures
/// 2. Extracts repo details (repo_id, owner_npub, pr_event_id)
/// 3. Clones the repository
/// 4. Creates the deterministic PR test commit
///
/// # Arguments
/// * `ctx` - The TestContext for fixture management
/// * `relay_url` - The relay URL for cloning (e.g., "localhost:7000")
///
/// # Returns
/// * `Ok((PathBuf, String, String, String))` - (clone_path, pr_event_id, repo_id, owner_npub)
/// * `Err(String)` - Error message if setup failed
#[allow(dead_code)]
async fn setup_pr_test_repo(
    ctx: &TestContext<'_>,
    relay_url: &str,
) -> Result<(PathBuf, String, String, String), String> {
    // Get fixtures
    let repo_event = ctx
        .get_fixture(FixtureKind::ValidRepo)
        .await
        .map_err(|e| format!("Failed to get repo announcement: {}", e))?;

    let pr_event = ctx
        .get_fixture(FixtureKind::PREvent)
        .await
        .map_err(|e| format!("Failed to get PR event: {}", e))?;

    // Extract repo details using nostr-sdk 0.43 API (field access)
    let repo_id = repo_event
        .tags
        .iter()
        .find(|t| t.kind() == TagKind::d())
        .and_then(|t| t.content())
        .ok_or("No repo identifier in announcement")?
        .to_string();

    let owner_npub = repo_event.pubkey.to_bech32().map_err(|e| e.to_string())?;
    let pr_event_id = pr_event.id.to_hex();

    // Clone the repository
    let clone_path = clone_repo(relay_url, &owner_npub, &repo_id)?;

    // Create the PR test commit
    create_pr_test_commit(&clone_path)?;

    Ok((clone_path, pr_event_id, repo_id, owner_npub))
}

// ============================================================
// PR Ref Push Test Setup Helpers - Minimize Test Duplication
// ============================================================

/// Result of setting up a repo with a wrong commit pushed before PR event exists.
/// Used as shared setup for tests 3, 4, 5 which all depend on this scenario.
#[allow(dead_code)]
struct PrRefTestSetup {
    clone_path: PathBuf,
    pr_event_id: String,
    repo_id: String,
    owner_npub: String,
    wrong_commit_hash: String,
    /// The unpublished PR event - store it so we can publish the SAME event later
    pr_event: Event,
}

impl PrRefTestSetup {
    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.clone_path);
    }
}

/// Sets up a repo and pushes a WRONG commit to refs/nostr/<pr-event-id> BEFORE PR event exists.
///
/// This is the shared setup for PR ref lifecycle tests:
/// - Creates repo (gets PREvent fixture for event-id but doesn't publish yet)
/// - Clones repo
/// - Creates a commit that does NOT match PR_TEST_COMMIT_HASH
/// - Pushes to refs/nostr/<pr-event-id> (should succeed - no event to validate against)
///
/// Tests using this setup:
/// - test_pr_push_to_nostr_ref_with_wrong_commit_accepted_before_event_received: verify initial push accepted
/// - test_pr_event_published_removes_nostr_ref_at_incorrect_commit: publish event, verify cleanup
/// - test_push_to_nostr_ref_with_wrong_commit_after_event_received_rejected: publish event, try push wrong commit
/// - test_push_to_nostr_ref_with_correct_commit_after_event_received_accepted: publish event, push correct commit
#[allow(dead_code)]
async fn setup_repo_with_wrong_commit_pushed(
    ctx: &TestContext<'_>,
    relay_domain: &str,
) -> Result<PrRefTestSetup, String> {
    // Get ValidRepo fixture (publishes repo announcement to relay)
    let repo_event = ctx
        .get_fixture(FixtureKind::ValidRepo)
        .await
        .map_err(|e| format!("Failed to get repo announcement: {}", e))?;

    // Build PR event WITHOUT publishing - we need its ID before the event exists on relay
    // This allows testing refs/nostr/<event-id> push behavior before the event is received
    let pr_event = ctx
        .build_fixture_only(FixtureKind::PREvent)
        .await
        .map_err(|e| format!("Failed to build PR event fixture: {}", e))?;

    let repo_id = repo_event
        .tags
        .iter()
        .find(|t| t.kind() == TagKind::d())
        .and_then(|t| t.content())
        .ok_or("No repo identifier in announcement")?
        .to_string();

    let owner_npub = repo_event.pubkey.to_bech32().map_err(|e| e.to_string())?;
    let pr_event_id = pr_event.id.to_hex();

    // Clone the repository
    let clone_path = clone_repo(relay_domain, &owner_npub, &repo_id)?;

    // Create a WRONG commit (not the one expected by PR event)
    let wrong_commit_hash =
        create_deterministic_commit_with_variant(&clone_path, CommitVariant::Owner)?;

    // Verify it's actually different from expected
    if wrong_commit_hash == PR_TEST_COMMIT_HASH {
        let _ = std::fs::remove_dir_all(&clone_path);
        return Err("Test setup error: wrong_commit_hash equals PR_TEST_COMMIT_HASH".to_string());
    }

    // Push to refs/nostr/<pr-event-id> (no event published yet, should succeed)
    let push_output = Command::new("git")
        .args([
            "push",
            "origin",
            &format!("master:refs/nostr/{}", pr_event_id),
        ])
        .current_dir(&clone_path)
        .output()
        .map_err(|e| format!("Failed to execute git push: {}", e))?;

    if !push_output.status.success() {
        let stderr = String::from_utf8_lossy(&push_output.stderr);
        let _ = std::fs::remove_dir_all(&clone_path);
        return Err(format!(
            "Initial push failed (expected success before PR event): {}",
            stderr
        ));
    }

    Ok(PrRefTestSetup {
        clone_path,
        pr_event_id,
        repo_id,
        owner_npub,
        wrong_commit_hash,
        pr_event,
    })
}

/// Publishes the SAME PR event that was built during setup.
/// Call this after setup_repo_with_wrong_commit_pushed to test post-event behavior.
///
/// IMPORTANT: We must publish the EXACT same event that was used during setup,
/// otherwise the event ID won't match the refs/nostr/<event-id> ref that was pushed.
#[allow(dead_code)]
async fn publish_pr_event_and_wait(ctx: &TestContext<'_>, pr_event: &Event) -> Result<(), String> {
    // Publish the exact same PR event that was created during setup
    ctx.client()
        .send_event(pr_event.clone())
        .await
        .map_err(|e| format!("Failed to publish PR event: {}", e))?;

    // Wait for relay to process
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    Ok(())
}

/// Creates the correct PR test commit (matching PR_TEST_COMMIT_HASH) in an existing clone.
/// Used after wrong commit was pushed to test pushing the correct commit.
#[allow(dead_code)]
fn reset_to_correct_pr_commit(clone_path: &Path) -> Result<String, String> {
    // Create the correct PR test commit (replaces current state)
    create_pr_test_commit(clone_path)
}

/// Attempts to push current HEAD to refs/nostr/<pr-event-id>.
/// Returns Ok(true) if push succeeded, Ok(false) if rejected, Err on git error.
#[allow(dead_code)]
fn push_to_pr_ref(clone_path: &Path, pr_event_id: &str) -> Result<bool, String> {
    let push_output = Command::new("git")
        .args([
            "push",
            "--force",
            "origin",
            &format!("HEAD:refs/nostr/{}", pr_event_id),
        ])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Failed to execute git push: {}", e))?;

    Ok(push_output.status.success())
}

/// Queries the git smart HTTP info/refs endpoint to determine the default branch.
///
/// This parses the git-upload-pack service response to find the symref=HEAD capability
/// which indicates what branch HEAD points to (i.e., the default branch).
///
/// # Arguments
/// * `relay_domain` - The relay domain (e.g., "localhost:7000")
/// * `npub` - The owner's npub (bech32 public key)
/// * `repo_id` - The repository identifier
///
/// # Returns
/// * `Ok(String)` - The default branch ref (e.g., "refs/heads/main")
/// * `Err(String)` - Error message if request or parsing failed
async fn get_default_branch_from_info_refs(
    relay_domain: &str,
    npub: &str,
    repo_id: &str,
) -> Result<String, String> {
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
            "info/refs returned status {} for URL: {}",
            response.status(),
            info_refs_url
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    // Parse the git smart HTTP response to find symref=HEAD:refs/heads/xxx
    // The format is: capabilities are space-separated after the first NUL byte
    // Example line: 0000000000000000000000000000000000000000 capabilities^{}\0symref=HEAD:refs/heads/master ...
    for line in body.lines() {
        if let Some(caps_start) = line.find('\0') {
            let caps = &line[caps_start + 1..];
            for cap in caps.split(' ') {
                if cap.starts_with("symref=HEAD:") {
                    let branch = cap.trim_start_matches("symref=HEAD:");
                    return Ok(branch.to_string());
                }
            }
        }
    }

    Err("No symref=HEAD capability found in info/refs response".to_string())
}

/// Checks if a ref exists on the remote.
#[allow(dead_code)]
fn ref_exists_on_remote(clone_path: &Path, ref_name: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .args(["ls-remote", "origin", ref_name])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Failed to execute git ls-remote: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(!stdout.trim().is_empty())
}

/// Test suite for Push Authorization operations
pub struct PushAuthorizationTests;

impl PushAuthorizationTests {
    /// Run all push authorization tests
    pub async fn run_all(client: &AuditClient, relay_domain: &str) -> crate::AuditResult {
        let mut results = crate::AuditResult::new("GRASP-01 Push Authorization Tests");

        results.add(Self::test_push_rejected_without_state_event(client, relay_domain).await);
        results.add(Self::test_push_authorized_by_owner_state(client, relay_domain).await);
        results.add(Self::test_push_rejected_wrong_commit(client, relay_domain).await);
        results
            .add(Self::test_push_authorized_by_maintainer_state_only(client, relay_domain).await);
        results.add(
            Self::test_push_authorized_by_recursive_maintainer_state(client, relay_domain).await,
        );
        results.add(
            Self::test_push_to_nostr_ref_with_invalid_event_id_rejected(client, relay_domain).await,
        );
        results.add(
            Self::test_pr_push_to_nostr_ref_with_wrong_commit_accepted_before_event_received(
                client,
                relay_domain,
            )
            .await,
        );
        results.add(
            Self::test_pr_event_published_removes_nostr_ref_at_incorrect_commit(
                client,
                relay_domain,
            )
            .await,
        );
        results.add(
            Self::test_push_to_nostr_ref_with_wrong_commit_after_event_received_rejected(
                client,
                relay_domain,
            )
            .await,
        );
        results.add(
            Self::test_push_to_nostr_ref_with_correct_commit_after_event_received_accepted(
                client,
                relay_domain,
            )
            .await,
        );
        results.add(
            Self::test_head_set_after_state_event_with_existing_commit(client, relay_domain).await,
        );
        results
            .add(Self::test_head_set_after_git_push_with_required_oids(client, relay_domain).await);

        results
    }

    /// Test that push is rejected when no state event exists
    pub async fn test_push_rejected_without_state_event(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_rejected_without_state_event";
        let ctx = TestContext::new(client);

        // Create repository (no state event)
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected without state event")
                    .fail(format!("Failed to create repo: {}", e))
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let repo_id = repo
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .unwrap()
            .to_string();
        let npub = repo.pubkey.to_bech32().unwrap();

        // Clone and create commit
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", "Push rejected without state event")
                    .fail(&e)
            }
        };
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        if let Err(e) = create_commit(&clone_path, "Unauthorized commit") {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Push rejected without state event")
                .fail(&e);
        }

        // Do NOT publish state event - push should be rejected
        let push_result = try_push(&clone_path);
        cleanup();

        match push_result {
            Ok(false) => {
                TestResult::new(test_name, "GRASP-01", "Push rejected without state event").pass()
            }
            Ok(true) => TestResult::new(test_name, "GRASP-01", "Push rejected without state event")
                .fail("Push accepted but should be rejected"),
            Err(e) => {
                TestResult::new(test_name, "GRASP-01", "Push rejected without state event").fail(&e)
            }
        }
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized with matching state",
                )
                .fail(format!("Failed to create RepoState fixture: {}", e));
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized with matching state",
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
                    "Push authorized with matching state",
                )
                .fail(format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // ============================================================
        // Step 2: SEND - Clone repo, create deterministic commit, push
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized with matching state",
                )
                .fail(format!("Failed to clone repo: {}", e));
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized with matching state",
                )
                .fail(format!("Failed to create deterministic commit: {}", e));
            }
        };

        // Verify commit hash matches expected
        if commit_hash != DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                .fail(format!(
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized with matching state",
                )
                .fail(format!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized with matching state",
                )
                .fail(format!(
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
                    "Push authorized with matching state",
                )
                .fail(format!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized with matching state",
                )
                .fail(format!(
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
                    format!(
                        "Push was rejected but should have been accepted. \
                        The state event points to commit {} which matches the pushed commit.",
                        DETERMINISTIC_COMMIT_HASH
                    ),
                )
            }
            Err(e) => TestResult::new(test_name, "GRASP-01", "Push authorized with matching state")
                .fail(format!("Push error: {}", e)),
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
    ///    (repo announcement + state event pointing to DETERMINISTIC_COMMIT_HASH)
    /// 2. **Send**: Clone repo, create WRONG deterministic commit (Maintainer variant),
    ///    try to push
    /// 3. **Verify**: Push should be rejected because the commit doesn't match state event
    ///
    /// Note: This test directly pushes the wrong commit instead of first establishing
    /// state on the relay. The state event already authorizes DETERMINISTIC_COMMIT_HASH,
    /// but we try to push MAINTAINER_DETERMINISTIC_COMMIT_HASH which should be rejected.
    pub async fn test_push_rejected_wrong_commit(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        use std::process::Command;

        let test_name = "test_push_rejected_wrong_commit";

        // ============================================================
        // Step 1: GENERATE - Create TestContext and get RepoState fixture
        // The state event points to DETERMINISTIC_COMMIT_HASH
        // ============================================================
        let ctx = TestContext::new(client);

        let state_event = match ctx.get_fixture(FixtureKind::RepoState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push rejected when commit not in state event",
                )
                .fail(format!("Failed to create RepoState fixture: {}", e));
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push rejected when commit not in state event",
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
                    "Push rejected when commit not in state event",
                )
                .fail(format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // ============================================================
        // Step 2: SEND - Clone repo and create an unauthorized commit
        // Any commit with a hash different from what's in the state event will work
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push rejected when commit not in state event",
                )
                .fail(format!("Failed to clone repo: {}", e));
            }
        };

        // Cleanup helper
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // Create/checkout main branch
        let branch_output = Command::new("git")
            .args(["checkout", "-B", "main"])
            .current_dir(&clone_path)
            .output();

        match branch_output {
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push rejected when commit not in state event",
                )
                .fail(format!("Failed to create/checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push rejected when commit not in state event",
                )
                .fail(format!(
                    "Failed to create/checkout main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            _ => {}
        }

        // Create a commit that is NOT in the state event
        // Any commit hash different from what's authorized in the state event will work
        if let Err(e) = create_commit(&clone_path, "Unauthorized commit - should be rejected") {
            cleanup();
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push rejected when commit not in state event",
            )
            .fail(format!("Failed to create wrong commit: {}", e));
        }

        // ============================================================
        // Step 3: VERIFY - Push should be rejected because the commit
        // doesn't match the state event
        // ============================================================
        let push_result = try_push(&clone_path);
        cleanup();

        match push_result {
            Ok(false) => TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event").pass(),
            Ok(true) => TestResult::new(test_name, "GRASP-01", "Push rejected when commit not in state event")
                .fail("Push accepted but should be rejected. The pushed commit is not in the state event."),
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
                .fail(format!("Failed to create RepoState fixture: {}", e));
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
                .fail(format!("Failed to create MaintainerState fixture: {}", e));
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
                .fail(format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

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

        // Reset to orphan state and create deterministic root commit
        // Step 1: Create orphan branch (removes all history)
        let _ = Command::new("git")
            .args(["checkout", "--orphan", "main-new"])
            .current_dir(&clone_path)
            .output();

        // Step 2: Clear staged files (orphan keeps files staged from previous branch)
        let _ = Command::new("git")
            .args(["rm", "-rf", "--cached", "."])
            .current_dir(&clone_path)
            .output();

        // Step 3: Create deterministic commit using existing function
        let commit_hash = match create_deterministic_commit_with_variant(
            &clone_path,
            CommitVariant::Maintainer,
        ) {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by maintainer state event only (no announcement)",
                )
                .fail(format!("Failed to create maintainer commit: {}", e));
            }
        };

        // Step 4: Replace main branch with our new orphan branch
        let _ = Command::new("git")
            .args(["branch", "-D", "main"])
            .current_dir(&clone_path)
            .output();

        let _ = Command::new("git")
            .args(["branch", "-m", "main"])
            .current_dir(&clone_path)
            .output();

        // Verify commit hash matches expected
        if commit_hash != MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by maintainer state event only (no announcement)",
            )
            .fail(format!(
                "Maintainer commit hash mismatch: got {}, expected {}",
                commit_hash, MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
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
            .fail(format!(
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
            .fail(format!("Push error: {}", e)),
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
                .fail(format!("Failed to create RepoState fixture: {}", e));
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
                .fail(format!(
                    "Failed to create MaintainerAnnouncement fixture: {}",
                    e
                ));
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
                .fail(format!("Failed to create MaintainerState fixture: {}", e));
            }
        };

        // Get RecursiveMaintainerRepoAndState fixture (completes 3-level delegation chain)
        match ctx
            .get_fixture(FixtureKind::RecursiveMaintainerRepoAndState)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(format!(
                    "Failed to create RecursiveMaintainerRepoAndState fixture: {}",
                    e
                ));
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
                .fail(format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

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

        // Reset to orphan state and create deterministic root commit
        // Step 1: Create orphan branch (removes all history)
        let _ = Command::new("git")
            .args(["checkout", "--orphan", "main-new"])
            .current_dir(&clone_path)
            .output();

        // Step 2: Clear staged files (orphan keeps files staged from previous branch)
        let _ = Command::new("git")
            .args(["rm", "-rf", "--cached", "."])
            .current_dir(&clone_path)
            .output();

        // Step 3: Create recursive maintainer deterministic commit
        let commit_hash = match create_deterministic_commit_with_variant(
            &clone_path,
            CommitVariant::RecursiveMaintainer,
        ) {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push authorized by recursive maintainer state event",
                )
                .fail(format!(
                    "Failed to create recursive maintainer commit: {}",
                    e
                ));
            }
        };

        // Step 4: Replace main branch with our new orphan branch
        let _ = Command::new("git")
            .args(["branch", "-D", "main"])
            .current_dir(&clone_path)
            .output();

        let _ = Command::new("git")
            .args(["branch", "-m", "main"])
            .current_dir(&clone_path)
            .output();

        // Verify commit hash matches expected
        if commit_hash != RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push authorized by recursive maintainer state event",
            )
            .fail(format!(
                "Recursive maintainer commit hash mismatch: got {}, expected {}",
                commit_hash, RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
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
            .fail(format!(
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
            .fail(format!("Push error: {}", e)),
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to create RepoState fixture: {}", e));
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
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
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        // ============================================================
        // Step 2: SEND - Clone repo, create deterministic commit, push
        // (establishes the state on the relay)
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to clone repo: {}", e));
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to create deterministic commit: {}", e));
            }
        };

        // Verify commit hash matches expected
        if commit_hash != DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                .fail(format!(
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!(
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
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!(
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to push initial commit: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!(
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to create commit: {}", e));
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
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Non-maintainer state events ignored",
                )
                .fail(format!("Failed to build rogue state event: {}", e));
            }
        };

        // Send the rogue state event using the raw client to bypass AuditClient's key check
        if let Err(e) = client.client().send_event(&rogue_state).await {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", "Non-maintainer state events ignored")
                .fail(format!("Failed to send rogue state event: {}", e));
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
                .fail(format!(
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

    /// Test that push to refs/nostr/<invalid> is rejected with invalid EventId format
    ///
    /// GRASP-01: "MUST accept pushes via this service to `refs/nostr/<event-id>`"
    /// The event_id must parse as a valid rust-nostr EventId (64-char hex string).
    /// Invalid formats (too short, non-hex, etc.) should be rejected.
    ///
    /// ## Fixture-First Pattern
    ///
    /// 1. **Generate**: Create repo with ValidRepo fixture (no state event needed)
    /// 2. **Send**: Clone repo, create commit, try to push to refs/nostr/123 (invalid)
    /// 3. **Verify**: Push should be rejected because event-id format is invalid
    pub async fn test_push_to_nostr_ref_with_invalid_event_id_rejected(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_to_nostr_ref_with_invalid_event_id_rejected";

        // ============================================================
        // Step 1: GENERATE - Create repo (no state event needed for refs/nostr/)
        // ============================================================
        let ctx = TestContext::new(client);

        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push to refs/nostr/<invalid-event-id> rejected",
                )
                .fail(format!("Failed to create repo: {}", e));
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let repo_id = repo
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .unwrap()
            .to_string();
        let npub = repo.pubkey.to_bech32().unwrap();

        // ============================================================
        // Step 2: SEND - Clone repo, create commit, try push to invalid ref
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Push to refs/nostr/<invalid-event-id> rejected",
                )
                .fail(&e);
            }
        };
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // Create a unique commit
        if let Err(e) = create_commit(&clone_path, "Test commit for invalid refs/nostr push") {
            cleanup();
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Push to refs/nostr/<invalid-event-id> rejected",
            )
            .fail(&e);
        }

        // Use an invalid event-id (too short, not a valid 64-char hex)
        let invalid_event_id = "123";
        let ref_name = format!("refs/nostr/{}", invalid_event_id);

        // ============================================================
        // Step 3: VERIFY - Push should be rejected with invalid event-id format
        // ============================================================
        let push_result = try_push_to_ref(&clone_path, &ref_name);
        cleanup();

        match push_result {
            Ok(false) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push to refs/nostr/<invalid-event-id> rejected",
            )
            .pass(),
            Ok(true) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push to refs/nostr/<invalid-event-id> rejected",
            )
            .fail(format!(
                "Push to {} was accepted but should be rejected. \
                The event-id '{}' is NOT a valid 64-character hex string (EventId format). \
                The relay should reject pushes to refs/nostr/ with invalid event-id format.",
                ref_name, invalid_event_id
            )),
            Err(e) => TestResult::new(
                test_name,
                "GRASP-01",
                "Push to refs/nostr/<invalid-event-id> rejected",
            )
            .fail(format!("Push error: {}", e)),
        }
    }

    /// Test 1: Push wrong commit to refs/nostr/<pr-event-id> BEFORE PR event is published
    ///
    /// This test verifies that the relay accepts pushes to refs/nostr/<event-id>
    /// when no corresponding event exists yet. This is expected behavior because
    /// there's no validation event to check against.
    ///
    /// Uses `setup_repo_with_wrong_commit_pushed` helper which handles all setup.
    pub async fn test_pr_push_to_nostr_ref_with_wrong_commit_accepted_before_event_received(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name =
            "test_pr_push_to_nostr_ref_with_wrong_commit_accepted_before_event_received";
        let desc = "Push wrong commit to refs/nostr/<pr-event-id> before PR event (should accept)";
        let ctx = TestContext::new(client);

        // Setup includes: create repo, clone, create wrong commit, push to refs/nostr/<event-id>
        // The push happens BEFORE PR event is published, so should succeed
        let setup = match setup_repo_with_wrong_commit_pushed(&ctx, relay_domain).await {
            Ok(s) => s,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
            }
        };

        // Setup already pushed and verified success - just cleanup and report pass
        setup.cleanup();

        TestResult::new(test_name, "GRASP-01", desc).pass()
    }

    /// Test 2: After publishing PR event, verify that incorrect refs get cleaned up
    ///
    /// This test verifies the expected behavior: when a PR event is published,
    /// the relay should validate any existing refs/nostr/<event-id> refs and
    /// delete those that don't match the commit in the PR event's `c` tag.
    ///
    /// Depends on: `setup_repo_with_wrong_commit_pushed` (wrong commit already pushed)
    pub async fn test_pr_event_published_removes_nostr_ref_at_incorrect_commit(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_pr_event_published_removes_nostr_ref_at_incorrect_commit";
        let desc = "Publishing PR event should trigger cleanup of incorrect refs";
        let ctx = TestContext::new(client);

        // Setup: wrong commit already pushed to refs/nostr/<pr-event-id>
        let setup = match setup_repo_with_wrong_commit_pushed(&ctx, relay_domain).await {
            Ok(s) => s,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
            }
        };

        // NOW publish the PR event - this should trigger cleanup validation
        if let Err(e) = publish_pr_event_and_wait(&ctx, &setup.pr_event).await {
            setup.cleanup();
            return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
        }

        // Check if the incorrect ref was deleted
        let ref_name = format!("refs/nostr/{}", setup.pr_event_id);
        let refs_exist = match ref_exists_on_remote(&setup.clone_path, &ref_name) {
            Ok(exists) => exists,
            Err(e) => {
                setup.cleanup();
                return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
            }
        };

        setup.cleanup();

        // Ref should be deleted since the pushed commit doesn't match the PR event's `c` tag
        if refs_exist {
            TestResult::new(test_name, "GRASP-01", desc).fail(format!(
                "Expected refs/nostr/{} to be deleted when PR event published with non-matching commit, \
                 but the ref still exists. The relay should delete refs that don't match the event's `c` tag.",
                setup.pr_event_id
            ))
        } else {
            TestResult::new(test_name, "GRASP-01", desc).pass()
        }
    }

    /// Test 3: Push wrong commit to refs/nostr/<pr-event-id> AFTER PR event exists
    ///
    /// This test verifies that the relay rejects pushes to refs/nostr/<event-id>
    /// when a corresponding event exists but the pushed commit doesn't match
    /// the commit in the PR event's `c` tag.
    ///
    /// Depends on: `setup_repo_with_wrong_commit_pushed` for repo/clone setup, then publishes PR event
    pub async fn test_push_to_nostr_ref_with_wrong_commit_after_event_received_rejected(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_to_nostr_ref_with_wrong_commit_after_event_received_rejected";
        let desc = "Push wrong commit to refs/nostr/<pr-event-id> after PR event (should reject)";
        let ctx = TestContext::new(client);

        // Setup: wrong commit already pushed (we'll use the same setup, but publish PR first)
        let setup = match setup_repo_with_wrong_commit_pushed(&ctx, relay_domain).await {
            Ok(s) => s,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
            }
        };

        // Publish PR event FIRST (before our test push)
        if let Err(e) = publish_pr_event_and_wait(&ctx, &setup.pr_event).await {
            setup.cleanup();
            return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
        }

        // Try to push again with wrong commit (should be rejected now that PR event exists)
        let push_succeeded = match push_to_pr_ref(&setup.clone_path, &setup.pr_event_id) {
            Ok(success) => success,
            Err(e) => {
                setup.cleanup();
                return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
            }
        };

        setup.cleanup();

        // Should REJECT - PR event exists with different commit hash
        if push_succeeded {
            return TestResult::new(test_name, "GRASP-01", desc)
                .fail("Push accepted (expected rejection due to commit hash mismatch)");
        }

        TestResult::new(test_name, "GRASP-01", desc).pass()
    }

    /// Test 4: Push correct commit to refs/nostr/<pr-event-id> AFTER PR event exists
    ///
    /// This test verifies that the relay accepts pushes to refs/nostr/<event-id>
    /// when a corresponding event exists AND the pushed commit matches
    /// the commit in the PR event's `c` tag.
    ///
    /// Depends on: `setup_repo_with_wrong_commit_pushed` for setup, then resets to correct commit
    pub async fn test_push_to_nostr_ref_with_correct_commit_after_event_received_accepted(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_push_to_nostr_ref_with_correct_commit_after_event_received_accepted";
        let desc = "Push correct commit to refs/nostr/<pr-event-id> after PR event (should accept)";
        let ctx = TestContext::new(client);

        // Setup: wrong commit already pushed
        let setup = match setup_repo_with_wrong_commit_pushed(&ctx, relay_domain).await {
            Ok(s) => s,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
            }
        };

        // Publish PR event FIRST
        if let Err(e) = publish_pr_event_and_wait(&ctx, &setup.pr_event).await {
            setup.cleanup();
            return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
        }

        // Reset to CORRECT commit (the one expected by PR event)
        if let Err(e) = reset_to_correct_pr_commit(&setup.clone_path) {
            setup.cleanup();
            return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
        }

        // Push correct commit (should succeed)
        let push_succeeded = match push_to_pr_ref(&setup.clone_path, &setup.pr_event_id) {
            Ok(success) => success,
            Err(e) => {
                setup.cleanup();
                return TestResult::new(test_name, "GRASP-01", desc).fail(&e);
            }
        };

        setup.cleanup();

        // Should ACCEPT - commit matches PR event's c tag
        if !push_succeeded {
            return TestResult::new(test_name, "GRASP-01", desc)
                .fail("Push rejected (expected acceptance since commit matches PR event)");
        }

        TestResult::new(test_name, "GRASP-01", desc).pass()
    }

    /// Test that HEAD is set after a state event is published with an existing commit
    ///
    /// GRASP-01: "MUST set repository HEAD per repository state announcement
    /// as soon as the git data related to that branch has been received."
    ///
    /// This test verifies the HEAD-setting behavior when:
    /// 1. A maintainer commit is pushed to the relay (git data exists)
    /// 2. A state event is published pointing to that commit with HEAD="refs/heads/develop"
    /// 3. The relay should update the repository's default branch to "develop"
    ///
    /// ## Fixture-First Pattern
    ///
    /// 1. **Generate**: Create TestContext and get RepoState + MaintainerState fixtures
    ///    (both commits are pushed as part of the fixture setup)
    /// 2. **Send**: Push maintainer commit to relay first, then publish state event with HEAD=develop
    /// 3. **Verify**: Query info/refs to verify HEAD symref points to refs/heads/develop
    pub async fn test_head_set_after_state_event_with_existing_commit(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        use std::process::Command;

        let test_name = "test_head_set_after_state_event_with_existing_commit";
        let desc = "HEAD is set when state event published with existing commit";

        // ============================================================
        // Step 1: GENERATE - Create TestContext and get fixtures
        // ============================================================
        let ctx = TestContext::new(client);

        // Get RepoState fixture (owner's repo announcement + state event)
        let state_event = match ctx.get_fixture(FixtureKind::RepoState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail(format!("Failed to create RepoState fixture: {}", e));
            }
        };

        // Extract repo_id and npub from owner's state event
        let repo_id = match state_event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            Some(id) => id.to_string(),
            None => {
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail("Missing repo_id in state event");
            }
        };

        let npub = match state_event.pubkey.to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail(format!("Failed to convert pubkey to bech32: {}", e));
            }
        };

        let _maintainer_ann_event = match ctx.get_fixture(FixtureKind::MaintainerAnnouncement).await
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc).fail(format!(
                    "Failed to create MaintainerAnnouncement fixture: {}",
                    e
                ));
            }
        };

        let _maintainer_state_event = match ctx.get_fixture(FixtureKind::MaintainerState).await {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail(format!("Failed to create MaintainerState fixture: {}", e));
            }
        };

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // ============================================================
        // Step 2: SEND - First push maintainer commit so relay has the git data
        // ============================================================
        let clone_path = match clone_repo(relay_domain, &npub, &repo_id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail(format!("Failed to clone repo: {}", e));
            }
        };

        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        // TODO - this should be pushed inside the MaintainerState fixture.

        // Reset to orphan state and create deterministic root commit
        // Step 1: Create orphan branch (removes all history)
        let _ = Command::new("git")
            .args(["checkout", "--orphan", "main"])
            .current_dir(&clone_path)
            .output();

        // Step 2: Clear staged files (orphan keeps files staged from previous branch)
        let _ = Command::new("git")
            .args(["rm", "-rf", "--cached", "."])
            .current_dir(&clone_path)
            .output();

        // Step 3: Create deterministic commit using Maintainer variant
        let commit_hash = match create_deterministic_commit_with_variant(
            &clone_path,
            CommitVariant::Maintainer,
        ) {
            Ok(h) => h,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail(format!("Failed to create maintainer commit: {}", e));
            }
        };

        // Verify commit hash matches expected
        if commit_hash != MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", desc).fail(format!(
                "Maintainer commit hash mismatch: got {}, expected {}",
                commit_hash, MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
        }

        // Push the develop branch with the maintainer commit
        let push_output = Command::new("git")
            .args(["push", "origin", "main"])
            .current_dir(&clone_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output();

        match push_output {
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail(format!("Failed to push develop branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                // this will fail when not in isolation - as the Recusive state will be the authorised state
                // but we need to do it here so the grasp server has the oid
            }
            _ => {}
        }

        let _recursive_maintainer_ann_event = match ctx
            .get_fixture(FixtureKind::RecursiveMaintainerAnnouncement)
            .await
        {
            Ok(e) => e,
            Err(e) => {
                return TestResult::new(test_name, "GRASP-01", desc).fail(format!(
                    "Failed to create RecursiveMaintainerAnnouncement fixture: {}",
                    e
                ));
            }
        };

        let _recursive_maintainer_state_event =
            match ctx.get_fixture(FixtureKind::RecursiveMaintainerState).await {
                Ok(e) => e,
                Err(e) => {
                    return TestResult::new(test_name, "GRASP-01", desc)
                        .fail(format!("Failed to create MaintainerState fixture: {}", e));
                }
            };

        // Verify commit hash matches expected
        if commit_hash != MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", desc).fail(format!(
                "Maintainer commit hash mismatch: got {}, expected {}",
                commit_hash, MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
        }

        // Reset to orphan state and create deterministic root commit
        // Step 1: Create orphan branch (removes all history)
        let _ = Command::new("git")
            .args(["checkout", "--orphan", "develop"])
            .current_dir(&clone_path)
            .output();

        // Step 2: Clear staged files (orphan keeps files staged from previous branch)
        let _ = Command::new("git")
            .args(["rm", "-rf", "--cached", "."])
            .current_dir(&clone_path)
            .output();

        // ============================================================
        // Step 3: Publish state event with HEAD pointing to develop branch
        // ============================================================

        // Create state event with HEAD=refs/heads/develop and develop branch pointing to maintainer commit
        let state_event = match client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("HEAD"),
                vec!["refs/heads/develop".to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/develop"),
                vec![MAINTAINER_DETERMINISTIC_COMMIT_HASH.to_string()],
            ))
            .build(client.maintainer_keys())
        {
            Ok(e) => e,
            Err(e) => {
                cleanup();
                return TestResult::new(test_name, "GRASP-01", desc)
                    .fail(format!("Failed to build state event: {}", e));
            }
        };

        // Send the state event
        if let Err(e) = client.client().send_event(&state_event).await {
            cleanup();
            return TestResult::new(test_name, "GRASP-01", desc)
                .fail(format!("Failed to send state event: {}", e));
        }

        // Wait for relay to process the state event
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // // Now that state event is published, try pushing again if previous push failed
        // let push_output = Command::new("git")
        //     .args(["push", "-f", "origin", "develop"])
        //     .current_dir(&clone_path)
        //     .env("GIT_TERMINAL_PROMPT", "0")
        //     .output();

        // match push_output {
        //     Err(e) => {
        //         cleanup();
        //         return TestResult::new(test_name, "GRASP-01", desc).fail(format!(
        //             "Failed to push develop branch after state event: {}",
        //             e
        //         ));
        //     }
        //     Ok(output) if !output.status.success() => {
        //         cleanup();
        //         return TestResult::new(test_name, "GRASP-01", desc).fail(format!(
        //             "Push of develop branch rejected after state event: {}",
        //             String::from_utf8_lossy(&output.stderr)
        //         ));
        //     }
        //     _ => {}
        // }

        cleanup();

        // Wait a bit more for HEAD to be updated
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // ============================================================
        // Step 4: VERIFY - Query info/refs to check the default branch
        // ============================================================
        let default_branch =
            match get_default_branch_from_info_refs(relay_domain, &npub, &repo_id).await {
                Ok(branch) => branch,
                Err(e) => {
                    return TestResult::new(test_name, "GRASP-01", desc)
                        .fail(format!("Failed to get default branch: {}", e));
                }
            };

        // Verify HEAD points to refs/heads/develop
        if default_branch == "refs/heads/develop" {
            TestResult::new(test_name, "GRASP-01", desc).pass()
        } else {
            TestResult::new(test_name, "GRASP-01", desc).fail(format!(
                "Expected HEAD to point to 'refs/heads/develop' but got '{}'. \
                GRASP-01 requires: 'MUST set repository HEAD per repository state announcement \
                as soon as the git data related to that branch has been received.'",
                default_branch
            ))
        }
    }

    /// Test that HEAD is set after git push with oids
    pub async fn test_head_set_after_git_push_with_required_oids(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_head_set_after_git_push_with_required_oids";
        let desc = "HEAD is set to match state event when git push sends required oids to formulate branch";

        // DO the above as prep. then create a unique commit, create state event with HEAD=develop1 branch at unqiue commit
        // git push the new develop1 branch. then check HEAD with this:
        // let default_branch =
        //     match get_default_branch_from_info_refs(relay_domain, &npub, &repo_id).await {
        //         Ok(branch) => branch,
        //         Err(e) => {
        //             return TestResult::new(test_name, "GRASP-01", desc)
        //                 .fail(format!("Failed to get default branch: {}", e));
        //         }
        //     };

        // // Verify HEAD points to refs/heads/develop1
        // if default_branch == "refs/heads/develop1" {
        //     TestResult::new(test_name, "GRASP-01", desc).pass()
        // } else {
        //     TestResult::new(test_name, "GRASP-01", desc).fail(format!(
        //         "Expected HEAD to point to 'refs/heads/develop' but got '{}'. \
        //         GRASP-01 requires: 'MUST set repository HEAD per repository state announcement \
        //         as soon as the git data related to that branch has been received.'",
        //         default_branch
        //     ))
        // }
        TestResult::new(test_name, "GRASP-01", desc).fail("test not implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test to discover the PR test commit hash
    ///
    /// This test creates a deterministic commit with PR-specific parameters
    /// and prints out the hash value. Once discovered, update PR_TEST_COMMIT_HASH.
    ///
    /// Run with: cd grasp-audit && nix develop -c cargo test --lib test_pr_test_commit_hash_discovery -- --nocapture
    #[test]
    fn test_pr_test_commit_hash_discovery() {
        use std::fs;
        use std::process::Command;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let path = temp_dir.path();

        // Initialize git repo
        let output = Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("Failed to init git");
        assert!(
            output.status.success(),
            "git init failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Configure git user - use PR Test Author identity
        let output = Command::new("git")
            .args(["config", "user.email", "pr-test@example.com"])
            .current_dir(path)
            .output()
            .expect("git config email failed");
        assert!(output.status.success(), "git config email failed");

        let output = Command::new("git")
            .args(["config", "user.name", "PR Test Author"])
            .current_dir(path)
            .output()
            .expect("git config name failed");
        assert!(output.status.success(), "git config name failed");

        // Create the deterministic file content
        let test_file = path.join("test.txt");
        fs::write(&test_file, "PR test deterministic commit").expect("Failed to write test file");

        // Add the file
        let output = Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(path)
            .output()
            .expect("git add failed");
        assert!(
            output.status.success(),
            "git add failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Create deterministic commit with fixed dates and GPG disabled
        let output = Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "PR test deterministic commit",
            ])
            .env("GIT_AUTHOR_DATE", "2024-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2024-01-01T00:00:00Z")
            .current_dir(path)
            .output()
            .expect("git commit failed");
        assert!(
            output.status.success(),
            "git commit failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Get the commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()
            .expect("git rev-parse failed");
        assert!(
            output.status.success(),
            "git rev-parse failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );

        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        println!("\n========================================");
        println!("PR_TEST_COMMIT_HASH should be: {}", hash);
        println!("========================================\n");

        // Verify we got a valid 40-character hex hash
        assert_eq!(hash.len(), 40, "Hash should be 40 hex chars, got: {}", hash);
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "Hash should be hex chars only"
        );

        // If the constant is not PLACEHOLDER, verify it matches
        if PR_TEST_COMMIT_HASH != "PLACEHOLDER" {
            assert_eq!(
                hash, PR_TEST_COMMIT_HASH,
                "Commit hash mismatch! Expected {}, got {}. Update PR_TEST_COMMIT_HASH if commit parameters changed.",
                PR_TEST_COMMIT_HASH, hash
            );
        }
    }
}
