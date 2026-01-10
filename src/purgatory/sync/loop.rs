//! Background sync loop for purgatory synchronization.
//!
//! This module provides the main sync loop that runs in the background and
//! processes identifiers that are ready for sync. The loop:
//!
//! 1. Runs every 1 second (hardcoded interval)
//! 2. Finds all ready identifiers (where `!in_progress && next_attempt <= now`)
//! 3. Spawns parallel tasks for each ready identifier
//! 4. Applies backoff when sync completes (if events remain)
//! 5. Removes identifiers from queue when sync completes or no events remain

use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::purgatory::Purgatory;
use crate::sync::naughty_list::NaughtyListTracker;

use super::context::SyncContext;
use super::functions::sync_identifier;
use super::throttle::ThrottleManager;

/// Interval between sync loop iterations (hardcoded, not configurable).
const SYNC_LOOP_INTERVAL: Duration = Duration::from_secs(1);

impl Purgatory {
    /// Start the background sync loop.
    ///
    /// This spawns a background task that periodically checks for identifiers
    /// ready for sync and processes them. The loop runs every 1 second and:
    ///
    /// 1. Finds all ready identifiers (where `!in_progress && next_attempt <= now`)
    /// 2. Spawns parallel tasks for each ready identifier
    /// 3. Each task calls `sync_identifier` to try fetching git data
    /// 4. On completion, applies backoff if events remain, or removes from queue
    ///
    /// # Arguments
    /// * `ctx` - The sync context providing repository data and fetch capabilities
    /// * `throttle_manager` - Used for rate limiting and domain queue management
    /// * `git_naughty_list` - Tracker for git remote domains with persistent errors
    ///
    /// # Returns
    /// A `JoinHandle` for the background task (can be used to cancel the loop)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let purgatory = Arc::new(Purgatory::new("/data/git"));
    /// let ctx = Arc::new(RealSyncContext::new(...));
    /// let throttle_manager = Arc::new(ThrottleManager::new(5, 30));
    /// let git_naughty_list = Arc::new(NaughtyListTracker::with_defaults());
    ///
    /// // Set context on throttle manager for queue processing
    /// throttle_manager.set_context(ctx.clone());
    ///
    /// // Start the sync loop
    /// let handle = purgatory.start_sync_loop(ctx, throttle_manager, git_naughty_list);
    ///
    /// // Later, to stop the loop:
    /// handle.abort();
    /// ```
    pub fn start_sync_loop(
        self: Arc<Self>,
        ctx: Arc<dyn SyncContext>,
        throttle_manager: Arc<ThrottleManager>,
        git_naughty_list: Arc<NaughtyListTracker>,
    ) -> JoinHandle<()> {
        info!(
            "Starting purgatory sync loop (interval: {:?})",
            SYNC_LOOP_INTERVAL
        );

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(SYNC_LOOP_INTERVAL);

            loop {
                interval.tick().await;

                // Find all ready identifiers
                let ready: Vec<String> = self
                    .sync_queue
                    .iter()
                    .filter(|entry| entry.value().is_ready())
                    .map(|entry| entry.key().clone())
                    .collect();

                if !ready.is_empty() {
                    debug!(
                        ready_count = ready.len(),
                        "Found identifiers ready for sync"
                    );
                }

                for identifier in ready {
                    // Check if events still exist for this identifier
                    if !self.has_pending_events(&identifier) {
                        debug!(
                            identifier = %identifier,
                            "No pending events - removing from sync queue"
                        );
                        self.sync_queue.remove(&identifier);
                        continue;
                    }

                    // Mark as in progress (skip if already in progress)
                    let should_process = {
                        if let Some(mut entry) = self.sync_queue.get_mut(&identifier) {
                            if entry.in_progress {
                                false
                            } else {
                                entry.in_progress = true;
                                true
                            }
                        } else {
                            false
                        }
                    };

                    if !should_process {
                        continue;
                    }

                    // Spawn sync task
                    let purgatory = self.clone();
                    let ctx = ctx.clone();
                    let throttle_manager = throttle_manager.clone();
                    let git_naughty_list = git_naughty_list.clone();
                    let id = identifier.clone();

                    tokio::spawn(async move {
                        debug!(
                            identifier = %id,
                            "Starting sync task for identifier"
                        );

                        let complete = sync_identifier(
                            ctx.as_ref(),
                            &id,
                            &throttle_manager,
                            git_naughty_list.as_ref(),
                        )
                        .await;

                        // Check final state and update queue
                        if complete || !purgatory.has_pending_events(&id) {
                            purgatory.sync_queue.remove(&id);
                            info!(
                                identifier = %id,
                                complete = complete,
                                "Sync complete - removed from sync queue"
                            );
                        } else {
                            // Apply backoff - will retry later
                            // (throttled domains are being processed independently by ThrottleManager)
                            if let Some(mut entry) = purgatory.sync_queue.get_mut(&id) {
                                entry.on_sync_complete();
                                debug!(
                                    identifier = %id,
                                    attempt_count = entry.attempt_count,
                                    next_backoff_secs = entry.backoff().as_secs(),
                                    "Sync incomplete - applying backoff"
                                );
                            }
                        }
                    });
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    // Note: The sync loop is tested via integration tests rather than unit tests
    // because testing async loops with timing is fragile and prone to flakiness.
    //
    // Integration tests in tests/purgatory_sync.rs verify:
    // - State events sync from remote
    // - PR events sync from remote
    // - Concurrent state and PR sync
    // - Partial OID aggregation from multiple servers
    // - Push triggers unified processing
    //
    // The individual components (SyncQueueEntry, ThrottleManager, sync_identifier)
    // are thoroughly unit tested in their respective modules.
}
