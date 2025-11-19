/// NIP-34 Git Repository Event Handling
///
/// This module handles Git repository announcements (kind 30617) and
/// repository state announcements (kind 30618) according to NIP-34 and GRASP-01.
///
/// Reference:
/// - NIP-34: https://nips.nostr.com/34
/// - GRASP-01: https://gitworkshop.dev/danconwaydev.com/grasp/01.md
use anyhow::{anyhow, Result};
use nostr_sdk::{Event, Kind, TagKind, ToBech32};

/// NIP-34 Repository Announcement (kind 30617)
pub const KIND_REPOSITORY_ANNOUNCEMENT: u16 = 30617;

/// NIP-34 Repository State Announcement (kind 30618)
pub const KIND_REPOSITORY_STATE: u16 = 30618;

/// Repository announcement details extracted from NIP-34 event
#[derive(Debug, Clone)]
pub struct RepositoryAnnouncement {
    pub event: Event,
    pub identifier: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub clone_urls: Vec<String>,
    pub relays: Vec<String>,
    pub web_urls: Vec<String>,
    pub maintainers: Vec<String>,
}

impl RepositoryAnnouncement {
    /// Parse a repository announcement from a NIP-34 kind 30617 event
    pub fn from_event(event: Event) -> Result<Self> {
        if event.kind != Kind::from(KIND_REPOSITORY_ANNOUNCEMENT) {
            return Err(anyhow!(
                "Invalid event kind: expected {}, got {}",
                KIND_REPOSITORY_ANNOUNCEMENT,
                event.kind
            ));
        }

        // Extract identifier (required)
        let identifier = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .ok_or_else(|| anyhow!("Repository announcement missing 'd' tag (identifier)"))?
            .to_string();

        // Extract optional name
        let name = event
            .tags
            .iter()
            .find(|t| matches!(t.kind(), TagKind::Name))
            .and_then(|t| t.content())
            .map(|s| s.to_string());

        // Extract description from content
        let description = if event.content.is_empty() {
            None
        } else {
            Some(event.content.clone())
        };

        // Extract clone URLs
        let clone_urls = event
            .tags
            .iter()
            .filter(|t| matches!(t.kind(), TagKind::Clone))
            .flat_map(|t| {
                let vec = t.clone().to_vec();
                // Skip first element (tag name), rest are values
                vec.into_iter().skip(1)
            })
            .collect();

        // Extract relays
        let relays = event
            .tags
            .iter()
            .filter(|t| matches!(t.kind(), TagKind::Relays))
            .flat_map(|t| {
                let vec = t.clone().to_vec();
                // Skip first element (tag name), rest are values
                vec.into_iter().skip(1)
            })
            .collect();

        // Extract web URLs
        let web_urls = event
            .tags
            .iter()
            .filter(|t| {
                if let TagKind::Custom(s) = t.kind() {
                    s.as_ref() == "web"
                } else {
                    false
                }
            })
            .flat_map(|t| {
                let vec = t.clone().to_vec();
                // Skip first element (tag name), rest are values
                vec.into_iter().skip(1)
            })
            .collect();

        // Extract maintainers (other-user tags)
        let maintainers = event
            .tags
            .iter()
            .filter(|t| t.kind() == TagKind::p())
            .filter_map(|t| t.content())
            .map(|s| s.to_string())
            .collect();

        Ok(RepositoryAnnouncement {
            event,
            identifier,
            name,
            description,
            clone_urls,
            relays,
            web_urls,
            maintainers,
        })
    }

    /// Check if this announcement lists the given domain in clone URLs
    pub fn has_clone_url(&self, domain: &str) -> bool {
        self.clone_urls.iter().any(|url| url.contains(domain))
    }

    /// Check if this announcement lists the given relay
    pub fn has_relay(&self, relay: &str) -> bool {
        self.relays.iter().any(|r| r.contains(relay))
    }

    /// Check if this announcement lists the service (both clone and relay)
    ///
    /// GRASP-01 requirement: MUST reject announcements that do not list
    /// the service in both `clone` and `relays` tags unless implementing GRASP-05.
    pub fn lists_service(&self, domain: &str) -> bool {
        self.has_clone_url(domain) && self.has_relay(domain)
    }

    /// Get the npub of the repository owner
    pub fn owner_npub(&self) -> String {
        self.event.pubkey.to_bech32().unwrap_or_default()
    }

    /// Get the repository path: <npub>/<identifier>.git
    pub fn repo_path(&self) -> String {
        format!("{}/{}.git", self.owner_npub(), self.identifier)
    }
}

/// Repository state details extracted from NIP-34 event
#[derive(Debug, Clone)]
pub struct RepositoryState {
    pub event: Event,
    pub identifier: String,
    pub branches: Vec<BranchState>,
    pub tags: Vec<TagState>,
}

/// Branch state (ref with commit hash)
#[derive(Debug, Clone)]
pub struct BranchState {
    pub name: String,
    pub commit: String,
}

/// Tag state (ref with commit hash)
#[derive(Debug, Clone)]
pub struct TagState {
    pub name: String,
    pub commit: String,
}

impl RepositoryState {
    /// Parse a repository state from a NIP-34 kind 30618 event
    pub fn from_event(event: Event) -> Result<Self> {
        if event.kind != Kind::from(KIND_REPOSITORY_STATE) {
            return Err(anyhow!(
                "Invalid event kind: expected {}, got {}",
                KIND_REPOSITORY_STATE,
                event.kind
            ));
        }

        // Extract identifier (required)
        let identifier = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .ok_or_else(|| anyhow!("Repository state missing 'd' tag (identifier)"))?
            .to_string();

        // Extract branches (refs/heads/*)
        let branches = event
            .tags
            .iter()
            .filter(|t| {
                if let TagKind::Custom(s) = t.kind() {
                    s.as_ref() == "ref"
                } else {
                    false
                }
            })
            .filter_map(|t| {
                let parts = t.clone().to_vec();
                if parts.len() >= 3 && parts[1].starts_with("refs/heads/") {
                    Some(BranchState {
                        name: parts[1].strip_prefix("refs/heads/").unwrap().to_string(),
                        commit: parts[2].clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        // Extract tags (refs/tags/*)
        let tags = event
            .tags
            .iter()
            .filter(|t| {
                if let TagKind::Custom(s) = t.kind() {
                    s.as_ref() == "ref"
                } else {
                    false
                }
            })
            .filter_map(|t| {
                let parts = t.clone().to_vec();
                if parts.len() >= 3 && parts[1].starts_with("refs/tags/") {
                    Some(TagState {
                        name: parts[1].strip_prefix("refs/tags/").unwrap().to_string(),
                        commit: parts[2].clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(RepositoryState {
            event,
            identifier,
            branches,
            tags,
        })
    }

    /// Get the commit hash for a branch
    pub fn get_branch_commit(&self, branch: &str) -> Option<&str> {
        self.branches
            .iter()
            .find(|b| b.name == branch)
            .map(|b| b.commit.as_str())
    }

    /// Get the commit hash for a tag
    pub fn get_tag_commit(&self, tag: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|t| t.name == tag)
            .map(|t| t.commit.as_str())
    }

    /// Get the owner npub
    pub fn owner_npub(&self) -> String {
        self.event.pubkey.to_bech32().unwrap_or_default()
    }
}

/// Validate a repository announcement according to GRASP-01
///
/// Returns Ok(()) if valid, Err with reason if invalid.
pub fn validate_announcement(event: &Event, domain: &str) -> Result<()> {
    // Must be kind 30617
    if event.kind != Kind::from(KIND_REPOSITORY_ANNOUNCEMENT) {
        return Err(anyhow!(
            "Invalid kind: expected {}",
            KIND_REPOSITORY_ANNOUNCEMENT
        ));
    }

    // Must have identifier
    let has_identifier = event.tags.iter().any(|t| t.kind() == TagKind::d());
    if !has_identifier {
        return Err(anyhow!("Missing required 'd' tag (identifier)"));
    }

    // Parse full announcement to validate structure
    let announcement = RepositoryAnnouncement::from_event(event.clone())?;

    // GRASP-01: MUST reject announcements that do not list the service
    // in both `clone` and `relays` tags unless implementing GRASP-05
    if !announcement.lists_service(domain) {
        return Err(anyhow!(
            "Announcement must list service in both 'clone' and 'relays' tags. \
             Found clone URLs: {:?}, relays: {:?}",
            announcement.clone_urls,
            announcement.relays
        ));
    }

    Ok(())
}

/// Validate a repository state announcement according to GRASP-01
///
/// Returns Ok(()) if valid, Err with reason if invalid.
pub fn validate_state(event: &Event) -> Result<()> {
    // Must be kind 30618
    if event.kind != Kind::from(KIND_REPOSITORY_STATE) {
        return Err(anyhow!("Invalid kind: expected {}", KIND_REPOSITORY_STATE));
    }

    // Must have identifier
    let has_identifier = event.tags.iter().any(|t| t.kind() == TagKind::d());
    if !has_identifier {
        return Err(anyhow!("Missing required 'd' tag (identifier)"));
    }

    // Parse full state to validate structure
    let _state = RepositoryState::from_event(event.clone())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Tag};

    fn create_test_keys() -> Keys {
        Keys::generate()
    }

    fn create_announcement_event(
        keys: &Keys,
        identifier: &str,
        clone_urls: Vec<&str>,
        relays: Vec<&str>,
    ) -> Event {
        use nostr_sdk::Tag;

        let mut tags = vec![Tag::custom(
            nostr_sdk::TagKind::d(),
            vec![identifier.to_string()],
        )];

        for url in clone_urls {
            tags.push(Tag::custom(
                nostr_sdk::TagKind::Clone,
                vec![url.to_string()],
            ));
        }

        for relay in relays {
            tags.push(Tag::custom(
                nostr_sdk::TagKind::Relays,
                vec![relay.to_string()],
            ));
        }

        EventBuilder::new(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT), "Test repository")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn create_state_event(keys: &Keys, identifier: &str, branches: Vec<(&str, &str)>) -> Event {
        use nostr_sdk::Tag;

        let mut tags = vec![Tag::custom(
            nostr_sdk::TagKind::d(),
            vec![identifier.to_string()],
        )];

        for (branch, commit) in branches {
            tags.push(Tag::custom(
                nostr_sdk::TagKind::Custom("ref".into()),
                vec![format!("refs/heads/{}", branch), commit.to_string()],
            ));
        }

        EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn test_parse_announcement() {
        let keys = create_test_keys();
        let event = create_announcement_event(
            &keys,
            "test-repo",
            vec!["https://gitnostr.com/alice/test-repo.git"],
            vec!["wss://gitnostr.com"],
        );

        let announcement = RepositoryAnnouncement::from_event(event).unwrap();

        assert_eq!(announcement.identifier, "test-repo");
        assert_eq!(announcement.clone_urls.len(), 1);
        assert_eq!(announcement.relays.len(), 1);
        assert!(announcement.has_clone_url("gitnostr.com"));
        assert!(announcement.has_relay("gitnostr.com"));
        assert!(announcement.lists_service("gitnostr.com"));
    }

    #[test]
    fn test_parse_announcement_missing_identifier() {
        let keys = create_test_keys();
        let event = EventBuilder::new(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT), "Test repository")
            .sign_with_keys(&keys)
            .unwrap();

        let result = RepositoryAnnouncement::from_event(event);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("identifier"));
    }

    #[test]
    fn test_parse_state() {
        let keys = create_test_keys();
        let event = create_state_event(
            &keys,
            "test-repo",
            vec![("main", "a1b2c3d4"), ("develop", "e5f6g7h8")],
        );

        let state = RepositoryState::from_event(event).unwrap();

        assert_eq!(state.identifier, "test-repo");
        assert_eq!(state.branches.len(), 2);
        assert_eq!(state.get_branch_commit("main"), Some("a1b2c3d4"));
        assert_eq!(state.get_branch_commit("develop"), Some("e5f6g7h8"));
    }

    #[test]
    fn test_validate_announcement_success() {
        let keys = create_test_keys();
        let event = create_announcement_event(
            &keys,
            "test-repo",
            vec!["https://gitnostr.com/alice/test-repo.git"],
            vec!["wss://gitnostr.com"],
        );

        let result = validate_announcement(&event, "gitnostr.com");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_announcement_missing_clone() {
        let keys = create_test_keys();
        let event = create_announcement_event(
            &keys,
            "test-repo",
            vec![], // No clone URLs
            vec!["wss://gitnostr.com"],
        );

        let result = validate_announcement(&event, "gitnostr.com");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("clone"));
    }

    #[test]
    fn test_validate_announcement_missing_relay() {
        let keys = create_test_keys();
        let event = create_announcement_event(
            &keys,
            "test-repo",
            vec!["https://gitnostr.com/alice/test-repo.git"],
            vec![], // No relays
        );

        let result = validate_announcement(&event, "gitnostr.com");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("relays"));
    }

    #[test]
    fn test_validate_announcement_wrong_domain() {
        let keys = create_test_keys();
        let event = create_announcement_event(
            &keys,
            "test-repo",
            vec!["https://other-service.com/alice/test-repo.git"],
            vec!["wss://other-service.com"],
        );

        let result = validate_announcement(&event, "gitnostr.com");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_state_success() {
        let keys = create_test_keys();
        let event = create_state_event(&keys, "test-repo", vec![("main", "a1b2c3d4")]);

        let result = validate_state(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_state_missing_identifier() {
        let keys = create_test_keys();
        let event = EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "")
            .sign_with_keys(&keys)
            .unwrap();

        let result = validate_state(&event);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("identifier"));
    }

    #[test]
    fn test_announcement_maintainers() {
        use nostr_sdk::Tag;

        let keys = create_test_keys();
        let maintainer_keys = create_test_keys();

        let mut tags = vec![
            Tag::custom(nostr_sdk::TagKind::d(), vec!["test-repo".to_string()]),
            Tag::custom(
                nostr_sdk::TagKind::Clone,
                vec!["https://gitnostr.com/alice/test-repo.git".to_string()],
            ),
            Tag::custom(
                nostr_sdk::TagKind::Relays,
                vec!["wss://gitnostr.com".to_string()],
            ),
        ];

        // Add maintainer
        tags.push(Tag::public_key(maintainer_keys.public_key()));

        let event = EventBuilder::new(Kind::from(KIND_REPOSITORY_ANNOUNCEMENT), "Test repository")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();

        let announcement = RepositoryAnnouncement::from_event(event).unwrap();
        assert_eq!(announcement.maintainers.len(), 1);
    }

    #[test]
    fn test_state_with_tags() {
        use nostr_sdk::Tag;

        let keys = create_test_keys();
        let mut tags = vec![Tag::custom(
            nostr_sdk::TagKind::d(),
            vec!["test-repo".to_string()],
        )];

        // Add branch
        tags.push(Tag::custom(
            nostr_sdk::TagKind::Custom("ref".into()),
            vec!["refs/heads/main".to_string(), "a1b2c3d4".to_string()],
        ));

        // Add tag
        tags.push(Tag::custom(
            nostr_sdk::TagKind::Custom("ref".into()),
            vec!["refs/tags/v1.0.0".to_string(), "e5f6g7h8".to_string()],
        ));

        let event = EventBuilder::new(Kind::from(KIND_REPOSITORY_STATE), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();

        let state = RepositoryState::from_event(event).unwrap();
        assert_eq!(state.branches.len(), 1);
        assert_eq!(state.tags.len(), 1);
        assert_eq!(state.get_branch_commit("main"), Some("a1b2c3d4"));
        assert_eq!(state.get_tag_commit("v1.0.0"), Some("e5f6g7h8"));
    }
}
