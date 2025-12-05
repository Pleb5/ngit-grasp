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
//!
//! ## Resilience & Health Tracking (Phase 3)
//!
//! - **Health tracking**: Per-relay connection health states (Healthy, Degraded, Dead)
//! - **Exponential backoff**: Smart retry delays on failures (5s -> 1h max)
//! - **Dead relay handling**: Minimal retry for 24h+ failed relays
//! - **Startup jitter**: Prevent thundering herd on launch (0-10s random delay)

mod connection;
mod filter;
pub mod health;
mod manager;
pub mod metrics;
pub mod negentropy;
mod subscription;

pub use filter::FilterService;
pub use health::{HealthState, RelayHealth, RelayHealthTracker};
pub use manager::SyncManager;
pub use metrics::SyncMetrics;
pub use negentropy::NegentropyService;
pub use subscription::SubscriptionManager;

// Re-export default sync source address for backward compatibility with modules like negentropy.rs
// Manager.rs derives sync_source_addr from config.bind_address at runtime
pub use manager::DEFAULT_SYNC_SOURCE_ADDR as SYNC_SOURCE_ADDR;

/// Kind for repository state events (NIP-34)
pub const KIND_REPOSITORY_STATE: u16 = 30617;