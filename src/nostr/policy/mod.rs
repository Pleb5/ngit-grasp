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
pub use state::{AlignmentResult, StatePolicy, StateResult};

use super::SharedDatabase;
use crate::purgatory::Purgatory;
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
}

impl PolicyContext {
    pub fn new(
        domain: impl Into<String>,
        database: SharedDatabase,
        git_data_path: impl Into<std::path::PathBuf>,
        purgatory: Arc<Purgatory>,
    ) -> Self {
        Self {
            domain: domain.into(),
            database,
            git_data_path: git_data_path.into(),
            purgatory,
            local_relay: Arc::new(std::sync::RwLock::new(None)),
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
}
