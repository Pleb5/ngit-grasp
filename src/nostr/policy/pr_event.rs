/// PR Event Policy - PR/PR Update validation
///
/// Handles validation of NIP-34 PR events (kind 1618) and PR Update events (kind 1619)
/// according to GRASP-01 specification.
use nostr_relay_builder::prelude::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};

use super::PolicyContext;
use crate::git;
use crate::nostr::events::{RepositoryAnnouncement, KIND_REPOSITORY_ANNOUNCEMENT};

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
    /// This checks:
    /// 1. If a placeholder exists (git-data-first scenario)
    /// 2. If the commit exists in any relevant repository
    ///
    /// # Returns
    /// - `Ok(true)` if git data ready (either placeholder found or commit exists)
    /// - `Ok(false)` if git data missing (should add to purgatory)
    /// - `Err(msg)` on errors
    pub async fn check_git_data_exists(&self, event: &Event) -> Result<bool, String> {
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
                return Err(format!("PR event {} has no 'c' tag", event_id));
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
                return Ok(true);
            } else {
                // Placeholder has different commit - incoming event supersedes
                tracing::info!(
                    "PR event {} supersedes placeholder: event expects commit {}, placeholder has {}",
                    event_id,
                    commit,
                    placeholder_commit
                );
                // Remove placeholder with old commit data
                self.ctx.purgatory.remove_pr(&event_id);
                // TODO: Also remove git data (refs/nostr/<event-id>) - Phase 5
                // Fall through to check if new commit exists
            }
        }

        // Check if commit exists in any repository referenced by this PR
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
            // No repo references - cannot check git data
            // This is unusual but let it through (other validation will catch issues)
            return Ok(true);
        }

        // Check each repository to see if commit exists
        for repo_ref in repo_refs {
            // Parse the repo reference: 30617:<pubkey>:<identifier>
            let parts: Vec<&str> = repo_ref.split(':').collect();
            if parts.len() < 3 {
                continue;
            }

            let repo_pubkey = match PublicKey::from_hex(parts[1]) {
                Ok(pk) => pk,
                Err(_) => continue,
            };
            let identifier = parts[2];

            // Look up repository announcement to get the npub for path
            let filter = Filter::new()
                .kind(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT))
                .author(repo_pubkey)
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::D),
                    identifier.to_string(),
                );

            let announcements: Vec<Event> = match self.ctx.database.query(filter).await {
                Ok(events) => events.into_iter().collect(),
                Err(e) => {
                    tracing::warn!(
                        "Failed to query for repository announcement for PR {}: {}",
                        event_id,
                        e
                    );
                    continue;
                }
            };

            if announcements.is_empty() {
                continue;
            }

            // Check each matching announcement
            for announcement_event in announcements {
                let announcement = match RepositoryAnnouncement::from_event(announcement_event) {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                // Build repository path
                let repo_path = self.ctx.git_data_path.join(announcement.repo_path());

                // Check if commit exists
                if git::commit_exists(&repo_path, &commit) {
                    tracing::debug!(
                        "Found commit {} for PR event {} in repository {}",
                        commit,
                        event_id,
                        repo_path.display()
                    );
                    return Ok(true);
                }
            }
        }

        // No git data found - should add to purgatory
        tracing::debug!(
            "No git data found for PR event {} with commit {}",
            event_id,
            commit
        );
        Ok(false)
    }

    /// Validate refs/nostr/<event-id> ref against a PR or PR Update event's `c` tag
    ///
    /// When a PR event (kind 1618) or PR Update event (kind 1619) is received,
    /// this checks if a corresponding refs/nostr/<event-id> ref exists in the
    /// repository and validates that it points to the correct commit (from the
    /// `c` tag). If the ref exists but points to a different commit, the ref is
    /// deleted.
    ///
    /// PR and PR Update events can have multiple `a` tags to update multiple
    /// repositories simultaneously.
    ///
    /// This is part of GRASP-01 compliance: ensuring refs/nostr refs are consistent
    /// with their corresponding events.
    ///
    /// # Returns
    /// Ok(Some(n)) if n refs were deleted, Ok(None) if no action taken, Err on failure
    pub async fn validate_nostr_ref(&self, event: &Event) -> Result<Option<usize>, String> {
        let event_id = event.id.to_hex();

        // Extract the `c` tag (commit hash) from the PR event
        let expected_commit = event.tags.iter().find_map(|tag| {
            let tag_vec = tag.clone().to_vec();
            if tag_vec.len() >= 2 && tag_vec[0] == "c" {
                Some(tag_vec[1].clone())
            } else {
                None
            }
        });

        let expected_commit = match expected_commit {
            Some(c) => c,
            None => {
                tracing::debug!(
                    "PR event {} has no 'c' tag, skipping ref validation",
                    event_id
                );
                return Ok(None);
            }
        };

        // Extract ALL `a` tags (repository references) from the PR event
        // PR events can reference multiple repositories
        // Format: 30617:<pubkey>:<identifier>
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
            tracing::debug!(
                "PR event {} has no repo 'a' tags, skipping ref validation",
                event_id
            );
            return Ok(None);
        }

        let mut deleted_count = 0;

        // Process each repository reference
        for repo_ref in repo_refs {
            // Parse the repo reference: 30617:<pubkey>:<identifier>
            let parts: Vec<&str> = repo_ref.split(':').collect();
            if parts.len() < 3 {
                tracing::debug!(
                    "PR event {} has invalid 'a' tag format: {}",
                    event_id,
                    repo_ref
                );
                continue;
            }

            let repo_pubkey = match PublicKey::from_hex(parts[1]) {
                Ok(pk) => pk,
                Err(_) => {
                    tracing::debug!(
                        "PR event {} has invalid pubkey in 'a' tag: {}",
                        event_id,
                        parts[1]
                    );
                    continue;
                }
            };
            let identifier = parts[2];

            // Look up repository announcement to get the npub for path
            let filter = Filter::new()
                .kind(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT))
                .author(repo_pubkey)
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::D),
                    identifier.to_string(),
                );

            let announcements: Vec<Event> = match self.ctx.database.query(filter).await {
                Ok(events) => events.into_iter().collect(),
                Err(e) => {
                    tracing::warn!(
                        "Failed to query for repository announcement for PR {}: {}",
                        event_id,
                        e
                    );
                    continue;
                }
            };

            if announcements.is_empty() {
                tracing::debug!(
                    "No repository announcement found for PR event {} (repo {}:{})",
                    event_id,
                    repo_pubkey.to_hex(),
                    identifier
                );
                continue;
            }

            // Process each matching announcement (there could be multiple)
            for announcement_event in announcements {
                let announcement = match RepositoryAnnouncement::from_event(announcement_event) {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse announcement for PR {} validation: {}",
                            event_id,
                            e
                        );
                        continue;
                    }
                };

                // Build repository path
                let repo_path = self.ctx.git_data_path.join(announcement.repo_path());

                // Validate the ref
                match git::validate_nostr_ref(&repo_path, &event_id, &expected_commit) {
                    Ok(true) => {
                        tracing::info!(
                            "Deleted mismatched refs/nostr/{} in {} (expected commit {})",
                            event_id,
                            repo_path.display(),
                            expected_commit
                        );
                        deleted_count += 1;
                    }
                    Ok(false) => {
                        tracing::debug!(
                            "refs/nostr/{} in {} is valid or doesn't exist",
                            event_id,
                            repo_path.display()
                        );
                    }
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
        }

        if deleted_count > 0 {
            Ok(Some(deleted_count))
        } else {
            Ok(None)
        }
    }
}
