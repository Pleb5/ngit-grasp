/// Nostr Relay Builder Configuration
///
/// This module integrates nostr-relay-builder with NIP-34 validation logic
/// using modular sub-policies for each event type.
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use nostr::nips::nip19::ToBech32;
use nostr_lmdb::NostrLMDB;
use nostr_relay_builder::prelude::*;

use crate::config::{Config, DatabaseBackend};
use crate::nostr::events::{
    RepositoryAnnouncement, RepositoryState, KIND_PR, KIND_PR_UPDATE, KIND_REPOSITORY_ANNOUNCEMENT,
    KIND_REPOSITORY_STATE,
};
use crate::nostr::policy::{
    AnnouncementPolicy, AnnouncementResult, PolicyContext, PrEventPolicy, RelatedEventPolicy,
    ReferenceResult, StatePolicy, StateResult,
};
use crate::sync::SYNC_SOURCE_ADDR;

/// Type alias for the shared database used by the relay
pub type SharedDatabase = Arc<dyn NostrDatabase>;

/// NIP-34 Write Policy with Full GRASP-01 Event Validation
///
/// Validates all events according to GRASP-01 specification using modular sub-policies:
/// - `AnnouncementPolicy` - Repository announcement validation
/// - `StatePolicy` - State event validation + ref alignment
/// - `PrEventPolicy` - PR/PR Update validation
/// - `RelatedEventPolicy` - Forward/backward reference checking
///
/// Uses stateful database queries to check event relationships.
#[derive(Clone)]
pub struct Nip34WritePolicy {
    ctx: PolicyContext,
    announcement_policy: AnnouncementPolicy,
    state_policy: StatePolicy,
    pr_event_policy: PrEventPolicy,
    related_event_policy: RelatedEventPolicy,
}

impl std::fmt::Debug for Nip34WritePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nip34WritePolicy")
            .field("domain", &self.ctx.domain)
            .field("git_data_path", &self.ctx.git_data_path)
            .field("database", &"<database>")
            .finish()
    }
}

impl Nip34WritePolicy {
    pub fn new(
        domain: impl Into<String>,
        database: SharedDatabase,
        git_data_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        let ctx = PolicyContext::new(domain, database, git_data_path);
        Self {
            announcement_policy: AnnouncementPolicy::new(ctx.clone()),
            state_policy: StatePolicy::new(ctx.clone()),
            pr_event_policy: PrEventPolicy::new(ctx.clone()),
            related_event_policy: RelatedEventPolicy::new(ctx.clone()),
            ctx,
        }
    }

    /// Handle repository announcement event
    ///
    /// # Arguments
    /// * `event` - The announcement event to validate
    /// * `from_sync` - Whether this event came from GRASP-02 sync (bypasses domain validation)
    async fn handle_announcement(&self, event: &Event, from_sync: bool) -> PolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        // GRASP-02: Accept Layer 1 events from sync without domain validation
        // This enables relay discovery chain - synced announcements are stored
        // for relay URL extraction even if they don't list our domain
        if from_sync {
            // Still validate basic structure
            match RepositoryAnnouncement::from_event(event.clone()) {
                Ok(_announcement) => {
                    tracing::debug!(
                        "Accepted synced repository announcement: {} (domain validation bypassed)",
                        event_id_str
                    );
                    // Don't create bare repository for external announcements
                    return PolicyResult::Accept;
                }
                Err(e) => {
                    tracing::warn!(
                        "Rejected malformed synced announcement {}: {}",
                        event_id_str,
                        e
                    );
                    return PolicyResult::Reject(format!("Failed to parse announcement: {}", e));
                }
            }
        }

        // Normal validation path - requires domain to be listed
        match self.announcement_policy.validate(event).await {
            AnnouncementResult::Accept => {
                // Parse announcement to get repository details
                match RepositoryAnnouncement::from_event(event.clone()) {
                    Ok(announcement) => {
                        // Try to create bare repository if it doesn't exist
                        if let Err(e) = self.announcement_policy.ensure_bare_repository(&announcement)
                        {
                            tracing::warn!(
                                "Failed to create bare repository for {}: {}",
                                event_id_str,
                                e
                            );
                            // Note: We still accept the event even if repo creation fails
                        }

                        tracing::debug!("Accepted repository announcement: {}", event_id_str);
                        PolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse repository announcement {}: {}",
                            event_id_str,
                            e
                        );
                        PolicyResult::Reject(format!("Failed to parse announcement: {}", e))
                    }
                }
            }
            AnnouncementResult::AcceptMaintainer => {
                // Parse announcement to get details for logging
                match RepositoryAnnouncement::from_event(event.clone()) {
                    Ok(announcement) => {
                        tracing::info!(
                            "Accepted maintainer announcement {} (author {} is listed as maintainer for {})",
                            event_id_str,
                            event.pubkey.to_hex(),
                            announcement.identifier
                        );
                        // Don't create bare repository for external announcements
                        PolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse maintainer announcement {}: {}",
                            event_id_str,
                            e
                        );
                        PolicyResult::Reject(format!("Failed to parse announcement: {}", e))
                    }
                }
            }
            AnnouncementResult::Reject(reason) => {
                tracing::warn!(
                    "Rejected repository announcement {}: {}",
                    event_id_str,
                    reason
                );
                PolicyResult::Reject(reason)
            }
        }
    }

    /// Handle repository state event
    async fn handle_state(&self, event: &Event) -> PolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        match self.state_policy.validate(event) {
            StateResult::Accept => {
                // Parse state to get HEAD and branch info
                match RepositoryState::from_event(event.clone()) {
                    Ok(_state) => {
                        // Process state alignment asynchronously
                        if let Err(e) = self.state_policy.process_state_event(event).await {
                            tracing::warn!(
                                "Failed to process state event {}: {}",
                                event_id_str,
                                e
                            );
                        }

                        tracing::debug!("Accepted repository state: {}", event_id_str);
                        PolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse repository state {}: {}",
                            event_id_str,
                            e
                        );
                        // Still accept the event even if we can't parse it
                        // The validation passed, so it's structurally valid
                        PolicyResult::Accept
                    }
                }
            }
            StateResult::Reject(reason) => {
                tracing::warn!("Rejected repository state {}: {}", event_id_str, reason);
                PolicyResult::Reject(reason)
            }
        }
    }

    /// Handle PR or PR Update event
    async fn handle_pr_event(&self, event: &Event) -> PolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        // Validate refs/nostr refs for this PR event
        // This deletes any refs/nostr/<event-id> that points to wrong commit
        if let Err(e) = self.pr_event_policy.validate_nostr_ref(event).await {
            tracing::warn!(
                "Failed to validate refs/nostr for PR event {}: {}",
                event_id_str,
                e
            );
            // Don't reject - just log the error and proceed with normal validation
        }

        // Continue with reference checking (same as related events)
        self.handle_related_event(event, "PR").await
    }

    /// Handle events that must reference accepted repositories or events
    async fn handle_related_event(&self, event: &Event, event_type: &str) -> PolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        match self.related_event_policy.check_references(event).await {
            Ok(ReferenceResult::ReferencesRepository(addr_ref)) => {
                tracing::debug!(
                    "Accepted {} event {}: references accepted repository {}",
                    event_type,
                    event_id_str,
                    addr_ref
                );
                PolicyResult::Accept
            }
            Ok(ReferenceResult::ReferencesEvent(event_ref)) => {
                tracing::debug!(
                    "Accepted {} event {}: references accepted event {}",
                    event_type,
                    event_id_str,
                    event_ref
                );
                PolicyResult::Accept
            }
            Ok(ReferenceResult::ReferencedByAccepted) => {
                tracing::debug!(
                    "Accepted {} event {}: referenced by accepted event",
                    event_type,
                    event_id_str
                );
                PolicyResult::Accept
            }
            Ok(ReferenceResult::Orphan) => {
                let (addressable_refs, event_refs) =
                    RelatedEventPolicy::extract_reference_tags(event);
                tracing::info!(
                    "Rejected orphan {} event {}: no references to accepted repos or events (checked {} addressable, {} event refs)",
                    event_type,
                    event_id_str,
                    addressable_refs.len(),
                    event_refs.len()
                );
                PolicyResult::Reject(format!(
                    "{} event must reference an accepted repository or accepted event",
                    event_type
                ))
            }
            Err(e) => {
                tracing::warn!(
                    "Database query failed for {} {}, rejecting (fail-secure): {}",
                    event_type,
                    event_id_str,
                    e
                );
                PolicyResult::Reject(format!("Database query failed: {}", e))
            }
        }
    }
}

impl WritePolicy for Nip34WritePolicy {
    fn admit_event<'a>(
        &'a self,
        event: &'a nostr_relay_builder::prelude::Event,
        addr: &'a SocketAddr,
    ) -> BoxedFuture<'a, PolicyResult> {
        Box::pin(async move {
            // GRASP-02: Detect sync source for Layer 1 domain validation bypass
            // Synced events use SYNC_SOURCE_ADDR (127.0.0.2:0) to identify them
            let from_sync = *addr == SYNC_SOURCE_ADDR;

            match event.kind.as_u16() {
                KIND_REPOSITORY_ANNOUNCEMENT => self.handle_announcement(event, from_sync).await,
                KIND_REPOSITORY_STATE => self.handle_state(event).await,
                KIND_PR | KIND_PR_UPDATE => self.handle_pr_event(event).await,
                _ => self.handle_related_event(event, "Event").await,
            }
        })
    }
}

/// Result of creating a relay - includes relay, database, and write policy
pub struct RelayWithDatabase {
    /// The local relay instance
    pub relay: LocalRelay,
    /// The database Arc that can be used for direct queries
    pub database: SharedDatabase,
    /// The write policy used for event validation
    pub write_policy: Nip34WritePolicy,
}

/// Create a configured LocalRelay with full GRASP-01 validation
///
/// Returns a `RelayWithDatabase` struct containing:
/// - The `LocalRelay` for handling WebSocket connections
/// - The `SharedDatabase` for direct database queries (e.g., push authorization)
pub fn create_relay(config: &Config) -> Result<RelayWithDatabase> {
    tracing::info!("Configuring nostr relay with GRASP-01 validation...");

    // Determine database path
    let db_path = Path::new(&config.relay_data_path);

    // Create database based on configuration
    let database: SharedDatabase = match config.database_backend {
        DatabaseBackend::Memory => {
            tracing::info!("Using in-memory database (no persistence)");
            Arc::new(MemoryDatabase::with_opts(MemoryDatabaseOptions {
                events: true,
                max_events: Some(100_000),
            }))
        }
        DatabaseBackend::NostrDb => {
            tracing::info!("Using NostrDB backend at: {}", db_path.display());
            // TODO: Implement NostrDB backend once nostr-relay-builder supports it
            // For now, fall back to memory database
            tracing::warn!("NostrDB backend not yet implemented, using in-memory database");
            Arc::new(MemoryDatabase::with_opts(MemoryDatabaseOptions {
                events: true,
                max_events: Some(100_000),
            }))
        }
        DatabaseBackend::Lmdb => {
            tracing::info!("Using LMDB backend at: {}", db_path.display());
            // Ensure the database directory exists
            std::fs::create_dir_all(db_path).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to create LMDB directory {}: {}",
                    db_path.display(),
                    e
                )
            })?;
            Arc::new(NostrLMDB::open(db_path).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to open LMDB database at {}: {}",
                    db_path.display(),
                    e
                )
            })?)
        }
    };

    // Build relay with GRASP-01 validation
    // Clone Arc for the write policy so both relay and policy can access the database
    let git_data_path = config.effective_git_data_path();
    let write_policy = Nip34WritePolicy::new(&config.domain, database.clone(), &git_data_path);

    let builder = RelayBuilder::default()
        .database(database.clone())
        .write_policy(write_policy.clone());

    tracing::info!(
        "Relay configured with GRASP-01 validation for domain: {}",
        config.domain
    );

    Ok(RelayWithDatabase {
        relay: LocalRelay::new(builder),
        database,
        write_policy,
    })
}