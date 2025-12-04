/// State Policy - State event validation + ref alignment
///
/// Handles validation of NIP-34 repository state events (kind 30618)
/// and aligns git refs with authorized state according to GRASP-01.
use nostr_relay_builder::prelude::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};

use super::PolicyContext;
use crate::git;
use crate::nostr::events::{
    validate_state, RepositoryAnnouncement, RepositoryState, KIND_REPOSITORY_ANNOUNCEMENT,
    KIND_REPOSITORY_STATE,
};

/// Result of aligning a repository with authorized state
#[derive(Debug, Default)]
pub struct AlignmentResult {
    /// Number of refs created
    pub refs_created: usize,
    /// Number of refs updated
    pub refs_updated: usize,
    /// Number of refs deleted
    pub refs_deleted: usize,
    /// Whether HEAD was set
    pub head_set: bool,
}

impl AlignmentResult {
    pub fn has_changes(&self) -> bool {
        self.refs_created > 0 || self.refs_updated > 0 || self.refs_deleted > 0 || self.head_set
    }
}

/// Result of state policy evaluation
#[derive(Debug)]
pub enum StateResult {
    /// Accept: Event passes validation
    Accept,
    /// Reject: Event fails validation with reason
    Reject(String),
}

/// Policy for validating repository state events and aligning refs
#[derive(Clone)]
pub struct StatePolicy {
    ctx: PolicyContext,
}

impl StatePolicy {
    pub fn new(ctx: PolicyContext) -> Self {
        Self { ctx }
    }

    /// Validate a repository state event
    pub fn validate(&self, event: &Event) -> StateResult {
        match validate_state(event) {
            Ok(_) => StateResult::Accept,
            Err(e) => StateResult::Reject(e.to_string()),
        }
    }

    /// Process a state event: validate and align owner repositories
    ///
    /// Returns the number of repositories aligned if successful.
    pub async fn process_state_event(&self, event: &Event) -> Result<usize, String> {
        // Parse state to get HEAD and branch info
        let state = RepositoryState::from_event(event.clone())
            .map_err(|e| format!("Failed to parse state: {}", e))?;

        // Identify owner repositories for which this is the latest authorized state
        let owner_repos = self.identify_owner_repositories(&state).await?;
        let repo_count = owner_repos.len();
        let mut total_aligned = 0;

        // Align each owner repository with the authorized state
        for (_announcement, repo_path) in owner_repos {
            let result = self.align_repository_with_state(&repo_path, &state);

            if result.has_changes() {
                tracing::info!(
                    "Aligned {} with state: created={}, updated={}, deleted={}, head_set={}",
                    repo_path.display(),
                    result.refs_created,
                    result.refs_updated,
                    result.refs_deleted,
                    result.head_set
                );
                total_aligned += 1;
            }
        }

        if repo_count > 0 {
            tracing::info!(
                "Processed state event for {} repo(s) ({} aligned) with identifier {}",
                repo_count,
                total_aligned,
                state.identifier
            );
        } else {
            tracing::debug!(
                "No owner repos to align for state - git data not available yet or not latest"
            );
        }

        Ok(total_aligned)
    }

    /// Check if this state event is the latest for its identifier among authorized authors
    ///
    /// A state is considered "latest" if no other state event in the database
    /// from an authorized author has a newer timestamp.
    async fn is_latest_state_for_identifier(
        &self,
        state: &RepositoryState,
        authorized_pubkeys: &[PublicKey],
    ) -> Result<bool, String> {
        let filter = Filter::new()
            .kind(Kind::from(KIND_REPOSITORY_STATE))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                state.identifier.clone(),
            );

        match self.ctx.database.query(filter).await {
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
    async fn find_authorized_announcements(
        &self,
        identifier: &str,
        state_author: &PublicKey,
    ) -> Result<Vec<RepositoryAnnouncement>, String> {
        let filter = Filter::new()
            .kind(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                identifier.to_string(),
            );

        match self.ctx.database.query(filter).await {
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
                                if is_owner {
                                    event.pubkey.to_hex()
                                } else {
                                    "n/a".to_string()
                                },
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

    /// Identify all owner repositories for which this state event is the latest authorized state
    async fn identify_owner_repositories(
        &self,
        state: &RepositoryState,
    ) -> Result<Vec<(RepositoryAnnouncement, std::path::PathBuf)>, String> {
        // Find all announcements where state author is authorized
        let announcements = self
            .find_authorized_announcements(&state.identifier, &state.event.pubkey)
            .await?;

        if announcements.is_empty() {
            tracing::debug!(
                "No authorized announcements found for state {} by {}",
                state.identifier,
                state.event.pubkey.to_hex()
            );
            return Ok(Vec::new());
        }

        let mut owner_repos = Vec::new();

        for announcement in announcements {
            // Build the list of authorized pubkeys for this specific announcement
            let mut authorized_pubkeys = vec![announcement.event.pubkey];
            for maintainer_hex in &announcement.maintainers {
                if let Ok(pk) = PublicKey::from_hex(maintainer_hex) {
                    authorized_pubkeys.push(pk);
                }
            }

            // Check if this is the latest state event for THIS announcement's context
            if !self
                .is_latest_state_for_identifier(state, &authorized_pubkeys)
                .await?
            {
                tracing::debug!(
                    "Skipping {} in {}'s repo - not the latest state event for this context",
                    state.identifier,
                    announcement.event.pubkey.to_hex()
                );
                continue;
            }

            // Build repository path: <git_data_path>/<owner_npub>/<identifier>.git
            let repo_path = self.ctx.git_data_path.join(announcement.repo_path().clone());
            owner_repos.push((announcement, repo_path));
        }

        Ok(owner_repos)
    }

    /// Align a repository's refs with the authorized state
    ///
    /// This function:
    /// 1. Deletes refs that are in the repo but not in the state (for refs/heads/ and refs/tags/)
    /// 2. Updates refs that exist in state if we have the commit
    /// 3. Sets HEAD if the HEAD branch's commit is available
    pub fn align_repository_with_state(
        &self,
        repo_path: &std::path::Path,
        state: &RepositoryState,
    ) -> AlignmentResult {
        let mut result = AlignmentResult::default();

        // Check if repository exists
        if !repo_path.exists() {
            tracing::debug!(
                "Repository not found at {}, cannot align with state",
                repo_path.display()
            );
            return result;
        }

        // Get current refs from the repository
        let current_refs = match git::list_refs(repo_path) {
            Ok(refs) => refs,
            Err(e) => {
                tracing::warn!("Failed to list refs in {}: {}", repo_path.display(), e);
                return result;
            }
        };

        // Build expected refs from state
        let mut expected_refs: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for branch in &state.branches {
            let ref_name = format!("refs/heads/{}", branch.name);
            expected_refs.insert(ref_name, branch.commit.clone());
        }

        for tag in &state.tags {
            let ref_name = format!("refs/tags/{}", tag.name);
            expected_refs.insert(ref_name, tag.commit.clone());
        }

        // Process current refs: update or delete as needed
        for (ref_name, current_commit) in &current_refs {
            // Only process refs/heads/ and refs/tags/
            if !ref_name.starts_with("refs/heads/") && !ref_name.starts_with("refs/tags/") {
                continue;
            }

            match expected_refs.get(ref_name) {
                Some(expected_commit) => {
                    // Ref should exist - check if commit matches
                    if current_commit != expected_commit {
                        // Check if we have the expected commit
                        if git::commit_exists(repo_path, expected_commit) {
                            // Update the ref
                            match git::update_ref(repo_path, ref_name, expected_commit) {
                                Ok(()) => {
                                    tracing::info!(
                                        "Updated {} from {} to {} in {}",
                                        ref_name,
                                        current_commit,
                                        expected_commit,
                                        repo_path.display()
                                    );
                                    result.refs_updated += 1;
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to update {} in {}: {}",
                                        ref_name,
                                        repo_path.display(),
                                        e
                                    );
                                }
                            }
                        } else {
                            tracing::debug!(
                                "Commit {} not available for {} in {}",
                                expected_commit,
                                ref_name,
                                repo_path.display()
                            );
                        }
                    }
                }
                None => {
                    // Ref should not exist - delete it
                    match git::delete_ref(repo_path, ref_name) {
                        Ok(()) => {
                            tracing::info!(
                                "Deleted {} (not in state) from {}",
                                ref_name,
                                repo_path.display()
                            );
                            result.refs_deleted += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to delete {} from {}: {}",
                                ref_name,
                                repo_path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        // Add refs that exist in state but not in repo (if we have the commit)
        for (ref_name, expected_commit) in &expected_refs {
            let exists = current_refs.iter().any(|(r, _)| r == ref_name);
            if !exists && git::commit_exists(repo_path, expected_commit) {
                match git::update_ref(repo_path, ref_name, expected_commit) {
                    Ok(()) => {
                        tracing::info!(
                            "Created {} at {} in {}",
                            ref_name,
                            expected_commit,
                            repo_path.display()
                        );
                        result.refs_created += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to create {} in {}: {}",
                            ref_name,
                            repo_path.display(),
                            e
                        );
                    }
                }
            }
        }

        // Set HEAD if specified in state
        if let Some(head_ref) = &state.head {
            if let Some(branch_name) = state.get_head_branch() {
                if let Some(head_commit) = state.get_branch_commit(branch_name) {
                    match git::try_set_head_if_available(repo_path, head_ref, head_commit) {
                        Ok(true) => {
                            tracing::info!(
                                "Set HEAD to {} in {} (from state by {})",
                                head_ref,
                                repo_path.display(),
                                state.event.pubkey.to_hex()
                            );
                            result.head_set = true;
                        }
                        Ok(false) => {
                            tracing::debug!(
                                "HEAD commit {} not available yet in {}",
                                head_commit,
                                repo_path.display()
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Failed to set HEAD in {}: {}", repo_path.display(), e);
                        }
                    }
                }
            }
        }

        result
    }
}