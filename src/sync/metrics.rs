//! Prometheus Metrics for Proactive Sync (GRASP-02)
//!
//! This module provides comprehensive sync monitoring metrics including:
//! - Connection status and attempts per relay
//! - Health state tracking (Healthy/Degraded/Dead)
//! - Event sync tracking (only newly saved events)
//!
//! All metrics follow the `ngit_sync_` prefix convention.

use prometheus::{IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry};

use super::health::HealthState;

/// Prometheus metrics for the proactive sync system.
///
/// Tracks relay connections, sync progress, health states, and operational statistics.
/// Designed for comprehensive monitoring of GRASP-02 proactive sync operations.
#[derive(Clone)]
pub struct SyncMetrics {
    // === Connection metrics ===
    /// Per-relay connection status (1=connected, 0=disconnected)
    relay_connected: IntGaugeVec,
    /// Connection attempts by relay and result (success/failure)
    connection_attempts_total: IntCounterVec,

    // === Health metrics ===
    /// Per-relay health status (healthy=1, degraded=2, dead=3)
    relay_status: IntGaugeVec,
    /// Per-relay consecutive failure count
    relay_failures: IntGaugeVec,

    // === Event metrics ===
    /// Total events synced (newly saved events only)
    events_synced_total: IntCounter,

    // === Summary metrics ===
    /// Total relays discovered and tracked
    relays_tracked_total: IntGauge,
    /// Currently connected relay count
    relays_connected_total: IntGauge,
    /// Relays marked as dead
    relays_dead_total: IntGauge,

    // === Rejected Announcements Index Metrics ===
    /// Current number of entries in hot cache
    rejected_announcements_hot_cache_current: IntGauge,
    /// Total hot cache hits (events re-processed from cache)
    rejected_announcements_hot_cache_hits_total: IntCounter,
    /// Total hot cache misses (events not in cache)
    rejected_announcements_hot_cache_misses_total: IntCounter,
    /// Total expired entries removed from hot cache
    rejected_announcements_hot_cache_expired_total: IntCounter,
    /// Current number of entries in cold index
    rejected_announcements_cold_index_current: IntGauge,
    /// Total cold index entries expired and removed
    rejected_announcements_cold_index_expired_total: IntCounter,
    /// Total invalidations (maintainer announcements invalidated)
    rejected_announcements_invalidated_total: IntCounter,

    // === Rejected States Index Metrics ===
    /// Current number of state events in hot cache
    rejected_states_hot_cache_current: IntGauge,
    /// Total hot cache hits (state events re-processed from cache)
    rejected_states_hot_cache_hits_total: IntCounter,
    /// Total hot cache misses (state events not in cache)
    rejected_states_hot_cache_misses_total: IntCounter,
    /// Total expired state events removed from hot cache
    rejected_states_hot_cache_expired_total: IntCounter,
    /// Current number of state event entries in cold index
    rejected_states_cold_index_current: IntGauge,
    /// Total state event cold index entries expired and removed
    rejected_states_cold_index_expired_total: IntCounter,
    /// Total state event invalidations
    rejected_states_invalidated_total: IntCounter,
}

impl SyncMetrics {
    /// Register all sync metrics with the provided Prometheus registry.
    ///
    /// # Errors
    ///
    /// Returns an error if metrics are already registered (e.g., in tests).
    pub fn register(registry: &Registry) -> Result<Self, prometheus::Error> {
        // Connection metrics
        let relay_connected = IntGaugeVec::new(
            Opts::new(
                "ngit_sync_relay_connected",
                "Relay connection status (0=disconnected, 1=connecting, 2=syncing, 3=connected, 4=connected_historic_sync_failures)",
            ),
            &["relay"],
        )?;
        registry.register(Box::new(relay_connected.clone()))?;

        let connection_attempts_total = IntCounterVec::new(
            Opts::new(
                "ngit_sync_connection_attempts_total",
                "Total connection attempts by relay and result",
            ),
            &["relay", "result"],
        )?;
        registry.register(Box::new(connection_attempts_total.clone()))?;

        // Health metrics
        let relay_status = IntGaugeVec::new(
            Opts::new(
                "ngit_sync_relay_status",
                "Relay health status (1=healthy, 2=disconnected, 3=degraded, 4=dead, 5=rate_limited)",
            ),
            &["relay"],
        )?;
        registry.register(Box::new(relay_status.clone()))?;

        let relay_failures = IntGaugeVec::new(
            Opts::new(
                "ngit_sync_relay_failures",
                "Consecutive failure count per relay",
            ),
            &["relay"],
        )?;
        registry.register(Box::new(relay_failures.clone()))?;

        // Event metrics
        let events_synced_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_events_synced_total",
            "Total events synced (newly saved events only)",
        ))?;
        registry.register(Box::new(events_synced_total.clone()))?;

        // Summary metrics
        let relays_tracked_total = IntGauge::with_opts(Opts::new(
            "ngit_sync_relays_tracked_total",
            "Total number of relays discovered and tracked",
        ))?;
        registry.register(Box::new(relays_tracked_total.clone()))?;

        let relays_connected_total = IntGauge::with_opts(Opts::new(
            "ngit_sync_relays_connected_total",
            "Number of currently connected relays",
        ))?;
        registry.register(Box::new(relays_connected_total.clone()))?;

        let relays_dead_total = IntGauge::with_opts(Opts::new(
            "ngit_sync_relays_dead_total",
            "Number of relays marked as dead",
        ))?;
        registry.register(Box::new(relays_dead_total.clone()))?;

        // Rejected announcements metrics
        let rejected_announcements_hot_cache_current = IntGauge::with_opts(Opts::new(
            "ngit_sync_rejected_announcements_hot_cache_current",
            "Current number of entries in hot cache (full events, 2 min expiry)",
        ))?;
        registry.register(Box::new(rejected_announcements_hot_cache_current.clone()))?;

        let rejected_announcements_hot_cache_hits_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_announcements_hot_cache_hits_total",
            "Total hot cache hits (events re-processed from cache)",
        ))?;
        registry.register(Box::new(
            rejected_announcements_hot_cache_hits_total.clone(),
        ))?;

        let rejected_announcements_hot_cache_misses_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_announcements_hot_cache_misses_total",
            "Total hot cache misses (events not in cache when invalidated)",
        ))?;
        registry.register(Box::new(
            rejected_announcements_hot_cache_misses_total.clone(),
        ))?;

        let rejected_announcements_hot_cache_expired_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_announcements_hot_cache_expired_total",
            "Total expired entries removed from hot cache",
        ))?;
        registry.register(Box::new(
            rejected_announcements_hot_cache_expired_total.clone(),
        ))?;

        let rejected_announcements_cold_index_current = IntGauge::with_opts(Opts::new(
            "ngit_sync_rejected_announcements_cold_index_current",
            "Current number of entries in cold index (metadata only, 7 day expiry)",
        ))?;
        registry.register(Box::new(rejected_announcements_cold_index_current.clone()))?;

        let rejected_announcements_cold_index_expired_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_announcements_cold_index_expired_total",
            "Total expired entries removed from cold index",
        ))?;
        registry.register(Box::new(
            rejected_announcements_cold_index_expired_total.clone(),
        ))?;

        let rejected_announcements_invalidated_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_announcements_invalidated_total",
            "Total invalidations (maintainer announcements invalidated when owner accepted)",
        ))?;
        registry.register(Box::new(rejected_announcements_invalidated_total.clone()))?;

        // Rejected states metrics
        let rejected_states_hot_cache_current = IntGauge::with_opts(Opts::new(
            "ngit_sync_rejected_states_hot_cache_current",
            "Current number of state events in hot cache (full events, 2 min expiry)",
        ))?;
        registry.register(Box::new(rejected_states_hot_cache_current.clone()))?;

        let rejected_states_hot_cache_hits_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_states_hot_cache_hits_total",
            "Total hot cache hits (state events re-processed from cache)",
        ))?;
        registry.register(Box::new(rejected_states_hot_cache_hits_total.clone()))?;

        let rejected_states_hot_cache_misses_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_states_hot_cache_misses_total",
            "Total hot cache misses (state events not in cache when invalidated)",
        ))?;
        registry.register(Box::new(rejected_states_hot_cache_misses_total.clone()))?;

        let rejected_states_hot_cache_expired_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_states_hot_cache_expired_total",
            "Total expired state events removed from hot cache",
        ))?;
        registry.register(Box::new(rejected_states_hot_cache_expired_total.clone()))?;

        let rejected_states_cold_index_current = IntGauge::with_opts(Opts::new(
            "ngit_sync_rejected_states_cold_index_current",
            "Current number of state event entries in cold index (metadata only, 7 day expiry)",
        ))?;
        registry.register(Box::new(rejected_states_cold_index_current.clone()))?;

        let rejected_states_cold_index_expired_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_states_cold_index_expired_total",
            "Total state event cold index entries expired and removed",
        ))?;
        registry.register(Box::new(rejected_states_cold_index_expired_total.clone()))?;

        let rejected_states_invalidated_total = IntCounter::with_opts(Opts::new(
            "ngit_sync_rejected_states_invalidated_total",
            "Total state event invalidations (when announcements accepted)",
        ))?;
        registry.register(Box::new(rejected_states_invalidated_total.clone()))?;

        Ok(Self {
            relay_connected,
            connection_attempts_total,
            relay_status,
            relay_failures,
            events_synced_total,
            relays_tracked_total,
            relays_connected_total,
            relays_dead_total,
            rejected_announcements_hot_cache_current,
            rejected_announcements_hot_cache_hits_total,
            rejected_announcements_hot_cache_misses_total,
            rejected_announcements_hot_cache_expired_total,
            rejected_announcements_cold_index_current,
            rejected_announcements_cold_index_expired_total,
            rejected_announcements_invalidated_total,
            rejected_states_hot_cache_current,
            rejected_states_hot_cache_hits_total,
            rejected_states_hot_cache_misses_total,
            rejected_states_hot_cache_expired_total,
            rejected_states_cold_index_current,
            rejected_states_cold_index_expired_total,
            rejected_states_invalidated_total,
        })
    }

    // === Connection Recording Methods ===

    /// Record a connection attempt (success or failure).
    ///
    /// # Arguments
    ///
    /// * `relay` - The relay URL
    /// * `success` - Whether the connection attempt succeeded
    pub fn record_connection_attempt(&self, relay: &str, success: bool) {
        let result = if success { "success" } else { "failure" };
        self.connection_attempts_total
            .with_label_values(&[relay, result])
            .inc();
        self.set_relay_connected(relay, success);
    }

    /// Set relay connection status.
    ///
    /// # Arguments
    ///
    /// * `relay` - The relay URL
    /// * `connected` - Whether the relay is currently connected
    pub fn set_relay_connected(&self, relay: &str, connected: bool) {
        self.relay_connected
            .with_label_values(&[relay])
            .set(if connected { 1 } else { 0 });

        // Note: Connected count should be updated via update_connected_count() for accuracy
    }

    /// Update the total connected relay count.
    ///
    /// This directly sets the count rather than deriving it from individual relay states,
    /// which is more accurate when relay connection states are managed elsewhere.
    pub fn update_connected_count(&self, count: i64) {
        self.relays_connected_total.set(count);
    }

    /// Increment connected count by one.
    pub fn inc_connected_count(&self) {
        self.relays_connected_total.inc();
    }

    /// Decrement connected count by one.
    pub fn dec_connected_count(&self) {
        self.relays_connected_total.dec();
    }

    // === Health Recording Methods ===

    /// Record relay health state change.
    ///
    /// Maps health states to numeric values for Prometheus:
    /// - Healthy = 1 (connected and stable)
    /// - Disconnected = 2 (not connected, but no issues)
    /// - Degraded = 3 (connection problems or unstable after recovery)
    /// - Dead = 4 (24h+ of failures)
    /// - RateLimited = 5 (rate limit cooldown active)
    ///
    /// # Arguments
    ///
    /// * `relay` - The relay URL
    /// * `state` - The current health state
    pub fn record_health_state(&self, relay: &str, state: HealthState) {
        let state_value = match state {
            HealthState::Healthy => 1,
            HealthState::Disconnected => 2,
            HealthState::Degraded => 3,
            HealthState::Dead => 4,
            HealthState::RateLimited => 5,
        };
        self.relay_status
            .with_label_values(&[relay])
            .set(state_value);
    }

    /// Record relay connection status change.
    ///
    /// Maps connection status to numeric values for Prometheus:
    /// - Disconnected = 0 (not connected)
    /// - Connecting = 1 (connection attempt in progress)
    /// - Syncing = 2 (connected, historic sync in progress)
    /// - Connected = 3 (connected, historic sync complete)
    /// - ConnectedHistoricSyncFailures = 4 (connected, historic sync had failures but live sync active)
    ///
    /// This is separate from health state and provides more granular connection lifecycle tracking.
    ///
    /// # Arguments
    ///
    /// * `relay` - The relay URL
    /// * `status` - The current connection status
    pub fn record_connection_status(&self, relay: &str, status: super::ConnectionStatus) {
        use super::ConnectionStatus;
        let status_value = match status {
            ConnectionStatus::Disconnected => 0,
            ConnectionStatus::Connecting => 1,
            ConnectionStatus::Syncing => 2,
            ConnectionStatus::Connected => 3,
            ConnectionStatus::ConnectedHistoricSyncFailures => 4,
        };
        self.relay_connected
            .with_label_values(&[relay])
            .set(status_value);
    }

    /// Record relay failure count.
    ///
    /// # Arguments
    ///
    /// * `relay` - The relay URL
    /// * `count` - The number of consecutive failures
    pub fn record_failure_count(&self, relay: &str, count: u32) {
        self.relay_failures
            .with_label_values(&[relay])
            .set(count as i64);
    }

    /// Update dead relay count.
    pub fn update_dead_count(&self, count: i64) {
        self.relays_dead_total.set(count);
    }

    /// Increment dead relay count by one.
    pub fn inc_dead_count(&self) {
        self.relays_dead_total.inc();
    }

    /// Decrement dead relay count by one.
    pub fn dec_dead_count(&self) {
        self.relays_dead_total.dec();
    }

    // === Event Recording Methods ===

    /// Record a successfully synced event (newly saved to database).
    ///
    /// Only events that are new AND pass write policy should be counted.
    /// Duplicates and rejected events are not counted.
    pub fn record_synced_event(&self) {
        self.events_synced_total.inc();
    }

    // === Summary Recording Methods ===

    /// Set the total tracked relay count.
    pub fn set_tracked_count(&self, count: i64) {
        self.relays_tracked_total.set(count);
    }

    /// Increment tracked relay count by one.
    pub fn inc_tracked_count(&self) {
        self.relays_tracked_total.inc();
    }

    /// Get current tracked relay count.
    pub fn get_tracked_count(&self) -> i64 {
        self.relays_tracked_total.get()
    }

    /// Get current connected relay count.
    pub fn get_connected_count(&self) -> i64 {
        self.relays_connected_total.get()
    }

    /// Get current dead relay count.
    pub fn get_dead_count(&self) -> i64 {
        self.relays_dead_total.get()
    }

    // === Rejected Announcements Recording Methods ===

    /// Update hot cache current size gauge.
    pub fn update_hot_cache_size(&self, size: usize) {
        self.rejected_announcements_hot_cache_current
            .set(size as i64);
    }

    /// Record hot cache hit (event re-processed from cache).
    pub fn record_hot_cache_hit(&self) {
        self.rejected_announcements_hot_cache_hits_total.inc();
    }

    /// Record hot cache miss (event not in cache when invalidated).
    pub fn record_hot_cache_miss(&self) {
        self.rejected_announcements_hot_cache_misses_total.inc();
    }

    /// Record hot cache expired entries.
    pub fn record_hot_cache_expired(&self, count: usize) {
        self.rejected_announcements_hot_cache_expired_total
            .inc_by(count as u64);
    }

    /// Update cold index current size gauge.
    pub fn update_cold_index_size(&self, size: usize) {
        self.rejected_announcements_cold_index_current
            .set(size as i64);
    }

    /// Record cold index expired entries.
    pub fn record_cold_index_expired(&self, count: usize) {
        self.rejected_announcements_cold_index_expired_total
            .inc_by(count as u64);
    }

    /// Record invalidation (maintainer announcement invalidated).
    pub fn record_invalidation(&self, count: usize) {
        self.rejected_announcements_invalidated_total
            .inc_by(count as u64);
    }

    // === Rejected States Recording Methods ===

    /// Update state events hot cache current size gauge.
    pub fn update_states_hot_cache_size(&self, size: usize) {
        self.rejected_states_hot_cache_current.set(size as i64);
    }

    /// Record state event hot cache hit (event re-processed from cache).
    pub fn record_states_hot_cache_hit(&self) {
        self.rejected_states_hot_cache_hits_total.inc();
    }

    /// Record state event hot cache miss (event not in cache when invalidated).
    pub fn record_states_hot_cache_miss(&self) {
        self.rejected_states_hot_cache_misses_total.inc();
    }

    /// Record state event hot cache expired entries.
    pub fn record_states_hot_cache_expired(&self, count: usize) {
        self.rejected_states_hot_cache_expired_total
            .inc_by(count as u64);
    }

    /// Update state events cold index current size gauge.
    pub fn update_states_cold_index_size(&self, size: usize) {
        self.rejected_states_cold_index_current.set(size as i64);
    }

    /// Record state event cold index expired entries.
    pub fn record_states_cold_index_expired(&self, count: usize) {
        self.rejected_states_cold_index_expired_total
            .inc_by(count as u64);
    }

    /// Record state event invalidation.
    pub fn record_states_invalidation(&self, count: usize) {
        self.rejected_states_invalidated_total.inc_by(count as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_registry() -> Registry {
        Registry::new()
    }

    #[test]
    fn test_metrics_registration() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry);
        assert!(metrics.is_ok());
    }

    #[test]
    fn test_connection_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        // Record connection attempts
        metrics.record_connection_attempt("wss://relay1.example.com", true);
        metrics.record_connection_attempt("wss://relay1.example.com", false);
        metrics.record_connection_attempt("wss://relay2.example.com", true);

        // Set relay connection status
        metrics.set_relay_connected("wss://relay1.example.com", true);
        metrics.inc_connected_count();

        assert_eq!(metrics.get_connected_count(), 1);

        // Test decrement
        metrics.dec_connected_count();
        assert_eq!(metrics.get_connected_count(), 0);
    }

    #[test]
    fn test_health_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        // Record health states
        metrics.record_health_state("wss://relay1.example.com", HealthState::Healthy);
        metrics.record_health_state("wss://relay2.example.com", HealthState::Degraded);
        metrics.record_health_state("wss://relay3.example.com", HealthState::Dead);

        // Record failure count
        metrics.record_failure_count("wss://relay2.example.com", 5);

        // Test dead count tracking
        metrics.update_dead_count(1);
        assert_eq!(metrics.get_dead_count(), 1);

        metrics.inc_dead_count();
        assert_eq!(metrics.get_dead_count(), 2);

        metrics.dec_dead_count();
        assert_eq!(metrics.get_dead_count(), 1);
    }

    #[test]
    fn test_event_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        // Record synced events
        metrics.record_synced_event();
        metrics.record_synced_event();
        metrics.record_synced_event();
    }

    #[test]
    fn test_summary_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        // Test tracked count
        metrics.set_tracked_count(5);
        assert_eq!(metrics.get_tracked_count(), 5);

        metrics.inc_tracked_count();
        assert_eq!(metrics.get_tracked_count(), 6);

        // Test connected count
        metrics.update_connected_count(3);
        assert_eq!(metrics.get_connected_count(), 3);
    }

    #[test]
    fn test_duplicate_registration_fails() {
        let registry = create_test_registry();

        // First registration should succeed
        let metrics1 = SyncMetrics::register(&registry);
        assert!(metrics1.is_ok());

        // Second registration should fail (metrics already registered)
        let metrics2 = SyncMetrics::register(&registry);
        assert!(metrics2.is_err());
    }

    #[test]
    fn test_rejected_announcements_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        // Test hot cache metrics
        metrics.update_hot_cache_size(10);
        metrics.record_hot_cache_hit();
        metrics.record_hot_cache_hit();
        metrics.record_hot_cache_miss();
        metrics.record_hot_cache_expired(5);

        // Test cold index metrics
        metrics.update_cold_index_size(100);
        metrics.record_cold_index_expired(10);

        // Test invalidation metrics
        metrics.record_invalidation(3);
        metrics.record_invalidation(2);
    }
}
