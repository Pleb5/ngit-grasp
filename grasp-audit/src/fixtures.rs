//! Test fixture management for dual-mode testing
//!
//! This module provides a TestContext abstraction that manages prerequisite events
//! differently based on the audit mode:
//!
//! - **CI Mode (Isolated)**: Creates fresh events for each test, ensuring complete isolation
//! - **Production Mode (Shared)**: Reuses shared fixtures to minimize event publication
//!
//! # Cache Sharing Strategy
//!
//! The fixture cache lives on the `AuditClient`, not on `TestContext`. This provides
//! natural cache sharing semantics:
//!
//! - **CLI mode**: Creates one `AuditClient` → fixtures shared across all tests
//! - **cargo test**: Creates one `AuditClient` per test → fixtures isolated per test
//!
//! This eliminates the need for global state while still enabling fixture reuse
//! when appropriate.
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
/// # Cache Location
///
/// The fixture cache lives on `AuditClient`, not on `TestContext`. This means:
/// - Multiple `TestContext` instances from the same client share the cache
/// - CLI mode (one client) naturally shares fixtures across all tests
/// - Test mode (one client per test) naturally isolates fixtures
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
}

impl<'a> TestContext<'a> {
    /// Create a new test context
    ///
    /// The context mode is automatically determined from the client's audit config.
    /// The fixture cache is borrowed from the client, enabling natural sharing:
    /// - Same client = shared cache (CLI mode behavior)
    /// - Different clients = isolated caches (test mode behavior)
    pub fn new(client: &'a AuditClient) -> Self {
        let mode = ContextMode::from(client.config.mode);
        Self { client, mode }
    }

    /// Create a test context with explicit mode override
    ///
    /// This is useful for testing the context itself or for advanced use cases
    /// where you want to override the default mode behavior.
    pub fn with_mode(client: &'a AuditClient, mode: ContextMode) -> Self {
        Self { client, mode }
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
    ///
    /// Uses the client's fixture cache to ensure fixtures are reused across
    /// all TestContext instances in Production mode.
    async fn get_or_create_shared(&self, kind: FixtureKind) -> Result<Event> {
        // Check client's cache first (shared across all TestContext instances using same client)
        {
            let cache = self.client.fixture_cache().lock().unwrap();
            if let Some(event) = cache.get(&kind) {
                tracing::debug!("get_or_create_shared({:?}) found in client cache", kind);
                return Ok(event.clone());
            }
        }

        // Check relay connection before attempting to build
        let is_connected = self.client.is_connected().await;
        if !is_connected {
            return Err(anyhow::anyhow!(
                "Relay connection lost before building {:?} fixture (shared cache mode)",
                kind
            ));
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

        // Store in client's cache (shared across all TestContext instances using same client)
        {
            let mut cache = self.client.fixture_cache().lock().unwrap();
            cache.insert(kind, event.clone());
            tracing::debug!("get_or_create_shared({:?}) stored in client cache ({} entries)", kind, cache.len());
        }

        Ok(event)
    }

    /// Get or create a ValidRepo, with caching.
    /// This is a helper method that avoids async recursion by not going
    /// through get_fixture. It handles the repo specifically.
    ///
    /// Uses client's fixture_cache for caching - in Shared mode this enables
    /// cross-test reuse when the same client is used.
    async fn get_or_create_repo(&self) -> Result<Event> {
        // Check client's cache first (shared across all TestContext instances using same client)
        {
            let cache = self.client.fixture_cache().lock().unwrap();
            if let Some(event) = cache.get(&FixtureKind::ValidRepo) {
                tracing::debug!("get_or_create_repo() found in client cache");
                return Ok(event.clone());
            }
        }

        // Check relay connection before creating repo
        let is_connected = self.client.is_connected().await;
        if !is_connected {
            return Err(anyhow::anyhow!(
                "Relay connection lost before creating ValidRepo fixture"
            ));
        }

        // Create a new repo
        let test_name = format!(
            "fixture-{:?}-{}",
            FixtureKind::ValidRepo,
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        
        let repo = self.client.create_repo_announcement(&test_name).await
            .with_context(|| format!("create_repo_announcement failed for {}", test_name))?;

        // Send it
        self.client.send_event(repo.clone()).await
            .with_context(|| "Failed to send repo announcement to relay")?;

        // Store in client's cache (shared across all TestContext instances using same client)
        {
            let mut cache = self.client.fixture_cache().lock().unwrap();
            cache.insert(FixtureKind::ValidRepo, repo.clone());
            tracing::debug!("get_or_create_repo() stored in client cache ({} entries)", cache.len());
        }

        Ok(repo)
    }

    /// Get or create a RepoWithIssue, with caching via the client.
    /// Returns the issue event (repo is already sent/cached via get_or_create_repo).
    async fn get_or_create_issue(&self) -> Result<Event> {
        // Check client's cache first
        {
            let cache = self.client.fixture_cache().lock().unwrap();
            if let Some(event) = cache.get(&FixtureKind::RepoWithIssue) {
                return Ok(event.clone());
            }
        }

        // Get or create repo (reuses cached via client)
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

        // Store in client's cache
        {
            let mut cache = self.client.fixture_cache().lock().unwrap();
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

                // Build and send the maintainer's repo announcement first
                // This establishes the chain: Owner -> Maintainer -> RecursiveMaintainer
                // The maintainer's announcement lists the recursive maintainer in its maintainers tag
                let maintainer_announcement = self.build_maintainer_announcement(&repo_id).await?;
                self.client.send_event(maintainer_announcement).await?;

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
    /// This clears the client's fixture cache, affecting all TestContext
    /// instances using the same client.
    pub fn clear_cache(&self) {
        let mut cache = self.client.fixture_cache().lock().unwrap();
        cache.clear();
    }
}

// ============================================================
// Verification Helpers
// ============================================================

/// Send event and verify it was accepted (stored by relay)
///
/// This is a common test pattern helper that:
/// 1. Sends an event to the relay via the client
/// 2. Waits for propagation (100ms)
/// 3. Queries the relay to verify the event was stored
///
/// # Arguments
/// * `client` - The AuditClient to use for sending and querying
/// * `event` - The event to send
/// * `description` - Human-readable description for error messages
///
/// # Returns
/// * `Ok(())` if the event was accepted and stored
/// * `Err(String)` with descriptive error if event was not stored
///
/// # Example
/// ```no_run
/// # use grasp_audit::*;
/// # async fn example(client: &AuditClient, event: nostr_sdk::Event) -> Result<(), String> {
/// send_and_verify_accepted(client, event, "issue referencing repo via 'a' tag").await?;
/// # Ok(())
/// # }
/// ```
pub async fn send_and_verify_accepted(
    client: &crate::AuditClient,
    event: Event,
    description: &str,
) -> Result<(), String> {
    use nostr_sdk::prelude::Filter;
    use std::time::Duration;
    
    let event_id = event.id;

    client
        .send_event(event)
        .await
        .map_err(|e| format!("Failed to send event to relay: {}", e))?;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let filter = Filter::new().id(event_id);
    let events = client
        .query(filter)
        .await
        .map_err(|e| format!("Failed to query relay for verification: {}", e))?;

    if events.is_empty() {
        return Err(format!("Event should be accepted: {}", description));
    }

    Ok(())
}

/// Send event and verify it was rejected (NOT stored by relay)
///
/// This is a common test pattern helper that:
/// 1. Sends an event to the relay via the client
/// 2. Handles both explicit rejection errors and silent rejection
/// 3. Verifies the event was NOT stored in the relay
///
/// # Arguments
/// * `client` - The AuditClient to use for sending and querying
/// * `event` - The event to send (expected to be rejected)
/// * `description` - Human-readable description for error messages
///
/// # Returns
/// * `Ok(())` if the event was rejected (not stored)
/// * `Err(String)` if the event was unexpectedly accepted
///
/// # Example
/// ```no_run
/// # use grasp_audit::*;
/// # async fn example(client: &AuditClient, event: nostr_sdk::Event) -> Result<(), String> {
/// send_and_verify_rejected(client, event, "orphan issue with no repo connection").await?;
/// # Ok(())
/// # }
/// ```
pub async fn send_and_verify_rejected(
    client: &crate::AuditClient,
    event: Event,
    description: &str,
) -> Result<(), String> {
    use nostr_sdk::prelude::Filter;
    use std::time::Duration;
    
    let event_id = event.id;

    // Try to send event - rejection may cause send_event to fail with an error
    let send_result = client.send_event(event).await;
    
    // If send succeeded, the relay might have accepted it (we'll verify below)
    // If send failed, check if it's a rejection error (expected)
    if let Err(e) = send_result {
        let err_msg = e.to_string().to_lowercase();
        // Check if error message indicates rejection (not network/other errors)
        if err_msg.contains("rejected") || err_msg.contains("blocked") {
            // Expected rejection - verify event is NOT in database
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            let filter = Filter::new().id(event_id);
            let events = client
                .query(filter)
                .await
                .map_err(|e| format!("Failed to query relay for verification: {}", e))?;

            if !events.is_empty() {
                return Err(format!("Event was rejected but still stored: {}", description));
            }
            
            return Ok(()); // Rejected as expected
        } else {
            // Unexpected error (network, etc.)
            return Err(format!("Failed to send event to relay: {}", e));
        }
    }

    // Send succeeded, verify event was NOT stored (relay should have rejected)
    tokio::time::sleep(Duration::from_millis(100)).await;

    let filter = Filter::new().id(event_id);
    let events = client
        .query(filter)
        .await
        .map_err(|e| format!("Failed to query relay for verification: {}", e))?;

    if !events.is_empty() {
        return Err(format!("Event should be rejected: {}", description));
    }

    Ok(())
}

// ============================================================
// Git Operation Helpers
// ============================================================

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Clone a repository from the relay and return the path
///
/// # Arguments
/// * `relay_domain` - The domain of the relay (e.g., "localhost:7000")
/// * `npub` - The bech32 public key of the repository owner
/// * `repo_id` - The repository identifier (d-tag value)
///
/// # Returns
/// * `Ok(PathBuf)` - Path to the cloned repository
/// * `Err(String)` - Error message if clone failed
///
/// # Example
/// ```no_run
/// # use grasp_audit::*;
/// # fn example() -> Result<(), String> {
/// let clone_path = clone_repo("localhost:7000", "npub1...", "my-repo")?;
/// // Use the cloned repo...
/// std::fs::remove_dir_all(&clone_path).ok(); // Cleanup
/// # Ok(())
/// # }
/// ```
pub fn clone_repo(
    relay_domain: &str,
    npub: &str,
    repo_id: &str,
) -> Result<PathBuf, String> {
    let temp_base = std::env::temp_dir();
    let clone_dir_name = format!("grasp-push-test-{}", uuid::Uuid::new_v4());
    let clone_path = temp_base.join(&clone_dir_name);
    let _ = fs::remove_dir_all(&clone_path);

    let clone_url = format!("http://{}/{}/{}.git", relay_domain, npub, repo_id);
    let output = Command::new("git")
        .args(["clone", &clone_url, clone_path.to_str().unwrap()])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Failed to execute git clone: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git clone failed: {}", stderr));
    }

    // Configure git user
    let _ = Command::new("git")
        .args(["config", "user.email", "test@grasp-audit.local"])
        .current_dir(&clone_path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "GRASP Audit Test"])
        .current_dir(&clone_path)
        .output();

    Ok(clone_path)
}

/// Create a commit with a unique file and return the commit hash
///
/// # Arguments
/// * `clone_path` - Path to the git repository
/// * `message` - Commit message
///
/// # Returns
/// * `Ok(String)` - The commit hash
/// * `Err(String)` - Error message if commit failed
///
/// # Example
/// ```no_run
/// # use grasp_audit::*;
/// # use std::path::Path;
/// # fn example() -> Result<(), String> {
/// let commit_hash = create_commit(Path::new("/tmp/my-repo"), "My commit message")?;
/// println!("Created commit: {}", commit_hash);
/// # Ok(())
/// # }
/// ```
pub fn create_commit(clone_path: &Path, message: &str) -> Result<String, String> {
    let test_file = clone_path.join(format!("test-{}.txt", uuid::Uuid::new_v4()));
    fs::write(&test_file, message).map_err(|e| format!("Failed to write file: {}", e))?;

    let filename = test_file.file_name().unwrap().to_str().unwrap();
    let output = Command::new("git")
        .args(["add", filename])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git add failed: {}", e))?;

    if !output.status.success() {
        return Err("Git add failed".to_string());
    }

    let output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git commit failed: {}", e))?;

    if !output.status.success() {
        return Err("Git commit failed".to_string());
    }

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git rev-parse failed: {}", e))?;

    if !output.status.success() {
        return Err("Failed to get commit hash".to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Variant of deterministic commit for different pubkey types
/// Each variant produces a different but reproducible commit hash
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitVariant {
    /// Main pubkey variant - uses "Initial commit" content
    Owner,
    /// Maintainer pubkey variant - uses "Maintainer initial commit" content
    Maintainer,
    /// Recursive maintainer pubkey variant - uses "Recursive maintainer initial commit" content
    RecursiveMaintainer,
}

impl CommitVariant {
    /// Get the file content for this variant
    pub fn file_content(&self) -> &'static str {
        match self {
            CommitVariant::Owner => "Initial commit",
            CommitVariant::Maintainer => "Maintainer initial commit",
            CommitVariant::RecursiveMaintainer => "Recursive maintainer initial commit",
        }
    }
    
    /// Get the commit message for this variant
    pub fn commit_message(&self) -> &'static str {
        match self {
            CommitVariant::Owner => "Initial commit",
            CommitVariant::Maintainer => "Maintainer initial commit",
            CommitVariant::RecursiveMaintainer => "Recursive maintainer initial commit",
        }
    }
}

/// Create a deterministic commit with fixed dates and GPG disabled
///
/// The variant parameter allows different commit hashes for different pubkey types:
/// - Owner: uses DETERMINISTIC_COMMIT_HASH
/// - Maintainer: uses MAINTAINER_DETERMINISTIC_COMMIT_HASH
/// - RecursiveMaintainer: uses RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
///
/// # Arguments
/// * `clone_path` - Path to the git repository
/// * `variant` - The commit variant to create
///
/// # Returns
/// * `Ok(String)` - The deterministic commit hash
/// * `Err(String)` - Error message if commit failed
pub fn create_deterministic_commit_with_variant(clone_path: &Path, variant: CommitVariant) -> Result<String, String> {
    let test_file = clone_path.join("test.txt");
    let content = variant.file_content();
    let message = variant.commit_message();
    
    fs::write(&test_file, content).map_err(|e| format!("Failed to write file: {}", e))?;

    let output = Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git add failed: {}", e))?;

    if !output.status.success() {
        return Err("Git add failed".to_string());
    }

    // Create deterministic commit with fixed dates and GPG disabled
    let output = Command::new("git")
        .args([
            "-c", "commit.gpgsign=false",
            "commit",
            "-m", message,
        ])
        .env("GIT_AUTHOR_DATE", "2024-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2024-01-01T00:00:00Z")
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git commit failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git commit failed: {}", stderr));
    }

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(clone_path)
        .output()
        .map_err(|e| format!("Git rev-parse failed: {}", e))?;

    if !output.status.success() {
        return Err("Failed to get commit hash".to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Create a deterministic commit (Owner variant)
///
/// This is a convenience wrapper around `create_deterministic_commit_with_variant`
/// that uses the Owner variant for backwards compatibility.
///
/// # Arguments
/// * `clone_path` - Path to the git repository
/// * `_message` - Ignored for compatibility (Owner variant always uses "Initial commit")
///
/// # Returns
/// * `Ok(String)` - The deterministic commit hash
/// * `Err(String)` - Error message if commit failed
pub fn create_deterministic_commit(clone_path: &Path, _message: &str) -> Result<String, String> {
    // Note: message parameter is ignored for backwards compatibility
    // The Owner variant always uses "Initial commit"
    create_deterministic_commit_with_variant(clone_path, CommitVariant::Owner)
}

/// Attempt a git push and return success/failure
///
/// # Arguments
/// * `clone_path` - Path to the git repository
///
/// # Returns
/// * `Ok(true)` - Push succeeded
/// * `Ok(false)` - Push was rejected
/// * `Err(String)` - Error executing git push
///
/// # Example
/// ```no_run
/// # use grasp_audit::*;
/// # use std::path::Path;
/// # fn example() -> Result<(), String> {
/// let success = try_push(Path::new("/tmp/my-repo"))?;
/// if success {
///     println!("Push succeeded");
/// } else {
///     println!("Push was rejected");
/// }
/// # Ok(())
/// # }
/// ```
pub fn try_push(clone_path: &Path) -> Result<bool, String> {
    let output = Command::new("git")
        .args(["push", "origin", "main"])
        .current_dir(clone_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Failed to execute git push: {}", e))?;

    Ok(output.status.success())
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
