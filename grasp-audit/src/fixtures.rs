//! Test fixture management for dual-mode testing
//!
//! This module provides a TestContext abstraction that manages prerequisite events
//! differently based on the audit mode:
//!
//! - **CI Mode (Isolated)**: Creates fresh events for each test, ensuring complete isolation
//! - **Production Mode (Shared)**: Reuses shared fixtures to minimize event publication
//!
//! # Example
//!
//! ```no_run
//! use grasp_audit::*;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = AuditConfig::ci();
//! let client = AuditClient::new("ws://localhost:7000", config).await?;
//! let ctx = TestContext::new(&client);
//!
//! // Request a fixture - behavior depends on mode
//! let repo = ctx.get_fixture(FixtureKind::ValidRepo).await?;
//! # Ok(())
//! # }
//! ```

use crate::{AuditClient, AuditMode};
use anyhow::{Context, Result};
use nostr_sdk::prelude::Event;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Deterministic commit hash used in RepoState fixtures (Owner variant)
/// This is the hash produced by creating a commit with:
/// - Message: "Initial commit"
/// - File: test.txt containing "Initial commit"
/// - Author date: 2024-01-01T00:00:00Z
/// - Committer date: 2024-01-01T00:00:00Z
/// - GPG signing: disabled
/// - User: "GRASP Audit Test <test@grasp-audit.local>"
/// - Parent: Initial empty commit (09cc37de80f3434fa98864a86730b8d7777bd6ae)
pub const DETERMINISTIC_COMMIT_HASH: &str = "64ea71d79a57a7acb334cd9651f8aec067c0ce5d";

/// Deterministic commit hash for maintainer fixtures (Maintainer variant)
/// This is the hash produced by creating a commit with:
/// - Message: "Maintainer initial commit"
/// - File: test.txt containing "Maintainer initial commit"
/// - Author date: 2024-01-01T00:00:00Z
/// - Committer date: 2024-01-01T00:00:00Z
/// - GPG signing: disabled
/// - User: "GRASP Audit Test <test@grasp-audit.local>"
/// - Parent: none (root commit)
/// NOTE: This value is different from DETERMINISTIC_COMMIT_HASH due to different content
pub const MAINTAINER_DETERMINISTIC_COMMIT_HASH: &str = "1c2d472c9b71ed51968a66500281a3c4a6840464";

/// Deterministic commit hash for recursive maintainer fixtures (RecursiveMaintainer variant)
/// This is the hash produced by creating a commit with:
/// - Message: "Recursive maintainer initial commit"
/// - File: test.txt containing "Recursive maintainer initial commit"
/// - Author date: 2024-01-01T00:00:00Z
/// - Committer date: 2024-01-01T00:00:00Z
/// - GPG signing: disabled
/// - User: "GRASP Audit Test <test@grasp-audit.local>"
/// - Parent: none (root commit)
/// NOTE: This value is different from DETERMINISTIC_COMMIT_HASH due to different content
pub const RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH: &str = "05939b82de66fbdb9c077d0a64fc68522f3cb8e0";

/// Types of test fixtures available
///
/// ## Fixture Dependencies
///
/// Several fixtures depend on `ValidRepo` - they all use the SAME repo_id
/// within a single TestContext instance to ensure proper fixture relationships:
/// - `RepoState` → uses ValidRepo's repo_id
/// - `MaintainerAnnouncement` + `MaintainerState` → uses ValidRepo's repo_id
/// - `RecursiveMaintainerRepoAndState` → uses ValidRepo's repo_id
///
/// This enables testing recursive maintainer authorization chains where multiple
/// parties publish announcements and state events for the same repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixtureKind {
    /// Basic repository announcement (kind 30617)
    /// - Signed by owner keys (`client.keys()`)
    /// - Lists `client.maintainer_pubkey_hex()` in maintainers tag
    ValidRepo,

    /// Repository with one issue (kind 1621)
    /// - Requires ValidRepo (reuses same repo_id)
    RepoWithIssue,

    /// Repository with issue and comment (kind 1111)
    /// - Requires RepoWithIssue (reuses same repo_id)
    RepoWithComment,

    /// Repository state announcement (kind 30618) for owner
    /// - Requires ValidRepo (uses same repo_id)
    /// - Signed by owner keys (`client.keys()`)
    /// - Points to DETERMINISTIC_COMMIT_HASH
    /// - Timestamp: 10 seconds in the past
    RepoState,

    /// Maintainer's repo announcement only for the SAME repo_id as ValidRepo
    /// - Requires ValidRepo (uses same repo_id for maintainer chain)
    /// - Announcement signed by `client.maintainer_keys()`
    /// - Lists `client.recursive_maintainer_pubkey_hex()` in maintainers tag
    /// - Does NOT include state event (use MaintainerState for that)
    MaintainerAnnouncement,

    /// Maintainer's state event only for the SAME repo_id as ValidRepo
    /// - Requires ValidRepo (uses same repo_id for maintainer chain)
    /// - State event signed by `client.maintainer_keys()`
    /// - Points to MAINTAINER_DETERMINISTIC_COMMIT_HASH
    /// - Timestamp: 5 seconds in the past (more recent than owner's state)
    /// - Does NOT include announcement (use MaintainerAnnouncement for that)
    MaintainerState,

    /// Recursive maintainer's announcement only for the SAME repo_id as ValidRepo
    /// - Requires ValidRepo (uses same repo_id for recursive chain)
    /// - Announcement signed by `client.recursive_maintainer_keys()`
    /// - Lists owner and maintainer in maintainers tag
    /// - Does NOT include state event (use RecursiveMaintainerState for that)
    RecursiveMaintainerAnnouncement,

    /// Recursive maintainer's state event only for the SAME repo_id as ValidRepo
    /// - Requires ValidRepo (uses same repo_id for recursive chain)
    /// - State event signed by `client.recursive_maintainer_keys()`
    /// - Points to RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
    /// - Timestamp: 2 seconds in the past (most recent)
    /// - Does NOT include announcement (use RecursiveMaintainerAnnouncement for that)
    RecursiveMaintainerState,

    /// Recursive maintainer's announcement + state for the SAME repo_id as ValidRepo
    /// - Requires ValidRepo (uses same repo_id for recursive chain)
    /// - Announcement signed by `client.recursive_maintainer_keys()`
    /// - Lists owner and maintainer in maintainers tag
    /// - State event points to RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
    /// - Timestamp: 2 seconds in the past (most recent)
    RecursiveMaintainerRepoAndState,
}

/// Context mode for fixture management
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMode {
    /// Create fresh fixtures for each request (test isolation)
    Isolated,

    /// Reuse shared fixtures across requests (minimal events)
    Shared,
}

impl From<AuditMode> for ContextMode {
    fn from(mode: AuditMode) -> Self {
        match mode {
            AuditMode::CI => ContextMode::Isolated,
            AuditMode::Production => ContextMode::Shared,
        }
    }
}

/// Test context for managing prerequisite events
///
/// The TestContext provides mode-aware fixture management:
/// - In Isolated mode: Creates fresh events for each test
/// - In Shared mode: Caches and reuses events across tests
///
/// # Example
///
/// ```no_run
/// # use grasp_audit::*;
/// # async fn example() -> anyhow::Result<()> {
/// let config = AuditConfig::ci();
/// let client = AuditClient::new("ws://localhost:7000", config).await?;
/// let ctx = TestContext::new(&client);
///
/// // Get a repository fixture
/// let repo = ctx.get_fixture(FixtureKind::ValidRepo).await?;
///
/// // In CI mode: Creates new repo
/// // In Production mode: Returns cached repo
/// # Ok(())
/// # }
/// ```
pub struct TestContext<'a> {
    client: &'a AuditClient,
    mode: ContextMode,
    cache: Arc<Mutex<HashMap<FixtureKind, Event>>>,
}

impl<'a> TestContext<'a> {
    /// Create a new test context
    ///
    /// The context mode is automatically determined from the client's audit config.
    pub fn new(client: &'a AuditClient) -> Self {
        let mode = ContextMode::from(client.config.mode);
        Self {
            client,
            mode,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a test context with explicit mode override
    ///
    /// This is useful for testing the context itself or for advanced use cases
    /// where you want to override the default mode behavior.
    pub fn with_mode(client: &'a AuditClient, mode: ContextMode) -> Self {
        Self {
            client,
            mode,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get a fixture, creating it if needed based on mode
    ///
    /// # Behavior
    ///
    /// - **Isolated mode**: Always creates a fresh fixture
    /// - **Shared mode**: Returns cached fixture or creates and caches if not present
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use grasp_audit::*;
    /// # async fn example(ctx: &TestContext<'_>) -> anyhow::Result<()> {
    /// let repo = ctx.get_fixture(FixtureKind::ValidRepo).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_fixture(&self, kind: FixtureKind) -> Result<Event> {
        match self.mode {
            ContextMode::Isolated => self.create_fresh(kind).await,
            ContextMode::Shared => self.get_or_create_shared(kind).await,
        }
    }

    /// Get the underlying client for direct access
    ///
    /// This allows tests to use the client directly when needed while still
    /// benefiting from the TestContext for fixture management.
    pub fn client(&self) -> &'a AuditClient {
        self.client
    }

    /// Get the current context mode
    pub fn mode(&self) -> ContextMode {
        self.mode
    }

    /// Create a fresh fixture (always creates new)
    async fn create_fresh(&self, kind: FixtureKind) -> Result<Event> {
        let event = self
            .build_fixture(kind)
            .await
            .with_context(|| format!("Failed to build {:?} fixture", kind))?;

        self.client
            .send_event(event.clone())
            .await
            .with_context(|| format!("Failed to send {:?} fixture event to relay", kind))?;

        Ok(event)
    }

    /// Get or create a shared fixture (caches for reuse)
    async fn get_or_create_shared(&self, kind: FixtureKind) -> Result<Event> {
        // Check cache first
        {
            let cache = self.cache.lock().unwrap();
            if let Some(event) = cache.get(&kind) {
                return Ok(event.clone());
            }
        }

        // Not in cache, create it
        let event = self
            .build_fixture(kind)
            .await
            .with_context(|| format!("Failed to build {:?} fixture for shared cache", kind))?;

        self.client
            .send_event(event.clone())
            .await
            .with_context(|| {
                format!(
                    "Failed to send {:?} fixture event to relay (shared cache)",
                    kind
                )
            })?;

        // Store in cache
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(kind, event.clone());
        }

        Ok(event)
    }

    /// Get or create a ValidRepo, with caching within the TestContext.
    /// This is a helper method that avoids async recursion by not going
    /// through get_fixture. It handles the repo specifically.
    ///
    /// IMPORTANT: We always cache within a TestContext instance to ensure
    /// fixture dependencies work correctly. The isolation between tests
    /// comes from each test having its own TestContext with a fresh cache.
    async fn get_or_create_repo(&self) -> Result<Event> {
        // Always check cache first - this ensures fixture dependencies work
        // (e.g., MaintainerRepoAndState needs the SAME repo_id as RepoState)
        {
            let cache = self.cache.lock().unwrap();
            if let Some(event) = cache.get(&FixtureKind::ValidRepo) {
                return Ok(event.clone());
            }
        }

        // Create a new repo
        let test_name = format!(
            "fixture-{:?}-{}",
            FixtureKind::ValidRepo,
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let repo = self.client.create_repo_announcement(&test_name).await?;

        // Send it
        self.client.send_event(repo.clone()).await?;

        // Always cache it - isolation comes from each test having its own TestContext
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(FixtureKind::ValidRepo, repo.clone());
        }

        Ok(repo)
    }

    /// Get or create a RepoWithIssue, with caching within the TestContext.
    /// Returns the issue event (repo is already sent/cached via get_or_create_repo).
    async fn get_or_create_issue(&self) -> Result<Event> {
        // Always check cache first - ensures fixture dependencies work
        {
            let cache = self.cache.lock().unwrap();
            if let Some(event) = cache.get(&FixtureKind::RepoWithIssue) {
                return Ok(event.clone());
            }
        }

        // Get or create repo (reuses cached within this TestContext)
        let repo = self.get_or_create_repo().await?;

        // Create the issue
        let issue = self.client.create_issue(
            &repo,
            "Test Issue",
            "Issue content for testing",
            vec![],
        )?;

        // Send it
        self.client.send_event(issue.clone()).await?;

        // Always cache it - isolation comes from each test having its own TestContext
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(FixtureKind::RepoWithIssue, issue.clone());
        }

        Ok(issue)
    }

    /// Build a fixture event (doesn't send it)
    async fn build_fixture(&self, kind: FixtureKind) -> Result<Event> {
        match kind {
            FixtureKind::ValidRepo => {
                // Delegate to get_or_create_repo() which handles caching properly.
                self.get_or_create_repo().await
            }

            FixtureKind::RepoWithIssue => {
                // Reuse ValidRepo fixture - this leverages caching in Shared mode
                // In Isolated mode: creates fresh repo
                // In Shared mode: returns cached repo (no duplicate events!)
                // Uses direct helper to avoid async recursion through get_fixture
                let repo = self.get_or_create_repo().await?;

                // Then create issue referencing it - this will have 'a' tag to repo
                // Note: We build the issue but DON'T send it here - the caller will send it
                let issue = self.client.create_issue(
                    &repo,
                    "Test Issue",
                    "Issue content for testing",
                    vec![],
                )?;

                // Return the issue - tests can extract repo reference from its 'a' tag
                // The caller (create_fresh/get_or_create_shared) will send this event
                Ok(issue)
            }

            FixtureKind::RepoWithComment => {
                // Reuse RepoWithIssue fixture - this leverages caching in Shared mode
                // In Isolated mode: creates fresh repo + issue
                // In Shared mode: returns cached issue (repo already cached too!)
                let issue = self.get_or_create_issue().await?;

                // Then create comment on issue
                // Note: We build the comment but DON'T send it here - the caller will send it
                self.client.create_comment(&issue, "Test comment", vec![])
            }

            FixtureKind::RepoState => {
                use nostr_sdk::prelude::*;

                // Reuse ValidRepo fixture - this leverages caching in Shared mode
                let repo = self.get_or_create_repo().await?;

                // Extract repo_id from repo announcement
                let repo_id = repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing d tag in repo announcement"))?
                    .to_string();

                // Create state announcement with deterministic commit hash
                let base_time = Timestamp::now().as_u64();
                let older_timestamp = Timestamp::from(base_time - 10); // 10 seconds ago

                // Tag format: ["refs/heads/main", "<commit_hash>"]
                // Note: We build the state but DON'T send it here - the caller will send it
                self.client
                    .event_builder(Kind::Custom(30618), "")
                    .tag(Tag::identifier(&repo_id))
                    .tag(Tag::custom(
                        TagKind::custom("refs/heads/main"),
                        vec![DETERMINISTIC_COMMIT_HASH.to_string()],
                    ))
                    .tag(Tag::custom(
                        TagKind::custom("HEAD"),
                        vec!["ref: refs/heads/main".to_string()],
                    ))
                    .custom_time(older_timestamp)
                    .build(self.client.keys())
                    .map_err(|e| anyhow::anyhow!("Failed to build state announcement: {}", e))
            }

            FixtureKind::MaintainerAnnouncement => {
                use nostr_sdk::prelude::*;

                // Get the owner's repo to use the SAME repo_id
                let owner_repo = self.get_or_create_repo().await?;
                
                // Extract repo_id from owner's repo announcement
                let repo_id = owner_repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing d tag in owner repo announcement"))?
                    .to_string();

                self.build_maintainer_announcement(&repo_id).await
            }

            FixtureKind::MaintainerState => {
                use nostr_sdk::prelude::*;

                // Get the owner's repo to use the SAME repo_id
                let owner_repo = self.get_or_create_repo().await?;
                
                // Extract repo_id from owner's repo announcement
                let repo_id = owner_repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing d tag in owner repo announcement"))?
                    .to_string();

                // Build state event ONLY - does NOT send announcement
                // This allows testing state-only scenarios
                self.build_maintainer_state(&repo_id)
            }

            FixtureKind::RecursiveMaintainerAnnouncement => {
                use nostr_sdk::prelude::*;

                // Get the owner's repo to use the SAME repo_id
                let owner_repo = self.get_or_create_repo().await?;
                
                // Extract repo_id from owner's repo announcement
                let repo_id = owner_repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing d tag in owner repo announcement"))?
                    .to_string();

                self.build_recursive_maintainer_announcement(&repo_id).await
            }

            FixtureKind::RecursiveMaintainerState => {
                use nostr_sdk::prelude::*;

                // Get the owner's repo to use the SAME repo_id
                let owner_repo = self.get_or_create_repo().await?;
                
                // Extract repo_id from owner's repo announcement
                let repo_id = owner_repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing d tag in owner repo announcement"))?
                    .to_string();

                // Build state event ONLY - does NOT send announcement
                self.build_recursive_maintainer_state(&repo_id)
            }

            FixtureKind::RecursiveMaintainerRepoAndState => {
                use nostr_sdk::prelude::*;

                // Get the owner's repo to use the SAME repo_id
                let owner_repo = self.get_or_create_repo().await?;
                
                // Extract repo_id from owner's repo announcement
                let repo_id = owner_repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing d tag in owner repo announcement"))?
                    .to_string();

                // Build and send the recursive maintainer's repo announcement
                let recursive_maintainer_announcement = self.build_recursive_maintainer_announcement(&repo_id).await?;
                self.client.send_event(recursive_maintainer_announcement).await?;

                // Return the state event (caller will send it)
                self.build_recursive_maintainer_state(&repo_id)
            }
        }
    }

    /// Build maintainer announcement event for the given repo_id
    async fn build_maintainer_announcement(&self, repo_id: &str) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // Get relay URL for clone tag
        let relay_url = self.client
            .client()
            .relays()
            .await
            .keys()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No relay connected"))?
            .to_string();
        let http_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");

        // Create maintainer's repo announcement for the SAME repo_id
        let maintainer_npub = self.client
            .maintainer_keys()
            .public_key()
            .to_bech32()
            .map_err(|e| anyhow::anyhow!("Failed to convert maintainer pubkey: {}", e))?;

        self.client
            .event_builder(
                Kind::GitRepoAnnouncement,
                format!("Maintainer announcement for {}", repo_id),
            )
            .tag(Tag::identifier(repo_id))
            .tag(Tag::custom(
                TagKind::custom("name"),
                vec![format!("{} (maintainer)", repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("clone"),
                vec![format!("{}/{}/{}.git", http_url, maintainer_npub, repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("relays"),
                vec![relay_url],
            ))
            .tag(Tag::custom(
                TagKind::custom("maintainers"),
                vec![self.client.recursive_maintainer_pubkey_hex()],
            ))
            .build(self.client.maintainer_keys())
            .map_err(|e| anyhow::anyhow!("Failed to build maintainer repo announcement: {}", e))
    }

    /// Build maintainer state event for the given repo_id
    fn build_maintainer_state(&self, repo_id: &str) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // Create state announcement 5 seconds in the past, signed by maintainer
        let base_time = Timestamp::now().as_u64();
        let older_timestamp = Timestamp::from(base_time - 5); // 5 seconds ago

        self.client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![MAINTAINER_DETERMINISTIC_COMMIT_HASH.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("HEAD"),
                vec!["ref: refs/heads/main".to_string()],
            ))
            .custom_time(older_timestamp)
            .build(self.client.maintainer_keys())
            .map_err(|e| anyhow::anyhow!("Failed to build maintainer state announcement: {}", e))
    }

    /// Build recursive maintainer announcement event for the given repo_id
    async fn build_recursive_maintainer_announcement(&self, repo_id: &str) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // Get relay URL for clone tag
        let relay_url = self.client
            .client()
            .relays()
            .await
            .keys()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No relay connected"))?
            .to_string();
        let http_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");

        // Create recursive maintainer's repo announcement for the SAME repo_id
        let recursive_maintainer_npub = self.client
            .recursive_maintainer_keys()
            .public_key()
            .to_bech32()
            .map_err(|e| anyhow::anyhow!("Failed to convert recursive maintainer pubkey: {}", e))?;

        self.client
            .event_builder(
                Kind::GitRepoAnnouncement,
                format!("Recursive maintainer announcement for {}", repo_id),
            )
            .tag(Tag::identifier(repo_id))
            .tag(Tag::custom(
                TagKind::custom("name"),
                vec![format!("{} (recursive maintainer)", repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("clone"),
                vec![format!("{}/{}/{}.git", http_url, recursive_maintainer_npub, repo_id)],
            ))
            .tag(Tag::custom(
                TagKind::custom("relays"),
                vec![relay_url],
            ))
            .tag(Tag::custom(
                TagKind::custom("maintainers"),
                vec![
                    self.client.public_key().to_hex(),
                    self.client.maintainer_pubkey_hex(),
                ],
            ))
            .build(self.client.recursive_maintainer_keys())
            .map_err(|e| anyhow::anyhow!("Failed to build recursive maintainer repo announcement: {}", e))
    }

    /// Build recursive maintainer state event for the given repo_id
    fn build_recursive_maintainer_state(&self, repo_id: &str) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // Create state announcement 2 seconds in the past, signed by recursive maintainer
        let base_time = Timestamp::now().as_u64();
        let older_timestamp = Timestamp::from(base_time - 2); // 2 seconds ago

        self.client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("HEAD"),
                vec!["ref: refs/heads/main".to_string()],
            ))
            .custom_time(older_timestamp)
            .build(self.client.recursive_maintainer_keys())
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to build recursive maintainer state announcement: {}",
                    e
                )
            })
    }

    /// Clear the fixture cache
    ///
    /// This is useful for tests that want to ensure fresh fixtures
    /// even in shared mode.
    pub fn clear_cache(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuditConfig;

    #[test]
    fn test_context_mode_from_audit_mode() {
        assert_eq!(ContextMode::from(AuditMode::CI), ContextMode::Isolated);
        assert_eq!(
            ContextMode::from(AuditMode::Production),
            ContextMode::Shared
        );
    }

    #[test]
    fn test_fixture_kind_hash() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(FixtureKind::ValidRepo);
        set.insert(FixtureKind::RepoWithIssue);

        assert!(set.contains(&FixtureKind::ValidRepo));
        assert!(!set.contains(&FixtureKind::RepoWithComment));
    }

    #[tokio::test]
    async fn test_context_creation() {
        let config = AuditConfig::ci();
        let client = crate::AuditClient::new_test(config);

        let ctx = TestContext::new(&client);
        assert_eq!(ctx.mode(), ContextMode::Isolated);

        let ctx = TestContext::with_mode(&client, ContextMode::Shared);
        assert_eq!(ctx.mode(), ContextMode::Shared);
    }
}
