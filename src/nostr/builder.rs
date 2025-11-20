/// Nostr Relay Builder Configuration
///
/// This module integrates nostr-relay-builder with NIP-34 validation logic
/// preserved from the original implementation.
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use nostr::nips::nip19::ToBech32;
use nostr::prelude::{Alphabet, SingleLetterTag};
use nostr::{EventId, Filter, Kind, PublicKey};
use nostr_relay_builder::prelude::*;

use crate::config::Config;
use crate::nostr::events::{
    validate_announcement, validate_state, KIND_REPOSITORY_ANNOUNCEMENT, KIND_REPOSITORY_STATE,
};

/// NIP-34 Write Policy with Full GRASP-01 Event Validation
///
/// Validates all events according to GRASP-01 specification:
/// - Repository announcements must list service in clone and relays tags
/// - Repository state announcements must have valid structure  
/// - Other events must reference accepted repositories or events
/// - Forward references are supported (events referenced by accepted events)
/// - Orphan events with no valid references are rejected
///
/// Uses stateful database queries to check event relationships.
#[derive(Debug, Clone)]
pub struct Nip34WritePolicy {
    domain: String,
    database: Arc<MemoryDatabase>,
}

impl Nip34WritePolicy {
    pub fn new(domain: impl Into<String>, database: Arc<MemoryDatabase>) -> Self {
        Self {
            domain: domain.into(),
            database,
        }
    }

    /// Extract all reference tags from an event (a, A, q, e, E)
    /// Returns (addressable_refs, event_refs)
    fn extract_reference_tags(event: &Event) -> (Vec<String>, Vec<EventId>) {
        let mut addressable_refs = Vec::new();
        let mut event_refs = Vec::new();

        for tag in event.tags.iter() {
            let tag_vec = tag.clone().to_vec();
            if tag_vec.is_empty() {
                continue;
            }

            match tag_vec[0].as_str() {
                // Addressable event references (a, A, q with kind:pubkey:identifier format)
                "a" | "A" | "q" if tag_vec.len() > 1 && tag_vec[1].contains(':') => {
                    addressable_refs.push(tag_vec[1].clone());
                }
                // Event ID references (e, E, q with event ID format)
                "e" | "E" if tag_vec.len() > 1 => {
                    if let Ok(event_id) = EventId::from_hex(&tag_vec[1]) {
                        event_refs.push(event_id);
                    }
                }
                "q" if tag_vec.len() > 1 && !tag_vec[1].contains(':') => {
                    if let Ok(event_id) = EventId::from_hex(&tag_vec[1]) {
                        event_refs.push(event_id);
                    }
                }
                _ => {}
            }
        }

        (addressable_refs, event_refs)
    }

    /// Check if an addressable event (repository) exists in database
    async fn is_accepted_repository(
        database: &Arc<MemoryDatabase>,
        addressable: &str,
    ) -> Result<bool, String> {
        // Parse addressable format: kind:pubkey:identifier
        let parts: Vec<&str> = addressable.split(':').collect();
        if parts.len() < 3 {
            return Ok(false);
        }

        let kind = parts[0]
            .parse::<u16>()
            .map_err(|e| format!("Invalid kind in addressable: {}", e))?;
        let pubkey = PublicKey::from_hex(parts[1])
            .map_err(|e| format!("Invalid pubkey in addressable: {}", e))?;
        let identifier = parts[2];

        // Query database for this addressable event
        let filter = Filter::new()
            .kind(Kind::from(kind))
            .author(pubkey)
            .identifier(identifier);

        match database.query(filter).await {
            Ok(events) => Ok(!events.is_empty()),
            Err(e) => Err(format!("Database query failed: {}", e)),
        }
    }

    /// Check if an event exists in database  
    async fn is_accepted_event(
        database: &Arc<MemoryDatabase>,
        event_id: &EventId,
    ) -> Result<bool, String> {
        let filter = Filter::new().id(*event_id);

        match database.query(filter).await {
            Ok(events) => Ok(!events.is_empty()),
            Err(e) => Err(format!("Database query failed: {}", e)),
        }
    }

    /// Check if any accepted event references this event ID (forward reference)
    /// Checks all reference tag types: e, E, q, Q (event ID references only, not addressable)
    ///
    /// Note: Must check each tag type separately as custom_tag chaining creates AND not OR
    async fn is_referenced_by_accepted(
        database: &Arc<MemoryDatabase>,
        event_id: &EventId,
    ) -> Result<bool, String> {
        let event_id_hex = event_id.to_hex();

        // Check each tag type that can reference event IDs
        // Note: 'a' and 'A' tags use addressable format (kind:pubkey:d), not event IDs
        let tag_types = [
            SingleLetterTag::lowercase(Alphabet::E), // 'e' - standard event reference
            SingleLetterTag::uppercase(Alphabet::E), // 'E' - NIP-22 root event reference
            SingleLetterTag::lowercase(Alphabet::Q), // 'q' - quote reference
            SingleLetterTag::uppercase(Alphabet::Q), // 'Q' - uppercase quote (if used)
        ];

        for tag_type in &tag_types {
            let filter = Filter::new().custom_tag(tag_type.clone(), event_id_hex.clone());
            
            match database.query(filter).await {
                Ok(events) => {
                    if !events.is_empty() {
                        return Ok(true);
                    }
                }
                Err(e) => return Err(format!("Database query failed: {}", e)),
            }
        }

        Ok(false)
    }
}

impl WritePolicy for Nip34WritePolicy {
    fn admit_event<'a>(
        &'a self,
        event: &'a nostr_relay_builder::prelude::Event,
        _addr: &'a SocketAddr,
    ) -> BoxedFuture<'a, PolicyResult> {
        let database = self.database.clone();
        let domain = self.domain.clone();

        Box::pin(async move {
            let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

            match event.kind.as_u16() {
                KIND_REPOSITORY_ANNOUNCEMENT => match validate_announcement(event, &domain) {
                    Ok(_) => {
                        tracing::debug!(
                            "Accepted repository announcement: {}",
                            event_id_str
                        );
                        PolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Rejected repository announcement {}: {}",
                            event_id_str,
                            e
                        );
                        PolicyResult::Reject(e.to_string())
                    }
                },
                KIND_REPOSITORY_STATE =>match validate_state(event) {
                    Ok(_) => {
                        tracing::debug!(
                            "Accepted repository state: {}",
                            event_id_str
                        );
                        PolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Rejected repository state {}: {}",
                            event_id_str,
                            e
                        );
                        PolicyResult::Reject(e.to_string())
                    }
                },
                // GRASP-01: Check if event references accepted repositories or events
                _ => {
                    // Extract all reference tags from event
                    let (addressable_refs, event_refs) = Self::extract_reference_tags(event);

                    // Check 1: Does this event reference an accepted repository?
                    for addr_ref in &addressable_refs {
                        match Self::is_accepted_repository(&database, addr_ref).await {
                            Ok(true) => {
                                tracing::debug!(
                                    "Accepted event {}: references accepted repository {}",
                                    event_id_str,
                                    addr_ref
                                );
                                return PolicyResult::Accept;
                            }
                            Ok(false) => {
                                // Continue checking other references
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Database query failed for event {}, rejecting (fail-secure): {}",
                                    event_id_str,
                                    e
                                );
                                return PolicyResult::Reject(format!("Database query failed: {}", e));
                            }
                        }
                    }

                    // Check 2: Does this event reference an accepted event? (transitive)
                    for event_ref in &event_refs {
                        match Self::is_accepted_event(&database, event_ref).await {
                            Ok(true) => {
                                tracing::debug!(
                                    "Accepted event {}: references accepted event {}",
                                    event_id_str,
                                    event_ref
                                );
                                return PolicyResult::Accept;
                            }
                            Ok(false) => {
                                // Continue checking other references
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Database query failed for event {}, rejecting (fail-secure): {}",
                                    event_id_str,
                                    e
                                );
                                return PolicyResult::Reject(format!("Database query failed: {}", e));
                            }
                        }
                    }

                    // Check 3: Is this event referenced by an accepted event? (forward reference)
                    match Self::is_referenced_by_accepted(&database, &event.id).await {
                        Ok(true) => {
                            tracing::debug!(
                                "Accepted event {}: referenced by accepted event",
                                event_id_str
                            );
                            return PolicyResult::Accept;
                        }
                        Ok(false) => {
                            // No forward references found, continue to rejection
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Database query failed for event {}, rejecting (fail-secure): {}",
                                event_id_str,
                                e
                            );
                            return PolicyResult::Reject(format!("Database query failed: {}", e));
                        }
                    }

                    // No valid references found - reject as orphan event
                    tracing::info!(
                        "Rejected orphan event {}: no references to accepted repos or events (checked {} addressable, {} event refs)",
                        event_id_str,
                        addressable_refs.len(),
                        event_refs.len()
                    );
                    PolicyResult::Reject(
                        "Event must reference an accepted repository or accepted event".to_string()
                    )
                }
            }
        })
    }
}

/// Create a configured LocalRelay with full GRASP-01 validation
pub fn create_relay(config: &Config) -> Result<LocalRelay> {
    tracing::info!("Configuring nostr relay with GRASP-01 validation...");

    // Determine database path
    let db_path = Path::new(&config.relay_data_path);

    // Create database - using in-memory for now, can switch to persistent later
    // TODO: Add configuration for NostrDB or LMDB backends
    let database = Arc::new(MemoryDatabase::with_opts(MemoryDatabaseOptions {
        events: true,
        max_events: Some(100_000),
    }));

    tracing::info!("Using in-memory database (path: {})", db_path.display());

    // Build relay with GRASP-01 validation
    // Clone Arc for the write policy so both relay and policy can access the database
    let builder = RelayBuilder::default()
        .database(database.clone())
        .write_policy(Nip34WritePolicy::new(&config.domain, database.clone()));

    tracing::info!(
        "Relay configured with GRASP-01 validation for domain: {}",
        config.domain
    );

    Ok(LocalRelay::new(builder))
}