//! Purgatory: In-memory holding area for events awaiting git data.
//!
//! Solves the "which arrives first?" problem where either nostr events or git pushes
//! can arrive in any order. Events and git data are held temporarily until their
//! counterpart arrives, at which point they can be processed together.
//!
//! ## Architecture
//!
//! - **In-memory only**: Data is lost on restart (acceptable per spec)
//! - **Thread-safe**: Uses DashMap for concurrent access from multiple handlers
//! - **Automatic expiry**: Entries expire after 30 minutes by default
//! - **Separate stores**: State events and PR events use different indexing strategies

mod helpers;
mod types;

use anyhow::{bail, Result};
pub use helpers::{can_satisfy_state, extract_refs_from_state, get_unpushed_refs};
pub use types::{PrPurgatoryEntry, RefPair, RefUpdate, StatePurgatoryEntry};

use dashmap::DashMap;
use nostr_sdk::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::git::authorization::{
    fetch_repository_data, pubkey_authorised_for_repo_owners, RepositoryData,
};
use crate::git::oid_exists;
use crate::nostr::builder::SharedDatabase;
use crate::nostr::events::RepositoryState;

/// Default expiry duration for purgatory entries (30 minutes)
const DEFAULT_EXPIRY: Duration = Duration::from_secs(1800);

/// Main purgatory structure holding events awaiting git data.
///
/// Provides thread-safe concurrent access to two separate stores:
/// - State events indexed by repository identifier
/// - PR events indexed by event ID
#[derive(Clone)]
pub struct Purgatory {
    /// State events (kind 30618) indexed by repository identifier.
    /// Multiple state events can wait for the same identifier (different maintainers).
    state_events: Arc<DashMap<String, Vec<StatePurgatoryEntry>>>,

    /// PR events (kind 1617/1618) or placeholders indexed by event ID (hex string).
    /// Event ID is from the 'e' tag in the PR event itself.
    pr_events: Arc<DashMap<String, PrPurgatoryEntry>>,

    git_data_path: PathBuf,
}

impl Purgatory {
    /// Create a new empty purgatory.
    pub fn new(git_data_path: impl Into<PathBuf>) -> Self {
        Self {
            state_events: Arc::new(DashMap::new()),
            pr_events: Arc::new(DashMap::new()),
            git_data_path: git_data_path.into(),
        }
    }

    /// Add a state event to purgatory.
    ///
    /// The event will expire after the default duration unless matched with git data.
    /// Multiple state events for the same identifier are allowed (from different authors).
    ///
    /// # Arguments
    /// * `event` - The state event (kind 30618) to hold
    /// * `identifier` - The repository identifier from the 'd' tag
    /// * `author` - The event author's public key
    pub fn add_state(&self, event: Event, identifier: String, author: PublicKey) {
        let now = Instant::now();
        let entry = StatePurgatoryEntry {
            event,
            identifier: identifier.clone(),
            author,
            created_at: now,
            expires_at: now + DEFAULT_EXPIRY,
        };

        self.state_events.entry(identifier).or_default().push(entry);
    }

    /// Trigger a background git data sync for a state event.
    ///
    /// This method spawns a background task to attempt fetching missing git data
    /// from remote servers listed in the repository announcements. It's called
    /// when a state event arrives but the required git data isn't available locally.
    ///
    /// # Arguments
    /// * `state` - The parsed repository state event
    /// * `database` - Database to query for repository announcements
    /// * `our_domain` - Our service domain to exclude from fetch targets
    pub fn start_state_sync(
        &self,
        state: RepositoryState,
        database: SharedDatabase,
        our_domain: Option<String>,
    ) {
        let git_data_path = self.git_data_path.clone();
        let identifier = state.identifier.clone();
        let event_id = state.event.id;

        tokio::spawn(async move {
            tracing::debug!(
                identifier = %identifier,
                event_id = %event_id,
                "Starting background git data sync for purgatory state event"
            );

            if let Err(e) =
                sync_state_git_data(state, &database, &git_data_path, our_domain.as_deref()).await
            {
                tracing::warn!(
                    identifier = %identifier,
                    event_id = %event_id,
                    error = %e,
                    "Failed to sync git data for purgatory state event"
                );
            }
        });
    }

    /// Add a PR event to purgatory.
    ///
    /// The event will expire after the default duration unless matched with git data.
    ///
    /// # Arguments
    /// * `event` - The PR event (kind 1617/1618) to hold
    /// * `event_id` - The event ID (hex string) from the 'e' tag
    /// * `commit` - The commit SHA from the 'c' tag
    pub fn add_pr(&self, event: Event, event_id: String, commit: String) {
        let now = Instant::now();
        let entry = PrPurgatoryEntry {
            event: Some(event),
            commit,
            created_at: now,
            expires_at: now + DEFAULT_EXPIRY,
        };

        self.pr_events.insert(event_id, entry);
    }

    /// Add a PR placeholder (git data arrived before PR event).
    ///
    /// Creates a placeholder entry waiting for the corresponding PR event.
    ///
    /// # Arguments
    /// * `event_id` - The expected event ID (from git ref name)
    /// * `commit` - The commit SHA that was pushed
    pub fn add_pr_placeholder(&self, event_id: String, commit: String) {
        let now = Instant::now();
        let entry = PrPurgatoryEntry {
            event: None, // Placeholder - no event yet
            commit,
            created_at: now,
            expires_at: now + DEFAULT_EXPIRY,
        };

        self.pr_events.insert(event_id, entry);
    }

    /// Find state events waiting for a specific repository identifier.
    ///
    /// Returns all state events (from all maintainers) waiting for git data
    /// matching this identifier.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to search for
    ///
    /// # Returns
    /// Vector of state events waiting for this identifier, or empty vec if none found
    pub fn find_state(&self, identifier: &str) -> Vec<StatePurgatoryEntry> {
        self.state_events
            .get(identifier)
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }

    /// Find a PR event or placeholder by event ID.
    ///
    /// # Arguments
    /// * `event_id` - The event ID to search for
    ///
    /// # Returns
    /// The PR entry if found, None otherwise
    pub fn find_pr(&self, event_id: &str) -> Option<PrPurgatoryEntry> {
        self.pr_events.get(event_id).map(|entry| entry.clone())
    }

    /// Find a PR placeholder specifically (git-data-first scenario).
    ///
    /// Returns the commit SHA only if a placeholder exists (entry with no event).
    /// Used to distinguish placeholders from actual PR events.
    ///
    /// # Arguments
    /// * `event_id` - The event ID to search for
    ///
    /// # Returns
    /// Some(commit_sha) if a placeholder exists, None if no entry or entry has an event
    pub fn find_pr_placeholder(&self, event_id: &str) -> Option<String> {
        self.pr_events.get(event_id).and_then(|entry| {
            if entry.event.is_none() {
                Some(entry.commit.clone())
            } else {
                None
            }
        })
    }

    /// Remove a state event from purgatory.
    ///
    /// Removes all entries for the given identifier.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to remove
    pub fn remove_state(&self, identifier: &str) {
        self.state_events.remove(identifier);
    }

    /// Remove a specific state event by comparing the full event.
    ///
    /// This allows removing a single state event while leaving others
    /// for the same identifier intact.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier
    /// * `event_id` - The specific event ID to remove
    pub fn remove_state_event(&self, identifier: &str, event_id: &EventId) {
        if let Some(mut entries) = self.state_events.get_mut(identifier) {
            entries.retain(|entry| entry.event.id != *event_id);
            if entries.is_empty() {
                drop(entries); // Release lock before removal
                self.state_events.remove(identifier);
            }
        }
    }

    /// Find state events that could be satisfied by ref updates.
    ///
    /// Returns state events waiting for this identifier where applying the
    /// ref updates to local state results in exactly the declared state.
    /// Uses late-binding ref extraction at git push time.
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier to search for
    /// * `pushed_updates` - Ref updates in the current push operation
    /// * `local_refs` - Refs already existing locally (ref_name -> SHA)
    ///
    /// # Returns
    /// Vector of events that can be satisfied by the push
    pub fn find_matching_states(
        &self,
        identifier: &str,
        pushed_updates: &[RefUpdate],
        local_refs: &std::collections::HashMap<String, String>,
    ) -> Vec<Event> {
        self.state_events
            .get(identifier)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| {
                        helpers::can_satisfy_state(&entry.event, pushed_updates, local_refs)
                    })
                    .map(|entry| entry.event.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Extend expiry for state events about to be processed.
    ///
    /// Ensures entries have at least `duration` remaining on their timer.
    /// Sets expiry to max(current_expiry, now + duration).
    ///
    /// # Arguments
    /// * `identifier` - The repository identifier
    /// * `event_ids` - Event IDs to extend expiry for
    /// * `duration` - Minimum duration to guarantee from now
    pub fn extend_expiry(&self, identifier: &str, event_ids: &[EventId], duration: Duration) {
        if let Some(mut entries) = self.state_events.get_mut(identifier) {
            let now = Instant::now();
            let new_expiry = now + duration;

            for entry in entries.iter_mut() {
                if event_ids.contains(&entry.event.id) {
                    // Set to max of current expiry and new expiry
                    if entry.expires_at < new_expiry {
                        entry.expires_at = new_expiry;
                    }
                }
            }
        }
    }

    /// Remove a PR event or placeholder from purgatory.
    ///
    /// # Arguments
    /// * `event_id` - The event ID to remove
    pub fn remove_pr(&self, event_id: &str) {
        self.pr_events.remove(event_id);
    }

    /// Get all event IDs currently stored in purgatory.
    ///
    /// Returns a HashSet of all event IDs for both state events and PR events
    /// held in purgatory. Useful for negentropy sync to avoid fetching events
    /// that are already in purgatory awaiting git data.
    ///
    /// # Returns
    /// HashSet of event IDs (as EventId) for all events in purgatory
    pub fn event_ids(&self) -> HashSet<EventId> {
        let mut ids = HashSet::new();

        // Collect state event IDs
        for entry in self.state_events.iter() {
            for state_entry in entry.value().iter() {
                ids.insert(state_entry.event.id);
            }
        }

        // Collect PR event IDs (only actual events, not placeholders)
        for entry in self.pr_events.iter() {
            if let Some(ref event) = entry.value().event {
                ids.insert(event.id);
            }
        }

        ids
    }

    /// Get all PR placeholder event IDs (git-data-first entries without events).
    ///
    /// Returns event IDs for entries where git data arrived before the PR event.
    /// These correspond to `refs/nostr/<event-id>` refs that should be cleaned up
    /// on shutdown since they don't have corresponding events.
    ///
    /// # Returns
    /// Vector of event IDs (hex strings) for placeholder entries
    pub fn get_placeholder_event_ids(&self) -> Vec<String> {
        self.pr_events
            .iter()
            .filter_map(|entry| {
                if entry.value().event.is_none() {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Remove expired entries from purgatory.
    ///
    /// Should be called periodically (every 60 seconds) by background task to clean up
    /// entries that have exceeded their expiry deadline.
    ///
    /// # Returns
    /// Tuple of (num_state_removed, num_pr_removed)
    pub fn cleanup(&self) -> (usize, usize) {
        let now = Instant::now();
        let mut state_removed = 0;

        // Remove expired state events
        self.state_events.retain(|_, entries| {
            let original_len = entries.len();
            entries.retain(|entry| entry.expires_at > now);
            state_removed += original_len - entries.len();
            !entries.is_empty()
        });

        // Remove expired PR events
        let expired_prs: Vec<String> = self
            .pr_events
            .iter()
            .filter(|entry| entry.value().expires_at <= now)
            .map(|entry| entry.key().clone())
            .collect();

        let pr_removed = expired_prs.len();
        for event_id in expired_prs {
            self.pr_events.remove(&event_id);
        }

        (state_removed, pr_removed)
    }

    /// Remove expired entries from purgatory (legacy method).
    ///
    /// # Returns
    /// Total number of entries removed (state + PR events)
    #[deprecated(since = "0.1.0", note = "Use cleanup() instead for separate counts")]
    pub fn remove_expired(&self) -> usize {
        let (state, pr) = self.cleanup();
        state + pr
    }

    /// Get current count of entries in purgatory.
    ///
    /// # Returns
    /// Tuple of (state_event_count, pr_event_count)
    pub fn count(&self) -> (usize, usize) {
        let state_count: usize = self.state_events.iter().map(|e| e.value().len()).sum();
        let pr_count = self.pr_events.len();
        (state_count, pr_count)
    }

    /// Clear all entries from purgatory (for testing).
    #[cfg(test)]
    pub fn clear(&self) {
        self.state_events.clear();
        self.pr_events.clear();
    }
}

/// Async function to sync git data for a state event from remote servers.
///
/// This function:
/// 1. Fetches repository data from the database
/// 2. Identifies which owners authorize the state event author
/// 3. Collects clone URLs from authorized announcements
/// 4. Finds the most complete local repo to fetch into
/// 5. Identifies missing OIDs and fetches them from remote servers
async fn sync_state_git_data(
    state: RepositoryState,
    database: &SharedDatabase,
    git_data_path: &Path,
    our_domain: Option<&str>,
) -> Result<()> {
    // Fetch repository data from database
    let db_repo_data = fetch_repository_data(database, &state.identifier).await?;

    if db_repo_data.announcements.is_empty() {
        bail!(
            "No announcements found for identifier: {}",
            state.identifier
        );
    }

    // Find owners that authorize this pubkey as a maintainer
    let repo_owners_authorising_pubkey =
        pubkey_authorised_for_repo_owners(&state.event.pubkey, &db_repo_data);

    if repo_owners_authorising_pubkey.is_empty() {
        bail!(
            "No owners authorize pubkey {} for identifier {}",
            state.event.pubkey,
            state.identifier
        );
    }

    // Collect clone URLs from authorized announcements, excluding our own service
    let servers: HashSet<String> = db_repo_data
        .announcements
        .iter()
        .filter(|a| repo_owners_authorising_pubkey.contains(&a.event.pubkey.to_hex()))
        .flat_map(|a| a.clone_urls.iter().cloned())
        .filter(|url| {
            // Exclude our own domain if specified
            if let Some(domain) = our_domain {
                !url.contains(domain)
            } else {
                true
            }
        })
        .collect();

    if servers.is_empty() {
        bail!(
            "No external clone URLs found for identifier: {}",
            state.identifier
        );
    }

    tracing::debug!(
        identifier = %state.identifier,
        servers = ?servers,
        "Found {} external servers for git data sync",
        servers.len()
    );

    // Find the most complete local repo to fetch into
    let (repo_path, missing_oids) =
        get_most_complete_local_repo(&db_repo_data, &state, git_data_path)?;

    if missing_oids.is_empty() {
        tracing::debug!(
            identifier = %state.identifier,
            repo_path = %repo_path.display(),
            "No missing OIDs - git data is already complete"
        );
        return Ok(());
    }

    tracing::info!(
        identifier = %state.identifier,
        repo_path = %repo_path.display(),
        missing_oids = ?missing_oids,
        "Attempting to fetch {} missing OIDs from remote servers",
        missing_oids.len()
    );

    // Try to fetch from each server until we get all missing OIDs
    let mut last_error: Option<String> = None;
    for server_url in &servers {
        match fetch_missing_oids_from_server(&repo_path, server_url, &missing_oids).await {
            Ok(fetched) => {
                if fetched > 0 {
                    tracing::info!(
                        identifier = %state.identifier,
                        server = %server_url,
                        fetched = %fetched,
                        "Successfully fetched git data"
                    );
                }

                // Check if all OIDs are now available
                let still_missing: Vec<_> = missing_oids
                    .iter()
                    .filter(|oid| !oid_exists(&repo_path, oid))
                    .collect();

                if still_missing.is_empty() {
                    tracing::info!(
                        identifier = %state.identifier,
                        "All missing OIDs fetched successfully"
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                tracing::debug!(
                    identifier = %state.identifier,
                    server = %server_url,
                    error = %e,
                    "Failed to fetch from server"
                );
                last_error = Some(e.to_string());
            }
        }
    }

    // Check final state
    let still_missing: Vec<_> = missing_oids
        .iter()
        .filter(|oid| !oid_exists(&repo_path, oid))
        .collect();

    if still_missing.is_empty() {
        Ok(())
    } else {
        bail!(
            "Failed to fetch {} OIDs from any server. Last error: {:?}",
            still_missing.len(),
            last_error
        )
    }
}

/// Fetch missing OIDs from a remote git server.
///
/// Uses `git fetch` to retrieve specific commits from the server.
async fn fetch_missing_oids_from_server(
    repo_path: &Path,
    server_url: &str,
    missing_oids: &[String],
) -> Result<usize> {
    if missing_oids.is_empty() {
        return Ok(0);
    }

    // Use tokio::task::spawn_blocking for the git operations since they're blocking
    let repo_path = repo_path.to_path_buf();
    let server_url = server_url.to_string();
    let oids = missing_oids.to_vec();

    tokio::task::spawn_blocking(move || {
        // Filter to only OIDs that don't already exist
        let missing: Vec<&String> = oids.iter().filter(|oid| !oid_exists(&repo_path, oid)).collect();

        if missing.is_empty() {
            return Ok(0);
        }

        // git fetch <remote> <sha1> <sha2> ... - fetch all OIDs in one command
        let mut args = vec!["fetch", "--depth=1", &server_url];
        args.extend(missing.iter().map(|s| s.as_str()));

        tracing::debug!(
            oids = ?missing,
            server = %server_url,
            "Fetching OIDs"
        );

        let output = Command::new("git")
            .args(&args)
            .current_dir(&repo_path)
            .output();

        match output {
            Ok(result) if result.status.success() => {
                // Count how many OIDs we now have
                let fetched_count = missing
                    .iter()
                    .filter(|oid| oid_exists(&repo_path, oid))
                    .count();

                tracing::debug!(
                    fetched_count = fetched_count,
                    server = %server_url,
                    "Successfully fetched OIDs"
                );

                Ok(fetched_count)
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                tracing::debug!(
                    oids = ?missing,
                    server = %server_url,
                    stderr = %stderr,
                    "git fetch failed for OIDs"
                );
                Ok(0)
            }
            Err(e) => {
                tracing::debug!(
                    oids = ?missing,
                    server = %server_url,
                    error = %e,
                    "git fetch command error"
                );
                Ok(0)
            }
        }
    })
    .await?
}

fn get_most_complete_local_repo(
    db_repo_data: &RepositoryData,
    state: &RepositoryState,
    git_path: &Path,
) -> Result<(PathBuf, Vec<String>)> {
    // should we filter for those where pubkey is authorised?

    let repo_onwers_authorising_pubkey =
        pubkey_authorised_for_repo_owners(&state.event.pubkey, db_repo_data);

    let mut res: Option<(Timestamp, PathBuf, Vec<String>)> = None;
    for announcement in &db_repo_data.announcements {
        if !repo_onwers_authorising_pubkey.contains(&announcement.event.pubkey.to_hex()) {
            continue; // skip where event author isn't a maintainer
        }
        let repo_path = git_path.join(announcement.repo_path().clone());
        if let Ok(missing_oids) = identify_missing_oids(state, &repo_path) {
            let commit_date = get_date_of_most_recent_commit_on_default_branch(&repo_path)
                .unwrap_or(Timestamp::zero());
            let newest_commmit_date = if let Some((d, _, _)) = &res {
                d
            } else {
                &Timestamp::zero()
            };
            if commit_date.gt(newest_commmit_date) {
                res = Some((commit_date, repo_path, missing_oids));
            }
        }
    }
    if let Some((_newest_commit_date, repo_path, missing_oids)) = res {
        Ok((repo_path, missing_oids))
    } else {
        bail!("no repo directories exists yet");
    }
}

fn identify_missing_oids(state: &RepositoryState, git_repo_path: &Path) -> Result<Vec<String>> {
    if !git_repo_path.exists() {
        bail!("repo directory doesn't exists");
    }
    let mut missing_oids = vec![];
    for branch_state in &state.branches {
        if !branch_state.commit.starts_with("ref: ")
            && !oid_exists(git_repo_path, &branch_state.commit)
        {
            missing_oids.push(branch_state.commit.clone());
        }
    }
    for tag_state in &state.tags {
        if !tag_state.commit.starts_with("ref: ") && !oid_exists(git_repo_path, &tag_state.commit) {
            missing_oids.push(tag_state.commit.clone());
        }
    }
    Ok(missing_oids)
}

fn get_date_of_most_recent_commit_on_default_branch(git_repo_path: &Path) -> Result<Timestamp> {
    if !git_repo_path.exists() {
        bail!("repo directory doesn't exists");
    }

    // Get the default branch (HEAD)
    let head_output = std::process::Command::new("git")
        .args(["symbolic-ref", "HEAD"])
        .current_dir(git_repo_path)
        .output()?;

    if !head_output.status.success() {
        bail!("Failed to get repository HEAD");
    }

    let head_ref = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();

    // Get the most recent commit timestamp on the default branch
    // Use %ct to get the committer date as Unix timestamp
    let log_output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct", &head_ref])
        .current_dir(git_repo_path)
        .output()?;

    if !log_output.status.success() {
        bail!("Failed to get commit timestamp for {}", head_ref);
    }

    let timestamp_str = String::from_utf8_lossy(&log_output.stdout)
        .trim()
        .to_string();
    let unix_timestamp: u64 = timestamp_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Failed to parse timestamp: {}", timestamp_str))?;

    Ok(Timestamp::from(unix_timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_purgatory_creation() {
        let purgatory = Purgatory::new(PathBuf::new());
        let (state_count, pr_count) = purgatory.count();
        assert_eq!(state_count, 0);
        assert_eq!(pr_count, 0);
    }

    #[test]
    fn test_purgatory_count() {
        let purgatory = Purgatory::new(PathBuf::new());

        // Add some test data
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test")
            .sign_with_keys(&keys)
            .unwrap();

        purgatory.add_state(event.clone(), "test-repo".to_string(), keys.public_key());
        purgatory.add_pr(event, "test-event-id".to_string(), "abc123".to_string());

        let (state_count, pr_count) = purgatory.count();
        assert_eq!(state_count, 1);
        assert_eq!(pr_count, 1);
    }
}

#[test]
fn test_pr_event_vs_placeholder() {
    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();
    let event = EventBuilder::text_note("test PR")
        .sign_with_keys(&keys)
        .unwrap();

    // Add a PR event with actual event
    purgatory.add_pr(
        event.clone(),
        "event-id-1".to_string(),
        "commit-abc".to_string(),
    );

    // Add a placeholder (no event)
    purgatory.add_pr_placeholder("event-id-2".to_string(), "commit-def".to_string());

    // find_pr should find both
    assert!(purgatory.find_pr("event-id-1").is_some());
    assert!(purgatory.find_pr("event-id-2").is_some());

    // find_pr_placeholder should only find the placeholder
    assert!(purgatory.find_pr_placeholder("event-id-1").is_none());
    assert_eq!(
        purgatory.find_pr_placeholder("event-id-2"),
        Some("commit-def".to_string())
    );
}

#[test]
fn test_pr_placeholder_creation_and_retrieval() {
    let purgatory = Purgatory::new(PathBuf::new());

    // Add a placeholder
    purgatory.add_pr_placeholder("placeholder-id".to_string(), "commit-123".to_string());

    // Should be findable by find_pr
    let entry = purgatory.find_pr("placeholder-id");
    assert!(entry.is_some());
    let entry = entry.unwrap();
    assert!(entry.event.is_none()); // No event yet
    assert_eq!(entry.commit, "commit-123");

    // Should be findable by find_pr_placeholder
    let commit = purgatory.find_pr_placeholder("placeholder-id");
    assert_eq!(commit, Some("commit-123".to_string()));
}

#[test]
fn test_cleanup_removes_expired_entries() {
    use std::time::Duration;

    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    // Create events
    let state_event = EventBuilder::text_note("state event")
        .sign_with_keys(&keys)
        .unwrap();
    let pr_event = EventBuilder::text_note("pr event")
        .sign_with_keys(&keys)
        .unwrap();

    // Add entries to purgatory
    purgatory.add_state(
        state_event.clone(),
        "test-repo".to_string(),
        keys.public_key(),
    );
    purgatory.add_pr(pr_event, "pr-123".to_string(), "commit-abc".to_string());
    purgatory.add_pr_placeholder("pr-456".to_string(), "commit-def".to_string());

    // Verify entries are there
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 1);
    assert_eq!(pr_count, 2);

    // Manually expire entries by modifying their expiry time
    // (This is a bit hacky but needed for testing without waiting 30 minutes)
    if let Some(mut entries) = purgatory.state_events.get_mut("test-repo") {
        for entry in entries.iter_mut() {
            entry.expires_at = Instant::now() - Duration::from_secs(1);
        }
    }

    // Expire PR events
    for mut entry in purgatory.pr_events.iter_mut() {
        entry.value_mut().expires_at = Instant::now() - Duration::from_secs(1);
    }

    // Run cleanup
    let (state_removed, pr_removed) = purgatory.cleanup();

    // Verify counts
    assert_eq!(state_removed, 1);
    assert_eq!(pr_removed, 2);

    // Verify entries are gone
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 0);
    assert_eq!(pr_count, 0);
}

#[test]
fn test_cleanup_preserves_non_expired_entries() {
    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    let state_event = EventBuilder::text_note("state event")
        .sign_with_keys(&keys)
        .unwrap();
    let pr_event = EventBuilder::text_note("pr event")
        .sign_with_keys(&keys)
        .unwrap();

    // Add fresh entries
    purgatory.add_state(state_event, "test-repo".to_string(), keys.public_key());
    purgatory.add_pr(pr_event, "pr-123".to_string(), "commit-abc".to_string());

    // Run cleanup
    let (state_removed, pr_removed) = purgatory.cleanup();

    // Nothing should be removed
    assert_eq!(state_removed, 0);
    assert_eq!(pr_removed, 0);

    // Verify entries are still there
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 1);
    assert_eq!(pr_count, 1);
}

#[test]
fn test_cleanup_mixed_expired_and_fresh() {
    use std::time::Duration;

    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    // Add multiple state events for same repo
    let event1 = EventBuilder::text_note("event1")
        .sign_with_keys(&keys)
        .unwrap();
    let event2 = EventBuilder::text_note("event2")
        .sign_with_keys(&keys)
        .unwrap();

    purgatory.add_state(event1, "test-repo".to_string(), keys.public_key());
    purgatory.add_state(event2, "test-repo".to_string(), keys.public_key());

    // Expire only the first one
    if let Some(mut entries) = purgatory.state_events.get_mut("test-repo") {
        if let Some(entry) = entries.get_mut(0) {
            entry.expires_at = Instant::now() - Duration::from_secs(1);
        }
    }

    // Add PR events
    let pr1 = EventBuilder::text_note("pr1")
        .sign_with_keys(&keys)
        .unwrap();
    let pr2 = EventBuilder::text_note("pr2")
        .sign_with_keys(&keys)
        .unwrap();

    purgatory.add_pr(pr1, "pr-1".to_string(), "commit-1".to_string());
    purgatory.add_pr(pr2, "pr-2".to_string(), "commit-2".to_string());

    // Expire only first PR
    if let Some(mut entry) = purgatory.pr_events.get_mut("pr-1") {
        entry.expires_at = Instant::now() - Duration::from_secs(1);
    }

    // Run cleanup
    let (state_removed, pr_removed) = purgatory.cleanup();

    // One of each should be removed
    assert_eq!(state_removed, 1);
    assert_eq!(pr_removed, 1);

    // Verify remaining counts
    let (state_count, pr_count) = purgatory.count();
    assert_eq!(state_count, 1); // One state event remains
    assert_eq!(pr_count, 1); // One PR event remains
}

#[test]
fn test_remove_expired_legacy_method() {
    use std::time::Duration;

    let purgatory = Purgatory::new(PathBuf::new());
    let keys = Keys::generate();

    let state_event = EventBuilder::text_note("state")
        .sign_with_keys(&keys)
        .unwrap();
    let pr_event = EventBuilder::text_note("pr").sign_with_keys(&keys).unwrap();

    purgatory.add_state(state_event, "repo".to_string(), keys.public_key());
    purgatory.add_pr(pr_event, "pr-id".to_string(), "commit".to_string());

    // Expire both
    if let Some(mut entries) = purgatory.state_events.get_mut("repo") {
        for entry in entries.iter_mut() {
            entry.expires_at = Instant::now() - Duration::from_secs(1);
        }
    }
    for mut entry in purgatory.pr_events.iter_mut() {
        entry.value_mut().expires_at = Instant::now() - Duration::from_secs(1);
    }

    // Test legacy method returns total
    #[allow(deprecated)]
    let total = purgatory.remove_expired();
    assert_eq!(total, 2); // 1 state + 1 PR
}
