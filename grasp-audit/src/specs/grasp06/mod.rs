//! GRASP-06 specification tests
//!
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//!
//! ## Test Suites
//!
//! - [`PrsEndpointTests`] - `/prs/<npub>/<id>.git` endpoint behaviour
//!   (discovery gate, empty-repo fetch, push acceptance/rejection)
//!
//! Additional suites for event-acceptance relaxation and mirroring will be
//! added as the implementation lands.

pub mod prs_endpoint;
pub mod spec_requirements;

pub use prs_endpoint::PrsEndpointTests;
pub use spec_requirements::{SpecRef, GRASP_06_COMMIT_ID};
