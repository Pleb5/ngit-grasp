//! GRASP-06 /prs/ endpoint tests
//!
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//!
//! Each test here maps 1:1 to a MUST in the spec, or to an audit-derived
//! invariant that follows directly from NIP-11 discovery semantics.

use crate::specs::grasp06::fixtures::advertises_grasp;
use crate::specs::grasp06::SpecRef;
use crate::{AuditClient, TestContext, TestResult};
use nostr_sdk::ToBech32;
use std::fs;
use std::process::Command;

pub struct PrsEndpointTests;

impl PrsEndpointTests {
    /// Test: if NIP-11 does not advertise GRASP-06, the `/prs/<npub>/<id>.git`
    /// namespace MUST return 404.
    ///
    /// This is the discovery gate: clients use NIP-11 `supported_grasps` to
    /// decide whether a relay implements GRASP-06. If a relay serves `/prs/`
    /// but does not advertise it, capability discovery is broken.
    ///
    /// Branches:
    /// - NIP-11 lists `GRASP-06`  -> test trivially passes (precondition not met).
    /// - NIP-11 does not list it -> `GET /prs/<valid-npub>/anything.git/info/refs?service=git-upload-pack`
    ///   MUST return HTTP 404.
    pub async fn test_prs_namespace_404_when_grasp06_not_advertised(
        client: &AuditClient,
    ) -> TestResult {
        TestResult::new(
            "prs_namespace_404_when_grasp06_not_advertised",
            SpecRef::Grasp06NotAdvertised404,
            "MUST return 404 on /prs/ when GRASP-06 is not advertised in NIP-11",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // 1. Capability check via the shared NIP-11 fixture.
            let advertised = advertises_grasp(&ctx, "GRASP-06")
                .await
                .map_err(|e| format!("Failed to determine GRASP-06 advertisement: {}", e))?;

            if advertised {
                // Precondition not met — the gate doesn't apply. Pass trivially;
                // the "what /prs/ must do when advertised" behaviour is covered
                // by other tests in this module.
                return Ok(());
            }

            // 2. Resolve the HTTP base URL for the probe.
            let ws_url = client
                .relay_url()
                .await
                .map_err(|e| format!("Failed to get relay URL: {}", e))?;
            let http_url = AuditClient::ws_to_http_url(&ws_url)
                .map_err(|e| format!("Failed to convert WebSocket URL to HTTP: {}", e))?;

            // 3. Build a /prs/<valid-npub>/<id>.git/info/refs URL with a known-valid
            //    npub. Using a fresh random npub guarantees no implementation could
            //    have a repo there by accident.
            let probe_keys = nostr_sdk::Keys::generate();
            let probe_npub = probe_keys
                .public_key()
                .to_bech32()
                .map_err(|e| format!("Failed to bech32-encode probe npub: {}", e))?;

            let probe_url = format!(
                "{}/prs/{}/audit-probe.git/info/refs?service=git-upload-pack",
                http_url.trim_end_matches('/'),
                probe_npub
            );

            let http_client = reqwest::Client::new();
            let response = http_client
                .get(&probe_url)
                .send()
                .await
                .map_err(|e| format!("Failed to GET {}: {}", probe_url, e))?;

            // 4. The spec gate: must be 404 (not 200, not 401, not 403, not 503).
            if response.status() != reqwest::StatusCode::NOT_FOUND {
                return Err(format!(
                    "Expected 404 on /prs/ when GRASP-06 not advertised in NIP-11, \
                     got {} from {}",
                    response.status(),
                    probe_url
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test: when GRASP-06 is advertised, fetching any well-formed
    /// `/prs/<npub>/<id>.git` path that has no accepted `refs/nostr/<event-id>`
    /// MUST respond as if serving an empty bare repository.
    ///
    /// Spec: 06.md line 13 — "MUST respond to upload-pack requests for any
    /// well-formed path as if serving an empty bare repository until at least
    /// one `refs/nostr/<event-id>` has been accepted for that path."
    ///
    /// Branches:
    /// - NIP-11 does not list `GRASP-06` -> test trivially passes (precondition
    ///   not met; the "off" contract is covered by
    ///   `test_prs_namespace_404_when_grasp06_not_advertised`).
    /// - NIP-11 lists `GRASP-06` -> `git clone` of a fresh random
    ///   `/prs/<npub>/<id>.git` MUST succeed and produce a repository with
    ///   zero refs.
    pub async fn test_prs_fetch_unknown_path_serves_empty_repo(client: &AuditClient) -> TestResult {
        TestResult::new(
            "prs_fetch_unknown_path_serves_empty_repo",
            SpecRef::Grasp06FetchEmptyRepo,
            "MUST serve empty bare repo on fetch for any well-formed /prs/ path \
             until refs/nostr/<event-id> has been accepted",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // 1. Capability check via the shared NIP-11 fixture.
            let advertised = advertises_grasp(&ctx, "GRASP-06")
                .await
                .map_err(|e| format!("Failed to determine GRASP-06 advertisement: {}", e))?;

            if !advertised {
                // Precondition not met — the spec invariant only applies when
                // the relay opts in. The "off" contract (404) is covered by
                // the discovery-gate test in this module.
                return Ok(());
            }

            // 2. Resolve the HTTP base URL.
            let ws_url = client
                .relay_url()
                .await
                .map_err(|e| format!("Failed to get relay URL: {}", e))?;
            let http_url = AuditClient::ws_to_http_url(&ws_url)
                .map_err(|e| format!("Failed to convert WebSocket URL to HTTP: {}", e))?;

            // 3. Build a /prs/<valid-npub>/<random-id>.git URL. Using fresh
            //    random keys and a UUID identifier guarantees no implementation
            //    could have prior state for this path.
            let probe_keys = nostr_sdk::Keys::generate();
            let probe_npub = probe_keys
                .public_key()
                .to_bech32()
                .map_err(|e| format!("Failed to bech32-encode probe npub: {}", e))?;
            let probe_id = format!("audit-probe-{}", uuid::Uuid::new_v4());

            let clone_url = format!(
                "{}/prs/{}/{}.git",
                http_url.trim_end_matches('/'),
                probe_npub,
                probe_id
            );

            // 4. Clone into a fresh temp dir. Existing GitCloneTests follows
            //    the same temp_dir + UUID pattern; reused here for consistency.
            let temp_base = std::env::temp_dir();
            let clone_path =
                temp_base.join(format!("grasp06-empty-clone-{}", uuid::Uuid::new_v4()));
            let _ = fs::remove_dir_all(&clone_path);

            let cleanup = || {
                let _ = fs::remove_dir_all(&clone_path);
            };

            let clone_output = Command::new("git")
                .args(["clone", &clone_url, clone_path.to_str().unwrap()])
                .env("GIT_TERMINAL_PROMPT", "0")
                .output();

            let clone_output = match clone_output {
                Ok(o) => o,
                Err(e) => {
                    cleanup();
                    return Err(format!("Failed to execute git clone: {}", e));
                }
            };

            if !clone_output.status.success() {
                let stderr = String::from_utf8_lossy(&clone_output.stderr).to_string();
                cleanup();
                return Err(format!("git clone {} failed: {}", clone_url, stderr.trim()));
            }

            // 5. Verify the cloned repo has zero refs. `for-each-ref` lists
            //    every ref one-per-line; empty stdout means no refs at all,
            //    which is the spec's "empty bare repository" invariant.
            let refs_output = Command::new("git")
                .args(["-C", clone_path.to_str().unwrap(), "for-each-ref"])
                .output();

            let refs_output = match refs_output {
                Ok(o) => o,
                Err(e) => {
                    cleanup();
                    return Err(format!("Failed to execute git for-each-ref: {}", e));
                }
            };

            if !refs_output.status.success() {
                let stderr = String::from_utf8_lossy(&refs_output.stderr).to_string();
                cleanup();
                return Err(format!(
                    "git for-each-ref failed in cloned repo: {}",
                    stderr.trim()
                ));
            }

            let refs_stdout = String::from_utf8_lossy(&refs_output.stdout).to_string();
            if !refs_stdout.trim().is_empty() {
                cleanup();
                return Err(format!(
                    "Expected empty bare repo, but clone of {} contained refs:\n{}",
                    clone_url,
                    refs_stdout.trim()
                ));
            }

            cleanup();
            Ok(())
        })
        .await
    }
}
