//! Prometheus Metrics for Proactive Sync (GRASP-02 Phase 6)
//!
//! This module provides comprehensive sync monitoring metrics including:
//! - Connection status and attempts per relay
//! - Health state tracking (Healthy/Degraded/Dead)
//! - Event sync tracking by source (live/startup/reconnect/daily catchup)
//! - Gap events filled during catchup operations
//!
//! All metrics follow the `ngit_sync_` prefix convention.

use prometheus::{IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry};

use super::health::HealthState;

/// Prometheus metrics for the proactive sync system
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
    /// Events synced by source (live/startup/reconnect/daily)
    events_total: IntCounterVec,
    /// Gap events filled during catchup, by relay
    gap_events_total: IntCounterVec,

    // === Summary metrics ===
    /// Total relays discovered and tracked
    relays_tracked_total: IntGauge,
    /// Currently connected relay count
    relays_connected_total: IntGauge,
    /// Relays marked as dead
    relays_dead_total: IntGauge,
}

impl SyncMetrics {
    /// Register all sync metrics with the provided Prometheus registry
    pub fn register(registry: &Registry) -> Result<Self, prometheus::Error> {
        // Connection metrics
        let relay_connected = IntGaugeVec::new(
            Opts::new(
                "ngit_sync_relay_connected",
                "Relay connection status (1=connected, 0=disconnected)",
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
                "Relay health status (1=healthy, 2=degraded, 3=dead)",
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
        let events_total = IntCounterVec::new(
            Opts::new(
                "ngit_sync_events_total",
                "Total events synced by source type",
            ),
            &["source"],
        )?;
        registry.register(Box::new(events_total.clone()))?;

        let gap_events_total = IntCounterVec::new(
            Opts::new(
                "ngit_sync_gap_events_total",
                "Gap events filled during catchup by relay",
            ),
            &["relay"],
        )?;
        registry.register(Box::new(gap_events_total.clone()))?;

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

        Ok(Self {
            relay_connected,
            connection_attempts_total,
            relay_status,
            relay_failures,
            events_total,
            gap_events_total,
            relays_tracked_total,
            relays_connected_total,
            relays_dead_total,
        })
    }

    // === Connection Recording Methods ===

    /// Record a connection attempt (success or failure)
    pub fn record_connection_attempt(&self, relay: &str, success: bool) {
        let result = if success { "success" } else { "failure" };
        self.connection_attempts_total
            .with_label_values(&[relay, result])
            .inc();
    }

    /// Set relay connection status
    pub fn set_relay_connected(&self, relay: &str, connected: bool) {
        self.relay_connected
            .with_label_values(&[relay])
            .set(if connected { 1 } else { 0 });

        // Update connected count based on all relay values
        // This is handled by update_connected_count() for accuracy
    }

    /// Update the total connected relay count
    pub fn update_connected_count(&self, count: i64) {
        self.relays_connected_total.set(count);
    }

    /// Increment connected count
    pub fn inc_connected_count(&self) {
        self.relays_connected_total.inc();
    }

    /// Decrement connected count
    pub fn dec_connected_count(&self) {
        self.relays_connected_total.dec();
    }

    // === Health Recording Methods ===

    /// Record relay health state change
    pub fn record_health_state(&self, relay: &str, state: HealthState) {
        let state_value = match state {
            HealthState::Healthy => 1,
            HealthState::Degraded => 2,
            HealthState::Dead => 3,
        };
        self.relay_status.with_label_values(&[relay]).set(state_value);
    }

    /// Record relay failure count
    pub fn record_failure_count(&self, relay: &str, count: u32) {
        self.relay_failures
            .with_label_values(&[relay])
            .set(count as i64);
    }

    /// Update dead relay count
    pub fn update_dead_count(&self, count: i64) {
        self.relays_dead_total.set(count);
    }

    /// Increment dead relay count
    pub fn inc_dead_count(&self) {
        self.relays_dead_total.inc();
    }

    /// Decrement dead relay count
    pub fn dec_dead_count(&self) {
        self.relays_dead_total.dec();
    }

    // === Event Recording Methods ===

    /// Record a synced event by source type
    ///
    /// Source types:
    /// - "live" - Real-time subscription events
    /// - "startup" - Events from startup catchup
    /// - "reconnect" - Events from reconnection catchup
    /// - "daily" - Events from daily catchup
    pub fn record_event(&self, source: &str) {
        self.events_total.with_label_values(&[source]).inc();
    }

    /// Record multiple events synced by source type
    pub fn record_events(&self, source: &str, count: u64) {
        self.events_total
            .with_label_values(&[source])
            .inc_by(count);
    }

    /// Record a gap event filled during catchup
    pub fn record_gap_event(&self, relay: &str) {
        self.gap_events_total.with_label_values(&[relay]).inc();
    }

    /// Record multiple gap events filled during catchup
    pub fn record_gap_events(&self, relay: &str, count: u64) {
        self.gap_events_total
            .with_label_values(&[relay])
            .inc_by(count);
    }

    // === Summary Recording Methods ===

    /// Set the total tracked relay count
    pub fn set_tracked_count(&self, count: i64) {
        self.relays_tracked_total.set(count);
    }

    /// Increment tracked relay count
    pub fn inc_tracked_count(&self) {
        self.relays_tracked_total.inc();
    }

    /// Get current tracked relay count
    pub fn get_tracked_count(&self) -> i64 {
        self.relays_tracked_total.get()
    }

    /// Get current connected relay count
    pub fn get_connected_count(&self) -> i64 {
        self.relays_connected_total.get()
    }

    /// Get current dead relay count
    pub fn get_dead_count(&self) -> i64 {
        self.relays_dead_total.get()
    }
}

/// Event source types for metrics tracking
pub mod event_source {
    /// Real-time subscription events
    pub const LIVE: &str = "live";
    /// Events from startup catchup
    pub const STARTUP: &str = "startup";
    /// Events from reconnection catchup  
    pub const RECONNECT: &str = "reconnect";
    /// Events from daily catchup
    pub const DAILY: &str = "daily";
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

        metrics.record_connection_attempt("wss://relay1.example.com", true);
        metrics.record_connection_attempt("wss://relay1.example.com", false);
        metrics.record_connection_attempt("wss://relay2.example.com", true);

        metrics.set_relay_connected("wss://relay1.example.com", true);
        metrics.inc_connected_count();

        assert_eq!(metrics.get_connected_count(), 1);
    }

    #[test]
    fn test_health_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        metrics.record_health_state("wss://relay1.example.com", HealthState::Healthy);
        metrics.record_health_state("wss://relay2.example.com", HealthState::Degraded);
        metrics.record_health_state("wss://relay3.example.com", HealthState::Dead);

        metrics.record_failure_count("wss://relay2.example.com", 5);
        metrics.update_dead_count(1);

        assert_eq!(metrics.get_dead_count(), 1);
    }

    #[test]
    fn test_event_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        metrics.record_event(event_source::LIVE);
        metrics.record_events(event_source::STARTUP, 10);
        metrics.record_gap_event("wss://relay1.example.com");
        metrics.record_gap_events("wss://relay2.example.com", 5);
    }

    #[test]
    fn test_summary_metrics() {
        let registry = create_test_registry();
        let metrics = SyncMetrics::register(&registry).unwrap();

        metrics.set_tracked_count(5);
        assert_eq!(metrics.get_tracked_count(), 5);

        metrics.inc_tracked_count();
        assert_eq!(metrics.get_tracked_count(), 6);

        metrics.update_connected_count(3);
        assert_eq!(metrics.get_connected_count(), 3);
    }
}