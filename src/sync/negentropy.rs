//! Negentropy Catchup Service for GRASP-02 Phase 5
//!
//! Implements gap-filling synchronization to ensure no events are missed during:
//! - Startup (initial sync after warm-up period)
//! - Reconnection (after connection restore)
//! - Daily maintenance (periodic full reconciliation)
//!
//! ## Note on NIP-77
//!
//! This implementation uses a simplified gap-filling strategy (fetch and compare)
//! rather than full NIP-77 negentropy set reconciliation. The nostr-sdk 0.44 does
//! not include built-in negentropy support, so we implement an equivalent approach:
//!
//! 1. Fetch events from relay using same filters as live sync
//! 2. Compare with local database (skip already-stored events)
//! 3. Validate and store missing events through policy
//!
//! Full NIP-77 support can be added in a future release if needed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nostr_relay_builder::prelude::*;
use nostr_sdk::prelude::*;
use tokio::sync::RwLock;

use super::filter::FilterService;
use super::SYNC_SOURCE_ADDR;
use crate::config::Config;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};

/// Default startup delay before first catchup (30 seconds)
const DEFAULT_STARTUP_DELAY_SECS: u64 = 30;

/// Default delay after reconnection before catchup (10 seconds)
const DEFAULT_RECONNECT_DELAY_SECS: u64 = 10;

/// Default lookback period for reconnect catchup (3 days)
const DEFAULT_RECONNECT_LOOKBACK_DAYS: u64 = 3;

/// Daily catchup interval (24 hours)
const DAILY_CATCHUP_INTERVAL_SECS: u64 = 86400;

/// Stagger delay between relays for catchup operations (5 minutes)
const RELAY_STAGGER_SECS: u64 = 300;

/// Timeout for fetching events during catchup
const CATCHUP_FETCH_TIMEOUT_SECS: u64 = 60;

/// Negentropy Catchup Service
///
/// Manages gap-filling operations for different scenarios:
/// - Startup catchup after warm-up period
/// - Reconnect catchup after connection restore
/// - Daily catchup for periodic maintenance
#[derive(Debug)]
pub struct NegentropyService {
    /// Database for storing and querying events
    database: SharedDatabase,
    /// Filter service for building catchup filters
    filter_service: Arc<FilterService>,
    /// Write policy for validating synced events
    write_policy: Nip34WritePolicy,
    /// Startup time of the service
    startup_time: Instant,
    /// Configuration values
    startup_delay_secs: u64,
    reconnect_delay_secs: u64,
    reconnect_lookback_days: u64,
    /// Whether startup catchup has been run
    startup_catchup_completed: Arc<RwLock<bool>>,
    /// Last daily catchup time per relay
    last_daily_catchup: Arc<RwLock<HashMap<String, Instant>>>,
}

impl NegentropyService {
    /// Create a new NegentropyService
    ///
    /// # Arguments
    /// * `database` - Shared database for storing events
    /// * `filter_service` - Filter service for building catchup filters
    /// * `write_policy` - Write policy for validating events
    /// * `config` - Configuration for catchup timing
    pub fn new(
        database: SharedDatabase,
        filter_service: Arc<FilterService>,
        write_policy: Nip34WritePolicy,
        config: &Config,
    ) -> Self {
        Self {
            database,
            filter_service,
            write_policy,
            startup_time: Instant::now(),
            startup_delay_secs: config.sync_startup_delay_secs,
            reconnect_delay_secs: config.sync_reconnect_delay_secs,
            reconnect_lookback_days: config.sync_reconnect_lookback_days,
            startup_catchup_completed: Arc::new(RwLock::new(false)),
            last_daily_catchup: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a NegentropyService with default configuration
    pub fn with_defaults(
        database: SharedDatabase,
        filter_service: Arc<FilterService>,
        write_policy: Nip34WritePolicy,
    ) -> Self {
        Self {
            database,
            filter_service,
            write_policy,
            startup_time: Instant::now(),
            startup_delay_secs: DEFAULT_STARTUP_DELAY_SECS,
            reconnect_delay_secs: DEFAULT_RECONNECT_DELAY_SECS,
            reconnect_lookback_days: DEFAULT_RECONNECT_LOOKBACK_DAYS,
            startup_catchup_completed: Arc::new(RwLock::new(false)),
            last_daily_catchup: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if startup catchup should run
    ///
    /// Returns true if:
    /// - Startup delay has elapsed (default 30s)
    /// - Startup catchup hasn't been completed yet
    pub async fn should_run_startup_catchup(&self) -> bool {
        let completed = *self.startup_catchup_completed.read().await;
        if completed {
            return false;
        }

        let elapsed = self.startup_time.elapsed();
        elapsed >= Duration::from_secs(self.startup_delay_secs)
    }

    /// Check if daily catchup should run for a specific relay
    ///
    /// Returns true if 24 hours have elapsed since last daily catchup
    pub async fn should_run_daily_catchup(&self, relay_url: &str) -> bool {
        let last_catchup = self.last_daily_catchup.read().await;
        
        match last_catchup.get(relay_url) {
            None => true, // Never run, should run
            Some(last_time) => {
                last_time.elapsed() >= Duration::from_secs(DAILY_CATCHUP_INTERVAL_SECS)
            }
        }
    }

    /// Get the startup delay in seconds
    pub fn startup_delay_secs(&self) -> u64 {
        self.startup_delay_secs
    }

    /// Get the reconnect delay in seconds
    pub fn reconnect_delay_secs(&self) -> u64 {
        self.reconnect_delay_secs
    }

    /// Get the relay stagger delay in seconds
    pub fn relay_stagger_secs(&self) -> u64 {
        RELAY_STAGGER_SECS
    }

    /// Run startup catchup for a relay
    ///
    /// Fetches all events matching the sync filters and stores any missing ones.
    /// This is called after the startup warm-up period (default 30s).
    ///
    /// Returns the count of gap events filled.
    pub async fn run_startup_catchup(
        &self,
        relay_url: &str,
        remote_domain: &str,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Starting startup catchup for {}", relay_url);

        // Run full catchup (no time restriction)
        let gap_count = self
            .run_catchup(relay_url, remote_domain, None, "startup")
            .await?;

        // Mark startup catchup as completed
        {
            let mut completed = self.startup_catchup_completed.write().await;
            *completed = true;
        }

        if gap_count > 0 {
            tracing::warn!(
                "Startup catchup filled {} gaps from {}",
                gap_count,
                relay_url
            );
        } else {
            tracing::info!("Startup catchup completed for {} (no gaps)", relay_url);
        }

        Ok(gap_count)
    }

    /// Run reconnect catchup for a relay
    ///
    /// Fetches events from the last 3 days (configurable) and stores any missing ones.
    /// This is called after a connection is restored (after reconnect delay).
    ///
    /// Returns the count of gap events filled.
    pub async fn run_reconnect_catchup(
        &self,
        relay_url: &str,
        remote_domain: &str,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Starting reconnect catchup for {}", relay_url);

        // Calculate "since" timestamp (3 days ago)
        let lookback_secs = self.reconnect_lookback_days * 24 * 60 * 60;
        let since = Timestamp::now() - lookback_secs;

        let gap_count = self
            .run_catchup(relay_url, remote_domain, Some(since), "reconnect")
            .await?;

        if gap_count > 0 {
            tracing::warn!(
                "Reconnect catchup filled {} gaps from {}",
                gap_count,
                relay_url
            );
        } else {
            tracing::debug!("Reconnect catchup completed for {} (no gaps)", relay_url);
        }

        Ok(gap_count)
    }

    /// Run daily catchup for a relay
    ///
    /// Performs full reconciliation and stores any missing events.
    /// This is called once per day per relay (with stagger).
    ///
    /// Returns the count of gap events filled.
    pub async fn run_daily_catchup(
        &self,
        relay_url: &str,
        remote_domain: &str,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Starting daily catchup for {}", relay_url);

        // Run full catchup (no time restriction)
        let gap_count = self
            .run_catchup(relay_url, remote_domain, None, "daily")
            .await?;

        // Update last daily catchup time
        {
            let mut last_catchup = self.last_daily_catchup.write().await;
            last_catchup.insert(relay_url.to_string(), Instant::now());
        }

        if gap_count > 0 {
            tracing::warn!(
                "Daily catchup filled {} gaps from {}",
                gap_count,
                relay_url
            );
        } else {
            tracing::info!("Daily catchup completed for {} (no gaps)", relay_url);
        }

        Ok(gap_count)
    }

    /// Core catchup implementation
    ///
    /// Fetches events from relay matching sync filters, compares with local database,
    /// validates through policy, and stores missing events.
    ///
    /// # Arguments
    /// * `relay_url` - URL of the relay to fetch from
    /// * `remote_domain` - Domain of the remote relay (for filter building)
    /// * `since` - Optional timestamp to filter events (for reconnect catchup)
    /// * `catchup_type` - Type of catchup for logging ("startup", "reconnect", "daily")
    async fn run_catchup(
        &self,
        relay_url: &str,
        remote_domain: &str,
        since: Option<Timestamp>,
        catchup_type: &str,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        // Create a client for fetching events
        let client = Client::default();
        client.add_relay(relay_url).await?;
        client.connect().await;

        let mut gap_count = 0;

        // Build filters (same as live sync uses)
        let mut all_filters = Vec::new();

        // Layer 1: Announcement discovery
        let layer1_filters = self.filter_service.get_layer1_filters();
        all_filters.extend(layer1_filters);

        // Layer 2: Repository events
        let layer2_filters = self.filter_service.get_layer2_filters(remote_domain).await;
        all_filters.extend(layer2_filters);

        // Layer 3: Related events
        let layer3_filters = self.filter_service.get_layer3_filters().await;
        all_filters.extend(layer3_filters);

        // Apply "since" filter if specified (for reconnect catchup)
        let filters: Vec<Filter> = if let Some(since_ts) = since {
            all_filters
                .into_iter()
                .map(|f| f.since(since_ts))
                .collect()
        } else {
            all_filters
        };

        if filters.is_empty() {
            tracing::debug!("No filters for {} catchup on {}", catchup_type, relay_url);
            client.disconnect().await;
            return Ok(0);
        }

        tracing::debug!(
            "Running {} catchup on {} with {} filters",
            catchup_type,
            relay_url,
            filters.len()
        );

        // Fetch events for each filter
        for filter in filters {
            match client
                .fetch_events(filter, Duration::from_secs(CATCHUP_FETCH_TIMEOUT_SECS))
                .await
            {
                Ok(events) => {
                    for event in events.into_iter() {
                        // Check if event already exists in local database
                        if self.event_exists_locally(&event).await {
                            continue;
                        }

                        // Validate through write policy
                        let result = self
                            .write_policy
                            .admit_event(&event, &SYNC_SOURCE_ADDR)
                            .await;

                        match result {
                            PolicyResult::Accept => {
                                // Log gap event at WARN level to distinguish from live events
                                tracing::warn!(
                                    "Gap event filled via {} catchup: {} (kind {})",
                                    catchup_type,
                                    event.id.to_hex(),
                                    event.kind.as_u16()
                                );

                                // Store the event
                                if let Err(e) = self.database.save_event(&event).await {
                                    tracing::error!(
                                        "Failed to store gap event {}: {}",
                                        event.id.to_hex(),
                                        e
                                    );
                                } else {
                                    gap_count += 1;
                                }
                            }
                            PolicyResult::Reject(reason) => {
                                tracing::debug!(
                                    "Gap event {} rejected by policy: {}",
                                    event.id.to_hex(),
                                    reason
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to fetch events for {} catchup from {}: {}",
                        catchup_type,
                        relay_url,
                        e
                    );
                }
            }
        }

        client.disconnect().await;

        Ok(gap_count)
    }

    /// Check if an event already exists in the local database
    async fn event_exists_locally(&self, event: &Event) -> bool {
        // Query for the specific event by ID
        let filter = Filter::new().id(event.id);

        match self.database.query(filter).await {
            Ok(events) => !events.is_empty(),
            Err(e) => {
                tracing::warn!(
                    "Failed to check if event {} exists locally: {}",
                    event.id.to_hex(),
                    e
                );
                // Assume it doesn't exist to avoid skipping events on error
                false
            }
        }
    }

    /// Mark startup catchup as completed (for testing)
    #[cfg(test)]
    pub async fn mark_startup_completed(&self) {
        let mut completed = self.startup_catchup_completed.write().await;
        *completed = true;
    }

    /// Reset startup catchup status (for testing)
    #[cfg(test)]
    pub async fn reset_startup_status(&self) {
        let mut completed = self.startup_catchup_completed.write().await;
        *completed = false;
    }
}

/// Create a shared NegentropyService wrapped in Arc
pub fn create_negentropy_service(
    database: SharedDatabase,
    filter_service: Arc<FilterService>,
    write_policy: Nip34WritePolicy,
    config: &Config,
) -> Arc<NegentropyService> {
    Arc::new(NegentropyService::new(
        database,
        filter_service,
        write_policy,
        config,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_constants() {
        assert_eq!(DEFAULT_STARTUP_DELAY_SECS, 30);
        assert_eq!(DEFAULT_RECONNECT_DELAY_SECS, 10);
        assert_eq!(DEFAULT_RECONNECT_LOOKBACK_DAYS, 3);
        assert_eq!(DAILY_CATCHUP_INTERVAL_SECS, 86400);
        assert_eq!(RELAY_STAGGER_SECS, 300);
    }

    #[test]
    fn test_reconnect_lookback_calculation() {
        // 3 days = 3 * 24 * 60 * 60 = 259,200 seconds
        let lookback_days: u64 = 3;
        let lookback_secs = lookback_days * 24 * 60 * 60;
        assert_eq!(lookback_secs, 259200);
    }

    #[test]
    fn test_stagger_delay_is_5_minutes() {
        assert_eq!(RELAY_STAGGER_SECS, 300); // 5 * 60 = 300
    }
}