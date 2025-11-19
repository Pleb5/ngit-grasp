/// Nostr Relay Builder Configuration
///
/// This module integrates nostr-relay-builder with NIP-34 validation logic
/// preserved from the original implementation.
use std::net::SocketAddr;
use std::path::Path;

use nostr::nips::nip19::ToBech32;
use nostr_relay_builder::prelude::*;

use crate::config::Config;
use crate::nostr::events::{
    validate_announcement, validate_state, KIND_REPOSITORY_ANNOUNCEMENT, KIND_REPOSITORY_STATE,
};

/// NIP-34 Write Policy
///
/// Validates repository announcement and state events according to GRASP-01 spec.
/// Preserves all original validation logic from src/nostr/events.rs.
#[derive(Debug, Clone)]
pub struct Nip34WritePolicy {
    domain: String,
}

impl Nip34WritePolicy {
    pub fn new(domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
        }
    }
}

impl WritePolicy for Nip34WritePolicy {
    fn admit_event<'a>(
        &'a self,
        event: &'a nostr_relay_builder::prelude::Event,
        _addr: &'a SocketAddr,
    ) -> BoxedFuture<'a, PolicyResult> {
        Box::pin(async move {
            match event.kind.as_u16() {
                KIND_REPOSITORY_ANNOUNCEMENT => match validate_announcement(event, &self.domain) {
                    Ok(_) => {
                        tracing::debug!(
                            "Accepted repository announcement: {}",
                            event
                                .id
                                .to_bech32()
                                .unwrap_or_else(|_| "invalid".to_string())
                        );
                        PolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Rejected repository announcement {}: {}",
                            event
                                .id
                                .to_bech32()
                                .unwrap_or_else(|_| "invalid".to_string()),
                            e
                        );
                        PolicyResult::Reject(e.to_string())
                    }
                },
                KIND_REPOSITORY_STATE => match validate_state(event) {
                    Ok(_) => {
                        tracing::debug!(
                            "Accepted repository state: {}",
                            event
                                .id
                                .to_bech32()
                                .unwrap_or_else(|_| "invalid".to_string())
                        );
                        PolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Rejected repository state {}: {}",
                            event
                                .id
                                .to_bech32()
                                .unwrap_or_else(|_| "invalid".to_string()),
                            e
                        );
                        PolicyResult::Reject(e.to_string())
                    }
                },
                // Accept all other event kinds without validation
                _ => PolicyResult::Accept,
            }
        })
    }
}

/// Create a configured LocalRelay with NIP-34 validation
pub fn create_relay(config: &Config) -> Result<LocalRelay> {
    tracing::info!("Configuring nostr relay...");

    // Determine database path
    let db_path = Path::new(&config.relay_data_path);

    // Create database - using in-memory for now, can switch to persistent later
    // TODO: Add configuration for NostrDB or LMDB backends
    let database = MemoryDatabase::with_opts(MemoryDatabaseOptions {
        events: true,
        max_events: Some(100_000),
    });

    tracing::info!("Using in-memory database (path: {})", db_path.display());

    // Build relay with NIP-34 validation
    let builder = RelayBuilder::default()
        .database(database)
        .write_policy(Nip34WritePolicy::new(&config.domain));

    tracing::info!(
        "Relay configured with NIP-34 validation for domain: {}",
        config.domain
    );

    Ok(LocalRelay::new(builder))
}
