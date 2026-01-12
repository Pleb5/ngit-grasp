/// Announcement Policy - Repository announcement validation
///
/// Handles validation of NIP-34 repository announcements (kind 30617)
/// according to GRASP-01 specification.
use nostr_relay_builder::prelude::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};

use super::PolicyContext;
use crate::config::ArchiveConfig;
use crate::nostr::events::{validate_announcement, RepositoryAnnouncement};

/// Result of announcement policy evaluation
#[derive(Debug, Clone, PartialEq)]
pub enum AnnouncementResult {
    /// Accept: Event lists our service (GRASP-01 compliant)
    Accept,
    /// Accept as maintainer: Event accepted via maintainer exception (multi-maintainer)
    AcceptMaintainer,
    /// Accept as archive: Event accepted via GRASP-05 archive whitelist (read-only)
    AcceptArchive,
    /// Reject: Event fails validation with reason
    Reject(String),
}

/// Policy for validating repository announcements
#[derive(Clone)]
pub struct AnnouncementPolicy {
    ctx: PolicyContext,
    archive_config: ArchiveConfig,
}

impl AnnouncementPolicy {
    pub fn new(ctx: PolicyContext, archive_config: ArchiveConfig) -> Self {
        Self {
            ctx,
            archive_config,
        }
    }

    /// Validate a repository announcement event
    ///
    /// Returns `Accept` if the announcement lists the service properly,
    /// `AcceptMaintainer` if accepted via maintainer exception,
    /// `AcceptArchive` if accepted via GRASP-05 archive config,
    /// or `Reject` with reason.
    pub async fn validate(&self, event: &Event) -> AnnouncementResult {
        // First, try validation (GRASP-01 + GRASP-05)
        let validation_result =
            validate_announcement(event, &self.ctx.domain, &self.archive_config);

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
            // Accept, AcceptArchive, or AcceptMaintainer - return as-is
            result => result,
        }
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

        let announcements: Vec<Event> = match self.ctx.database.query(filter).await {
            Ok(events) => events.into_iter().collect(),
            Err(e) => return Err(format!("Database query failed: {}", e)),
        };

        if announcements.is_empty() {
            // No existing announcements for this identifier - author cannot be a maintainer
            return Ok(false);
        }

        let author_hex = author.to_hex();

        // Check each announcement to see if author is listed as a maintainer
        for event in &announcements {
            // Check if author is the owner of this announcement
            if event.pubkey == *author {
                return Ok(true);
            }

            // Check if author is listed in the maintainers tag
            if let Ok(announcement) = RepositoryAnnouncement::from_event(event.clone()) {
                if announcement.maintainers.contains(&author_hex) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}
