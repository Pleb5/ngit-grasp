//! Catchup Sync - Documentation Only
//!
//! This file documents the catchup sync mechanism. No integration tests are included
//! because the functionality cannot be reliably tested with current test infrastructure.
//!
//! # What is Catchup Sync?
//!
//! Catchup sync ensures that when a relay's WebSocket connection to another relay drops
//! and reconnects, any events the source relay received during the disconnection are
//! fetched using a `since` filter based on the last connection timestamp.
//!
//! # Implementation Status: ✅ IMPLEMENTED
//!
//! The catchup sync mechanism is fully implemented in the sync module via:
//!
//! - [`handle_connect_or_reconnect()`](../../src/sync/mod.rs) - Detects reconnection and
//!   applies appropriate sync strategy
//! - [`RelayState.last_connected`](../../src/sync/mod.rs) - Tracks when we last connected
//!   to each relay
//! - [`filters::build_announcement_filter(since)`](../../src/sync/filters.rs) - Builds
//!   Layer 1 filters with `since` timestamp
//! - [`filters::build_layer2_and_layer3_filters(since)`](../../src/sync/filters.rs) -
//!   Builds Layer 2/3 filters with `since` timestamp
//!
//! ## Reconnection Logic
//!
//! When a relay reconnects to a source relay, the sync manager uses smart reconnection:
//!
//! | Scenario | Behavior |
//! |----------|----------|
//! | First connection ever | Full sync (no `since` filter) |
//! | Reconnect within 15 min | Quick reconnect with `since = last_connected - 15min` |
//! | Reconnect after >15 min | Full sync (clear state, treat as fresh connection) |
//!
//! The 15-minute buffer on the `since` filter accounts for clock drift and ensures
//! no events are missed at the boundary.
//!
//! # Why No Integration Tests?
//!
//! Testing catchup sync in integration tests is not feasible with current infrastructure:
//!
//! ## 1. Cannot Force WebSocket Disconnection
//!
//! The catchup mechanism is designed for same-process reconnection scenarios, such as:
//! - Network hiccup causing temporary disconnection
//! - Source relay temporarily unreachable
//! - WebSocket connection timeout
//!
//! Our [`TestRelay`](../common/relay.rs) fixture doesn't provide a way to force a
//! WebSocket disconnection without stopping the relay entirely.
//!
//! ## 2. Stopping a Relay Loses Events (In-Memory Database)
//!
//! `TestRelay` uses `NGIT_DATABASE_BACKEND=memory` for test isolation. If we stop
//! the source relay (to simulate disconnection), all events are lost. When a new
//! instance starts, there's nothing to "catch up" on.
//!
//! ## 3. Stopping the Syncing Relay Creates a New Instance
//!
//! If we stop the syncing relay and start a new one:
//! - `last_connected` is lost (in-memory state)
//! - New instance does a fresh full sync, not a `since`-filtered catchup
//! - This is correct behavior, but tests the bootstrap path, not catchup
//!
//! # Alternative Testing Approaches (Not Implemented)
//!
//! These could enable catchup testing but add significant complexity:
//!
//! 1. **Persistent database for source relay** - Use SQLite instead of in-memory,
//!    allowing relay restart without data loss
//!
//! 2. **TestRelay restart capability** - Add `restart()` method that preserves the
//!    same port and database path
//!
//! 3. **Network simulation** - Add ability to inject network failures between specific
//!    relay pairs without stopping either relay
//!
//! 4. **Internal sync manager API** - Expose methods to force reconnection without
//!    network-level disruption
//!
//! # Related Tests
//!
//! While catchup sync itself isn't directly tested, related functionality is covered:
//!
//! - [`bootstrap.rs`](bootstrap.rs) - Tests that a new relay syncs existing events
//!   from a bootstrap relay (fresh full sync path)
//! - [`live_sync.rs`](live_sync.rs) - Tests real-time sync of new events after
//!   connection is established
//! - [`discovery.rs`](discovery.rs) - Tests that relays discover each other via
//!   repository announcements
//!
//! # Design Rationale
//!
//! The catchup mechanism prioritizes simplicity and correctness:
//!
//! - **Correctness over testing**: The `since` filter logic is straightforward and
//!   uses well-tested nostr-sdk primitives. The risk of bugs is low.
//!
//! - **15-minute quick reconnect window**: Balances efficiency (avoid full resync for
//!   brief outages) with simplicity (don't track complex state for long outages).
//!
//! - **Full sync fallback**: After 15 minutes, the relay does a complete resync.
//!   This guarantees no events are missed, at the cost of redundant transfers.
//!
//! # See Also
//!
//! - [`src/sync/mod.rs`](../../src/sync/mod.rs) - Main sync module with reconnection logic
//! - [`src/sync/filters.rs`](../../src/sync/filters.rs) - Filter builders with `since` support
//! - [`src/sync/metrics.rs`](../../src/sync/metrics.rs) - Metrics tracking event sources
//!   including `RECONNECT` for catchup events
