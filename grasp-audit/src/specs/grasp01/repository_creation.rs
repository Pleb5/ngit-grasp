//! GRASP-01 Repository Creation Tests
//!
//! Tests that verify bare Git repositories are created when repository announcements
//! are accepted by the relay.
//!
//! ## Test Coverage
//!
//! - Repository creation on valid announcement (verified via HTTP)
//! - Idempotent creation (no error if repo already exists)
//! - Repository accessibility via Smart HTTP service
//!
//! ## Running Tests
//!
//! ```bash
//! cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test
//! ```

use crate::{AuditClient, FixtureKind, TestContext, TestResult};
use nostr_sdk::prelude::*;

/// Test suite for repository creation
pub struct RepositoryCreationTests;

impl RepositoryCreationTests {
    /// Run all repository creation tests
    pub async fn run_all(
        client: &AuditClient,
        relay_domain: &str,
    ) -> crate::AuditResult {
        let mut results = crate::AuditResult::new("GRASP-01 Repository Creation Tests");

        results.add(Self::test_bare_repo_created_on_announcement(client, relay_domain).await);
        results.add(Self::test_repo_creation_idempotent(client, relay_domain).await);
        results.add(Self::test_repo_accessible_via_http(client, relay_domain).await);

        results
    }

    /// Test that a bare repository is created when a valid announcement is accepted
    ///
    /// This test:
    /// 1. Sends a valid repository announcement via TestContext
    /// 2. Verifies the announcement was accepted
    /// 3. Verifies the repository is accessible via Smart HTTP service
    pub async fn test_bare_repo_created_on_announcement(
        client: &AuditClient,
        relay_domain: &str,
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

        // Verify repository exists via HTTP (info/refs endpoint)
        if let Err(e) = check_repo_accessible_via_http(relay_domain, &npub, &repo_id).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Bare repository must be created when announcement is accepted",
            )
            .fail(&format!("Repository not accessible via HTTP: {}", e));
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
        relay_domain: &str,
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

        // Verify repository still exists and is accessible via HTTP
        let repo_id = repo
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .ok_or("Missing d tag")
            .unwrap()
            .to_string();

        let npub = repo.pubkey.to_bech32().unwrap();

        if let Err(e) = check_repo_accessible_via_http(relay_domain, &npub, &repo_id).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Repository creation must be idempotent",
            )
            .fail(&format!("Repository not accessible after second send: {}", e));
        }

        TestResult::new(
            test_name,
            "GRASP-01",
            "Repository creation must be idempotent",
        )
        .pass()
    }

    /// Test that the repository is accessible via Smart HTTP service
    ///
    /// This test verifies:
    /// 1. Repository responds to git-upload-pack service discovery
    /// 2. URL format follows http://domain/npub/identifier.git pattern
    pub async fn test_repo_accessible_via_http(
        client: &AuditClient,
        relay_domain: &str,
    ) -> TestResult {
        let test_name = "test_repo_accessible_via_http";
        let ctx = TestContext::new(client);

        // Create and send repository announcement via TestContext
        let repo = match ctx.get_fixture(FixtureKind::ValidRepo).await {
            Ok(r) => r,
            Err(e) => {
                return TestResult::new(
                    test_name,
                    "GRASP-01",
                    "Repository must be accessible via Smart HTTP service",
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

        // Verify repository is accessible via HTTP
        if let Err(e) = check_repo_accessible_via_http(relay_domain, &npub, &repo_id).await {
            return TestResult::new(
                test_name,
                "GRASP-01",
                "Repository must be accessible via Smart HTTP service",
            )
            .fail(&e);
        }

        TestResult::new(
            test_name,
            "GRASP-01",
            "Repository must be accessible via Smart HTTP service",
        )
        .pass()
    }
}

/// Helper function to check if a repository is accessible via Smart HTTP service
///
/// Verifies that the repository responds correctly to git-upload-pack service discovery
/// at the URL: http://domain/npub/identifier.git/info/refs?service=git-upload-pack
async fn check_repo_accessible_via_http(
    relay_domain: &str,
    npub: &str,
    repo_id: &str,
) -> Result<(), String> {
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

    // Verify Content-Type indicates git-upload-pack service
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("application/x-git-upload-pack-advertisement") {
        return Err(format!(
            "Expected Content-Type: application/x-git-upload-pack-advertisement, got: {}",
            content_type
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exists() {
        // Simple compilation test
        assert!(true);
    }
}
