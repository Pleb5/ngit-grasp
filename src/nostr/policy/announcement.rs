/// Announcement Policy - Repository announcement validation
///
/// Handles validation of NIP-34 repository announcements (kind 30617)
/// according to GRASP-01 specification.
use nostr_relay_builder::prelude::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};
use std::collections::HashSet;

use super::PolicyContext;
use crate::config::Config;
use crate::nostr::events::{validate_announcement, RepositoryAnnouncement};

/// Result of announcement policy evaluation
#[derive(Debug, Clone, PartialEq)]
pub enum AnnouncementResult {
    /// Accept: Event lists our service (GRASP-01 compliant) - replacement announcement
    Accept,
    /// Accept as maintainer: Event accepted via maintainer exception (multi-maintainer)
    AcceptMaintainer,
    /// Accept as archive: Event accepted via GRASP-05 archive whitelist (read-only)
    AcceptArchive,
    /// Accept to purgatory: New announcement, waiting for git data
    AcceptPurgatory,
    /// Reject: Event fails validation with reason
    Reject(String),
}

/// Policy for validating repository announcements
#[derive(Clone)]
pub struct AnnouncementPolicy {
    ctx: PolicyContext,
    config: Config,
}

impl AnnouncementPolicy {
    pub fn new(ctx: PolicyContext, config: Config) -> Self {
        Self { ctx, config }
    }

    /// Validate a repository announcement event
    ///
    /// Returns:
    /// - `Accept` if this is a replacement announcement (active announcement exists)
    /// - `AcceptPurgatory` if this is a new announcement (no active announcement exists)
    /// - `AcceptMaintainer` if accepted via maintainer exception
    /// - `AcceptArchive` if accepted via GRASP-05 archive config
    /// - `Reject` with reason if validation fails
    pub async fn validate(&self, event: &Event) -> AnnouncementResult {
        // First, try validation (GRASP-01 + GRASP-05)
        let validation_result = validate_announcement(event, &self.config);

        match validation_result {
            AnnouncementResult::Reject(reason) => {
                // Validation failed - check maintainer exception
                // GRASP-01 Exception: Accept announcements from recursive maintainers
                match RepositoryAnnouncement::from_event(event.clone()) {
                    Ok(announcement) => {
                        match self
                            .is_maintainer_in_any_announcement(
                                &announcement.identifier,
                                &event.pubkey,
                            )
                            .await
                        {
                            Ok(true) => AnnouncementResult::AcceptMaintainer,
                            Ok(false) => AnnouncementResult::Reject(reason),
                            Err(_) => {
                                // Fail-secure: reject on database errors
                                AnnouncementResult::Reject(reason)
                            }
                        }
                    }
                    Err(_) => AnnouncementResult::Reject(reason),
                }
            }
            AnnouncementResult::Accept | AnnouncementResult::AcceptArchive => {
                // Parse announcement to check for existing active announcement
                match RepositoryAnnouncement::from_event(event.clone()) {
                    Ok(announcement) => {
                        // Check if there's already an active announcement for this (pubkey, identifier)
                        match self
                            .has_active_announcement(&event.pubkey, &announcement.identifier)
                            .await
                        {
                            Ok(true) => {
                                // Replacement announcement - accept immediately
                                tracing::debug!(
                                    identifier = %announcement.identifier,
                                    "Replacement announcement - accepting immediately"
                                );
                                validation_result
                            }
                            Ok(false) => {
                                // New announcement - route to purgatory
                                tracing::debug!(
                                    identifier = %announcement.identifier,
                                    "New announcement - routing to purgatory"
                                );
                                AnnouncementResult::AcceptPurgatory
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "Failed to check for existing announcement - rejecting"
                                );
                                AnnouncementResult::Reject(format!(
                                    "Database error checking existing announcement: {}",
                                    e
                                ))
                            }
                        }
                    }
                    Err(e) => AnnouncementResult::Reject(format!(
                        "Failed to parse announcement: {}",
                        e
                    )),
                }
            }
            // AcceptPurgatory shouldn't come from validate_announcement, but handle it
            result => result,
        }
    }

    /// Check if there's an active announcement in the database for this (pubkey, identifier)
    async fn has_active_announcement(
        &self,
        pubkey: &PublicKey,
        identifier: &str,
    ) -> Result<bool, String> {
        let filter = Filter::new()
            .kind(Kind::GitRepoAnnouncement)
            .author(*pubkey)
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                identifier.to_string(),
            );

        let events: Vec<Event> = match self.ctx.database.query(filter).await {
            Ok(events) => events.into_iter().collect(),
            Err(e) => return Err(format!("Database query failed: {}", e)),
        };

        Ok(!events.is_empty())
    }

    /// Add an announcement to purgatory
    ///
    /// Creates the bare repository and stores the announcement in purgatory
    /// until git data arrives.
    pub fn add_to_purgatory(&self, event: &Event) -> Result<(), String> {
        let announcement = RepositoryAnnouncement::from_event(event.clone())
            .map_err(|e| format!("Failed to parse announcement: {}", e))?;

        // Create bare repository
        self.ensure_bare_repository(&announcement)?;

        // Build repo path
        let repo_path = self.ctx.git_data_path.join(announcement.repo_path());

        // Extract relays from announcement
        let relays: HashSet<String> = announcement.relays.iter().cloned().collect();

        // Add to purgatory
        self.ctx.purgatory.add_announcement(
            event.clone(),
            announcement.identifier.clone(),
            event.pubkey,
            repo_path,
            relays,
        );

        tracing::info!(
            identifier = %announcement.identifier,
            event_id = %event.id,
            "Added announcement to purgatory"
        );

        Ok(())
    }

    /// Create a bare git repository if it doesn't exist
    /// Path format: <git_data_path>/<npub>/<identifier>.git
    pub fn ensure_bare_repository(
        &self,
        announcement: &RepositoryAnnouncement,
    ) -> Result<(), String> {
        let repo_path = self.ctx.git_data_path.join(announcement.repo_path());

        // Check if repository already exists
        if repo_path.exists() {
            tracing::debug!("Repository already exists at {}", repo_path.display());
            return Ok(());
        }

        // Create parent directory (npub directory)
        let parent = repo_path
            .parent()
            .ok_or_else(|| format!("Invalid repository path: {}", repo_path.display()))?;

        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;

        // Initialize bare repository using git command
        let output = std::process::Command::new("git")
            .args(["init", "--bare", repo_path.to_str().unwrap()])
            .output()
            .map_err(|e| format!("Failed to execute git init: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git init failed: {}", stderr));
        }

        tracing::info!("Created bare repository at {}", repo_path.display());
        Ok(())
    }

    /// Check if a pubkey is listed as a maintainer in any announcement for this identifier
    ///
    /// A pubkey is considered a maintainer if:
    /// 1. They are the owner (pubkey) of an accepted announcement with this identifier, OR
    /// 2. They are listed in the maintainers tag of ANY announcement with this identifier
    ///
    /// This enables accepting announcements from maintainers even when they don't list
    /// this GRASP server, for maintainer chain discovery and GRASP-02 sync.
    ///
    /// Checks both the database (promoted announcements) and purgatory (announcements
    /// waiting for git data). This is necessary because a maintainer's announcement
    /// (which lists the recursive maintainer) may still be in purgatory when the
    /// recursive maintainer's announcement arrives.
    async fn is_maintainer_in_any_announcement(
        &self,
        identifier: &str,
        author: &PublicKey,
    ) -> Result<bool, String> {
        // Query all announcements with this identifier that are already in the database
        let filter = Filter::new().kind(Kind::GitRepoAnnouncement).custom_tag(
            SingleLetterTag::lowercase(Alphabet::D),
            identifier.to_string(),
        );

        let db_announcements: Vec<Event> = match self.ctx.database.query(filter).await {
            Ok(events) => events.into_iter().collect(),
            Err(e) => return Err(format!("Database query failed: {}", e)),
        };

        // Also collect purgatory announcements for this identifier
        let purgatory_announcements: Vec<Event> = self
            .ctx
            .purgatory
            .get_announcements_by_identifier(identifier)
            .into_iter()
            .map(|entry| entry.event)
            .collect();

        let all_announcements: Vec<&Event> = db_announcements
            .iter()
            .chain(purgatory_announcements.iter())
            .collect();

        if all_announcements.is_empty() {
            // No existing announcements for this identifier - author cannot be a maintainer
            return Ok(false);
        }

        let author_hex = author.to_hex();

        // Check each announcement to see if author is listed as a maintainer
        for event in &all_announcements {
            // Check if author is the owner of this announcement
            if event.pubkey == *author {
                return Ok(true);
            }

            // Check if author is listed in the maintainers tag
            if let Ok(announcement) = RepositoryAnnouncement::from_event((*event).clone()) {
                if announcement.maintainers.contains(&author_hex) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}
