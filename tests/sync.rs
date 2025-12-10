//! Proactive Sync Integration Tests
//!
//! This test file organizes tests for ngit-grasp's proactive sync functionality.
//! Tests are grouped into submodules by sync scenario:
//!
//! - `bootstrap` - Tests for sync from pre-configured bootstrap relay
//! - `discovery` - Tests for relay discovery from announcement events
//!
//! # Running Tests
//!
//! ```bash
//! # Run all sync tests
//! cargo test --test sync
//!
//! # Run with output
//! cargo test --test sync -- --nocapture
//!
//! # Run specific test
//! cargo test --test sync test_bootstrap_syncs -- --nocapture
//! ```

// Include the common test utilities
mod common;

// Include sync test submodules (located in tests/sync/)
mod sync {
    pub mod bootstrap;
    pub mod discovery;
}