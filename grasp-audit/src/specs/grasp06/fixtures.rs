//! GRASP-06 test fixtures (non-Event)
//!
//! Reusable, cached prerequisites for the GRASP-06 audit suite. Each fixture
//! here implements the generic [`crate::Fixture`] trait and is fetched via
//! `TestContext::get(&FixtureType)`.
//!
//! See [`crate::Fixture`] for the caching model.

use crate::{AuditClient, Fixture, TestContext};
use anyhow::{anyhow, Result};
use serde_json::Value;

/// Fixture: the relay's NIP-11 relay-information document, parsed as JSON.
///
/// Cached once per audit run (in Shared mode) or per `TestContext`
/// (in Isolated mode) so every GRASP-06 test that needs to inspect the
/// document — capability discovery, curation flags, software/version, etc. —
/// pays for the HTTP fetch at most once.
///
/// # Cache key
///
/// `"grasp06.nip11_document"` — `Output` type `serde_json::Value`.
pub struct Nip11DocFixture;

impl Fixture for Nip11DocFixture {
    type Output = Value;

    fn cache_key(&self) -> &'static str {
        "grasp06.nip11_document"
    }

    async fn build(&self, ctx: &TestContext<'_>) -> Result<Value> {
        let client = ctx.client();
        let ws_url = client.relay_url().await?;
        let http_url = AuditClient::ws_to_http_url(&ws_url)
            .map_err(|e| anyhow!("Failed to convert WebSocket URL to HTTP: {}", e))?;

        let http_client = reqwest::Client::new();
        let response = http_client
            .get(&http_url)
            .header("Accept", "application/nostr+json")
            .send()
            .await
            .map_err(|e| anyhow!("Failed to fetch NIP-11 document: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "NIP-11 fetch returned non-success status: {}",
                response.status()
            ));
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| anyhow!("NIP-11 response is not valid JSON: {}", e))?;

        Ok(json)
    }
}

/// Convenience: does the relay's NIP-11 `supported_grasps` array include the
/// given GRASP identifier (e.g. `"GRASP-06"`)?
///
/// Uses [`Nip11DocFixture`] under the hood, so subsequent calls in the same
/// audit run / `TestContext` hit cache.
pub async fn advertises_grasp(ctx: &TestContext<'_>, grasp: &str) -> Result<bool> {
    let doc = ctx.get(&Nip11DocFixture).await?;
    Ok(doc
        .get("supported_grasps")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().any(|v| v.as_str() == Some(grasp)))
        .unwrap_or(false))
}
