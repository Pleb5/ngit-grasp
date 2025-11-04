//! GRASP Audit Tool
//!
//! A reusable compliance and audit testing tool for GRASP protocol implementations.
//!
//! # Features
//!
//! - **Isolated Testing**: Tests run in parallel with unique audit IDs
//! - **Production Audit**: Test live services with minimal impact
//! - **Clean Audit Events**: Special tags for easy cleanup without deletion trails
//! - **Spec-Mirrored Tests**: Test structure matches GRASP protocol exactly
//!
//! # Usage
//!
//! ```no_run
//! use grasp_audit::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create audit client for CI testing
//!     let config = AuditConfig::ci();
//!     let client = AuditClient::new("ws://localhost:7000", config).await?;
//!     
//!     // Run smoke tests
//!     let results = specs::nip01_smoke::Nip01SmokeTests::run_all(&client).await;
//!     results.print_report();
//!     
//!     Ok(())
//! }
//! ```

pub mod audit;
pub mod client;
pub mod isolation;
pub mod result;
pub mod specs;

pub use audit::{AuditConfig, AuditMode};
pub use client::AuditClient;
pub use result::{AuditResult, TestResult};

// Re-export commonly used types
pub use anyhow::{anyhow, Result};
pub use nostr_sdk::prelude::*;
