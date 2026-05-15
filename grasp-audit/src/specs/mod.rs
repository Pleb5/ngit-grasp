//! Test specifications
//!
//! This module contains all GRASP specification test suites.

pub mod grasp01;
pub mod grasp06;

// Re-export all test structs from grasp01 module
pub use grasp01::{
    CorsTests, EventAcceptancePolicyTests, GitCloneTests, GitFilterTests, Nip01SmokeTests,
    Nip11DocumentTests, PurgatoryTests, PushAuthorizationTests, RepositoryCreationTests,
};

// Re-export GRASP-06 test structs
pub use grasp06::{EventAcceptanceTests, Grasp06Tests, Nip11Tests, PrsEndpointTests};
