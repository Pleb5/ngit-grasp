//! GRASP-01 Repository Creation Tests
//!
//! Tests that verify bare Git repositories are created when repository announcements
//! are accepted by the relay.
//!
//! ## Test Coverage
//!
//! - Repository creation on valid announcement
//! - Idempotent creation (no error if repo already exists)
//! - Proper directory structure (<npub>/<identifier>.git)
//! - Bare repository validation (has HEAD, config, objects, refs)
//!
//! ## Running Tests
//!
//! ```bash
//! cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test
//! ```

use crate::{AuditClient, FixtureKind, TestContext, TestResult};
use nostr_sdk::prelude::*;
use std::path::Path;

/// Test suite for repository creation
pub struct RepositoryCreationTests;

impl RepositoryCreationTests {
    /// Run all repository creation tests
    pub async fn run_all(
        client: &AuditClient,
        git_data_dir: &Path,
    ) -> crate::AuditResult {
        let mut results = crate::AuditResult::new("GRASP-01 Repository Creation Tests");

        results.add(Self::test_bare_repo_created_on_announcement(client, git_data_dir).await);
        results.add(Self::test_repo_creation_idempotent(client, git_data_dir).await);
        results.add(Self::test_bare_repo_structure(client, git_data_dir).await);

        results
    }

    /// Test that a bare repository is created when a valid announcement is accepted
    ///
    /// This test:
    /// 1. Sends a valid repository announcement via TestContext
    /// 2. Verifies the announcement was accepted
    /// 3. Checks that a bare git repository was created at the expected path
    pub async fn test_bare_repo_created_on_announcement(
        client: &AuditClient,
        git_data_dir: &Path,
    ) -> TestResult {
        let test_name = "test_bare_repo_created_on_announcement";
        let ctx = TestContext::new(client);

        // Use TestContext to create and send repository announcement
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Bare repository must be created when announcement is accepted",
                )
                .fail(&format!("Failed to create repo fixture: {}", e))
            }
        };

        // Wait a bit for repository creation
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo identifier and npub from announcement
        let repo_id = match repo
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
                    "Bare repository must be created when announcement is accepted",
                )
                .fail("Repository announcement missing d tag")
            }
        };

        let npub = match repo.pubkey.to_bech32() {
            Ok(n) => n,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Bare repository must be created when announcement is accepted",
                )
                .fail(&format!("Failed to convert pubkey to npub: {}", e))
            }
        };

        // Check if repository was created
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));

        if !is_bare_repository(&repo_path) {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must be created when announcement is accepted",
            )
            .fail(&format!(
                "Bare repository not found at: {}",
                repo_path.display()
            ));
        }

        TestResult::new(
            test_name,
            "GRASP-01",
            "Bare repository must be created when announcement is accepted",
        )
        .pass()
    }

    /// Test that repository creation is idempotent
    ///
    /// This test:
    /// 1. Sends a repository announcement (creates repo) via TestContext
    /// 2. Sends the same announcement again
    /// 3. Verifies no error occurs and repo still exists
    pub async fn test_repo_creation_idempotent(
        client: &AuditClient,
        git_data_dir: &Path,
    ) -> TestResult {
        let test_name = "test_repo_creation_idempotent";
        let ctx = TestContext::new(client);

        // Create and send repository announcement first time via TestContext
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Repository creation must be idempotent",
                )
                .fail(&format!("Failed to create repo fixture: {}", e))
            }
        };

        // Wait for repository creation
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Send the same announcement again (should be idempotent)
        if let Err(e) = client.send_event(repo.clone()).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Repository creation must be idempotent",
            )
            .fail(&format!("Second send failed (not idempotent): {}", e));
        }

        // Wait again
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Verify repository still exists and is valid
        let repo_id = repo
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .ok_or("Missing d tag")
            .unwrap()
            .to_string();

        let npub = repo.pubkey.to_bech32().unwrap();
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));

        if !is_bare_repository(&repo_path) {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Repository creation must be idempotent",
            )
            .fail("Repository not found after second send");
        }

        TestResult::new(
            test_name,
            "GRASP-01",
            "Repository creation must be idempotent",
        )
        .pass()
    }

    /// Test that the repository has the correct structure
    ///
    /// This test verifies:
    /// 1. Repository is at <git_data_path>/<npub>/<identifier>.git
    /// 2. Repository is bare (no working directory)
    /// 3. Repository has required git structure (HEAD, config, objects/, refs/)
    pub async fn test_bare_repo_structure(client: &AuditClient, git_data_dir: &Path) -> TestResult {
        let test_name = "test_bare_repo_structure";
        let ctx = TestContext::new(client);

        // Create and send repository announcement via TestContext
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Bare repository must have correct structure",
                )
                .fail(&format!("Failed to create repo fixture: {}", e))
            }
        };

        // Wait for repository creation
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo identifier and npub
        let repo_id = repo
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .ok_or("Missing d tag")
            .unwrap()
            .to_string();

        let npub = repo.pubkey.to_bech32().unwrap();

        // Verify correct path structure: <git_data_path>/<npub>/<identifier>.git
        let expected_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));

        if !expected_path.exists() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must have correct structure",
            )
            .fail(&format!(
                "Repository not at expected path: {}",
                expected_path.display()
            ));
        }

        // Verify it's a bare repository with correct structure
        if !expected_path.join("HEAD").is_file() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must have correct structure",
            )
            .fail("Missing HEAD file");
        }

        if !expected_path.join("config").is_file() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must have correct structure",
            )
            .fail("Missing config file");
        }

        if !expected_path.join("objects").is_dir() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must have correct structure",
            )
            .fail("Missing objects/ directory");
        }

        if !expected_path.join("refs").is_dir() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must have correct structure",
            )
            .fail("Missing refs/ directory");
        }

        // Verify the helper function agrees
        if !is_bare_repository(&expected_path) {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must have correct structure",
            )
            .fail("Helper function does not recognize repository as bare");
        }

        TestResult::new(
            test_name,
            "GRASP-01",
            "Bare repository must have correct structure",
        )
        .pass()
    }
}

/// Helper function to check if a path is a valid bare git repository
///
/// A bare repository must have:
/// - HEAD file
/// - config file
/// - objects/ directory
/// - refs/ directory
pub fn is_bare_repository(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    // Check for required bare repository components
    let has_head = path.join("HEAD").is_file();
    let has_config = path.join("config").is_file();
    let has_objects = path.join("objects").is_dir();
    let has_refs = path.join("refs").is_dir();

    has_head && has_config && has_objects && has_refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn test_is_bare_repository_detects_valid_repo() {
        // Create a temporary bare repository for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path().join("test.git");

        // Initialize a bare repository
        Command::new("git")
            .args(&["init", "--bare", repo_path.to_str().unwrap()])
            .output()
            .expect("Failed to create test repository");

        // Verify our helper function detects it
        assert!(
            is_bare_repository(&repo_path),
            "Should detect valid bare repository"
        );
    }

    #[test]
    fn test_is_bare_repository_rejects_non_repo() {
        let temp_dir = tempfile::tempdir().unwrap();
        assert!(
            !is_bare_repository(temp_dir.path()),
            "Should reject non-repository directory"
        );
    }

    #[test]
    fn test_is_bare_repository_rejects_nonexistent() {
        let path = Path::new("/nonexistent/path/to/repo.git");
        assert!(!is_bare_repository(path), "Should reject nonexistent path");
    }
}
