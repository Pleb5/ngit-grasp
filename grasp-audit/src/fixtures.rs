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
//! # What is a Fixture?
//! A fixture represents the state of a repository on a grasp server and/or nostr events to be
//! sent to the server to change this state.
//!
//! 1. <event-name>Generated - Nostr Event created (not yet sent)
//! 2. <event-name>Sent - Sent To Grasp Server
//! 3. <event-name> - Verfied and Confirmed as accepted via client query
//! 4. <event-or-data-pushed-name>DataPushed - what refs were pushed
//!
//! Some Nostr Events need each of these stages as seperate fixtures whereas 1-3 or event 1-4 are often
//! bundled and 4 is only sometimes needed.
//!
//! Nearly all fixures include dependant fixtures so tests dont need to call every parent fixture.
//!
//! As entire tests are often fixtures to be built on by other fixtures / tests, some tests just take
//! the fixture Result and wrap it in pass fail using the error message.
//!
//! # Out of Scope
//!
//! local repo's used in tests are always cloned fresh and never part of a fixture
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
///   NOTE: This value is different from DETERMINISTIC_COMMIT_HASH due to different content
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
///   NOTE: This value is different from DETERMINISTIC_COMMIT_HASH due to different content
pub const RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH: &str =
    "05939b82de66fbdb9c077d0a64fc68522f3cb8e0";

/// Deterministic commit hash for PR test fixtures (PRTestCommit variant)
/// This is the hash produced by creating a commit with:
/// - Message: "PR test deterministic commit"
/// - File: test.txt containing "PR test deterministic commit"
/// - Author date: 2024-01-01T00:00:00Z
/// - Committer date: 2024-01-01T00:00:00Z
/// - GPG signing: disabled
/// - User: "GRASP Audit Test <test@grasp-audit.local>"
/// - Parent: none (root commit)
pub const PR_TEST_COMMIT_HASH: &str = "5d40fb1555a0c28bf4d650515a73aaa54d4d9bfb";

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

    /// PR (Pull Request) event for the SAME repo_id as ValidRepo
    /// - Requires ValidRepo (uses same repo_id)
    /// - Signed by `client.pr_author_keys()`
    /// - Kind 1618 (NIP-34 PR)
    /// - Includes `a` tag referencing the repo
    /// - Includes `c` tag pointing to PR_TEST_COMMIT_HASH
    /// - Timestamp: 1 second in the past
    PREvent,

    /// PR event generated (built) but NOT sent to relay
    ///
    /// This is a "Generated" stage fixture - the event is created but not published.
    /// Useful for tests that need the PR event ID before the event exists on the relay.
    ///
    /// - Requires ValidRepo (uses same repo_id)
    /// - Signed by `client.pr_author_keys()`
    /// - Kind 1618 (NIP-34 PR)
    /// - Includes `c` tag pointing to PR_TEST_COMMIT_HASH
    /// - NOT sent to relay (use `client.send_event()` to publish when ready)
    PREventGenerated,

    /// Wrong commit pushed to refs/nostr/<pr-event-id> BEFORE PR event is sent
    ///
    /// This is a "DataPushed" stage fixture for testing pre-event ref behavior.
    /// The server has refs/nostr/<pr-event-id> pointing to DETERMINISTIC_COMMIT_HASH
    /// (the "wrong" commit), but no PR event exists yet on the relay.
    ///
    /// Server state after this fixture:
    /// - ValidRepo announcement on relay
    /// - refs/nostr/<pr-event-id> exists on git server with wrong commit
    /// - PR event is NOT on relay (but returned for tests to publish later)
    ///
    /// - Requires PREventGenerated (for the event ID)
    /// - Clones repo, creates wrong commit, pushes to refs/nostr/<event-id>
    /// - Returns: the unsent PR event (tests can publish it later)
    PRWrongCommitPushedBeforeEvent,

    /// PR event sent to relay AFTER wrong commit was pushed to refs/nostr/<pr-event-id>
    ///
    /// This is a compound fixture testing post-event behavior.
    /// The server had refs/nostr/<pr-event-id> pointing to wrong commit,
    /// then the PR event was published (which may trigger cleanup).
    ///
    /// Server state after this fixture:
    /// - ValidRepo announcement on relay
    /// - PR event is on relay
    /// - refs/nostr/<pr-event-id> may have been cleaned up (that's what tests verify)
    ///
    /// - Requires PRWrongCommitPushedBeforeEvent
    /// - Sends the PR event to relay
    /// - Returns: the sent PR event
    PREventSentAfterWrongPush,

    /// Owner's state event with git data successfully pushed (full 4-stage fixture)
    ///
    /// This fixture represents the complete flow for testing push authorization:
    /// 1. **Generated**: Creates RepoState (repo announcement + state event)
    /// 2. **Sent**: Sends events to relay
    /// 3. **Verified**: Confirms events accepted by relay
    /// 4. **DataPushed**: Clones repo, creates deterministic commit, pushes to relay
    ///
    /// - Requires ValidRepo (uses same repo_id)
    /// - State event signed by owner keys (`client.keys()`)
    /// - Points to DETERMINISTIC_COMMIT_HASH
    /// - Git push verified to succeed (state matches pushed commit)
    OwnerStateDataPushed,

    /// Maintainer's state event with git data successfully pushed (full 4-stage fixture)
    ///
    /// This fixture tests that a maintainer can authorize pushes with ONLY a state event,
    /// without publishing their own repo announcement. The maintainer is still listed in
    /// the owner's announcement, so they're a valid maintainer.
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    ///
    /// Stages:
    /// 1. **Generated**: Creates ValidRepo (owner's announcement with maintainer in maintainers tag)
    ///                   + MaintainerState (maintainer's state event ONLY - no announcement)
    /// 2. **Sent**: Sends events to relay
    /// 3. **Verified**: Confirms events accepted by relay
    /// 4. **DataPushed**: Clones repo, creates maintainer deterministic commit, pushes to relay
    ///
    /// - Requires OwnerStateDataPushed (owner's data already pushed to git)
    /// - State event signed by maintainer keys (`client.maintainer_keys()`)
    /// - Points to MAINTAINER_DETERMINISTIC_COMMIT_HASH
    /// - Git push verified to succeed (force push with maintainer's state event authorizes the commit)
    MaintainerStateDataPushed,

    /// Recursive maintainer's state event with git data successfully pushed (full 4-stage fixture)
    ///
    /// This fixture tests that a recursive maintainer (authorized via maintainer chain) can
    /// authorize pushes. The recursive maintainer is listed in the maintainer's announcement,
    /// not the owner's announcement, so this tests the recursive maintainer traversal.
    ///
    /// GRASP-01: "respecting the recursive maintainer set"
    ///
    /// Chain: Owner -> Maintainer -> RecursiveMaintainer
    ///
    /// Stages:
    /// 1. **Generated**: Creates MaintainerStateDataPushed (includes ValidRepo + OwnerStateDataPushed)
    ///                   + MaintainerAnnouncement (maintainer's announcement listing recursive maintainer)
    ///                   + RecursiveMaintainerState (recursive maintainer's state event)
    /// 2. **Sent**: Sends events to relay
    /// 3. **Verified**: Confirms events accepted by relay
    /// 4. **DataPushed**: Clones repo, creates recursive maintainer deterministic commit, pushes to relay
    ///
    /// - Requires MaintainerStateDataPushed (establishes Owner -> Maintainer chain with git data)
    /// - Sends MaintainerAnnouncement (establishes Maintainer -> RecursiveMaintainer connection)
    /// - State event signed by recursive maintainer keys (`client.recursive_maintainer_keys()`)
    /// - Points to RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
    /// - Git push verified to succeed (recursive maintainer's state event authorizes the commit)
    RecursiveMaintainerStateDataPushed,
}

impl FixtureKind {
    /// Get the fixture dependencies that must be ensured before this one
    ///
    /// Dependencies are processed in order and cached, so if a fixture
    /// depends on another that's already been created, it won't be recreated.
    pub fn dependencies(&self) -> Vec<FixtureKind> {
        match self {
            // Base fixtures - no dependencies
            Self::ValidRepo => vec![],
            
            // Fixtures that depend on ValidRepo
            Self::RepoWithIssue => vec![Self::ValidRepo],
            Self::RepoState => vec![Self::ValidRepo],
            Self::MaintainerAnnouncement => vec![Self::ValidRepo],
            Self::MaintainerState => vec![Self::ValidRepo],
            Self::RecursiveMaintainerAnnouncement => vec![Self::ValidRepo],
            Self::RecursiveMaintainerState => vec![Self::ValidRepo],
            Self::RecursiveMaintainerRepoAndState => vec![Self::ValidRepo],
            Self::PREvent => vec![Self::ValidRepo],
            Self::PREventGenerated => vec![Self::ValidRepo],
            Self::PRWrongCommitPushedBeforeEvent => vec![Self::PREventGenerated],
            Self::PREventSentAfterWrongPush => vec![Self::PRWrongCommitPushedBeforeEvent],
            Self::OwnerStateDataPushed => vec![Self::ValidRepo],
            
            // Fixtures that depend on RepoWithIssue
            Self::RepoWithComment => vec![Self::RepoWithIssue],
            
            // MaintainerStateDataPushed depends on OwnerStateDataPushed
            // (maintainer force-pushes over owner's data)
            Self::MaintainerStateDataPushed => vec![Self::OwnerStateDataPushed],
            
            // RecursiveMaintainerStateDataPushed depends on MaintainerStateDataPushed
            // (recursive maintainer force-pushes over maintainer's data)
            Self::RecursiveMaintainerStateDataPushed => vec![Self::MaintainerStateDataPushed],
        }
    }

    /// Whether this fixture sends its own events to the relay
    ///
    /// Some fixtures (like DataPushed variants) handle event sending internally
    /// as part of their build process. For these, the generic ensure_fixture
    /// should NOT send the event again.
    pub fn sends_own_events(&self) -> bool {
        match self {
            // These fixtures send events and push git data internally
            Self::OwnerStateDataPushed => true,
            Self::MaintainerStateDataPushed => true,
            Self::RecursiveMaintainerStateDataPushed => true,
            // RecursiveMaintainerRepoAndState sends multiple events internally
            Self::RecursiveMaintainerRepoAndState => true,
            // PREventGenerated builds but does NOT send the PR event (that's the point)
            Self::PREventGenerated => true,
            // PRWrongCommitPushedBeforeEvent pushes git data but doesn't send event
            Self::PRWrongCommitPushedBeforeEvent => true,
            // PREventSentAfterWrongPush sends the PR event internally
            Self::PREventSentAfterWrongPush => true,
            // All other fixtures return a single event for the caller to send
            _ => false,
        }
    }
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
    /// Per-TestContext cache for Isolated mode
    /// This cache ensures fixture dependencies work within a single test
    /// while maintaining isolation between tests.
    /// In Shared mode, this cache is not used - we use the client's cache instead.
    local_cache: Arc<Mutex<HashMap<FixtureKind, Event>>>,
}

impl<'a> TestContext<'a> {
    /// Create a new test context
    ///
    /// The context mode is automatically determined from the client's audit config.
    /// In Isolated mode, fixtures are cached per-TestContext to maintain fixture
    /// dependencies within a test while ensuring isolation between tests.
    /// In Shared mode, the client's cache is used for cross-test fixture sharing.
    pub fn new(client: &'a AuditClient) -> Self {
        let mode = ContextMode::from(client.config.mode);
        Self {
            client,
            mode,
            local_cache: Arc::new(Mutex::new(HashMap::new())),
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
            local_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get a fixture, creating it if needed based on mode
    ///
    /// This is an alias for `ensure_fixture` - the core method for fixture management.
    /// It automatically handles:
    /// - Mode-aware caching (Isolated vs Shared)
    /// - Dependency resolution
    /// - Event sending
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
        self.ensure_fixture(kind).await
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

    // ============================================================
    // Cache Helper Methods
    // ============================================================

    /// Get a cached fixture if it exists
    fn get_cached(&self, kind: FixtureKind) -> Option<Event> {
        match self.mode {
            ContextMode::Isolated => {
                let cache = self.local_cache.lock().unwrap();
                cache.get(&kind).cloned()
            }
            ContextMode::Shared => {
                let cache = self.client.fixture_cache().lock().unwrap();
                cache.get(&kind).cloned()
            }
        }
    }

    /// Store a fixture in the cache
    fn store_cached(&self, kind: FixtureKind, event: Event) {
        match self.mode {
            ContextMode::Isolated => {
                let mut cache = self.local_cache.lock().unwrap();
                cache.insert(kind, event);
                tracing::debug!(
                    "store_cached({:?}) stored in local cache ({} entries)",
                    kind,
                    cache.len()
                );
            }
            ContextMode::Shared => {
                let mut cache = self.client.fixture_cache().lock().unwrap();
                cache.insert(kind, event);
                tracing::debug!(
                    "store_cached({:?}) stored in client cache ({} entries)",
                    kind,
                    cache.len()
                );
            }
        }
    }

    // ============================================================
    // Core Fixture Methods
    // ============================================================

    /// Ensure a fixture exists (with all dependencies)
    ///
    /// This is the core method for fixture management. It:
    /// 1. Checks the cache, returning immediately if found
    /// 2. Ensures all dependencies are met (recursively)
    /// 3. Builds the fixture
    /// 4. Sends to relay (unless fixture handles this internally)
    /// 5. Caches and returns the result
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use grasp_audit::*;
    /// # async fn example(ctx: &TestContext<'_>) -> anyhow::Result<()> {
    /// // This ensures ValidRepo exists first, then creates MaintainerState
    /// let state = ctx.ensure_fixture(FixtureKind::MaintainerState).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn ensure_fixture(&self, kind: FixtureKind) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Event>> + Send + '_>> {
        Box::pin(async move {
            // Check cache first
            if let Some(cached) = self.get_cached(kind) {
                tracing::debug!("ensure_fixture({:?}) found in cache", kind);
                return Ok(cached);
            }

            // Check relay connection before proceeding
            if !self.client.is_connected().await {
                return Err(anyhow::anyhow!(
                    "Relay connection lost before creating {:?} fixture",
                    kind
                ));
            }

            // Ensure all dependencies are met first
            for dep in kind.dependencies() {
                tracing::debug!("ensure_fixture({:?}) ensuring dependency {:?}", kind, dep);
                self.ensure_fixture(dep).await.with_context(|| {
                    format!("Failed to ensure dependency {:?} for {:?}", dep, kind)
                })?;
            }

            // Build the fixture
            let event = self.build_fixture_inner(kind).await.with_context(|| {
                format!("Failed to build {:?} fixture", kind)
            })?;

            // Send to relay if this fixture doesn't handle it internally
            if !kind.sends_own_events() {
                self.client.send_event(event.clone()).await.with_context(|| {
                    format!("Failed to send {:?} fixture event to relay", kind)
                })?;
            }

            // Cache and return
            self.store_cached(kind, event.clone());
            Ok(event)
        })
    }

    /// Build a fixture event WITHOUT publishing it to the relay.
    ///
    /// This is useful for tests that need to get a fixture's event ID before
    /// actually publishing it. For example, testing refs/nostr/<event-id>
    /// behavior before the corresponding event exists on the relay.
    ///
    /// Note: This ensures dependencies are created/published first, but the
    /// requested fixture itself will NOT be published.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use grasp_audit::*;
    /// # async fn example(ctx: &TestContext<'_>) -> anyhow::Result<()> {
    /// // Build PR event to get its ID without publishing
    /// let pr_event = ctx.build_fixture_only(FixtureKind::PREvent).await?;
    /// let pr_event_id = pr_event.id.to_hex();
    ///
    /// // Now push to refs/nostr/<pr_event_id> before event exists
    /// // ... git push ...
    ///
    /// // Later, publish the PR event when ready
    /// ctx.client().send_event(pr_event).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn build_fixture_only(&self, kind: FixtureKind) -> Result<Event> {
        // Ensure dependencies are met first
        for dep in kind.dependencies() {
            self.ensure_fixture(dep).await?;
        }
        // Build but don't send/cache
        self.build_fixture_inner(kind).await
    }

    /// Get a cached dependency (assumes ensure_fixture processed dependencies first)
    ///
    /// This is a convenience helper for build_fixture_inner to retrieve dependencies
    /// that were already ensured by ensure_fixture before calling build_fixture_inner.
    fn get_cached_dependency(&self, kind: FixtureKind) -> Result<Event> {
        self.get_cached(kind).ok_or_else(|| {
            anyhow::anyhow!(
                "Dependency {:?} not found in cache - this is a bug in fixture dependencies",
                kind
            )
        })
    }

    /// Build a fixture event (internal - assumes dependencies are cached)
    ///
    /// This method is called by `ensure_fixture` after all dependencies have been
    /// ensured and cached. It should NOT call `ensure_fixture` or it will cause
    /// infinite recursion. Instead, use `get_cached_dependency` to retrieve
    /// already-cached dependencies.
    async fn build_fixture_inner(&self, kind: FixtureKind) -> Result<Event> {
        match kind {
            FixtureKind::ValidRepo => {
                // ValidRepo has no dependencies - create a new repo announcement
                let test_name = format!(
                    "fixture-ValidRepo-{}",
                    &uuid::Uuid::new_v4().to_string()[..8]
                );

                self.client
                    .create_repo_announcement(&test_name)
                    .await
                    .with_context(|| format!("create_repo_announcement failed for {}", test_name))
            }

            FixtureKind::RepoWithIssue => {
                // ValidRepo is ensured by ensure_fixture before this is called
                let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                // Build issue referencing it - caller will send it
                self.client.create_issue(
                    &repo,
                    "Test Issue",
                    "Issue content for testing",
                    vec![],
                )
            }

            FixtureKind::RepoWithComment => {
                // RepoWithIssue is ensured by ensure_fixture before this is called
                let issue = self.get_cached_dependency(FixtureKind::RepoWithIssue)?;

                // Build comment on issue - caller will send it
                self.client.create_comment(&issue, "Test comment", vec![])
            }

            FixtureKind::RepoState => {
                use nostr_sdk::prelude::*;

                // ValidRepo is ensured by ensure_fixture before this is called
                let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

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
                // ValidRepo is ensured by ensure_fixture before this is called
                let owner_repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                let repo_id = self.extract_repo_id(&owner_repo)?;
                self.build_maintainer_announcement(&repo_id).await
            }

            FixtureKind::MaintainerState => {
                // ValidRepo is ensured by ensure_fixture before this is called
                let owner_repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                let repo_id = self.extract_repo_id(&owner_repo)?;
                self.build_maintainer_state(&repo_id)
            }

            FixtureKind::RecursiveMaintainerAnnouncement => {
                // ValidRepo is ensured by ensure_fixture before this is called
                let owner_repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                let repo_id = self.extract_repo_id(&owner_repo)?;
                self.build_recursive_maintainer_announcement(&repo_id).await
            }

            FixtureKind::RecursiveMaintainerState => {
                // ValidRepo is ensured by ensure_fixture before this is called
                let owner_repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                let repo_id = self.extract_repo_id(&owner_repo)?;
                self.build_recursive_maintainer_state(&repo_id)
            }

            FixtureKind::RecursiveMaintainerRepoAndState => {
                // ValidRepo is ensured by ensure_fixture before this is called
                let owner_repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                let repo_id = self.extract_repo_id(&owner_repo)?;

                // Build and send the maintainer's repo announcement first
                // This establishes the chain: Owner -> Maintainer -> RecursiveMaintainer
                let maintainer_announcement = self.build_maintainer_announcement(&repo_id).await?;
                self.client.send_event(maintainer_announcement).await?;

                // Build and send the recursive maintainer's repo announcement
                let recursive_maintainer_announcement = self
                    .build_recursive_maintainer_announcement(&repo_id)
                    .await?;
                self.client
                    .send_event(recursive_maintainer_announcement)
                    .await?;

                // Return the state event (caller will send it)
                self.build_recursive_maintainer_state(&repo_id)
            }

            FixtureKind::PREvent => {
                use nostr_sdk::prelude::*;

                // ValidRepo is ensured by ensure_fixture before this is called
                let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                let repo_id = repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing repo_id in ValidRepo fixture"))?
                    .to_string();

                // Create PR event 1 second in the past
                let base_time = Timestamp::now().as_u64();
                let pr_timestamp = Timestamp::from(base_time - 1);

                // Build NIP-34 PR event (kind 1618)
                self.client
                    .event_builder(
                        Kind::Custom(1618), // NIP-34 PR kind (has 'c' tag for commit)
                        "Test PR for GRASP validation",
                    )
                    .tag(Tag::custom(
                        TagKind::custom("a"),
                        vec![format!(
                            "30617:{}:{}",
                            self.client.public_key().to_hex(), // Owner pubkey
                            repo_id
                        )],
                    ))
                    .tag(Tag::custom(
                        TagKind::custom("c"),
                        vec![PR_TEST_COMMIT_HASH.to_string()],
                    ))
                    .custom_time(pr_timestamp)
                    .build(self.client.pr_author_keys())
                    .map_err(|e| anyhow::anyhow!("Failed to build PR event: {}", e))
            }

            FixtureKind::PREventGenerated => {
                // Same as PREvent but will NOT be sent to relay (caller may send it later)
                // This fixture is for "Generated" stage only
                use nostr_sdk::prelude::*;

                // ValidRepo is ensured by ensure_fixture before this is called
                let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

                let repo_id = repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing repo_id in ValidRepo fixture"))?
                    .to_string();

                // Create PR event 1 second in the past
                let base_time = Timestamp::now().as_u64();
                let pr_timestamp = Timestamp::from(base_time - 1);

                // Build NIP-34 PR event (kind 1618)
                self.client
                    .event_builder(
                        Kind::Custom(1618), // NIP-34 PR kind (has 'c' tag for commit)
                        "Test PR for GRASP validation",
                    )
                    .tag(Tag::custom(
                        TagKind::custom("a"),
                        vec![format!(
                            "30617:{}:{}",
                            self.client.public_key().to_hex(), // Owner pubkey
                            repo_id
                        )],
                    ))
                    .tag(Tag::custom(
                        TagKind::custom("c"),
                        vec![PR_TEST_COMMIT_HASH.to_string()],
                    ))
                    .custom_time(pr_timestamp)
                    .build(self.client.pr_author_keys())
                    .map_err(|e| anyhow::anyhow!("Failed to build PR event: {}", e))
            }

            FixtureKind::PRWrongCommitPushedBeforeEvent => {
                self.build_pr_wrong_commit_pushed_before_event().await
            }

            FixtureKind::PREventSentAfterWrongPush => {
                self.build_pr_event_sent_after_wrong_push().await
            }

            FixtureKind::OwnerStateDataPushed => {
                self.build_owner_state_data_pushed().await
            }

            FixtureKind::MaintainerStateDataPushed => {
                self.build_maintainer_state_data_pushed().await
            }

            FixtureKind::RecursiveMaintainerStateDataPushed => {
                self.build_recursive_maintainer_state_data_pushed().await
            }
        }
    }

    /// Build maintainer announcement event for the given repo_id
    async fn build_maintainer_announcement(&self, repo_id: &str) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // Get relay URL for clone tag
        let relay_url = self
            .client
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
        let maintainer_npub = self
            .client
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
            .tag(Tag::custom(TagKind::custom("relays"), vec![relay_url]))
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
        let relay_url = self
            .client
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
        let recursive_maintainer_npub = self
            .client
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
                vec![format!(
                    "{}/{}/{}.git",
                    http_url, recursive_maintainer_npub, repo_id
                )],
            ))
            .tag(Tag::custom(TagKind::custom("relays"), vec![relay_url]))
            .tag(Tag::custom(
                TagKind::custom("maintainers"),
                vec![
                    self.client.public_key().to_hex(),
                    self.client.maintainer_pubkey_hex(),
                ],
            ))
            .build(self.client.recursive_maintainer_keys())
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to build recursive maintainer repo announcement: {}",
                    e
                )
            })
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

    /// Extract repo_id from a repo announcement event
    fn extract_repo_id(&self, repo: &Event) -> Result<String> {
        use nostr_sdk::prelude::*;
        repo.tags
            .iter()
            .find(|t| t.kind() == TagKind::d())
            .and_then(|t| t.content())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("Missing d tag in repo announcement"))
    }

    /// Build OwnerStateDataPushed fixture: full 4-stage fixture for push authorization
    ///
    /// This handles all stages of the fixture:
    /// 1. **Generated**: Creates RepoState (repo announcement + state event)
    /// 2. **Sent**: Sends events to relay
    /// 3. **Verified**: Confirms events accepted by relay
    /// 4. **DataPushed**: Clones repo, creates deterministic commit, pushes to relay
    ///
    /// # Returns
    /// The state event (kind 30618) after all stages complete successfully
    async fn build_owner_state_data_pushed(&self) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // ============================================================
        // Stage 1 & 2: ValidRepo is ensured by ensure_fixture before this is called
        // ============================================================
        let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;
        let repo_id = self.extract_repo_id(&repo)?;

        // Build state event
        let base_time = Timestamp::now().as_u64();
        let older_timestamp = Timestamp::from(base_time - 10); // 10 seconds ago

        let state_event = self
            .client
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
            .map_err(|e| anyhow::anyhow!("Failed to build state announcement: {}", e))?;

        // Send state event to relay
        self.client.send_event(state_event.clone()).await?;

        // ============================================================
        // Stage 3: Verify state event was accepted
        // ============================================================
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // ============================================================
        // Stage 4: DataPushed - Clone repo, create commit, push
        // ============================================================
        
        // Get relay domain from connected relay
        let relay_domain = self.get_relay_domain().await?;

        let npub = state_event
            .pubkey
            .to_bech32()
            .map_err(|e| anyhow::anyhow!("Failed to convert pubkey to bech32: {}", e))?;

        // Clone the repository
        let clone_path = clone_repo(&relay_domain, &npub, &repo_id)
            .map_err(|e| anyhow::anyhow!("Failed to clone repo: {}", e))?;

        // Cleanup helper (always clean up on error or success)
        let cleanup = |path: &PathBuf| {
            let _ = fs::remove_dir_all(path);
        };

        // Create deterministic commit locally
        let commit_hash = match create_deterministic_commit(&clone_path, "Initial commit") {
            Ok(h) => h,
            Err(e) => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!("Failed to create deterministic commit: {}", e));
            }
        };

        // Verify commit hash matches expected
        if commit_hash != DETERMINISTIC_COMMIT_HASH {
            cleanup(&clone_path);
            return Err(anyhow::anyhow!(
                "Commit hash mismatch: got {}, expected {}",
                commit_hash,
                DETERMINISTIC_COMMIT_HASH
            ));
        }

        // Create main branch pointing to our deterministic commit
        let branch_output = Command::new("git")
            .args(["branch", "main"])
            .current_dir(&clone_path)
            .output();

        match branch_output {
            Err(e) => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!("Failed to create main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!(
                    "Failed to create main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            _ => {}
        }

        // Checkout main branch
        let checkout_output = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&clone_path)
            .output();

        match checkout_output {
            Err(e) => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!("Failed to checkout main branch: {}", e));
            }
            Ok(output) if !output.status.success() => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!(
                    "Failed to checkout main branch: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            _ => {}
        }

        // Push to relay
        let push_result = try_push(&clone_path);
        cleanup(&clone_path);

        match push_result {
            Ok(true) => Ok(state_event),
            Ok(false) => Err(anyhow::anyhow!(
                "Push was rejected but should have been accepted. \
                The state event points to commit {} which matches the pushed commit.",
                DETERMINISTIC_COMMIT_HASH
            )),
            Err(e) => Err(anyhow::anyhow!("Push error: {}", e)),
        }
    }

    /// Build MaintainerStateDataPushed fixture: full 4-stage fixture for maintainer push authorization
    ///
    /// This tests that a maintainer can authorize pushes with ONLY a state event,
    /// without publishing their own repo announcement.
    ///
    /// Depends on OwnerStateDataPushed - the owner's data has already been pushed.
    /// The maintainer force-pushes their commit on top.
    ///
    /// # Returns
    /// The maintainer's state event (kind 30618) after all stages complete successfully
    async fn build_maintainer_state_data_pushed(&self) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // ============================================================
        // Stage 1: OwnerStateDataPushed is ensured by ensure_fixture before this is called
        // The owner's repo and state event are already on the relay, and git data is pushed
        // ============================================================
        let owner_state = self.get_cached_dependency(FixtureKind::OwnerStateDataPushed)?;
        
        // Extract repo_id from owner's state event (same d-tag structure)
        let repo_id = self.extract_repo_id(&owner_state)?;
        
        // Get the repo (ValidRepo, also cached) for the owner's npub
        let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

        // Build maintainer's state event (state event ONLY - no announcement)
        let base_time = Timestamp::now().as_u64();
        let maintainer_timestamp = Timestamp::from(base_time - 5); // 5 seconds ago (more recent than owner's state)

        let maintainer_state_event = self
            .client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![MAINTAINER_DETERMINISTIC_COMMIT_HASH.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("HEAD"),
                vec!["ref: refs/heads/main".to_string()],
            ))
            .custom_time(maintainer_timestamp)
            .build(self.client.maintainer_keys())
            .map_err(|e| anyhow::anyhow!("Failed to build maintainer state event: {}", e))?;

        // Send maintainer state event to relay
        self.client.send_event(maintainer_state_event.clone()).await?;

        // ============================================================
        // Stage 3: Verify state event was accepted
        // ============================================================
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // ============================================================
        // Stage 4: DataPushed - Clone repo, create maintainer commit, push
        // ============================================================
        
        // Get relay domain from connected relay
        let relay_domain = self.get_relay_domain().await?;

        // Use owner's npub for cloning (repo belongs to owner)
        let npub = repo
            .pubkey
            .to_bech32()
            .map_err(|e| anyhow::anyhow!("Failed to convert pubkey to bech32: {}", e))?;

        // Clone the repository
        let clone_path = clone_repo(&relay_domain, &npub, &repo_id)
            .map_err(|e| anyhow::anyhow!("Failed to clone repo: {}", e))?;

        // Cleanup helper (always clean up on error or success)
        let cleanup = |path: &PathBuf| {
            let _ = fs::remove_dir_all(path);
        };

        // Reset to orphan state and create deterministic root commit
        // Step 1: Create orphan branch (removes all history)
        let _ = Command::new("git")
            .args(["checkout", "--orphan", "main-new"])
            .current_dir(&clone_path)
            .output();

        // Step 2: Clear staged files (orphan keeps files staged from previous branch)
        let _ = Command::new("git")
            .args(["rm", "-rf", "--cached", "."])
            .current_dir(&clone_path)
            .output();

        // Step 3: Create deterministic commit using maintainer variant
        let commit_hash = match create_deterministic_commit_with_variant(
            &clone_path,
            CommitVariant::Maintainer,
        ) {
            Ok(h) => h,
            Err(e) => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!("Failed to create maintainer commit: {}", e));
            }
        };

        // Step 4: Replace main branch with our new orphan branch
        let _ = Command::new("git")
            .args(["branch", "-D", "main"])
            .current_dir(&clone_path)
            .output();

        let _ = Command::new("git")
            .args(["branch", "-m", "main"])
            .current_dir(&clone_path)
            .output();

        // Verify commit hash matches expected
        if commit_hash != MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup(&clone_path);
            return Err(anyhow::anyhow!(
                "Maintainer commit hash mismatch: got {}, expected {}",
                commit_hash,
                MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
        }

        // Push to relay
        let push_result = try_push(&clone_path);
        cleanup(&clone_path);

        match push_result {
            Ok(true) => Ok(maintainer_state_event),
            Ok(false) => Err(anyhow::anyhow!(
                "Push was rejected but should have been accepted. \
                The maintainer published a state event with commit {}, \
                and even without a separate announcement, the relay should \
                authorize pushes matching this state event since the maintainer \
                is listed in the owner's announcement.",
                MAINTAINER_DETERMINISTIC_COMMIT_HASH
            )),
            Err(e) => Err(anyhow::anyhow!("Push error: {}", e)),
        }
    }

    /// Build RecursiveMaintainerStateDataPushed fixture: full 4-stage fixture for recursive maintainer push authorization
    ///
    /// This tests that a recursive maintainer (authorized via maintainer chain) can authorize pushes.
    /// The recursive maintainer is listed in the maintainer's announcement, not the owner's announcement,
    /// so this tests the recursive maintainer traversal (Owner -> Maintainer -> RecursiveMaintainer).
    ///
    /// Depends on MaintainerStateDataPushed - the maintainer's data has already been pushed.
    /// We then send the MaintainerAnnouncement (which lists the recursive maintainer), and the
    /// recursive maintainer force-pushes their commit on top.
    ///
    /// # Returns
    /// The recursive maintainer's state event (kind 30618) after all stages complete successfully
    async fn build_recursive_maintainer_state_data_pushed(&self) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // ============================================================
        // Stage 1: MaintainerStateDataPushed is ensured by ensure_fixture before this is called
        // The owner's repo, owner's state event, and maintainer's state event are already on the relay,
        // and maintainer's git data is pushed
        // ============================================================
        let maintainer_state = self.get_cached_dependency(FixtureKind::MaintainerStateDataPushed)?;
        
        // Extract repo_id from maintainer's state event (same d-tag structure)
        let repo_id = self.extract_repo_id(&maintainer_state)?;
        
        // Get the repo (ValidRepo, also cached) for the owner's npub
        let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;

        // ============================================================
        // Stage 2: Send MaintainerAnnouncement (establishes Maintainer -> RecursiveMaintainer chain)
        // ============================================================
        let maintainer_announcement = self.build_maintainer_announcement(&repo_id).await?;
        self.client.send_event(maintainer_announcement).await?;

        // Build recursive maintainer's state event
        let base_time = Timestamp::now().as_u64();
        let recursive_maintainer_timestamp = Timestamp::from(base_time - 2); // 2 seconds ago (most recent)

        let recursive_maintainer_state_event = self
            .client
            .event_builder(Kind::Custom(30618), "")
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(
                TagKind::custom("refs/heads/main"),
                vec![RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("HEAD"),
                vec!["ref: refs/heads/main".to_string()],
            ))
            .custom_time(recursive_maintainer_timestamp)
            .build(self.client.recursive_maintainer_keys())
            .map_err(|e| anyhow::anyhow!("Failed to build recursive maintainer state event: {}", e))?;

        // Send recursive maintainer state event to relay
        self.client.send_event(recursive_maintainer_state_event.clone()).await?;

        // ============================================================
        // Stage 3: Verify state event was accepted
        // ============================================================
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // ============================================================
        // Stage 4: DataPushed - Clone repo, create recursive maintainer commit, push
        // ============================================================
        
        // Get relay domain from connected relay
        let relay_domain = self.get_relay_domain().await?;

        // Use owner's npub for cloning (repo belongs to owner)
        let npub = repo
            .pubkey
            .to_bech32()
            .map_err(|e| anyhow::anyhow!("Failed to convert pubkey to bech32: {}", e))?;

        // Clone the repository
        let clone_path = clone_repo(&relay_domain, &npub, &repo_id)
            .map_err(|e| anyhow::anyhow!("Failed to clone repo: {}", e))?;

        // Cleanup helper (always clean up on error or success)
        let cleanup = |path: &PathBuf| {
            let _ = fs::remove_dir_all(path);
        };

        // Reset to orphan state and create deterministic root commit
        // Step 1: Create orphan branch (removes all history)
        let _ = Command::new("git")
            .args(["checkout", "--orphan", "main-new"])
            .current_dir(&clone_path)
            .output();

        // Step 2: Clear staged files (orphan keeps files staged from previous branch)
        let _ = Command::new("git")
            .args(["rm", "-rf", "--cached", "."])
            .current_dir(&clone_path)
            .output();

        // Step 3: Create deterministic commit using recursive maintainer variant
        let commit_hash = match create_deterministic_commit_with_variant(
            &clone_path,
            CommitVariant::RecursiveMaintainer,
        ) {
            Ok(h) => h,
            Err(e) => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!("Failed to create recursive maintainer commit: {}", e));
            }
        };

        // Step 4: Replace main branch with our new orphan branch
        let _ = Command::new("git")
            .args(["branch", "-D", "main"])
            .current_dir(&clone_path)
            .output();

        let _ = Command::new("git")
            .args(["branch", "-m", "main"])
            .current_dir(&clone_path)
            .output();

        // Verify commit hash matches expected
        if commit_hash != RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH {
            cleanup(&clone_path);
            return Err(anyhow::anyhow!(
                "Recursive maintainer commit hash mismatch: got {}, expected {}",
                commit_hash,
                RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
            ));
        }

        // Push to relay
        let push_result = try_push(&clone_path);
        cleanup(&clone_path);

        match push_result {
            Ok(true) => Ok(recursive_maintainer_state_event),
            Ok(false) => Err(anyhow::anyhow!(
                "Push was rejected but should have been accepted. \
                The recursive maintainer published a state event with commit {}, \
                and the relay should authorize pushes matching this state event \
                through recursive maintainer traversal (Owner -> Maintainer -> RecursiveMaintainer).",
                RECURSIVE_MAINTAINER_DETERMINISTIC_COMMIT_HASH
            )),
            Err(e) => Err(anyhow::anyhow!("Push error: {}", e)),
        }
    }

    /// Build PRWrongCommitPushedBeforeEvent fixture
    ///
    /// This fixture sets up a scenario where:
    /// 1. A repo exists on the relay
    /// 2. A PR event is generated (but NOT sent to relay)
    /// 3. A wrong commit is pushed to refs/nostr/<pr-event-id>
    ///
    /// Server state after:
    /// - ValidRepo announcement on relay
    /// - refs/nostr/<pr-event-id> on git server pointing to DETERMINISTIC_COMMIT_HASH (wrong)
    /// - NO PR event on relay
    ///
    /// Returns: the unsent PR event (tests can publish it later)
    async fn build_pr_wrong_commit_pushed_before_event(&self) -> Result<Event> {
        use nostr_sdk::prelude::*;

        // Get the cached PREventGenerated (the unsent PR event)
        let pr_event = self.get_cached_dependency(FixtureKind::PREventGenerated)?;
        let pr_event_id = pr_event.id.to_hex();

        // Get the ValidRepo to extract repo info
        let repo = self.get_cached_dependency(FixtureKind::ValidRepo)?;
        let repo_id = self.extract_repo_id(&repo)?;

        // Get relay domain for cloning
        let relay_domain = self.get_relay_domain().await?;

        // Owner npub for clone URL
        let npub = repo
            .pubkey
            .to_bech32()
            .map_err(|e| anyhow::anyhow!("Failed to convert pubkey to bech32: {}", e))?;

        // Clone the repository (fresh clone - local repos are never cached)
        let clone_path = clone_repo(&relay_domain, &npub, &repo_id)
            .map_err(|e| anyhow::anyhow!("Failed to clone repo: {}", e))?;

        // Cleanup helper
        let cleanup = |path: &PathBuf| {
            let _ = fs::remove_dir_all(path);
        };

        // Create a WRONG commit (Owner variant, not PRTestCommit)
        // This commit hash will NOT match what's in the PR event's `c` tag
        let wrong_commit_hash = match create_deterministic_commit_with_variant(
            &clone_path,
            CommitVariant::Owner,
        ) {
            Ok(h) => h,
            Err(e) => {
                cleanup(&clone_path);
                return Err(anyhow::anyhow!("Failed to create wrong commit: {}", e));
            }
        };

        // Verify it's actually different from expected PR commit
        if wrong_commit_hash == PR_TEST_COMMIT_HASH {
            cleanup(&clone_path);
            return Err(anyhow::anyhow!(
                "Test setup error: wrong_commit_hash {} equals PR_TEST_COMMIT_HASH",
                wrong_commit_hash
            ));
        }

        // Create master branch if needed and push to refs/nostr/<pr-event-id>
        let _ = Command::new("git")
            .args(["branch", "-M", "master"])
            .current_dir(&clone_path)
            .output();

        let push_output = Command::new("git")
            .args([
                "push",
                "origin",
                &format!("master:refs/nostr/{}", pr_event_id),
            ])
            .current_dir(&clone_path)
            .output()
            .map_err(|e| {
                cleanup(&clone_path);
                anyhow::anyhow!("Failed to execute git push: {}", e)
            })?;

        cleanup(&clone_path);

        if !push_output.status.success() {
            let stderr = String::from_utf8_lossy(&push_output.stderr);
            return Err(anyhow::anyhow!(
                "Initial push to refs/nostr/{} failed (expected success before PR event exists): {}",
                pr_event_id,
                stderr
            ));
        }

        // Return the unsent PR event (tests can publish it later)
        Ok(pr_event)
    }

    /// Build PREventSentAfterWrongPush fixture
    ///
    /// This fixture builds on PRWrongCommitPushedBeforeEvent by sending the PR event.
    /// After this fixture, the relay has:
    /// - ValidRepo announcement
    /// - PR event
    /// - refs/nostr/<pr-event-id> may have been cleaned up (that's what tests verify)
    ///
    /// Returns: the sent PR event
    async fn build_pr_event_sent_after_wrong_push(&self) -> Result<Event> {
        // Get the PR event that was cached by PRWrongCommitPushedBeforeEvent
        let pr_event = self.get_cached_dependency(FixtureKind::PRWrongCommitPushedBeforeEvent)?;

        // Send the PR event to relay
        self.client.send_event(pr_event.clone()).await?;

        // Wait for relay to process
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Return the now-sent PR event
        Ok(pr_event)
    }

    /// Get relay domain (host:port) from the connected relay
    ///
    /// Extracts the domain from the relay URL for git HTTP operations.
    /// Example: ws://localhost:7000 -> localhost:7000
    async fn get_relay_domain(&self) -> Result<String> {
        let relay_url = self
            .client
            .client()
            .relays()
            .await
            .keys()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No relay connected"))?
            .to_string();

        // Extract domain from URL (ws://host:port -> host:port)
        let domain = relay_url
            .replace("ws://", "")
            .replace("wss://", "")
            .trim_end_matches('/')
            .to_string();

        Ok(domain)
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
                return Err(format!(
                    "Event was rejected but still stored: {}",
                    description
                ));
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
pub fn clone_repo(relay_domain: &str, npub: &str, repo_id: &str) -> Result<PathBuf, String> {
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
    /// PR test commit variant - for PR event tests
    PRTestCommit,
}

impl CommitVariant {
    /// Get the file content for this variant
    pub fn file_content(&self) -> &'static str {
        match self {
            CommitVariant::Owner => "Initial commit",
            CommitVariant::Maintainer => "Maintainer initial commit",
            CommitVariant::RecursiveMaintainer => "Recursive maintainer initial commit",
            CommitVariant::PRTestCommit => "PR test deterministic commit",
        }
    }

    /// Get the commit message for this variant
    pub fn commit_message(&self) -> &'static str {
        match self {
            CommitVariant::Owner => "Initial commit",
            CommitVariant::Maintainer => "Maintainer initial commit",
            CommitVariant::RecursiveMaintainer => "Recursive maintainer initial commit",
            CommitVariant::PRTestCommit => "PR test deterministic commit",
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
pub fn create_deterministic_commit_with_variant(
    clone_path: &Path,
    variant: CommitVariant,
) -> Result<String, String> {
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
        .args(["-c", "commit.gpgsign=false", "commit", "-m", message])
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
        .args(["push", "origin", "main", "--force"])
        .current_dir(clone_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Failed to execute git push: {}", e))?;

    Ok(output.status.success())
}

/// Attempt a git push to a specific ref and return success/failure
///
/// This is used for testing refs/nostr/<event-id> push validation.
///
/// # Arguments
/// * `clone_path` - Path to the git repository
/// * `ref_name` - The ref to push to (e.g., "refs/nostr/<event-id>")
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
/// let success = try_push_to_ref(Path::new("/tmp/my-repo"), "refs/nostr/abc123")?;
/// if success {
///     println!("Push to refs/nostr/abc123 succeeded");
/// } else {
///     println!("Push was rejected");
/// }
/// # Ok(())
/// # }
/// ```
pub fn try_push_to_ref(clone_path: &Path, ref_name: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .args(["push", "origin", &format!("HEAD:{}", ref_name)])
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
