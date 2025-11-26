//! GRASP-01 specification tests

pub mod cors;
pub mod event_acceptance_policy;
pub mod git_clone;
pub mod nip01_smoke;
pub mod nip11_document;
pub mod push_authorization;
pub mod repository_creation;

pub use cors::CorsTests;
pub use event_acceptance_policy::EventAcceptancePolicyTests;
pub use git_clone::GitCloneTests;
pub use nip01_smoke::Nip01SmokeTests;
pub use nip11_document::Nip11DocumentTests;
pub use push_authorization::PushAuthorizationTests;
pub use repository_creation::RepositoryCreationTests;
