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
use std::sync::Arc;

/// Shared context for all sub-policies
#[derive(Clone)]
pub struct PolicyContext {
    pub domain: String,
    pub database: SharedDatabase,
    pub git_data_path: std::path::PathBuf,
    pub purgatory: Arc<Purgatory>,
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
        }
    }
}
