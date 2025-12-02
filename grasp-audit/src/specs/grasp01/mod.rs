//! GRASP-01 specification tests
//!
//! This module contains all test suites for GRASP-01 compliance testing.
//!
//! ## Test Suites
//!
//! - [`Nip01SmokeTests`] - Basic NIP-01 relay functionality (WebSocket-only)
//! - [`Nip11DocumentTests`] - NIP-11 relay information document (WebSocket-only)
//! - [`EventAcceptancePolicyTests`] - Event acceptance rules (WebSocket-only)
//! - [`CorsTests`] - CORS headers on Git HTTP endpoints (requires git-data-dir)
//! - [`GitCloneTests`] - Git clone operations (requires git-data-dir)
//! - [`PushAuthorizationTests`] - Push authorization (requires git-data-dir)
//! - [`RepositoryCreationTests`] - Repository creation (requires git-data-dir)

pub mod cors;
pub mod event_acceptance_policy;
pub mod git_clone;
pub mod nip01_smoke;
pub mod nip11_document;
pub mod push_authorization;
pub mod repository_creation;
pub mod spec_requirements;

pub use cors::CorsTests;
pub use event_acceptance_policy::EventAcceptancePolicyTests;
pub use git_clone::GitCloneTests;
pub use nip01_smoke::Nip01SmokeTests;
pub use nip11_document::Nip11DocumentTests;
pub use push_authorization::PushAuthorizationTests;
pub use repository_creation::RepositoryCreationTests;
pub use spec_requirements::{
    get_requirement, get_requirements_for_section, get_sections, RequirementLevel,
    SpecRequirement, GRASP_01_REQUIREMENTS, GRASP_COMMIT_ID,
};
