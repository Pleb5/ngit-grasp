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
use crate::git::sync::align_repository_with_state;
use crate::git::{self};
use crate::nostr::events::{validate_state, RepositoryAnnouncement, RepositoryState};

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

                                if !repo_path.exists() {
                                    // eg if annoucement doesnt list repo (but stored as its in maintainer set)
                                    continue;
                                }
                                // If repo_path != repo_with_git_data, copy missing oids first
                                if repo_path != repo_with_git_data {
                                    if let Err(e) = self.copy_missing_oids(
                                        &repo_with_git_data,
                                        &repo_path,
                                        &state,
                                    ) {
                                        tracing::warn!(
                                            "Failed to copy oids from {} to {}: {}",
                                            repo_with_git_data.display(),
                                            repo_path.display(),
                                            e
                                        );
                                    }
                                }

                                let result = align_repository_with_state(&repo_path, &state);
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

            // Trigger background git data sync from remote servers
            self.ctx.purgatory.start_state_sync(
                state.clone(),
                self.ctx.database.clone(),
                Some(self.ctx.domain.clone()),
                self.ctx.get_local_relay(),
            );

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

    /// Copy missing OIDs from a source repository to a target repository
    ///
    /// Identifies commits referenced in the state that are missing from the target
    /// repository and copies them from the source repository using git fetch.
    ///
    /// # Arguments
    /// * `source_repo` - Path to repository containing the commits
    /// * `target_repo` - Path to repository to receive the commits
    /// * `state` - Repository state containing commit references
    ///
    /// # Returns
    /// Ok(()) on success, Err with error message on failure
    fn copy_missing_oids(
        &self,
        source_repo: &Path,
        target_repo: &Path,
        state: &RepositoryState,
    ) -> Result<(), String> {
        use std::process::Command;

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
            if !git::oid_exists(target_repo, commit) {
                missing_commits.push(commit);
            }
        }

        if missing_commits.is_empty() {
            tracing::debug!(
                "No missing commits to copy from {} to {}",
                source_repo.display(),
                target_repo.display()
            );
            return Ok(());
        }

        tracing::info!(
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

            tracing::debug!("Copied commit {} to {}", commit, target_repo.display());
        }

        Ok(())
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
