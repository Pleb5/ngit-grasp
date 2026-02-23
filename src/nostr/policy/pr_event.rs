/// PR Event Policy - PR/PR Update validation
///
/// Handles validation of NIP-34 PR events (kind 1618) and PR Update events (kind 1619)
/// according to GRASP-01 specification.
use anyhow::{bail, Result};
use nostr_relay_builder::prelude::Event;

use super::PolicyContext;
use crate::git;
use crate::git::authorization::{collect_authorized_maintainers, fetch_repository_data};

/// Policy for validating PR and PR Update events
#[derive(Clone)]
pub struct PrEventPolicy {
    ctx: PolicyContext,
}

impl PrEventPolicy {
    pub fn new(ctx: PolicyContext) -> Self {
        Self { ctx }
    }

    /// Check if git data exists for a PR event
    ///
    /// This unified method checks for git data existence and handles:
    /// 1. Placeholder validation (git-data-first scenario)
    /// 2. Commit existence in referenced repositories
    /// 3. Deletion of incorrect refs/nostr/<event-id> refs
    /// 4. Deletion of incorrect placeholders
    /// 5. Processing PR event with unified function
    ///
    /// # Returns
    /// - `Ok(true)` if git data ready (commit exists and is synced to all repos)
    /// - `Ok(false)` if git data missing (should add to purgatory)
    /// - `Err(msg)` on errors
    pub async fn git_data_check(&self, event: &Event) -> Result<bool> {
        let event_id = event.id.to_hex();

        // Extract the `c` tag (commit hash) from the PR event
        let commit = event.tags.iter().find_map(|tag| {
            let tag_vec = tag.clone().to_vec();
            if tag_vec.len() >= 2 && tag_vec[0] == "c" {
                Some(tag_vec[1].clone())
            } else {
                None
            }
        });

        let commit = match commit {
            Some(c) => c,
            None => {
                bail!(format!("PR event {} has no 'c' tag", event_id));
            }
        };

        // Check for placeholder first (git-data-first scenario)
        if let Some(placeholder_commit) = self.ctx.purgatory.find_pr_placeholder(&event_id) {
            if placeholder_commit == commit {
                // Perfect match - git data arrived first with matching commit
                tracing::debug!(
                    "Found matching placeholder for PR event {} with commit {}",
                    event_id,
                    commit
                );
                // Remove placeholder - event processing will continue normally
                self.ctx.purgatory.remove_pr(&event_id);
            } else {
                // Placeholder has different commit - incoming event supersedes
                tracing::info!(
                    "PR event {} supersedes placeholder: event expects commit {}, placeholder has {}",
                    event_id,
                    commit,
                    placeholder_commit
                );
                // Remove incorrect placeholder
                self.ctx.purgatory.remove_pr(&event_id);
                // Delete incorrect git data (refs/nostr/<event-id>) will be handled below
            }
        }

        let repo_paths = self.find_relevant_repo_paths(event).await?;

        if repo_paths.is_empty() {
            tracing::debug!("No repository paths found for PR event {}", event_id);
            return Ok(false);
        }

        // Delete incorrect refs/nostr/<event-id>
        for repo_path in &repo_paths {
            match git::validate_nostr_ref(repo_path, &event_id, &commit) {
                Ok(true) => {
                    tracing::info!(
                        "Deleted mismatched refs/nostr/{} in {}",
                        event_id,
                        repo_path.display()
                    );
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(
                        "Failed to validate refs/nostr/{} in {}: {}",
                        event_id,
                        repo_path.display(),
                        e
                    );
                }
            }
        }

        // Find location of correct git data (if exists)
        let mut source_repo: Option<std::path::PathBuf> = None;
        for repo_path in &repo_paths {
            if git::commit_exists(repo_path, &commit) {
                source_repo = Some(repo_path.clone());
                tracing::debug!(
                    "Found commit {} in repository {}",
                    commit,
                    repo_path.display()
                );
                break;
            }
        }

        if let Some(source_repo) = source_repo {
            // Extract identifier
            let identifier = crate::git::sync::extract_identifier_from_pr_event(event)
                .ok_or_else(|| anyhow::anyhow!("No identifier in PR event"))?;

            // Fetch repository data
            // NOTE: Only fetch from database, NOT purgatory. Incoming PR events should
            // only be accepted for announcements that have been promoted (validated).
            // If the announcement is still in purgatory, the PR event should also go
            // to purgatory and wait for the announcement to be promoted.
            let db_repo_data = fetch_repository_data(&self.ctx.database, &identifier).await?;

            // Extract owner pubkey from source repo path
            let owner_pubkey = crate::git::sync::extract_owner_from_repo_path(
                &source_repo,
                &self.ctx.git_data_path,
            )
            .unwrap_or_default();

            // Use unified processing function
            let result = crate::git::process::process_pr_with_git_data(
                event,
                &commit,
                &source_repo,
                &db_repo_data,
                &self.ctx.git_data_path,
                &owner_pubkey,
            );

            tracing::info!(
                identifier = %identifier,
                event_id = %event_id,
                repos_synced = result.repos_synced,
                refs_created = result.refs_created,
                "Processed PR event with git data already available"
            );

            if !result.errors.is_empty() {
                for error in &result.errors {
                    tracing::warn!(
                        identifier = %identifier,
                        event_id = %event_id,
                        error = %error,
                        "Error processing PR event"
                    );
                }
            }

            Ok(true)
        } else {
            tracing::debug!(
                "No git data found for PR event {} with commit {}",
                event_id,
                commit
            );
            Ok(false)
        }
    }

    async fn find_relevant_repo_paths(&self, event: &Event) -> Result<Vec<std::path::PathBuf>> {
        // Extract ALL `a` tags (repository references) from the PR event
        let repo_refs: Vec<String> = event
            .tags
            .iter()
            .filter_map(|tag| {
                let tag_vec = tag.clone().to_vec();
                if tag_vec.len() >= 2 && tag_vec[0] == "a" && tag_vec[1].starts_with("30617:") {
                    Some(tag_vec[1].clone())
                } else {
                    None
                }
            })
            .collect();

        if repo_refs.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Find identifier from first a tag starting with "30617:"
        let parts: Vec<&str> = repo_refs[0].split(':').collect();
        if parts.len() < 3 {
            return Err(anyhow::anyhow!("Invalid repository reference format"));
        }
        let identifier = parts[2];

        // 2. Fetch repo data
        // NOTE: Only fetch from database, NOT purgatory. Incoming PR events should
        // only be accepted for announcements that have been promoted (validated).
        // If the announcement is still in purgatory, the PR event should also go
        // to purgatory and wait for the announcement to be promoted.
        let db_repo_data = fetch_repository_data(&self.ctx.database, identifier).await?;

        // 3. Extract list of maintainers from "a 30617:<maintainer>:<identifier>" tags
        let mut maintainer_pubkeys = std::collections::HashSet::new();
        for repo_ref in &repo_refs {
            let parts: Vec<&str> = repo_ref.split(':').collect();
            if parts.len() >= 2 {
                maintainer_pubkeys.insert(parts[1].to_string());
            }
        }

        // 4. Identify owner repos that list any of the maintainers using this function
        let by_owner = collect_authorized_maintainers(&db_repo_data.announcements);

        // 5. Return the repo_path for each owner whose authorized maintainers include any of our maintainers
        let mut repo_paths = Vec::new();
        for announcement in &db_repo_data.announcements {
            let owner_pubkey = announcement.event.pubkey.to_hex();

            // Check if this owner's authorized maintainers overlap with our maintainer list
            if let Some(authorized_maintainers) = by_owner.get(&owner_pubkey) {
                let has_overlap = authorized_maintainers
                    .iter()
                    .any(|m| maintainer_pubkeys.contains(m));

                if has_overlap {
                    let repo_path = self.ctx.git_data_path.join(announcement.repo_path());
                    repo_paths.push(repo_path);
                }
            }
        }

        Ok(repo_paths)
    }
}
