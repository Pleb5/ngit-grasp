//! Git Data Synchronization Across Owner Repositories
//!
//! This module provides functions to sync git data across multiple owner repositories
//! that are authorized by the same state event. This is used when:
//!
//! 1. A push is received that satisfies a state event - the git data needs to be
//!    copied to other owner repos that authorize the same state
//! 2. Purgatory sync fetches git data from remote - needs to distribute to all
//!    authorized owner repos
//! 3. A push to refs/nostr/<event-id> (PR data) is received - needs to be synced
//!    to all other owner repos that share maintainers
//!
//! ## Architecture
//!
//! The key insight is that multiple owners can have announcements for the same
//! repository identifier, and they may share maintainers. When a state event
//! authorizes a push, that push should be reflected in ALL owner repositories
//! that would authorize the same state.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use tracing::{debug, info, warn};

use crate::git::{self, oid_exists};
use crate::git::authorization::{collect_authorized_maintainers, RepositoryData};
use crate::nostr::events::RepositoryState;

/// Result of syncing git data to owner repositories
#[derive(Debug, Default)]
pub struct SyncResult {
    /// Number of repositories synced
    pub repos_synced: usize,
    /// Number of refs created across all repos
    pub refs_created: usize,
    /// Number of refs updated across all repos
    pub refs_updated: usize,
    /// Number of refs deleted across all repos
    pub refs_deleted: usize,
    /// Number of repositories where HEAD was set
    pub heads_set: usize,
    /// Errors encountered (repo path -> error message)
    pub errors: Vec<(String, String)>,
}

/// Result of aligning a single repository with state
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

/// Result of syncing PR refs to owner repositories
#[derive(Debug, Default)]
pub struct PrSyncResult {
    /// Number of repositories synced
    pub repos_synced: usize,
    /// Number of refs created across all repos
    pub refs_created: usize,
    /// Errors encountered (repo path -> error message)
    pub errors: Vec<(String, String)>,
}

/// Sync PR data (refs/nostr/<event-id>) from a source repository to all other
/// owner repositories that share maintainers.
///
/// This function:
/// 1. Collects all authorized maintainers per owner from announcements
/// 2. For each owner that shares at least one maintainer with the source owner:
///    - Copies missing OIDs for the PR commits
///    - Creates the refs/nostr/<event-id> ref pointing to the same commit
///
/// # Arguments
/// * `source_repo_path` - Path to the repository that has the PR git data
/// * `pr_refs` - List of (event_id, commit_hash) tuples for PR refs that were pushed
/// * `db_repo_data` - Repository data from database (announcements + states)
/// * `git_data_path` - Base path for git repositories
/// * `source_owner_pubkey` - The owner pubkey of the source repository
///
/// # Returns
/// A `PrSyncResult` with statistics about what was synced
pub fn sync_pr_refs_to_owner_repos(
    source_repo_path: &Path,
    pr_refs: &[(String, String)], // (event_id, commit_hash)
    db_repo_data: &RepositoryData,
    git_data_path: &Path,
    source_owner_pubkey: &str,
) -> PrSyncResult {
    let mut result = PrSyncResult::default();

    if pr_refs.is_empty() {
        return result;
    }

    // Collect authorized maintainers per owner
    let by_owner = collect_authorized_maintainers(&db_repo_data.announcements);

    // Get the maintainer set for the source owner
    let source_maintainers = match by_owner.get(source_owner_pubkey) {
        Some(maintainers) => maintainers,
        None => {
            debug!(
                "No maintainer set found for source owner {}",
                source_owner_pubkey
            );
            return result;
        }
    };

    debug!(
        source_owner = %source_owner_pubkey,
        pr_refs_count = pr_refs.len(),
        owners = by_owner.len(),
        "Syncing PR refs to owner repositories"
    );

    for (owner, maintainers) in &by_owner {
        // Skip the source owner - we already have the data there
        if owner == source_owner_pubkey {
            continue;
        }

        // Check if this owner shares any maintainers with the source owner
        // (i.e., there's overlap in their maintainer sets)
        let has_shared_maintainer = maintainers.iter().any(|m| source_maintainers.contains(m));

        if !has_shared_maintainer {
            debug!(
                owner = %owner,
                "Skipping owner - no shared maintainers with source"
            );
            continue;
        }

        // Find the announcement for this owner
        let announcement = db_repo_data
            .announcements
            .iter()
            .find(|a| a.event.pubkey.to_hex() == *owner);

        let Some(announcement) = announcement else {
            continue;
        };

        let target_repo_path = git_data_path.join(announcement.repo_path());

        if !target_repo_path.exists() {
            debug!(
                owner = %owner,
                repo_path = %target_repo_path.display(),
                "Skipping owner - repository doesn't exist"
            );
            continue;
        }

        // Sync each PR ref
        let mut refs_created_for_owner = 0;
        for (event_id, commit_hash) in pr_refs {
            // Copy the commit if missing
            if !oid_exists(&target_repo_path, commit_hash) {
                if let Err(e) =
                    copy_single_commit_between_repos(source_repo_path, &target_repo_path, commit_hash)
                {
                    warn!(
                        event_id = %event_id,
                        source = %source_repo_path.display(),
                        target = %target_repo_path.display(),
                        error = %e,
                        "Failed to copy PR commit between repos"
                    );
                    result
                        .errors
                        .push((target_repo_path.display().to_string(), e));
                    continue;
                }
            }

            // Create the refs/nostr/<event-id> ref
            let ref_name = format!("refs/nostr/{}", event_id);
            match git::update_ref(&target_repo_path, &ref_name, commit_hash) {
                Ok(()) => {
                    info!(
                        event_id = %event_id,
                        commit = %commit_hash,
                        target = %target_repo_path.display(),
                        "Created PR ref in target repository"
                    );
                    refs_created_for_owner += 1;
                }
                Err(e) => {
                    warn!(
                        event_id = %event_id,
                        target = %target_repo_path.display(),
                        error = %e,
                        "Failed to create PR ref in target repository"
                    );
                    result.errors.push((target_repo_path.display().to_string(), e));
                }
            }
        }

        if refs_created_for_owner > 0 {
            result.repos_synced += 1;
            result.refs_created += refs_created_for_owner;

            info!(
                owner = %owner,
                repo_path = %target_repo_path.display(),
                refs_created = refs_created_for_owner,
                "Synced PR refs to owner repository"
            );
        }
    }

    info!(
        repos_synced = result.repos_synced,
        refs_created = result.refs_created,
        errors = result.errors.len(),
        "Completed PR ref sync to owner repositories"
    );

    result
}

/// Copy a single commit from source repository to target repository
fn copy_single_commit_between_repos(
    source_repo: &Path,
    target_repo: &Path,
    commit_hash: &str,
) -> Result<(), String> {
    debug!(
        "Copying commit {} from {} to {}",
        commit_hash,
        source_repo.display(),
        target_repo.display()
    );

    let output = Command::new("git")
        .args([
            "fetch",
            source_repo.to_str().ok_or("Invalid source path")?,
            commit_hash,
        ])
        .current_dir(target_repo)
        .output()
        .map_err(|e| format!("Failed to execute git fetch: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git fetch failed for commit {}: {}",
            commit_hash, stderr
        ));
    }

    debug!(
        "Copied commit {} to {}",
        commit_hash,
        target_repo.display()
    );
    Ok(())
}

/// Sync git data from a source repository to all other owner repositories
/// that authorize the given state event.
///
/// This function:
/// 1. Collects all authorized maintainers per owner from announcements
/// 2. For each owner whose maintainer set authorizes the state author:
///    - Skips if a newer state already exists for that owner
///    - Copies missing OIDs from source repo to target repo
///    - Aligns refs with the state
///
/// # Arguments
/// * `source_repo_path` - Path to the repository that has the git data
/// * `state` - The repository state event that authorized the push
/// * `db_repo_data` - Repository data from database (announcements + states)
/// * `git_data_path` - Base path for git repositories
///
/// # Returns
/// A `SyncResult` with statistics about what was synced
pub fn sync_to_owner_repos(
    source_repo_path: &Path,
    state: &RepositoryState,
    db_repo_data: &RepositoryData,
    git_data_path: &Path,
) -> SyncResult {
    let mut result = SyncResult::default();

    // Collect authorized maintainers per owner
    let by_owner = collect_authorized_maintainers(&db_repo_data.announcements);
    let state_author = state.event.pubkey.to_hex();

    debug!(
        identifier = %state.identifier,
        owners = by_owner.len(),
        "Syncing git data to owner repositories"
    );

    for (owner, maintainers) in &by_owner {
        // Check if this state's author is authorized for this owner
        if !maintainers.contains(&state_author) {
            debug!(
                identifier = %state.identifier,
                owner = %owner,
                "Skipping owner - state author not in maintainer set"
            );
            continue;
        }

        // Find the previous latest state for this owner's maintainer set
        let previous_state = db_repo_data
            .states
            .iter()
            .filter(|s| maintainers.contains(&s.event.pubkey.to_hex()))
            .max_by_key(|s| s.event.created_at);

        // Only update if this state is newer than any existing state
        // TODO: in event of a tie, the event with the biggest event id wins
        if let Some(prev) = previous_state {
            if state.event.created_at <= prev.event.created_at {
                debug!(
                    identifier = %state.identifier,
                    owner = %owner,
                    "Skipping owner - existing state is newer or equal"
                );
                continue;
            }
        }

        // Find the announcement for this owner
        let announcement = db_repo_data
            .announcements
            .iter()
            .find(|a| a.event.pubkey.to_hex() == *owner);

        let Some(announcement) = announcement else {
            continue;
        };

        let target_repo_path = git_data_path.join(announcement.repo_path());

        if !target_repo_path.exists() {
            // Repository doesn't exist (e.g., announcement doesn't list this service)
            debug!(
                identifier = %state.identifier,
                owner = %owner,
                repo_path = %target_repo_path.display(),
                "Skipping owner - repository doesn't exist"
            );
            continue;
        }

        // Copy missing OIDs from source repo to target repo if different
        if target_repo_path != source_repo_path {
            if let Err(e) = copy_missing_oids_between_repos(source_repo_path, &target_repo_path, state)
            {
                warn!(
                    identifier = %state.identifier,
                    source = %source_repo_path.display(),
                    target = %target_repo_path.display(),
                    error = %e,
                    "Failed to copy OIDs between repos"
                );
                result.errors.push((target_repo_path.display().to_string(), e));
                // Continue anyway - we'll try to align what we can
            }
        }

        // Align refs with state
        let align_result = align_repository_with_state(&target_repo_path, state);
        result.repos_synced += 1;
        result.refs_created += align_result.refs_created;
        result.refs_updated += align_result.refs_updated;
        result.refs_deleted += align_result.refs_deleted;
        if align_result.head_set {
            result.heads_set += 1;
        }

        info!(
            identifier = %state.identifier,
            owner = %owner,
            repo_path = %target_repo_path.display(),
            refs_created = align_result.refs_created,
            refs_updated = align_result.refs_updated,
            refs_deleted = align_result.refs_deleted,
            head_set = align_result.head_set,
            "Aligned repository with state"
        );
    }

    info!(
        identifier = %state.identifier,
        repos_synced = result.repos_synced,
        refs_created = result.refs_created,
        refs_updated = result.refs_updated,
        refs_deleted = result.refs_deleted,
        heads_set = result.heads_set,
        "Completed git data sync to owner repositories"
    );

    result
}

/// Copy missing OIDs from a source repository to a target repository.
///
/// Identifies commits referenced in the state that are missing from the target
/// repository and copies them from the source repository using git fetch.
pub fn copy_missing_oids_between_repos(
    source_repo: &Path,
    target_repo: &Path,
    state: &RepositoryState,
) -> Result<(), String> {
    // Collect all commits referenced in the state
    let mut commits_to_check = Vec::new();

    for branch in &state.branches {
        if !branch.commit.starts_with("ref: ") {
            commits_to_check.push(&branch.commit);
        }
    }

    for tag in &state.tags {
        if !tag.commit.starts_with("ref: ") {
            commits_to_check.push(&tag.commit);
        }
    }

    // Identify missing commits
    let mut missing_commits = Vec::new();
    for commit in commits_to_check {
        if !oid_exists(target_repo, commit) {
            missing_commits.push(commit);
        }
    }

    if missing_commits.is_empty() {
        debug!(
            "No missing commits to copy from {} to {}",
            source_repo.display(),
            target_repo.display()
        );
        return Ok(());
    }

    info!(
        "Copying {} missing commits from {} to {}",
        missing_commits.len(),
        source_repo.display(),
        target_repo.display()
    );

    // Fetch each missing commit from source to target
    for commit in &missing_commits {
        let output = Command::new("git")
            .args([
                "fetch",
                source_repo.to_str().ok_or("Invalid source path")?,
                commit,
            ])
            .current_dir(target_repo)
            .output()
            .map_err(|e| format!("Failed to execute git fetch: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "git fetch failed for commit {}: {}",
                commit, stderr
            ));
        }

        debug!("Copied commit {} to {}", commit, target_repo.display());
    }

    Ok(())
}

/// Align a repository's refs with the authorized state.
///
/// This function:
/// 1. Deletes refs that are in the repo but not in the state (for refs/heads/ and refs/tags/)
/// 2. Updates refs that exist in state if we have the commit
/// 3. Sets HEAD if the HEAD branch's commit is available
pub fn align_repository_with_state(repo_path: &Path, state: &RepositoryState) -> AlignmentResult {
    let mut result = AlignmentResult::default();

    // Check if repository exists
    if !repo_path.exists() {
        debug!(
            "Repository not found at {}, cannot align with state",
            repo_path.display()
        );
        return result;
    }

    // Get current refs from the repository
    let current_refs = match git::list_refs(repo_path) {
        Ok(refs) => refs,
        Err(e) => {
            warn!("Failed to list refs in {}: {}", repo_path.display(), e);
            return result;
        }
    };

    // Build expected refs from state
    let mut expected_refs: HashMap<String, String> = HashMap::new();

    for branch in &state.branches {
        let ref_name = format!("refs/heads/{}", branch.name);
        expected_refs.insert(ref_name, branch.commit.clone());
    }

    for tag in &state.tags {
        let ref_name = format!("refs/tags/{}", tag.name);
        expected_refs.insert(ref_name, tag.commit.clone());
    }

    // Delete refs that exist in repo but not in state (only for refs/heads/ and refs/tags/)
    for (ref_name, _current_commit) in &current_refs {
        if (ref_name.starts_with("refs/heads/") || ref_name.starts_with("refs/tags/"))
            && !expected_refs.contains_key(ref_name)
        {
            match git::delete_ref(repo_path, ref_name) {
                Ok(()) => {
                    info!(
                        "Deleted {} from {} (not in state)",
                        ref_name,
                        repo_path.display()
                    );
                    result.refs_deleted += 1;
                }
                Err(e) => {
                    warn!(
                        "Failed to delete {} from {}: {}",
                        ref_name,
                        repo_path.display(),
                        e
                    );
                }
            }
        }
    }

    // Update refs that exist in state (if we have the commit)
    for (ref_name, expected_commit) in &expected_refs {
        // Skip symbolic refs
        if expected_commit.starts_with("ref: ") {
            continue;
        }

        // Check if we have the commit
        if !git::oid_exists(repo_path, expected_commit) {
            debug!(
                "Commit {} not available for {} in {}",
                expected_commit,
                ref_name,
                repo_path.display()
            );
            continue;
        }

        // Check current value
        let current_commit = current_refs
            .iter()
            .find(|(r, _)| r == ref_name)
            .map(|(_, c)| c.as_str());

        if current_commit == Some(expected_commit.as_str()) {
            // Already correct
            continue;
        }

        // Update or create the ref
        match git::update_ref(repo_path, ref_name, expected_commit) {
            Ok(()) => {
                if current_commit.is_some() {
                    info!(
                        "Updated {} to {} in {}",
                        ref_name,
                        expected_commit,
                        repo_path.display()
                    );
                    result.refs_updated += 1;
                } else {
                    info!(
                        "Created {} at {} in {}",
                        ref_name,
                        expected_commit,
                        repo_path.display()
                    );
                    result.refs_created += 1;
                }
            }
            Err(e) => {
                warn!(
                    "Failed to update {} in {}: {}",
                    ref_name,
                    repo_path.display(),
                    e
                );
            }
        }
    }

    // Set HEAD if specified in state
    if let Some(head_ref) = &state.head {
        if let Some(branch_name) = state.get_head_branch() {
            if let Some(head_commit) = state.get_branch_commit(branch_name) {
                match git::try_set_head_if_available(repo_path, head_ref, head_commit) {
                    Ok(true) => {
                        info!(
                            "Set HEAD to {} in {}",
                            head_ref,
                            repo_path.display()
                        );
                        result.head_set = true;
                    }
                    Ok(false) => {
                        debug!(
                            "HEAD commit {} not available yet in {}",
                            head_commit,
                            repo_path.display()
                        );
                    }
                    Err(e) => {
                        warn!("Failed to set HEAD in {}: {}", repo_path.display(), e);
                    }
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_result_default() {
        let result = SyncResult::default();
        assert_eq!(result.repos_synced, 0);
        assert_eq!(result.refs_created, 0);
        assert_eq!(result.refs_updated, 0);
        assert_eq!(result.refs_deleted, 0);
        assert_eq!(result.heads_set, 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_alignment_result_default() {
        let result = AlignmentResult::default();
        assert_eq!(result.refs_created, 0);
        assert_eq!(result.refs_updated, 0);
        assert_eq!(result.refs_deleted, 0);
        assert!(!result.head_set);
    }

    #[test]
    fn test_pr_sync_result_default() {
        let result = PrSyncResult::default();
        assert_eq!(result.repos_synced, 0);
        assert_eq!(result.refs_created, 0);
        assert!(result.errors.is_empty());
    }
}
