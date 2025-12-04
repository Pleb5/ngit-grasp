//! Proactive Sync Module for GRASP-02
//!
//! This module implements proactive synchronization of kind 30617 (repository state)
//! events from configured relay(s). Events are validated through the same write policy
//! as directly-submitted events.
//!
//! ## Three-Layer Filter Strategy (Phase 2)
//!
//! - **Layer 1**: Announcement discovery (kinds 30617 + 30618)
//! - **Layer 2**: Repository events (A/a tags for shared repos)
//! - **Layer 3**: Related events (E/e tags for discussions, reviews)

mod connection;
mod filter;
mod manager;

pub use filter::FilterService;
pub use manager::SyncManager;

use std::net::SocketAddr;

/// Synthetic source address used for synced events
///
/// This distinguishes synced events from directly-submitted events in logs and metrics.
/// Uses 127.0.0.2:0 as a recognizable "synced event" marker.
pub const SYNC_SOURCE_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 2)), 0);

/// Kind for repository state events (NIP-34)
pub const KIND_REPOSITORY_STATE: u16 = 30617;