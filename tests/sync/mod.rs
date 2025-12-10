//! Proactive Sync Integration Tests
//!
//! This module organizes tests for ngit-grasp's proactive sync functionality.
//! Tests are grouped by sync scenario:
//!
//! - Bootstrap sync (relay syncs from pre-configured bootstrap relay)
//! - Relay discovery (relay discovers other relays from announcement events)
//! - Live sync (events sync in real-time after connection established)
//! - Tag variations (testing different Layer 2/3 tag types: a/A/q, e/E/q)
//! - Catchup sync (events from disconnected period sync on reconnect)
//!
//! # Test Files
//!
//! - `bootstrap.rs` - Tests 1, 4: sync from bootstrap relay
//! - `discovery.rs` - Tests 2, 3: relay discovery from announcements
//! - `live_sync.rs` - Tests 5, 6, 7: real-time sync after connection
//! - `tag_variations.rs` - Tests 8, 9: Layer 2/3 tag type coverage
//! - `catchup.rs` - Test 0: catchup after disconnect (stub, `#[ignore]`)
//!
//! # Shared Imports
//!
//! All sync tests use helpers from `common::sync_helpers`:
//! - `TestClient` - Client with retry logic
//! - Event builders for Layer 2/3 events
//! - `wait_for_event_on_relay()` - Non-panicking assertion helper

// Test modules
pub mod bootstrap;
pub mod catchup;
pub mod discovery;
pub mod live_sync;
pub mod tag_variations;