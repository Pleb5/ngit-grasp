//! GRASP-06 /prs/ endpoint tests
//!
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//!
//! Each test here maps 1:1 to a MUST in the spec, or to an audit-derived
//! invariant that follows directly from NIP-11 discovery semantics.

use crate::specs::grasp06::SpecRef;
use crate::{AuditClient, TestResult};
use nostr_sdk::ToBech32;

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
            // 1. Resolve the HTTP base URL.
            let ws_url = client
                .relay_url()
                .await
                .map_err(|e| format!("Failed to get relay URL: {}", e))?;
            let http_url = AuditClient::ws_to_http_url(&ws_url)
                .map_err(|e| format!("Failed to convert WebSocket URL to HTTP: {}", e))?;

            let http_client = reqwest::Client::new();

            // 2. Fetch NIP-11 and see whether GRASP-06 is advertised.
            let nip11 = http_client
                .get(&http_url)
                .header("Accept", "application/nostr+json")
                .send()
                .await
                .map_err(|e| format!("Failed to fetch NIP-11 document: {}", e))?;

            if !nip11.status().is_success() {
                return Err(format!(
                    "NIP-11 fetch returned non-success status: {}",
                    nip11.status()
                ));
            }

            let nip11_json: serde_json::Value = nip11
                .json()
                .await
                .map_err(|e| format!("NIP-11 response is not valid JSON: {}", e))?;

            let advertises_grasp06 = nip11_json
                .get("supported_grasps")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("GRASP-06")))
                .unwrap_or(false);

            if advertises_grasp06 {
                // Precondition not met — the gate doesn't apply. Pass trivially;
                // the "what /prs/ must do when advertised" behaviour is covered
                // by other tests in this module.
                return Ok(());
            }

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
}
