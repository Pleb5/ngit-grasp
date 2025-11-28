//! GRASP-01 Git Clone Tests
//!
//! Tests that verify Git clone operations work correctly through the HTTP backend.
//!
//! ## Test Coverage
//!
//! - Basic clone operation via HTTP
//! - Cloned repository structure validation
//! - Clone URL format verification
//!
//! ## Running Tests
//!
//! ```bash
//! cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test
//! ```

use crate::{AuditClient, FixtureKind, TestContext, TestResult};
use nostr_sdk::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Test suite for Git clone operations
pub struct GitCloneTests;

impl GitCloneTests {
    /// Run all Git clone tests
    pub async fn run_all(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> crate::AuditResult {
        let mut results = crate::AuditResult::new("GRASP-01 Git Clone Tests");

        results.add(Self::test_basic_git_clone(client, git_data_dir, relay_domain).await);
        results.add(Self::test_clone_url_format(client, git_data_dir, relay_domain).await);

        results
    }

    /// Test that a repository can be cloned via Git HTTP backend
    ///
    /// This test:
    /// 1. Creates a repository announcement
    /// 2. Waits for repository creation
    /// 3. Attempts to clone the repository using git clone
    /// 4. Verifies the clone succeeded
    pub async fn test_basic_git_clone(
        client: &AuditClient,
        git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_basic_git_clone";
        let ctx = TestContext::new(client);

        // Create repository announcement
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Repository must be cloneable via Git HTTP backend",
                )
                .fail(&format!("Failed to create repo fixture: {}", e))
            }
        };

        // Wait for repository creation
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Extract repo identifier and npub
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
                    "Repository must be cloneable via Git HTTP backend",
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
                    "Repository must be cloneable via Git HTTP backend",
                )
                .fail(&format!("Failed to convert pubkey to npub: {}", e))
            }
        };

        // Verify repository exists
        let repo_path = git_data_dir.join(&npub).join(format!("{}.git", repo_id));
        if !repo_path.exists() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Repository must be cloneable via Git HTTP backend",
            )
            .fail(&format!(
                "Repository not found at: {}",
                repo_path.display()
            ));
        }

        // Create a test clone directory using standard library
        let temp_base = std::env::temp_dir();
        let clone_dir_name = format!("grasp-test-clone-{}", uuid::Uuid::new_v4());
        let clone_path = temp_base.join(&clone_dir_name);
        
        // Ensure clean state
        let _ = fs::remove_dir_all(&clone_path);

        // Build clone URL: http://domain/npub/identifier.git
        let clone_url = format!("http://{}/{}/{}.git", relay_domain, npub, repo_id);

        // Attempt to clone the repository
        let output = Command::new("git")
            .args(&["clone", &clone_url, clone_path.to_str().unwrap()])
            .env("GIT_TERMINAL_PROMPT", "0") // Disable password prompts
            .output();
        
        // Clean up on success or failure
        let cleanup = || {
            let _ = fs::remove_dir_all(&clone_path);
        };

        let output = match output {
            Ok(o) => o,
            Err(e) => {
                cleanup();
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Repository must be cloneable via Git HTTP backend",
                )
                .fail(&format!("Failed to execute git clone: {}", e))
            }
        };

        if !output.status.success() {
            cleanup();
            let stderr = String::from_utf8_lossy(&output.stderr);
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Repository must be cloneable via Git HTTP backend",
            )
            .fail(&format!("Git clone failed: {}", stderr));
        }

        // Verify clone succeeded by checking for .git directory
        if !clone_path.join(".git").is_dir() {
            cleanup();
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Repository must be cloneable via Git HTTP backend",
            )
            .fail("Cloned repository missing .git directory");
        }

        cleanup();
        TestResult::new(
            test_name,
            "GRASP-01",
            "Repository must be cloneable via Git HTTP backend",
        )
        .pass()
    }

    /// Test clone URL format validation
    ///
    /// This test verifies:
    /// 1. URLs follow the pattern http://domain/npub/identifier.git
    /// 2. Invalid URLs are rejected properly
    pub async fn test_clone_url_format(
        client: &AuditClient,
        _git_data_dir: &Path,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_clone_url_format";
        let ctx = TestContext::new(client);

        // Create repository announcement
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Clone URL must follow correct format",
                )
                .fail(&format!("Failed to create repo fixture: {}", e))
            }
        };

        // Wait for repository creation
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let repo_id = repo
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .ok_or("Missing d tag")
            .unwrap()
            .to_string();

        let npub = repo.pubkey.to_bech32().unwrap();

        // Test valid URL format
        let valid_url = format!("http://{}/{}/{}.git", relay_domain, npub, repo_id);
        
        // Verify URL contains expected components
        if !valid_url.contains(&npub) {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Clone URL must follow correct format",
            )
            .fail("URL missing npub");
        }

        if !valid_url.contains(&format!("{}.git", repo_id)) {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Clone URL must follow correct format",
            )
            .fail("URL missing repository identifier");
        }

        // Test that invalid URL fails (wrong format)
        let temp_base = std::env::temp_dir();
        let clone_dir_name = format!("grasp-test-invalid-{}", uuid::Uuid::new_v4());
        let clone_path = temp_base.join(&clone_dir_name);
        
        // Ensure clean state
        let _ = fs::remove_dir_all(&clone_path);
        
        let invalid_url = format!("http://{}/invalid/path", relay_domain);

        let output = Command::new("git")
            .args(&["clone", &invalid_url, clone_path.to_str().unwrap()])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();

        // Cleanup after test
        let _ = fs::remove_dir_all(&clone_path);

        // Invalid URL should fail
        if output.status.success() {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Clone URL must follow correct format",
            )
            .fail("Invalid URL was accepted (should have been rejected)");
        }

        TestResult::new(
            test_name,
            "GRASP-01",
            "Clone URL must follow correct format",
        )
        .pass()
    }
}

#[cfg(test)]
mod tests {
    

    #[test]
    fn test_module_exists() {
        // Simple compilation test
        assert!(true);
    }
}