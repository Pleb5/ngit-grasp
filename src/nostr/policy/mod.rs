/// Policy module for NIP-34 write policies
///
/// This module splits the large Nip34WritePolicy into focused sub-policies:
/// - `AnnouncementPolicy` - Repository announcement validation
/// - `StatePolicy` - State event validation + ref alignment
/// - `PrEventPolicy` - PR/PR Update validation
/// - `RelatedEventPolicy` - Forward/backward reference checking
mod announcement;
mod pr_event;
mod related;
mod state;

pub use announcement::{AnnouncementPolicy, AnnouncementResult};
pub use pr_event::PrEventPolicy;
pub use related::{ReferenceResult, RelatedEventPolicy};
pub use state::{StatePolicy, StateResult};

// Re-export AlignmentResult from git::sync (canonical location)
pub use crate::git::sync::AlignmentResult;

use super::SharedDatabase;
use crate::purgatory::Purgatory;
use crate::sync::RepoSyncIndex;
use nostr_relay_builder::LocalRelay;
use std::sync::Arc;

/// Shared context for all sub-policies
#[derive(Clone)]
pub struct PolicyContext {
    pub domain: String,
    pub database: SharedDatabase,
    pub git_data_path: std::path::PathBuf,
    pub purgatory: Arc<Purgatory>,
    /// Local relay for notifying WebSocket subscribers (set after relay creation)
    pub local_relay: Arc<std::sync::RwLock<Option<LocalRelay>>>,
    /// Configuration reference for policy settings (includes blacklists)
    pub config: crate::config::Config,
    /// Repo sync index for registering purgatory announcements (set after SyncManager creation)
    pub repo_sync_index: Arc<std::sync::RwLock<Option<RepoSyncIndex>>>,
}

impl PolicyContext {
    pub fn new(
        domain: impl Into<String>,
        database: SharedDatabase,
        git_data_path: impl Into<std::path::PathBuf>,
        purgatory: Arc<Purgatory>,
        config: crate::config::Config,
    ) -> Self {
        Self {
            domain: domain.into(),
            database,
            git_data_path: git_data_path.into(),
            purgatory,
            local_relay: Arc::new(std::sync::RwLock::new(None)),
            config,
            repo_sync_index: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Set the local relay after it's been created.
    ///
    /// This is called after the relay is built since the relay depends on the policy
    /// but the policy needs the relay for purgatory notifications.
    pub fn set_local_relay(&self, relay: LocalRelay) {
        let mut guard = self.local_relay.write().unwrap();
        *guard = Some(relay);
    }

    /// Get a clone of the local relay if it's been set.
    pub fn get_local_relay(&self) -> Option<LocalRelay> {
        let guard = self.local_relay.read().unwrap();
        guard.clone()
    }

    /// Set the repo sync index after SyncManager has been created.
    ///
    /// This allows purgatory announcements submitted by users to be registered
    /// in the sync index so state event sync starts promptly.
    pub fn set_repo_sync_index(&self, index: RepoSyncIndex) {
        let mut guard = self.repo_sync_index.write().unwrap();
        *guard = Some(index);
    }

    /// Get a clone of the repo sync index if it has been set.
    pub fn get_repo_sync_index(&self) -> Option<RepoSyncIndex> {
        let guard = self.repo_sync_index.read().unwrap();
        guard.clone()
    }
}
