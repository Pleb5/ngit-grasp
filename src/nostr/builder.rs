/// Nostr Relay Builder Configuration
///
/// This module integrates nostr-relay-builder with NIP-34 validation logic
/// using modular sub-policies for each event type.
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use nostr::nips::nip19::ToBech32;
use nostr_lmdb::NostrLmdb;
use nostr_relay_builder::prelude::*;

use crate::config::{Config, DatabaseBackend};
use crate::nostr::events::{
    RepositoryAnnouncement, KIND_PR, KIND_PR_UPDATE, KIND_REPOSITORY_ANNOUNCEMENT,
    KIND_REPOSITORY_STATE, KIND_USER_GRASP_LIST,
};
use crate::nostr::policy::{
    AnnouncementPolicy, AnnouncementResult, PolicyContext, PrEventPolicy, ReferenceResult,
    RelatedEventPolicy, StatePolicy, StateResult,
};

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
        purgatory: std::sync::Arc<crate::purgatory::Purgatory>,
    ) -> Self {
        let ctx = PolicyContext::new(domain, database, git_data_path, purgatory);
        Self {
            announcement_policy: AnnouncementPolicy::new(ctx.clone()),
            state_policy: StatePolicy::new(ctx.clone()),
            pr_event_policy: PrEventPolicy::new(ctx.clone()),
            related_event_policy: RelatedEventPolicy::new(ctx.clone()),
            ctx,
        }
    }

    /// Handle repository announcement event
    async fn handle_announcement(&self, event: &Event) -> WritePolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        match self.announcement_policy.validate(event).await {
            AnnouncementResult::Accept => {
                // Parse announcement to get repository details
                match RepositoryAnnouncement::from_event(event.clone()) {
                    Ok(announcement) => {
                        // Try to create bare repository if it doesn't exist
                        if let Err(e) = self
                            .announcement_policy
                            .ensure_bare_repository(&announcement)
                        {
                            tracing::warn!(
                                "Failed to create bare repository for {}: {}",
                                event_id_str,
                                e
                            );
                            // Note: We still accept the event even if repo creation fails
                        }

                        tracing::debug!("Accepted repository announcement: {}", event_id_str);
                        WritePolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse repository announcement {}: {}",
                            event_id_str,
                            e
                        );
                        WritePolicyResult::reject(format!("Failed to parse announcement: {}", e))
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
                        WritePolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse maintainer announcement {}: {}",
                            event_id_str,
                            e
                        );
                        WritePolicyResult::reject(format!("Failed to parse announcement: {}", e))
                    }
                }
            }
            AnnouncementResult::Reject(reason) => {
                tracing::warn!(
                    "Rejected repository announcement {}: {}",
                    event_id_str,
                    reason
                );
                WritePolicyResult::reject(reason)
            }
        }
    }

    /// Handle repository state event
    async fn handle_state(&self, event: &Event) -> WritePolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        match self.state_policy.validate(event) {
            StateResult::Accept => {
                // Parse state to get identifier for purgatory message
                let identifier = event
                    .tags
                    .iter()
                    .find_map(|tag| {
                        let tag_vec = tag.clone().to_vec();
                        if tag_vec.len() >= 2 && tag_vec[0] == "d" {
                            Some(tag_vec[1].clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                // Process state alignment asynchronously
                match self.state_policy.process_state_event(event).await {
                    Ok(0) => {
                        // No repos aligned - event was added to purgatory
                        tracing::info!(
                            "State event {} added to purgatory: waiting for git data for identifier {}",
                            event_id_str,
                            identifier
                        );
                        WritePolicyResult::Reject {
                            status: true, // Client sees OK
                            message: format!(
                                "purgatory: state event stored, waiting for git push for {}",
                                identifier
                            )
                            .into(),
                        }
                    }
                    Ok(count) => {
                        // Successfully aligned repos
                        tracing::debug!(
                            "Accepted repository state {}: aligned {} repo(s)",
                            event_id_str,
                            count
                        );
                        WritePolicyResult::Accept
                    }
                    Err(e) => {
                        tracing::warn!("Failed to process state event {}: {}", event_id_str, e);
                        // Still accept the event even if processing failed
                        WritePolicyResult::Accept
                    }
                }
            }
            StateResult::Reject(reason) => {
                tracing::warn!("Rejected repository state {}: {}", event_id_str, reason);
                WritePolicyResult::reject(reason)
            }
        }
    }

    /// Handle PR or PR Update event
    async fn handle_pr_event(&self, event: &Event) -> WritePolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        // Check if git data exists (checks placeholders and commit existence)
        match self.pr_event_policy.check_git_data_exists(event).await {
            Ok(false) => {
                // No git data exists - add to purgatory
                let commit = event
                    .tags
                    .iter()
                    .find_map(|tag| {
                        let tag_vec = tag.clone().to_vec();
                        if tag_vec.len() >= 2 && tag_vec[0] == "c" {
                            Some(tag_vec[1].clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                tracing::info!(
                    "PR event {} added to purgatory: waiting for git push with commit {}",
                    event_id_str,
                    commit
                );

                // Add to purgatory
                self.ctx
                    .purgatory
                    .add_pr(event.clone(), event.id.to_hex(), commit.clone());

                return WritePolicyResult::Reject {
                    status: true, // Client sees OK
                    message: format!(
                        "purgatory: PR event stored, waiting for git push with commit {}",
                        commit
                    )
                    .into(),
                };
            }
            Ok(true) => {
                // Git data exists - proceed with normal validation
                tracing::debug!("Git data exists for PR event {}", event_id_str);
            }
            Err(e) => {
                // Error checking git data - reject event
                tracing::warn!(
                    "Failed to check git data for PR event {}: {}",
                    event_id_str,
                    e
                );
                return WritePolicyResult::reject(format!("Failed to check git data: {}", e));
            }
        }

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
    async fn handle_related_event(&self, event: &Event, event_type: &str) -> WritePolicyResult {
        let event_id_str = event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex());

        match self.related_event_policy.check_references(event).await {
            Ok(ReferenceResult::ReferencesRepository(addr_ref)) => {
                tracing::debug!(
                    "Accepted {} event {}: references accepted repository {}",
                    event_type,
                    event_id_str,
                    addr_ref
                );
                WritePolicyResult::Accept
            }
            Ok(ReferenceResult::ReferencesEvent(event_ref)) => {
                tracing::debug!(
                    "Accepted {} event {}: references accepted event {}",
                    event_type,
                    event_id_str,
                    event_ref
                );
                WritePolicyResult::Accept
            }
            Ok(ReferenceResult::ReferencedByAccepted) => {
                tracing::debug!(
                    "Accepted {} event {}: referenced by accepted event",
                    event_type,
                    event_id_str
                );
                WritePolicyResult::Accept
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
                WritePolicyResult::reject(format!(
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
                WritePolicyResult::reject(format!("Database query failed: {}", e))
            }
        }
    }
}

impl WritePolicy for Nip34WritePolicy {
    fn admit_event<'a>(
        &'a self,
        event: &'a nostr_relay_builder::prelude::Event,
        _addr: &'a SocketAddr,
    ) -> BoxedFuture<'a, WritePolicyResult> {
        Box::pin(async move {
            match event.kind.as_u16() {
                KIND_REPOSITORY_ANNOUNCEMENT => self.handle_announcement(event).await,
                KIND_REPOSITORY_STATE => self.handle_state(event).await,
                KIND_PR | KIND_PR_UPDATE => self.handle_pr_event(event).await,
                KIND_USER_GRASP_LIST => {
                    // Accept all kind 10317 (User Grasp List) events
                    // for better GRASP repository discovery
                    tracing::debug!(
                        event_id = %event.id.to_bech32().unwrap_or_else(|_| event.id.to_hex()),
                        author = %event.pubkey.to_hex(),
                        "Accepted kind 10317 user grasp list"
                    );
                    WritePolicyResult::Accept
                }
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
pub async fn create_relay(
    config: &Config,
    purgatory: Arc<crate::purgatory::Purgatory>,
) -> Result<RelayWithDatabase> {
    tracing::info!("Configuring nostr relay with GRASP-01 validation...");

    // Determine database path
    let db_path = Path::new(&config.relay_data_path);

    // Create database based on configuration
    let database: SharedDatabase = match config.database_backend {
        DatabaseBackend::Memory => {
            tracing::info!("Using in-memory database (no persistence)");
            Arc::new(MemoryDatabase::with_opts(MemoryDatabaseOptions {
                events: true,
                max_events: Some(NonZeroUsize::new(100_000).unwrap()),
            }))
        }
        DatabaseBackend::NostrDb => {
            tracing::info!("Using NostrDB backend at: {}", db_path.display());
            // TODO: Implement NostrDB backend once nostr-relay-builder supports it
            // For now, fall back to memory database
            tracing::warn!("NostrDB backend not yet implemented, using in-memory database");
            Arc::new(MemoryDatabase::with_opts(MemoryDatabaseOptions {
                events: true,
                max_events: Some(NonZeroUsize::new(100_000).unwrap()),
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
            Arc::new(NostrLmdb::open(db_path).await.map_err(|e| {
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

    // Create write policy with purgatory integration
    let write_policy =
        Nip34WritePolicy::new(&config.domain, database.clone(), &git_data_path, purgatory);

    let relay = LocalRelayBuilder::default()
        .database(database.clone())
        .write_policy(write_policy.clone())
        .build();

    tracing::info!(
        "Relay configured with GRASP-01 validation for domain: {}",
        config.domain
    );

    Ok(RelayWithDatabase {
        relay,
        database,
        write_policy,
    })
}
