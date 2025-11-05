//! GRASP-01 specification tests

pub mod event_acceptance_policy;
pub mod nip01_smoke;
pub mod nip11_document;

pub use event_acceptance_policy::EventAcceptancePolicyTests;
pub use nip01_smoke::Nip01SmokeTests;
pub use nip11_document::Nip11DocumentTests;