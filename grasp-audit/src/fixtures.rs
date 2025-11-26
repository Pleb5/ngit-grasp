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

/// Deterministic commit hash used in RepoState fixtures
/// This is the hash produced by creating a commit with:
/// - Message: "Initial commit"
/// - File: test.txt containing "Initial commit"
/// - Author date: 2024-01-01T00:00:00Z
/// - Committer date: 2024-01-01T00:00:00Z
/// - GPG signing: disabled
/// - User: "GRASP Audit Test <test@grasp-audit.local>"
/// - Parent: Initial empty commit (09cc37de80f3434fa98864a86730b8d7777bd6ae)
pub const DETERMINISTIC_COMMIT_HASH: &str = "64ea71d79a57a7acb334cd9651f8aec067c0ce5d";

/// Types of test fixtures available
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixtureKind {
    /// Basic repository announcement (kind 30617)
    ValidRepo,

    /// Repository with one issue (kind 1621)
    RepoWithIssue,

    /// Repository with issue and comment (kind 1111)
    RepoWithComment,

    /// Repository state announcement (kind 30618)
    RepoState,
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

    /// Build a fixture event (doesn't send it)
    async fn build_fixture(&self, kind: FixtureKind) -> Result<Event> {
        match kind {
            FixtureKind::ValidRepo => {
                let test_name = format!(
                    "fixture-{:?}-{}",
                    kind,
                    &uuid::Uuid::new_v4().to_string()[..8]
                );
                self.client.create_repo_announcement(&test_name).await
            }

            FixtureKind::RepoWithIssue => {
                // First create and send repo
                let test_name = format!(
                    "fixture-{:?}-{}",
                    FixtureKind::ValidRepo,
                    &uuid::Uuid::new_v4().to_string()[..8]
                );
                let repo = self.client.create_repo_announcement(&test_name).await?;
                self.client.send_event(repo.clone()).await?;

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
                // First create repo with issue
                let test_name = format!(
                    "fixture-{:?}-{}",
                    FixtureKind::ValidRepo,
                    &uuid::Uuid::new_v4().to_string()[..8]
                );
                let repo = self.client.create_repo_announcement(&test_name).await?;
                self.client.send_event(repo.clone()).await?;

                let issue =
                    self.client
                        .create_issue(&repo, "Test Issue", "Issue content", vec![])?;
                self.client.send_event(issue.clone()).await?;

                // Then create comment on issue
                self.client.create_comment(&issue, "Test comment", vec![])
            }

            FixtureKind::RepoState => {
                use nostr_sdk::prelude::*;

                // First create repo announcement
                let test_name = format!(
                    "fixture-{:?}-{}",
                    FixtureKind::ValidRepo,
                    &uuid::Uuid::new_v4().to_string()[..8]
                );
                let repo = self.client.create_repo_announcement(&test_name).await?;
                self.client.send_event(repo.clone()).await?;

                // Extract repo_id from repo announcement
                let repo_id = repo
                    .tags
                    .iter()
                    .find(|t| t.kind() == TagKind::d())
                    .and_then(|t| t.content())
                    .ok_or_else(|| anyhow::anyhow!("Missing d tag in repo announcement"))?
                    .to_string();

                // Create state announcement with deterministic commit hash
                // Tag format: ["refs/heads/main", "<commit_hash>"]
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
                    .build(self.client.keys())
                    .map_err(|e| anyhow::anyhow!("Failed to build state announcement: {}", e))
            }
        }
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
