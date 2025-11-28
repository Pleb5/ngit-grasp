/// Nostr Relay Builder Configuration
///
/// This module integrates nostr-relay-builder with NIP-34 validation logic
/// preserved from the original implementation.
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nostr::nips::nip19::ToBech32;
use nostr::prelude::{Alphabet, SingleLetterTag};
use nostr::{EventId, Filter, Kind, PublicKey};
use nostr_relay_builder::prelude::*;

use crate::config::{Config, DatabaseBackend};
use crate::git;
use crate::nostr::events::{
    validate_announcement, validate_state, RepositoryAnnouncement, RepositoryState,
    KIND_REPOSITORY_ANNOUNCEMENT, KIND_REPOSITORY_STATE,
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
    git_data_path: PathBuf,
}

impl Nip34WritePolicy {
    pub fn new(domain: impl Into<String>, database: Arc<MemoryDatabase>, git_data_path: impl Into<PathBuf>) -> Self {
        Self {
            domain: domain.into(),
            database,
            git_data_path: git_data_path.into(),
        }
    }

    /// Create a bare git repository if it doesn't exist
    /// Path format: <git_data_path>/<npub>/<identifier>.git
    fn ensure_bare_repository(&self, announcement: &RepositoryAnnouncement) -> Result<(), String> {
        let repo_path = self.git_data_path.join(&announcement.repo_path());
        
        // Check if repository already exists
        if repo_path.exists() {
            tracing::debug!("Repository already exists at {}", repo_path.display());
            return Ok(());
        }

        // Create parent directory (npub directory)
        let parent = repo_path.parent().ok_or_else(|| {
            format!("Invalid repository path: {}", repo_path.display())
        })?;
        
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("Failed to create directory {}: {}", parent.display(), e)
        })?;

        // Initialize bare repository using git command
        let output = std::process::Command::new("git")
            .args(&["init", "--bare", repo_path.to_str().unwrap()])
            .output()
            .map_err(|e| format!("Failed to execute git init: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git init failed: {}", stderr));
        }

        tracing::info!("Created bare repository at {}", repo_path.display());
        Ok(())
    }

    /// Check if this state event is the latest for its identifier among authorized authors
    ///
    /// A state is considered "latest" if no other state event in the database
    /// from an authorized author has a newer timestamp. This handles out-of-order
    /// delivery where an older event arrives after a newer one.
    ///
    /// The authorized_pubkeys should be the owner and maintainers of a specific
    /// announcement, so different owners with the same identifier don't interfere.
    async fn is_latest_state_for_identifier(
        database: &Arc<MemoryDatabase>,
        state: &RepositoryState,
        authorized_pubkeys: &[PublicKey],
    ) -> Result<bool, String> {
        let filter = Filter::new()
            .kind(Kind::from(KIND_REPOSITORY_STATE))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                state.identifier.clone(),
            );

        match database.query(filter).await {
            Ok(events) => {
                for event in events {
                    // Skip comparing to self (same event ID)
                    if event.id == state.event.id {
                        continue;
                    }
                    // Only consider events from authorized authors for this announcement
                    if !authorized_pubkeys.contains(&event.pubkey) {
                        continue;
                    }
                    // If any existing event from an authorized author is newer, this is not the latest
                    if event.created_at > state.event.created_at {
                        tracing::debug!(
                            "State {} is not latest: found newer state {} from {} (ts {} > {})",
                            state.event.id.to_hex(),
                            event.id.to_hex(),
                            event.pubkey.to_hex(),
                            event.created_at.as_secs(),
                            state.event.created_at.as_secs()
                        );
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Err(e) => Err(format!("Database query failed: {}", e)),
        }
    }

    /// Find all repository announcements where the given pubkey is authorized
    ///
    /// A pubkey is authorized for an announcement if:
    /// - They are the owner (pubkey of the announcement event), OR
    /// - They are listed in the "maintainers" tag
    ///
    /// This is needed because a maintainer can publish a state event that
    /// should update HEAD in the repository of the announcement owner,
    /// not in the maintainer's own (possibly non-existent) repository.
    async fn find_authorized_announcements(
        database: &Arc<MemoryDatabase>,
        identifier: &str,
        state_author: &PublicKey,
    ) -> Result<Vec<RepositoryAnnouncement>, String> {
        let filter = Filter::new()
            .kind(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                identifier.to_string(),
            );

        match database.query(filter).await {
            Ok(events) => {
                let mut authorized = Vec::new();
                let state_author_hex = state_author.to_hex();

                for event in events {
                    if let Ok(announcement) = RepositoryAnnouncement::from_event(event.clone()) {
                        // Check if state author is authorized for this announcement
                        let is_owner = event.pubkey == *state_author;
                        let is_maintainer = announcement.maintainers.contains(&state_author_hex);

                        if is_owner || is_maintainer {
                            tracing::debug!(
                                "Found authorized announcement for {}: owner={}, maintainer={}",
                                identifier,
                                if is_owner { event.pubkey.to_hex() } else { "n/a".to_string() },
                                is_maintainer
                            );
                            authorized.push(announcement);
                        }
                    }
                }
                Ok(authorized)
            }
            Err(e) => Err(format!("Database query failed: {}", e)),
        }
    }

    /// Try to set repository HEAD for all authorized announcement owners
    ///
    /// Per GRASP-01: "MUST set repository HEAD per repository state announcement
    /// as soon as the git data related to that branch has been received."
    ///
    /// This function:
    /// 1. Checks if this state event is the latest for the identifier
    /// 2. Finds all announcements where the state author is authorized
    /// 3. Updates HEAD in each relevant repository
    ///
    /// Returns Ok(count) with the number of repositories updated.
    async fn try_set_head_for_authorized_repos(
        &self,
        database: &Arc<MemoryDatabase>,
        state: &RepositoryState,
    ) -> Result<usize, String> {
        // Check if state has a HEAD reference
        let head_ref = match &state.head {
            Some(h) => h,
            None => {
                tracing::debug!(
                    "State event for {} has no HEAD reference",
                    state.identifier
                );
                return Ok(0);
            }
        };

        // Get the branch name and commit
        let branch_name = match state.get_head_branch() {
            Some(b) => b,
            None => {
                tracing::debug!(
                    "State event for {} has invalid HEAD format: {}",
                    state.identifier,
                    head_ref
                );
                return Ok(0);
            }
        };

        let head_commit = match state.get_branch_commit(branch_name) {
            Some(c) => c,
            None => {
                tracing::debug!(
                    "State event for {} HEAD branch {} has no commit in state",
                    state.identifier,
                    branch_name
                );
                return Ok(0);
            }
        };

        // Find all announcements where state author is authorized
        let announcements = Self::find_authorized_announcements(
            database,
            &state.identifier,
            &state.event.pubkey,
        ).await?;

        if announcements.is_empty() {
            tracing::debug!(
                "No authorized announcements found for state {} by {}",
                state.identifier,
                state.event.pubkey.to_hex()
            );
            return Ok(0);
        }

        // Update HEAD in each authorized announcement's repository
        let mut updated_count = 0;
        for announcement in &announcements {
            // Build the list of authorized pubkeys for this specific announcement
            // (owner + maintainers)
            let mut authorized_pubkeys = vec![announcement.event.pubkey];
            for maintainer_hex in &announcement.maintainers {
                if let Ok(pk) = PublicKey::from_hex(maintainer_hex) {
                    authorized_pubkeys.push(pk);
                }
            }

            // Check if this is the latest state event for THIS announcement's context
            // Different owners with the same identifier should not interfere
            if !Self::is_latest_state_for_identifier(database, state, &authorized_pubkeys).await? {
                tracing::debug!(
                    "Skipping HEAD update for {} in {}'s repo - not the latest state event for this context",
                    state.identifier,
                    announcement.event.pubkey.to_hex()
                );
                continue;
            }

            // Build repository path: <git_data_path>/<owner_npub>/<identifier>.git
            let repo_path = self.git_data_path.join(&announcement.repo_path());

            match git::try_set_head_if_available(&repo_path, head_ref, head_commit) {
                Ok(true) => {
                    tracing::info!(
                        "Set HEAD to {} in repository {} (from state by {})",
                        head_ref,
                        repo_path.display(),
                        state.event.pubkey.to_hex()
                    );
                    updated_count += 1;
                }
                Ok(false) => {
                    tracing::debug!(
                        "HEAD commit {} not available yet in {}",
                        head_commit,
                        repo_path.display()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to set HEAD in {}: {}",
                        repo_path.display(),
                        e
                    );
                }
            }
        }

        Ok(updated_count)
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

    /// Check if any addressable events (repositories) exist in database
    /// Returns the first matching addressable reference found, or None if none match
    async fn find_accepted_repository(
        database: &Arc<MemoryDatabase>,
        addressables: &[String],
    ) -> Result<Option<String>, String> {
        if addressables.is_empty() {
            return Ok(None);
        }

        // Parse all addressable references
        let mut parsed_refs = Vec::new();
        for addr in addressables {
            let parts: Vec<&str> = addr.split(':').collect();
            if parts.len() < 3 {
                continue; // Skip invalid format
            }

            let kind = match parts[0].parse::<u16>() {
                Ok(k) => k,
                Err(_) => continue, // Skip invalid kind
            };
            let pubkey = match PublicKey::from_hex(parts[1]) {
                Ok(pk) => pk,
                Err(_) => continue, // Skip invalid pubkey
            };
            let identifier = parts[2].to_string();

            parsed_refs.push((addr.clone(), kind, pubkey, identifier));
        }

        if parsed_refs.is_empty() {
            return Ok(None);
        }

        // Group by kind to reduce queries
        use std::collections::HashMap;
        let mut by_kind: HashMap<u16, Vec<_>> = HashMap::new();
        for (addr, kind, pubkey, identifier) in parsed_refs {
            by_kind.entry(kind).or_default().push((addr, pubkey, identifier));
        }

        // Query each kind group
        for (kind, refs) in by_kind {
            let authors: Vec<PublicKey> = refs.iter().map(|(_, pk, _)| *pk).collect();
            
            let filter = Filter::new()
                .kind(Kind::from(kind))
                .authors(authors);

            match database.query(filter).await {
                Ok(events) => {
                    // Check if any event matches our identifier requirements
                    for event in events {
                        for (addr, _pubkey, identifier) in &refs {
                            // Match identifier tag
                            if event.tags.iter().any(|tag| {
                                let tag_vec = tag.clone().to_vec();
                                tag_vec.len() >= 2 && tag_vec[0] == "d" && tag_vec[1] == *identifier
                            }) {
                                return Ok(Some(addr.clone()));
                            }
                        }
                    }
                }
                Err(e) => return Err(format!("Database query failed: {}", e)),
            }
        }

        Ok(None)
    }

    /// Check if any events exist in database
    /// Returns the first matching event ID found, or None if none match
    async fn find_accepted_event(
        database: &Arc<MemoryDatabase>,
        event_ids: &[EventId],
    ) -> Result<Option<EventId>, String> {
        if event_ids.is_empty() {
            return Ok(None);
        }

        // Single query for all event IDs
        let filter = Filter::new().ids(event_ids.iter().copied());

        match database.query(filter).await {
            Ok(events) => {
                // Get first event from the iterator
                Ok(events.into_iter().next().map(|e| e.id))
            }
            Err(e) => Err(format!("Database query failed: {}", e)),
        }
    }

    /// Check if any accepted event references this event (forward reference)
    ///
    /// For regular replaceable events (10000-19999): Checks addressable tags with kind:pubkey format
    /// For parameterized replaceable (30000-39999): Checks addressable tags with kind:pubkey:d-identifier format
    /// For regular events: Only checks event ID reference tags (e, E, q)
    ///
    /// This optimization recognizes that replaceable events are referenced by coordinate address,
    /// while regular events are referenced by event ID.
    async fn is_referenced_by_accepted(
        database: &Arc<MemoryDatabase>,
        event: &Event,
    ) -> Result<bool, String> {
        let kind_u16 = event.kind.as_u16();
        
        // Check if this is any kind of replaceable event
        let is_regular_replaceable = kind_u16 >= 10000 && kind_u16 < 20000;
        let is_parameterized_replaceable = kind_u16 >= 30000 && kind_u16 < 40000;

        if is_regular_replaceable || is_parameterized_replaceable {
            // Build the appropriate address format based on event type
            let address = if is_parameterized_replaceable {
                // For parameterized replaceable: kind:pubkey:d-identifier format (2 colons)
                let identifier = event.tags.iter()
                    .find_map(|tag| {
                        let tag_vec = tag.clone().to_vec();
                        if tag_vec.len() >= 2 && tag_vec[0] == "d" {
                            Some(tag_vec[1].clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default(); // Empty string if no 'd' tag
                format!("{}:{}:{}", event.kind.as_u16(), event.pubkey.to_hex(), identifier)
            } else {
                // For regular replaceable: kind:pubkey format (1 colon)
                format!("{}:{}", event.kind.as_u16(), event.pubkey.to_hex())
            };
            
            // Check addressable reference tags: a, A, q (with address format)
            let addressable_tags = [
                SingleLetterTag::lowercase(Alphabet::A), // 'a' - addressable event reference
                SingleLetterTag::uppercase(Alphabet::A), // 'A' - uppercase addressable reference
                SingleLetterTag::lowercase(Alphabet::Q), // 'q' - quote (can be address or ID)
            ];

            for tag_type in &addressable_tags {
                let filter = Filter::new().custom_tag(tag_type.clone(), address.clone());
                
                match database.query(filter).await {
                    Ok(events) => {
                        if !events.is_empty() {
                            return Ok(true);
                        }
                    }
                    Err(e) => return Err(format!("Database query failed: {}", e)),
                }
            }
        } else {
            // For regular events, check event ID reference tags: e, E, q (with hex ID)
            let event_id_hex = event.id.to_hex();
            
            let event_id_tags = [
                SingleLetterTag::lowercase(Alphabet::E), // 'e' - standard event reference
                SingleLetterTag::uppercase(Alphabet::E), // 'E' - NIP-22 root event reference
                SingleLetterTag::lowercase(Alphabet::Q), // 'q' - quote reference
            ];

            for tag_type in &event_id_tags {
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
                        // Parse announcement to get repository details
                        match RepositoryAnnouncement::from_event(event.clone()) {
                            Ok(announcement) => {
                                // Try to create bare repository if it doesn't exist
                                if let Err(e) = self.ensure_bare_repository(&announcement) {
                                    tracing::warn!(
                                        "Failed to create bare repository for {}: {}",
                                        event_id_str,
                                        e
                                    );
                                    // Note: We still accept the event even if repo creation fails
                                    // The git operation failure shouldn't prevent event acceptance
                                }
                                
                                tracing::debug!(
                                    "Accepted repository announcement: {}",
                                    event_id_str
                                );
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
                    Err(e) => {
                        tracing::warn!(
                            "Rejected repository announcement {}: {}",
                            event_id_str,
                            e
                        );
                        PolicyResult::Reject(e.to_string())
                    }
                },
                KIND_REPOSITORY_STATE => match validate_state(event) {
                    Ok(_) => {
                        // Parse state to get HEAD and branch info
                        match RepositoryState::from_event(event.clone()) {
                            Ok(state) => {
                                // Try to set HEAD for all authorized repos if this is the latest state
                                match self.try_set_head_for_authorized_repos(&database, &state).await {
                                    Ok(count) if count > 0 => {
                                        tracing::info!(
                                            "Set HEAD from state event {} for {} repo(s) with identifier {}",
                                            event_id_str,
                                            count,
                                            state.identifier
                                        );
                                    }
                                    Ok(_) => {
                                        tracing::debug!(
                                            "HEAD not set from state {} - git data not available yet or not latest",
                                            event_id_str
                                        );
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to process HEAD from state {}: {}",
                                            event_id_str,
                                            e
                                        );
                                    }
                                }
                                
                                tracing::debug!(
                                    "Accepted repository state: {}",
                                    event_id_str
                                );
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

                    // Check 1: Does this event reference an accepted repository? (batched)
                    match Self::find_accepted_repository(&database, &addressable_refs).await {
                        Ok(Some(addr_ref)) => {
                            tracing::debug!(
                                "Accepted event {}: references accepted repository {}",
                                event_id_str,
                                addr_ref
                            );
                            return PolicyResult::Accept;
                        }
                        Ok(None) => {
                            // No matching repositories, continue to next check
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

                    // Check 2: Does this event reference an accepted event? (batched, transitive)
                    match Self::find_accepted_event(&database, &event_refs).await {
                        Ok(Some(event_ref)) => {
                            tracing::debug!(
                                "Accepted event {}: references accepted event {}",
                                event_id_str,
                                event_ref
                            );
                            return PolicyResult::Accept;
                        }
                        Ok(None) => {
                            // No matching events, continue to next check
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

                    // Check 3: Is this event referenced by an accepted event? (forward reference)
                    match Self::is_referenced_by_accepted(&database, event).await {
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

    // Create database based on configuration
    let database = match config.database_backend {
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
            // TODO: Implement LMDB backend once nostr-relay-builder supports it
            // For now, fall back to memory database
            tracing::warn!("LMDB backend not yet implemented, using in-memory database");
            Arc::new(MemoryDatabase::with_opts(MemoryDatabaseOptions {
                events: true,
                max_events: Some(100_000),
            }))
        }
    };

    // Build relay with GRASP-01 validation
    // Clone Arc for the write policy so both relay and policy can access the database
    let builder = RelayBuilder::default()
        .database(database.clone())
        .write_policy(Nip34WritePolicy::new(
            &config.domain,
            database.clone(),
            &config.git_data_path,
        ));

    tracing::info!(
        "Relay configured with GRASP-01 validation for domain: {}",
        config.domain
    );

    Ok(LocalRelay::new(builder))
}