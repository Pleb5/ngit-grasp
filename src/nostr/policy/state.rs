use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nostr_relay_builder::builder::WritePolicyResult;
/// State Policy - State event validation + ref alignment
///
/// Handles validation of NIP-34 repository state events (kind 30618)
/// and aligns git refs with authorized state according to GRASP-01.
use nostr_relay_builder::prelude::Event;

use super::PolicyContext;
use crate::git::authorization::{collect_authorized_maintainers, fetch_repository_data};
use crate::git::{self};
use crate::nostr::events::{validate_state, RepositoryAnnouncement, RepositoryState};

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
    /// Returns the true if git data already availale or false if added to purgatory
    pub async fn process_state_event(&self, event: &Event) -> Result<WritePolicyResult> {
        // Parse state to get HEAD and branch info
        let state =
            RepositoryState::from_event(event.clone()).context("Failed to parse state event")?;

        // duplicate check in purgatory
        if self
            .ctx
            .purgatory
            .find_state(&state.identifier)
            .iter()
            .any(|e| e.event.id.eq(&event.id))
        {
            tracing::debug!(
                "processed state event duplicate (already in purgatory): {}",
                event.id,
            );
            return Ok(WritePolicyResult::Reject {
                status: true, // Client sees OK
                message: "duplicate: in purgatory".into(),
            });
        }
        // get all repositories and state events from db with identifier
        let db_repo_data = fetch_repository_data(&self.ctx.database, &state.identifier).await?;

        // duplicate check in db
        if db_repo_data.states.iter().any(|e| e.event.id.eq(&event.id)) {
            tracing::debug!("processed state event duplicate (in db): {}", event.id,);
            return Ok(WritePolicyResult::Reject {
                status: true, // Client sees OK
                message: "duplicate".into(),
            });
        }

        // check if git data is avialable
        if let Some(repo_with_git_data) =
            find_repo_with_git_data(&db_repo_data.announcements, &state, &self.ctx.git_data_path)
        {
            tracing::debug!(
                "processing state event git as data already available: {}",
                event.id,
            );
            // find repos for which this state is authorised and align the git refs to this state
            let by_owner = collect_authorized_maintainers(&db_repo_data.announcements);
            let mut repo_count = 0;
            for (owner, maintainers) in by_owner {
                if maintainers.contains(&event.pubkey.to_string()) {
                    if let Some(previous_state) = db_repo_data
                        .states
                        .iter()
                        .filter(|e| maintainers.contains(&e.event.pubkey.to_string()))
                        .max_by_key(|e| e.event.created_at)
                    {
                        // TODO in event of a tie the event with the biggest event id wins
                        if state.event.created_at > previous_state.event.created_at {
                            if let Some(annoucement) = db_repo_data
                                .announcements
                                .iter()
                                .find(|a| a.event.pubkey.to_string().eq(&owner))
                            {
                                let repo_path =
                                    self.ctx.git_data_path.join(annoucement.repo_path().clone());
                                // TODO - if repo_path != repo_with_git_data, pass as a datasource for missing data?
                                let result = self.align_repository_with_state(&repo_path, &state);
                                repo_count += 1;
                                tracing::info!(
                                    "Aligned {} with state: created={}, updated={}, deleted={}, head_set={}",
                                    repo_path.display(),
                                    result.refs_created,
                                    result.refs_updated,
                                    result.refs_deleted,
                                    result.head_set
                                );
                            }
                        }
                    }
                }
            }

            tracing::info!(
                "immediately accepting state event. Was latest authorised state and git data updated for {repo_count} repositories: eventid: {}",
                state.event.id,
            );
            // immediately accept the event, bypassing purgatory
            Ok(WritePolicyResult::Accept) // event should be saved and broadcast
        } else {
            // if no git data - add to purgatory
            self.ctx
                .purgatory
                .add_state(event.clone(), state.identifier.clone(), event.pubkey);
            tracing::info!(
                "state event added to purgatory: eventid: {}, identifier: {}",
                state.event.id,
                state.identifier,
            );
            Ok(WritePolicyResult::Reject {
                status: true, // Client sees OK
                message: "purgatory: won't be served until git data arrives".into(),
            })
        }
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

fn find_repo_with_git_data(
    announcements: &[RepositoryAnnouncement],
    state: &RepositoryState,
    git_data_path: &Path,
) -> Option<PathBuf> {
    for announcement in announcements {
        let repo_path = git_data_path.join(announcement.repo_path().clone());
        if state.branches.iter().all(|branch_state| {
            if branch_state.commit.starts_with("ref: ") {
                true // ignore symlinks
            } else {
                git::oid_exists(&repo_path, &branch_state.commit)
            }
        }) && state
            .tags
            .iter()
            .all(|tag_state| git::oid_exists(&repo_path, &tag_state.commit))
        {
            return Some(repo_path);
        }
    }
    None
}
