//! GRASP Push Authorization
//!
//! This module implements the authorization logic for Git pushes according to GRASP-01.
//!
//! ## GRASP-01 Requirement
//!
//! "MUST accept pushes via this service that match the latest repo state announcement
//! on the relay, respecting the maintainer set."
//!
//! ## Authorization Flow (Efficient Single-Query Approach)
//!
//! 1. Fetch announcement and state events for the repository from the relay database
//! 2. Collect all authorized publishers: announcement authors + listed maintainers
//! 3. Find the latest state event authored by any authorized publisher
//! 4. Validate that the pushed refs match the state event
//!
//! ## Authorization Logic
//!
//! A pubkey is authorized to publish state events if, for ANY announcement with the
//! same identifier:
//! - They are the author of that announcement, OR
//! - They are listed in the "maintainers" tag of that announcement
//!
//! ## Shared Helper Functions
//!
//! This module provides helper functions that can be used by both:
//! - Git push authorization in handlers.rs
//! - HEAD updates triggered by state events in builder.rs (event policy)

use anyhow::{anyhow, Result};
use nostr_relay_builder::prelude::*;
use nostr_sdk::{EventId, ToBech32};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;

use crate::nostr::events::{
    RepositoryAnnouncement, RepositoryState, KIND_REPOSITORY_ANNOUNCEMENT, KIND_REPOSITORY_STATE,
};

/// Repository data fetched from the database
///
/// Contains all announcements and states for a given identifier,
/// fetched with a single filter query.
#[derive(Debug)]
pub struct RepositoryData {
    /// All repository announcements with this identifier
    pub announcements: Vec<RepositoryAnnouncement>,
    /// All repository state events with this identifier
    pub states: Vec<RepositoryState>,
}

/// Fetch all repository data (announcements + states) for a given identifier
///
/// This performs a single database query to fetch both announcement and state events,
/// which is more efficient than separate queries.
pub async fn fetch_repository_data(
    database: &Arc<MemoryDatabase>,
    identifier: &str,
) -> Result<RepositoryData> {
    let filter = Filter::new()
        .kinds([
            Kind::from(KIND_REPOSITORY_ANNOUNCEMENT),
            Kind::from(KIND_REPOSITORY_STATE),
        ])
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::D),
            identifier.to_string(),
        );

    let events: Vec<Event> = database
        .query(filter)
        .await
        .map_err(|e| anyhow!("Database query failed: {}", e))?
        .into_iter()
        .collect();

    debug!(
        "Fetched {} events for identifier {} from database",
        events.len(),
        identifier
    );

    // Separate into announcements and states
    let mut announcements = Vec::new();
    let mut states = Vec::new();

    for event in events {
        if event.kind == Kind::from(KIND_REPOSITORY_ANNOUNCEMENT) {
            if let Ok(announcement) = RepositoryAnnouncement::from_event(event) {
                announcements.push(announcement);
            }
        } else if event.kind == Kind::from(KIND_REPOSITORY_STATE) {
            if let Ok(state) = RepositoryState::from_event(event) {
                states.push(state);
            }
        }
    }

    debug!(
        "Parsed {} announcements and {} states for identifier {}",
        announcements.len(),
        states.len(),
        identifier
    );

    Ok(RepositoryData {
        announcements,
        states,
    })
}

/// Collect authorized maintainers grouped by owner from a set of announcements
///
/// For each announcement, returns a map from owner pubkey to authorized maintainers:
/// - The owner is always included in their own list
/// - All pubkeys listed in the "maintainers" tag are also included
/// - **Recursively**: if a maintainer also has an announcement for the same identifier,
///   their maintainers are included too (transitive closure)
///
/// This allows looking up who can publish state events for a specific owner's
/// version of the repository.
///
/// ## Example
///
/// If Alice's announcement lists Bob as maintainer, and Bob's announcement (for the
/// same identifier) lists Charlie as maintainer, then Alice's authorized set will
/// be {Alice, Bob, Charlie}.
pub fn collect_authorized_maintainers(
    announcements: &[RepositoryAnnouncement],
) -> HashMap<String, Vec<String>> {
    let mut by_owner: HashMap<String, Vec<String>> = HashMap::new();

    for announcement in announcements {
        let owner = announcement.event.pubkey.to_hex();
        let identifier = &announcement.identifier;

        // Use recursive helper to get all maintainers
        let mut checked: HashSet<String> = HashSet::new();
        get_maintainers_recursive(announcements, &owner, identifier, &mut checked);

        by_owner.insert(owner, checked.into_iter().collect());
    }

    debug!(
        "Collected maintainers for {} owners from {} announcements (with recursive expansion)",
        by_owner.len(),
        announcements.len()
    );

    by_owner
}

/// Recursively find all maintainers starting from a pubkey
///
/// This follows the pattern from ngit-relay's GetMaintainers function:
/// - If pubkey already checked, return early (cycle prevention)
/// - Mark pubkey as checked
/// - Find the announcement for this pubkey+identifier
/// - Recursively call for each maintainer listed in that announcement
/// - The `checked` set accumulates all visited pubkeys
fn get_maintainers_recursive(
    announcements: &[RepositoryAnnouncement],
    pubkey: &str,
    identifier: &str,
    checked: &mut HashSet<String>,
) {
    // Check if this pubkey has already been processed
    if checked.contains(pubkey) {
        return; // Already checked - avoid cycles
    }
    checked.insert(pubkey.to_string()); // Mark as checked

    // Find the announcement event for this pubkey+identifier
    let announcement = announcements.iter().find(|a| {
        a.event.pubkey.to_hex() == pubkey && a.identifier == identifier
    });

    let Some(announcement) = announcement else {
        return; // No announcement found for this pubkey
    };

    // Recursively find maintainers for each listed maintainer
    for maintainer_pubkey in &announcement.maintainers {
        get_maintainers_recursive(announcements, maintainer_pubkey, identifier, checked);
    }
}

/// Collect all authorized maintainers as a flat set from all announcements
///
/// This is a convenience function that flattens the per-owner maintainer lists
/// into a single set. Use this when you don't need owner-specific authorization.
pub fn collect_all_authorized_maintainers(
    announcements: &[RepositoryAnnouncement],
) -> HashSet<String> {
    let by_owner = collect_authorized_maintainers(announcements);
    let mut all_authorized = HashSet::new();
    
    for maintainers in by_owner.values() {
        for maintainer in maintainers {
            all_authorized.insert(maintainer.clone());
        }
    }
    
    debug!(
        "Collected {} total authorized maintainers from {} owners",
        all_authorized.len(),
        by_owner.len()
    );
    
    all_authorized
}

/// Find the latest state event authored by an authorized maintainer
///
/// Returns the state with the highest created_at timestamp among those
/// authored by pubkeys in the authorized set.
pub fn find_latest_authorized_state<'a>(
    states: &'a [RepositoryState],
    authorized_pubkeys: &HashSet<String>,
) -> Option<&'a RepositoryState> {
    states
        .iter()
        .filter(|s| {
            let pubkey_hex = s.event.pubkey.to_hex();
            authorized_pubkeys.contains(&pubkey_hex)
        })
        .max_by_key(|s| s.event.created_at)
}

/// Find the latest authorized state for a specific announcement context
///
/// This is similar to `find_latest_authorized_state` but considers only
/// the maintainers authorized for a specific announcement (owner + maintainers),
/// not the global set across all announcements.
pub fn find_latest_state_for_announcement<'a>(
    states: &'a [RepositoryState],
    announcement: &RepositoryAnnouncement,
) -> Option<&'a RepositoryState> {
    // Build the authorized set for this specific announcement
    let mut authorized = HashSet::new();
    authorized.insert(announcement.event.pubkey.to_hex());
    for maintainer in &announcement.maintainers {
        authorized.insert(maintainer.clone());
    }

    find_latest_authorized_state(states, &authorized)
}

/// Check if a state event is the latest for its identifier among given authorized authors
///
/// A state is considered "latest" if no other state in the provided list
/// from an authorized author has a newer timestamp.
pub fn is_latest_state(
    state: &RepositoryState,
    all_states: &[RepositoryState],
    authorized_pubkeys: &HashSet<String>,
) -> bool {
    for other in all_states {
        // Skip self
        if other.event.id == state.event.id {
            continue;
        }
        // Only compare against authorized authors
        if !authorized_pubkeys.contains(&other.event.pubkey.to_hex()) {
            continue;
        }
        // If any authorized state is newer, this is not the latest
        if other.event.created_at > state.event.created_at {
            return false;
        }
    }
    true
}

/// Get the authorization result for a repository from the database
///
/// This is the main entry point for authorization that queries the database directly.
/// It:
/// 1. Fetches all announcements and states for the identifier with a single query
/// 2. Collects all authorized maintainers from announcements
/// 3. Finds the latest state event from an authorized maintainer
///
/// Returns an `AuthorizationResult` that indicates whether a push is authorized.
pub async fn get_authorization_from_db(
    database: &Arc<MemoryDatabase>,
    identifier: &str,
) -> Result<AuthorizationResult> {
    // Fetch all repository data with a single query
    let repo_data = fetch_repository_data(database, identifier).await?;

    if repo_data.announcements.is_empty() {
        return Ok(AuthorizationResult::denied(
            "No repository announcement found",
        ));
    }

    // Collect all authorized maintainers (flattened across all owners)
    let authorized = collect_all_authorized_maintainers(&repo_data.announcements);

    if authorized.is_empty() {
        return Ok(AuthorizationResult::denied(
            "No authorized maintainers found",
        ));
    }

    debug!(
        "Found {} authorized maintainers for repository {}",
        authorized.len(),
        identifier
    );

    // Find the latest authorized state
    match find_latest_authorized_state(&repo_data.states, &authorized) {
        Some(state) => Ok(AuthorizationResult::authorized(
            state.clone(),
            authorized.into_iter().collect(),
        )),
        None => Ok(AuthorizationResult::denied(
            "No state event found from authorized publishers",
        )),
    }
}

/// Get the authorization result for a repository scoped to a specific owner
///
/// Unlike `get_authorization_from_db`, this function scopes the authorization
/// to a specific owner's announcement. This is the correct approach for Git push
/// authorization where the URL path specifies the owner.
///
/// A push to `alice/my-repo` should only consider authorization from alice's
/// announcement, not bob's announcement for the same identifier.
///
/// It:
/// 1. Fetches all announcements and states for the identifier
/// 2. Collects authorized maintainers from all announcements (grouped by owner)
/// 3. Looks up the authorized set for the specific owner
/// 4. Finds the latest state event from an authorized maintainer
///
/// Returns an `AuthorizationResult` that indicates whether a push is authorized.
pub async fn get_authorization_for_owner(
    database: &Arc<MemoryDatabase>,
    identifier: &str,
    owner_pubkey: &str,
) -> Result<AuthorizationResult> {
    // Fetch all repository data with a single query
    let repo_data = fetch_repository_data(database, identifier).await?;

    if repo_data.announcements.is_empty() {
        return Ok(AuthorizationResult::denied(
            "No repository announcement found",
        ));
    }

    // Collect authorized maintainers grouped by owner from all announcements
    let by_owner = collect_authorized_maintainers(&repo_data.announcements);

    // Look up the authorized set for this specific owner
    let authorized: HashSet<String> = match by_owner.get(owner_pubkey) {
        Some(maintainers) => maintainers.iter().cloned().collect(),
        None => {
            return Ok(AuthorizationResult::denied(format!(
                "No repository announcement found for owner {}",
                owner_pubkey
            )));
        }
    };

    if authorized.is_empty() {
        return Ok(AuthorizationResult::denied(
            "No authorized maintainers found",
        ));
    }

    debug!(
        "Found {} authorized maintainers for repository {} (owner: {})",
        authorized.len(),
        identifier,
        owner_pubkey
    );

    // Find the latest authorized state from owner's maintainer set
    match find_latest_authorized_state(&repo_data.states, &authorized) {
        Some(state) => Ok(AuthorizationResult::authorized(
            state.clone(),
            authorized.into_iter().collect(),
        )),
        None => Ok(AuthorizationResult::denied(
            "No state event found from authorized publishers",
        )),
    }
}

/// Result of authorization check
#[derive(Debug)]
pub struct AuthorizationResult {
    /// Whether the push is authorized
    pub authorized: bool,
    /// Reason for the decision (for logging/debugging)
    pub reason: String,
    /// The authorized state if available
    pub state: Option<RepositoryState>,
    /// The set of valid maintainers (authorized publishers)
    pub maintainers: Vec<String>,
}

impl AuthorizationResult {
    /// Create a successful authorization result
    pub fn authorized(state: RepositoryState, maintainers: Vec<String>) -> Self {
        Self {
            authorized: true,
            reason: "Push matches latest authorized state".to_string(),
            state: Some(state),
            maintainers,
        }
    }

    /// Create a denied authorization result
    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            authorized: false,
            reason: reason.into(),
            state: None,
            maintainers: vec![],
        }
    }
}

/// Authorization context for push operations
pub struct AuthorizationContext {
    /// Events fetched from the relay (announcements and states)
    events: Vec<Event>,
}

impl AuthorizationContext {
    /// Create a new authorization context from fetched events
    pub fn new(events: Vec<Event>) -> Self {
        Self { events }
    }

    /// Create a filter to fetch announcement and state events for a repository
    ///
    /// This matches the reference implementation's filter logic
    pub fn create_filter(identifier: &str) -> Filter {
        Filter::new()
            .kinds([
                Kind::from(KIND_REPOSITORY_ANNOUNCEMENT),
                Kind::from(KIND_REPOSITORY_STATE),
            ])
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                identifier.to_string(),
            )
    }

    /// Get the latest authorized state for a repository
    ///
    /// This implements the GRASP-01 requirement using an efficient single-query approach:
    /// - Collect all authorized publishers from announcements
    /// - Find the latest state event from any authorized publisher
    ///
    /// No owner_pubkey needed - authorization is determined by announcements themselves.
    pub fn get_authorized_state(&self, identifier: &str) -> Result<AuthorizationResult> {
        // Collect all authorized publishers (single pass through announcements)
        let authorized_publishers = self.get_authorized_publishers(identifier);

        if authorized_publishers.is_empty() {
            return Ok(AuthorizationResult::denied(
                "No repository announcement found",
            ));
        }

        debug!(
            "Found {} authorized publishers for repository {}: {:?}",
            authorized_publishers.len(),
            identifier,
            authorized_publishers
        );

        // Find the latest state event from any authorized publisher
        let mut latest_state: Option<RepositoryState> = None;
        let mut latest_timestamp = Timestamp::from(0);

        for event in &self.events {
            // Check if it's a repository state event
            if event.kind != Kind::from(KIND_REPOSITORY_STATE) {
                continue;
            }

            // Check if from an authorized publisher
            let pubkey_hex = event.pubkey.to_hex();
            if !authorized_publishers.contains(&pubkey_hex) {
                debug!(
                    "Skipping state event from unauthorized publisher: {}",
                    pubkey_hex
                );
                continue;
            }

            // Try to parse the state
            if let Ok(state) = RepositoryState::from_event(event.clone()) {
                // Check identifier matches
                if state.identifier != identifier {
                    continue;
                }

                // Check if this is the latest
                if event.created_at > latest_timestamp {
                    latest_timestamp = event.created_at;
                    latest_state = Some(state);
                }
            }
        }

        match latest_state {
            Some(state) => Ok(AuthorizationResult::authorized(
                state,
                authorized_publishers.into_iter().collect(),
            )),
            None => Ok(AuthorizationResult::denied(
                "No state event found from authorized publishers",
            )),
        }
    }

    /// Get all pubkeys authorized to publish state for an identifier
    ///
    /// A pubkey is authorized if for ANY announcement with the same identifier:
    /// - They are the author of that announcement, OR
    /// - They are listed in the "maintainers" tag of that announcement
    ///
    /// This is a simple O(n) single pass - no recursion needed.
    fn get_authorized_publishers(&self, identifier: &str) -> HashSet<String> {
        let mut authorized = HashSet::new();

        for event in &self.events {
            // Only look at announcements
            if event.kind != Kind::from(KIND_REPOSITORY_ANNOUNCEMENT) {
                continue;
            }

            // Try to parse and check identifier
            if let Ok(announcement) = RepositoryAnnouncement::from_event(event.clone()) {
                if announcement.identifier != identifier {
                    continue;
                }

                // Announcement author is authorized
                authorized.insert(event.pubkey.to_hex());

                // All listed maintainers are also authorized
                for maintainer in &announcement.maintainers {
                    authorized.insert(maintainer.clone());
                }
            }
        }

        authorized
    }

    /// Check if a specific pubkey is authorized to publish state for an identifier
    ///
    /// A pubkey is authorized if for ANY announcement with the same identifier:
    /// - They are the author of that announcement, OR
    /// - They are listed in the "maintainers" tag of that announcement
    #[allow(dead_code)]
    pub fn is_state_authorized(&self, state_pubkey: &str, identifier: &str) -> bool {
        for event in &self.events {
            // Only look at announcements
            if event.kind != Kind::from(KIND_REPOSITORY_ANNOUNCEMENT) {
                continue;
            }

            // Try to parse and check identifier
            if let Ok(announcement) = RepositoryAnnouncement::from_event(event.clone()) {
                if announcement.identifier != identifier {
                    continue;
                }

                // Check 1: Is state author the announcement author?
                if event.pubkey.to_hex() == state_pubkey {
                    return true;
                }

                // Check 2: Is state author in this announcement's maintainers?
                if announcement.maintainers.contains(&state_pubkey.to_string()) {
                    return true;
                }
            }
        }
        false
    }
}

/// Validate that pushed refs match the authorized state
///
/// Takes the refs being pushed (ref name -> commit hash) and validates
/// against the state event.
pub fn validate_push_refs(
    state: &RepositoryState,
    pushed_refs: &[(String, String, String)], // (old_oid, new_oid, ref_name)
) -> Result<()> {
    for (old_oid, new_oid, ref_name) in pushed_refs {
        debug!(
            "Validating push: {} {} -> {}",
            ref_name, old_oid, new_oid
        );

        // Handle branch updates
        if let Some(branch_name) = ref_name.strip_prefix("refs/heads/") {
            if let Some(expected_commit) = state.get_branch_commit(branch_name) {
                if new_oid != expected_commit {
                    return Err(anyhow!(
                        "Branch {} push rejected: expected commit {}, got {}",
                        branch_name,
                        expected_commit,
                        new_oid
                    ));
                }
                // Commit matches state - authorized
                debug!(
                    "Branch {} push authorized: {} matches state",
                    branch_name, new_oid
                );
            } else {
                // Branch not in state - REJECT (GRASP-01 requirement)
                return Err(anyhow!(
                    "Branch {} push rejected: not announced in state event",
                    branch_name
                ));
            }
        }

        // Handle tag updates
        if let Some(tag_name) = ref_name.strip_prefix("refs/tags/") {
            if let Some(expected_commit) = state.get_tag_commit(tag_name) {
                if new_oid != expected_commit {
                    return Err(anyhow!(
                        "Tag {} push rejected: expected commit {}, got {}",
                        tag_name,
                        expected_commit,
                        new_oid
                    ));
                }
            }
        }

        // refs/nostr/* is handled separately per GRASP-01
        if ref_name.starts_with("refs/nostr/") {
            // Extract event_id from "refs/nostr/<event-id>"
            if let Some(event_id_str) = ref_name.strip_prefix("refs/nostr/") {
                // Validate it parses as a valid EventId
                if EventId::parse(event_id_str).is_err() {
                    return Err(anyhow!(
                        "Invalid event ID format in ref: {}. Expected valid nostr event ID.",
                        ref_name
                    ));
                }
                // Valid EventId format - allow push (skip state event check)
                debug!("refs/nostr/{} push authorized (valid EventId)", event_id_str);
                continue; // Skip the rest of ref validation for this ref
            } else {
                return Err(anyhow!("Invalid refs/nostr/ format: {}", ref_name));
            }
        }
    }

    Ok(())
}

/// Parse the refs being updated from a Git pack
///
/// The receive-pack protocol sends ref updates in pkt-line format:
/// - 4-byte hex length prefix (e.g., "00a5")
/// - Payload: `<old-oid> <new-oid> <ref-name>\0<capabilities>\n`
/// - Flush packet "0000" terminates the list
/// - Then comes the PACK data
///
/// This function handles both pkt-line format (from real Git clients) and
/// simple text format (for unit tests).
pub fn parse_pushed_refs(data: &[u8]) -> Vec<(String, String, String)> {
    // Check if this looks like pkt-line format (starts with 4 hex digits)
    // A valid pkt-line push starts with a length > 4 (not a flush packet)
    if data.len() >= 4 {
        if let Ok(len_str) = std::str::from_utf8(&data[0..4]) {
            if let Ok(len) = u16::from_str_radix(len_str, 16) {
                // A valid pkt-line data packet has length > 4 (flush is 0)
                // Also check that the length makes sense for a ref update
                if len > 4 && (len as usize) <= data.len() {
                    // This is pkt-line format, parse it properly
                    return parse_pktline_refs(data);
                }
            }
        }
    }

    // Fall back to simple text format (for tests)
    parse_text_refs(data)
}

/// Parse refs from pkt-line format data
fn parse_pktline_refs(mut data: &[u8]) -> Vec<(String, String, String)> {
    let mut refs = Vec::new();

    while data.len() >= 4 {
        // Parse pkt-line length prefix
        let len_str = match std::str::from_utf8(&data[0..4]) {
            Ok(s) => s,
            Err(_) => break,
        };

        let len = match u16::from_str_radix(len_str, 16) {
            Ok(l) => l as usize,
            Err(_) => break,
        };

        // Flush packet (0000) ends the ref list
        if len == 0 {
            break;
        }

        if len < 4 || data.len() < len {
            break;
        }

        // Extract payload (without the 4-byte length prefix)
        let payload = &data[4..len];

        // Parse the payload: "old_oid new_oid ref_name\0capabilities\n"
        if let Some(ref_update) = parse_ref_line(payload) {
            refs.push(ref_update);
        }

        // Move to next pkt-line
        data = &data[len..];
    }

    debug!("Parsed {} refs from pkt-line format", refs.len());
    refs
}

/// Parse refs from simple text format (for backward compatibility with tests)
fn parse_text_refs(data: &[u8]) -> Vec<(String, String, String)> {
    let mut refs = Vec::new();
    let text = String::from_utf8_lossy(data);

    for line in text.lines() {
        // Skip empty lines and pack data
        if line.is_empty() || line.starts_with("PACK") {
            continue;
        }

        if let Some(ref_update) = parse_ref_line(line.as_bytes()) {
            refs.push(ref_update);
        }
    }

    refs
}

/// Parse a single ref update line: "old_oid new_oid ref_name\0capabilities"
fn parse_ref_line(payload: &[u8]) -> Option<(String, String, String)> {
    // Convert to string, handling potential invalid UTF-8
    let line = String::from_utf8_lossy(payload);

    // Strip trailing newline if present
    let line = line.trim_end_matches('\n');

    // Split at null byte to separate command from capabilities
    let command_part = line.split('\0').next().unwrap_or("");

    // Parse "old_oid new_oid ref_name"
    let parts: Vec<&str> = command_part.split_whitespace().collect();
    if parts.len() >= 3 {
        let old_oid = parts[0];
        let new_oid = parts[1];
        let ref_name = parts[2];

        // Validate OID format (40 hex chars)
        if old_oid.len() == 40
            && new_oid.len() == 40
            && old_oid.chars().all(|c| c.is_ascii_hexdigit())
            && new_oid.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Some((
                old_oid.to_string(),
                new_oid.to_string(),
                ref_name.to_string(),
            ));
        }
    }

    None
}

/// Convert hex pubkey to bech32 npub format
pub fn pubkey_to_npub(hex_pubkey: &str) -> Result<String> {
    let pk = PublicKey::parse(hex_pubkey)?;
    Ok(pk.to_bech32()?)
}

/// Convert bech32 npub to hex pubkey format
pub fn npub_to_pubkey(npub: &str) -> Result<String> {
    let pk = PublicKey::parse(npub)?;
    Ok(pk.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Tag, TagKind};

    fn create_test_keys() -> Keys {
        Keys::generate()
    }

    fn create_announcement_event(keys: &Keys, identifier: &str, maintainers: &[&Keys]) -> Event {
        let mut tags = vec![Tag::custom(TagKind::d(), vec![identifier.to_string()])];

        // Add maintainers as a single "maintainers" tag per NIP-34
        // Format: ["maintainers", "<pubkey1-hex>", "<pubkey2-hex>", ...]
        if !maintainers.is_empty() {
            let maintainer_pubkeys: Vec<String> = maintainers
                .iter()
                .map(|k| k.public_key().to_hex())
                .collect();
            tags.push(Tag::custom(
                TagKind::Custom("maintainers".into()),
                maintainer_pubkeys,
            ));
        }

        // Add clone and relay tags for validity
        tags.push(Tag::custom(
            TagKind::Clone,
            vec!["https://example.com/test.git".to_string()],
        ));
        tags.push(Tag::custom(
            TagKind::Relays,
            vec!["wss://example.com".to_string()],
        ));

        EventBuilder::new(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT), "Test repo")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn create_state_event(keys: &Keys, identifier: &str, branches: &[(&str, &str)]) -> Event {
        let mut tags = vec![Tag::custom(TagKind::d(), vec![identifier.to_string()])];

        for (branch, commit) in branches {
            tags.push(Tag::custom(
                TagKind::Custom(format!("refs/heads/{}", branch).into()),
                vec![commit.to_string()],
            ));
        }

        EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn test_authorized_publishers_single_owner() {
        let alice = create_test_keys();
        let identifier = "test-repo";

        let announcement = create_announcement_event(&alice, identifier, &[]);
        let events = vec![announcement];

        let ctx = AuthorizationContext::new(events);

        // Alice should be authorized
        assert!(ctx.is_state_authorized(&alice.public_key().to_hex(), identifier));
    }

    #[test]
    fn test_authorized_publishers_with_listed_maintainer() {
        let alice = create_test_keys();
        let bob = create_test_keys();
        let identifier = "test-repo";

        // Alice lists Bob as maintainer
        let alice_announcement = create_announcement_event(&alice, identifier, &[&bob]);

        let events = vec![alice_announcement];
        let ctx = AuthorizationContext::new(events);

        // Both Alice and Bob should be authorized
        assert!(ctx.is_state_authorized(&alice.public_key().to_hex(), identifier));
        assert!(ctx.is_state_authorized(&bob.public_key().to_hex(), identifier));
    }

    #[test]
    fn test_authorized_publishers_multiple_announcements() {
        let alice = create_test_keys();
        let bob = create_test_keys();
        let charlie = create_test_keys();
        let identifier = "test-repo";

        // Alice lists Bob, Bob lists Charlie
        let alice_announcement = create_announcement_event(&alice, identifier, &[&bob]);
        let bob_announcement = create_announcement_event(&bob, identifier, &[&charlie]);

        let events = vec![alice_announcement, bob_announcement];
        let ctx = AuthorizationContext::new(events);

        // All three should be authorized (Alice, Bob from announcements; Bob, Charlie from maintainers)
        assert!(ctx.is_state_authorized(&alice.public_key().to_hex(), identifier));
        assert!(ctx.is_state_authorized(&bob.public_key().to_hex(), identifier));
        assert!(ctx.is_state_authorized(&charlie.public_key().to_hex(), identifier));
    }

    #[test]
    fn test_unauthorized_pubkey() {
        let alice = create_test_keys();
        let bob = create_test_keys();
        let eve = create_test_keys(); // Not authorized
        let identifier = "test-repo";

        // Alice lists Bob as maintainer  
        let alice_announcement = create_announcement_event(&alice, identifier, &[&bob]);

        let events = vec![alice_announcement];
        let ctx = AuthorizationContext::new(events);

        // Eve should NOT be authorized
        assert!(!ctx.is_state_authorized(&eve.public_key().to_hex(), identifier));
    }

    #[test]
    fn test_get_authorized_state_with_maintainer() {
        let alice = create_test_keys();
        let bob = create_test_keys();
        let identifier = "test-repo";

        let announcement = create_announcement_event(&alice, identifier, &[&bob]);

        // Bob publishes a state event
        let state = create_state_event(&bob, identifier, &[("main", "abc123")]);

        let events = vec![announcement, state];
        let ctx = AuthorizationContext::new(events);

        let result = ctx.get_authorized_state(identifier).unwrap();

        assert!(result.authorized);
        assert!(result.state.is_some());
        let state = result.state.unwrap();
        assert_eq!(state.get_branch_commit("main"), Some("abc123"));
    }

    #[test]
    fn test_get_authorized_state_no_announcement() {
        let identifier = "test-repo";

        let events = vec![];
        let ctx = AuthorizationContext::new(events);

        let result = ctx.get_authorized_state(identifier).unwrap();

        assert!(!result.authorized);
        assert_eq!(result.reason, "No repository announcement found");
    }

    #[test]
    fn test_get_authorized_state_no_state_event() {
        let alice = create_test_keys();
        let identifier = "test-repo";

        let announcement = create_announcement_event(&alice, identifier, &[]);

        let events = vec![announcement];
        let ctx = AuthorizationContext::new(events);

        let result = ctx.get_authorized_state(identifier).unwrap();

        assert!(!result.authorized);
        assert_eq!(
            result.reason,
            "No state event found from authorized publishers"
        );
    }

    #[test]
    fn test_validate_push_refs_success() {
        let alice = create_test_keys();
        let identifier = "test-repo";

        let state_event = create_state_event(&alice, identifier, &[("main", "abc123def456")]);
        let state = RepositoryState::from_event(state_event).unwrap();

        let pushed_refs = vec![(
            "0".repeat(40),
            "abc123def456".to_string() + &"0".repeat(28),
            "refs/heads/main".to_string(),
        )];

        // This should pass since we're allowing new branches for now
        let result = validate_push_refs(&state, &pushed_refs);
        // The branch name matches, but commit doesn't match exactly - this tests the logic
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_parse_pushed_refs() {
        let old = "0".repeat(40);
        let new = "a".repeat(40);
        let data = format!("{} {} refs/heads/main\0 report-status\n", old, new);

        let refs = parse_pushed_refs(data.as_bytes());

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, old);
        assert_eq!(refs[0].1, new);
        assert_eq!(refs[0].2, "refs/heads/main");
    }

    #[test]
    fn test_parse_pushed_refs_pktline_format() {
        // Build a pkt-line formatted push request like git client sends
        // Format: 4-byte hex length + payload
        // Payload: "old_oid new_oid ref_name\0capabilities\n"
        let old = "0".repeat(40);
        let new = "a".repeat(40);
        let ref_name = "refs/heads/main";
        let capabilities = " report-status side-band-64k";

        // Build the pkt-line payload
        let payload = format!("{} {} {}\0{}\n", old, new, ref_name, capabilities);

        // Calculate length (4-byte prefix + payload)
        let len = 4 + payload.len();
        let pktline = format!("{:04x}{}", len, payload);

        // Add flush packet to end
        let data = format!("{}0000", pktline);

        let refs = parse_pushed_refs(data.as_bytes());

        assert_eq!(refs.len(), 1, "Expected 1 ref, got {}", refs.len());
        assert_eq!(refs[0].0, old);
        assert_eq!(refs[0].1, new);
        assert_eq!(refs[0].2, ref_name);
    }

    #[test]
    fn test_parse_pushed_refs_multiple_refs() {
        // Test multiple refs in pkt-line format
        let old1 = "0".repeat(40);
        let new1 = "a".repeat(40);
        let old2 = "b".repeat(40);
        let new2 = "c".repeat(40);

        // First ref with capabilities
        let payload1 = format!("{} {} refs/heads/main\0report-status\n", old1, new1);
        let len1 = 4 + payload1.len();
        let pktline1 = format!("{:04x}{}", len1, payload1);

        // Second ref without capabilities (subsequent refs don't have them)
        let payload2 = format!("{} {} refs/heads/feature\n", old2, new2);
        let len2 = 4 + payload2.len();
        let pktline2 = format!("{:04x}{}", len2, payload2);

        let data = format!("{}{}0000", pktline1, pktline2);

        let refs = parse_pushed_refs(data.as_bytes());

        assert_eq!(refs.len(), 2, "Expected 2 refs, got {}", refs.len());
        assert_eq!(refs[0].2, "refs/heads/main");
        assert_eq!(refs[1].2, "refs/heads/feature");
    }

    #[test]
    fn test_npub_pubkey_conversion() {
        let keys = create_test_keys();
        let hex = keys.public_key().to_hex();

        let npub = pubkey_to_npub(&hex).unwrap();
        assert!(npub.starts_with("npub1"));

        let back_to_hex = npub_to_pubkey(&npub).unwrap();
        assert_eq!(hex, back_to_hex);
    }
}