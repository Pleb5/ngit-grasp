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
use crate::sync::{AddFilters, RepoSyncIndex};
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
    /// Optional repo sync index for triggering relay discovery when announcements
    /// go to purgatory via user submission (not via the sync path).
    /// Wrapped in Arc<RwLock> for interior mutability (PolicyContext is Clone).
    pub repo_sync_index: Arc<std::sync::RwLock<Option<RepoSyncIndex>>>,
    /// Optional sender for AddFilters actions to SyncManager.
    /// Used to trigger relay discovery when user-submitted purgatory announcements
    /// are registered with StateOnly sync level.
    /// Wrapped in Arc<RwLock> for interior mutability (PolicyContext is Clone).
    pub sync_action_tx:
        Arc<std::sync::RwLock<Option<tokio::sync::mpsc::Sender<AddFilters>>>>,
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
            sync_action_tx: Arc::new(std::sync::RwLock::new(None)),
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

    /// Set the repo sync index for relay discovery from user-submitted purgatory announcements.
    pub fn set_repo_sync_index(&self, index: RepoSyncIndex) {
        let mut guard = self.repo_sync_index.write().unwrap();
        *guard = Some(index);
    }

    /// Get a clone of the repo sync index if it's been set.
    pub fn get_repo_sync_index(&self) -> Option<RepoSyncIndex> {
        let guard = self.repo_sync_index.read().unwrap();
        guard.clone()
    }

    /// Set the sync action sender for sending AddFilters actions to SyncManager.
    pub fn set_sync_action_tx(&self, tx: tokio::sync::mpsc::Sender<AddFilters>) {
        let mut guard = self.sync_action_tx.write().unwrap();
        *guard = Some(tx);
    }

    /// Get a clone of the sync action sender if it's been set.
    pub fn get_sync_action_tx(&self) -> Option<tokio::sync::mpsc::Sender<AddFilters>> {
        let guard = self.sync_action_tx.read().unwrap();
        guard.clone()
    }
}
