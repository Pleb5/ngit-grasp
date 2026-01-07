//! Purgatory sync module for background git data synchronization.
//!
//! This module implements identifier-based syncing with:
//! - Batched OID fetching across all purgatory events for an identifier
//! - Domain-based throttling (configurable requests/minute per domain)
//! - Exponential backoff per identifier (20s → 2m, then 2m intervals)
//! - Debouncing for burst event arrivals

mod context;
mod queue;
mod throttle;

pub use context::{ProcessResult, SyncContext};
pub use queue::SyncQueueEntry;
pub use throttle::{DomainThrottle, ThrottleManager};

#[cfg(test)]
pub use context::mock::MockSyncContext;
