//! GRASP-06 specification tests
//!
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//!
//! ## Test Suites
//!
//! - [`PrsEndpointTests`] - `/prs/<npub>/<id>.git` endpoint behaviour
//!   (discovery gate, empty-repo fetch, push acceptance/rejection)
//! - [`Nip11Tests`] - NIP-11 advertisement of GRASP-06 capability
//!
//! Additional suites for event-acceptance relaxation and mirroring will be
//! added as the implementation lands.
//!
//! ## Shared fixtures
//!
//! [`fixtures`] holds non-Event prerequisites shared across the suite
//! (currently the NIP-11 document). New checks needing the doc should reuse
//! [`fixtures::advertises_grasp`] rather than re-fetching.

pub mod fixtures;
pub mod nip11;
pub mod prs_endpoint;
pub mod spec_requirements;

pub use nip11::Nip11Tests;
pub use prs_endpoint::PrsEndpointTests;
pub use spec_requirements::{SpecRef, GRASP_06_COMMIT_ID};
