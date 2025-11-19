//! GRASP-01 NIP-11 Document
//!
//! Tests for GRASP-01 NIP-11 relay information document requirements (lines 11-14 of ../grasp/01.md)
//!
//! These tests validate that a GRASP-01 compliant relay:
//! - Serves a valid NIP-11 relay information document
//! - Includes supported_grasps field listing supported GRASPs
//! - Includes repo_acceptance_criteria field describing acceptance policy
//! - Handles curation field correctly (present if curated, absent otherwise)

use crate::{AuditClient, AuditResult, TestResult};

pub struct Nip11DocumentTests;

impl Nip11DocumentTests {
    /// Run all NIP-11 document tests
    pub async fn run_all(client: &AuditClient) -> AuditResult {
        let mut results = AuditResult::new("GRASP-01 NIP-11 Document Tests");

        // NIP-11 relay information tests
        results.add(Self::test_nip11_document_exists(client).await);
        results.add(Self::test_nip11_supported_grasps_field(client).await);
        results.add(Self::test_nip11_repo_acceptance_criteria_field(client).await);
        results.add(Self::test_nip11_curation_field(client).await);

        results
    }

    // =========================================================================
    // NIP-11 Relay Information Tests
    // =========================================================================

    /// Test: Serve NIP-11 document
    ///
    /// Spec: Line 11 of ../grasp/01.md
    /// Requirement: MUST serve NIP-11 document
    async fn test_nip11_document_exists(_client: &AuditClient) -> TestResult {
        TestResult::new(
            "nip11_document_exists",
            "GRASP-01:nostr-relay:11",
            "Serve NIP-11 relay information document",
        )
        .run(|| async {
            // TODO: Implementation
            // 1. Extract HTTP(S) URL from client's WebSocket URL
            //    - ws://localhost:8081 -> http://localhost:8081
            //    - wss://relay.example.com -> https://relay.example.com
            // 2. HTTP GET to base URL with header:
            //    - Accept: application/nostr+json
            // 3. Verify 200 OK response
            // 4. Verify response is valid JSON
            // 5. Parse as NIP-11 document
            // 6. Verify has required fields (name, description, etc.)

            Err("Not implemented yet".to_string())
        })
        .await
    }

    /// Test: NIP-11 includes supported_grasps field
    ///
    /// Spec: Line 12 of ../grasp/01.md
    /// Requirement: MUST list supported GRASPs as string array
    async fn test_nip11_supported_grasps_field(_client: &AuditClient) -> TestResult {
        TestResult::new(
            "nip11_supported_grasps_field",
            "GRASP-01:nostr-relay:12",
            "NIP-11 document includes supported_grasps field with GRASP-01",
        )
        .run(|| async {
            // TODO: Implementation
            // 1. Fetch NIP-11 document (same as above)
            // 2. Verify `supported_grasps` field exists
            // 3. Verify it's a JSON array of strings
            // 4. Verify array includes "GRASP-01"
            // 5. Verify format: each entry matches pattern "GRASP-\d{2}"
            // 6. Document other GRASPs found (for info)

            Err("Not implemented yet".to_string())
        })
        .await
    }

    /// Test: NIP-11 includes repo_acceptance_criteria field
    ///
    /// Spec: Line 13 of ../grasp/01.md
    /// Requirement: MUST list repository acceptance criteria
    async fn test_nip11_repo_acceptance_criteria_field(_client: &AuditClient) -> TestResult {
        TestResult::new(
            "nip11_repo_acceptance_criteria_field",
            "GRASP-01:nostr-relay:13",
            "NIP-11 document includes repo_acceptance_criteria field",
        )
        .run(|| async {
            // TODO: Implementation
            // 1. Fetch NIP-11 document
            // 2. Verify `repo_acceptance_criteria` field exists
            // 3. Verify it's a string (human-readable)
            // 4. Verify non-empty
            // 5. Document the criteria (for info)
            // Examples: "Must list this relay in clone and relays tags"
            //           "Pre-payment required via Lightning invoice"

            Err("Not implemented yet".to_string())
        })
        .await
    }

    /// Test: NIP-11 curation field handling
    ///
    /// Spec: Line 14 of ../grasp/01.md
    /// Requirement: MUST include curation if curated, omit otherwise
    async fn test_nip11_curation_field(_client: &AuditClient) -> TestResult {
        TestResult::new(
            "nip11_curation_field",
            "GRASP-01:nostr-relay:14",
            "NIP-11 curation field present if curated, absent otherwise",
        )
        .run(|| async {
            // TODO: Implementation
            // 1. Fetch NIP-11 document
            // 2. Check if `curation` field exists
            // 3. If present:
            //    - Verify it's a non-empty string
            //    - Document the curation policy
            // 4. If absent:
            //    - Document that no curation beyond SPAM prevention
            // 5. Both cases are valid per spec

            Err("Not implemented yet".to_string())
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
    async fn test_grasp01_nip11_document_against_relay() {
        // Read relay URL from environment variable - must be supplied
        let relay_url = std::env::var("RELAY_URL").expect(
            "RELAY_URL environment variable must be set. Example: RELAY_URL=ws://localhost:18081",
        );

        let config = AuditConfig::ci();
        let client = AuditClient::new(&relay_url, config)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "Failed to connect to relay at {}. Ensure relay is running and accessible. \
            Try: docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest",
                    relay_url
                )
            });

        let results = Nip11DocumentTests::run_all(&client).await;
        results.print_report();

        // Don't assert all passed yet - tests not implemented
        // assert!(results.all_passed(), "Some GRASP-01 NIP-11 document tests failed");
    }
}
