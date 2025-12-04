//! Prometheus metrics for ngit-grasp relay.
//!
//! This module provides comprehensive monitoring metrics including:
//! - WebSocket connection tracking (with privacy-preserving IP aggregation)
//! - Git operation metrics (clone, fetch, push)
//! - Repository bandwidth tracking (top-N only for cardinality control)
//! - Nostr event metrics
//! - Sync metrics (GRASP-02 proactive sync)
//!
//! # Privacy
//! IP addresses are NEVER exposed in metrics. The `ConnectionTracker` maintains
//! per-IP counts internally only for abuse detection. Only aggregate counts
//! are exposed to Prometheus.

pub mod bandwidth;
pub mod connection;

pub use crate::sync::SyncMetrics;

use std::sync::Arc;
use std::time::Instant;

use lazy_static::lazy_static;
use prometheus::{
    Counter, CounterVec, Encoder, Gauge, GaugeVec, Histogram, HistogramOpts, HistogramVec, Opts,
    Registry, TextEncoder,
};

use bandwidth::BandwidthTracker;
use connection::ConnectionTracker;

lazy_static! {
    /// Global Prometheus registry for ngit-grasp metrics
    pub static ref REGISTRY: Registry = Registry::new();
}

/// Central metrics collection for ngit-grasp relay.
///
/// Thread-safe and designed for concurrent access from multiple tokio tasks.
#[derive(Clone)]
pub struct Metrics {
    inner: Arc<MetricsInner>,
}

struct MetricsInner {
    /// Connection tracking with abuse detection
    pub connection_tracker: ConnectionTracker,

    /// Repository bandwidth tracking (top-N only)
    pub bandwidth_tracker: BandwidthTracker,

    /// Sync metrics (GRASP-02 proactive sync)
    pub sync_metrics: Option<crate::sync::SyncMetrics>,

    // === WebSocket Metrics ===
    /// Total WebSocket connections since startup
    pub websocket_connections_total: Counter,
    /// Connection duration histogram
    pub websocket_connection_duration: Histogram,
    /// Messages received by type (REQ, EVENT, CLOSE)
    pub websocket_messages_received: CounterVec,
    /// Messages sent by type (EVENT, EOSE, OK, NOTICE)
    pub websocket_messages_sent: CounterVec,

    // === Git Operation Metrics ===
    /// Git operations by type and status
    pub git_operations_total: CounterVec,
    /// Git operation duration histogram
    pub git_operation_duration: HistogramVec,
    /// Total bytes transferred
    pub git_bytes_total: CounterVec,
    /// Push authorization results
    pub git_push_authorization: CounterVec,

    // === Nostr Event Metrics ===
    /// Events received by kind
    pub events_received_total: CounterVec,
    /// Events successfully stored by kind
    pub events_stored_total: CounterVec,
    /// Events rejected by kind and reason
    pub events_rejected_total: CounterVec,

    // === Repository Metrics ===
    /// Total repositories hosted
    pub repositories_total: Gauge,

    // === System Health Metrics ===
    /// Server start time for uptime calculation
    pub start_time: Instant,
    /// Build information gauge
    pub build_info: GaugeVec,
}

impl Metrics {
    /// Creates a new Metrics instance and registers all metrics with Prometheus.
    ///
    /// # Arguments
    /// * `abuse_threshold` - Number of connections from a single IP before flagging as abuse
    pub fn new(abuse_threshold: u32) -> Self {
        let inner = MetricsInner::new(abuse_threshold);
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Returns the sync metrics if registered.
    pub fn sync_metrics(&self) -> Option<&crate::sync::SyncMetrics> {
        self.inner.sync_metrics.as_ref()
    }

    /// Returns the connection tracker for WebSocket connection management.
    pub fn connection_tracker(&self) -> &ConnectionTracker {
        &self.inner.connection_tracker
    }

    /// Returns the bandwidth tracker for repository bandwidth tracking.
    pub fn bandwidth_tracker(&self) -> &BandwidthTracker {
        &self.inner.bandwidth_tracker
    }

    // === WebSocket Recording Methods ===

    /// Record a new WebSocket connection
    pub fn record_websocket_connection(&self) {
        self.inner.websocket_connections_total.inc();
    }

    /// Start timing a WebSocket connection, returns timer that records on drop
    pub fn start_connection_timer(&self) -> HistogramTimer {
        HistogramTimer::new(self.inner.websocket_connection_duration.clone())
    }

    /// Record a received WebSocket message
    pub fn record_message_received(&self, msg_type: &str) {
        self.inner
            .websocket_messages_received
            .with_label_values(&[msg_type])
            .inc();
    }

    /// Record a sent WebSocket message
    pub fn record_message_sent(&self, msg_type: &str) {
        self.inner
            .websocket_messages_sent
            .with_label_values(&[msg_type])
            .inc();
    }

    // === Git Operation Recording Methods ===

    /// Record a git operation completion
    pub fn record_git_operation(&self, operation: &str, status: &str) {
        self.inner
            .git_operations_total
            .with_label_values(&[operation, status])
            .inc();
    }

    /// Start timing a git operation, returns a timer
    pub fn start_git_operation_timer(&self, operation: &str) -> GitOperationTimer {
        GitOperationTimer::new(self.inner.git_operation_duration.clone(), operation.to_string())
    }

    /// Record bytes transferred for a git operation
    pub fn record_git_bytes(&self, direction: &str, bytes: u64) {
        self.inner
            .git_bytes_total
            .with_label_values(&[direction])
            .inc_by(bytes as f64);
    }

    /// Record a push authorization result
    pub fn record_push_authorization(&self, result: &str) {
        self.inner
            .git_push_authorization
            .with_label_values(&[result])
            .inc();
    }

    // === Nostr Event Recording Methods ===

    /// Record a received Nostr event
    pub fn record_event_received(&self, kind: u64) {
        self.inner
            .events_received_total
            .with_label_values(&[&kind.to_string()])
            .inc();
    }

    /// Record a stored Nostr event
    pub fn record_event_stored(&self, kind: u64) {
        self.inner
            .events_stored_total
            .with_label_values(&[&kind.to_string()])
            .inc();
    }

    /// Record a rejected Nostr event
    pub fn record_event_rejected(&self, kind: u64, reason: &str) {
        self.inner
            .events_rejected_total
            .with_label_values(&[&kind.to_string(), reason])
            .inc();
    }

    // === Repository Metrics ===

    /// Set the total number of repositories
    pub fn set_repositories_total(&self, count: u64) {
        self.inner.repositories_total.set(count as f64);
    }

    /// Increment the repository count
    pub fn inc_repositories_total(&self) {
        self.inner.repositories_total.inc();
    }

    // === Rendering ===

    /// Render all metrics in Prometheus text format.
    ///
    /// This method:
    /// 1. Refreshes the top-N bandwidth metrics if needed
    /// 2. Updates uptime
    /// 3. Gathers all metrics from the registry
    /// 4. Encodes them in Prometheus text format
    pub fn render(&self) -> String {
        // Refresh top-N bandwidth repos if needed
        self.inner.bandwidth_tracker.maybe_refresh_top_n();

        // Gather and encode metrics
        let encoder = TextEncoder::new();
        let metric_families = REGISTRY.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();

        // Add uptime as a comment (it's derived, not a registered metric)
        let uptime = self.inner.start_time.elapsed().as_secs();
        let mut output = String::from_utf8(buffer).unwrap();
        output.push_str(&format!(
            "\n# HELP ngit_uptime_seconds Seconds since server startup\n# TYPE ngit_uptime_seconds counter\nngit_uptime_seconds {}\n",
            uptime
        ));

        output
    }

    /// Check if the system is under high load (for sync scheduling)
    pub fn is_high_load(&self, threshold: u64) -> bool {
        self.inner.connection_tracker.active_connections() > threshold
    }
}

impl MetricsInner {
    fn new(abuse_threshold: u32) -> Self {
        // Create connection tracker
        let connection_tracker = ConnectionTracker::new(abuse_threshold, &REGISTRY);

        // Create bandwidth tracker
        let bandwidth_tracker = BandwidthTracker::new(&REGISTRY);

        // Create sync metrics (may fail if already registered in tests)
        let sync_metrics = crate::sync::SyncMetrics::register(&REGISTRY).ok();
        if sync_metrics.is_some() {
            tracing::info!("Sync metrics registered with Prometheus");
        }

        // WebSocket metrics
        let websocket_connections_total = Counter::with_opts(
            Opts::new(
                "ngit_websocket_connections_total",
                "Total WebSocket connections since startup",
            )
        ).unwrap();
        REGISTRY.register(Box::new(websocket_connections_total.clone())).unwrap();

        let websocket_connection_duration = Histogram::with_opts(
            HistogramOpts::new(
                "ngit_websocket_connection_duration_seconds",
                "Duration of WebSocket connections",
            )
            .buckets(vec![1.0, 5.0, 15.0, 30.0, 60.0, 300.0, 900.0, 3600.0]),
        ).unwrap();
        REGISTRY.register(Box::new(websocket_connection_duration.clone())).unwrap();

        let websocket_messages_received = CounterVec::new(
            Opts::new(
                "ngit_websocket_messages_received_total",
                "WebSocket messages received by type",
            ),
            &["type"],
        ).unwrap();
        REGISTRY.register(Box::new(websocket_messages_received.clone())).unwrap();

        let websocket_messages_sent = CounterVec::new(
            Opts::new(
                "ngit_websocket_messages_sent_total",
                "WebSocket messages sent by type",
            ),
            &["type"],
        ).unwrap();
        REGISTRY.register(Box::new(websocket_messages_sent.clone())).unwrap();

        // Git operation metrics
        let git_operations_total = CounterVec::new(
            Opts::new(
                "ngit_git_operations_total",
                "Git operations by type and status",
            ),
            &["operation", "status"],
        ).unwrap();
        REGISTRY.register(Box::new(git_operations_total.clone())).unwrap();

        let git_operation_duration = HistogramVec::new(
            HistogramOpts::new(
                "ngit_git_operation_duration_seconds",
                "Duration of git operations",
            )
            .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]),
            &["operation"],
        ).unwrap();
        REGISTRY.register(Box::new(git_operation_duration.clone())).unwrap();

        let git_bytes_total = CounterVec::new(
            Opts::new(
                "ngit_git_bytes_total",
                "Total bytes transferred for git operations",
            ),
            &["direction"],
        ).unwrap();
        REGISTRY.register(Box::new(git_bytes_total.clone())).unwrap();

        let git_push_authorization = CounterVec::new(
            Opts::new(
                "ngit_git_push_authorization_total",
                "Push authorization results",
            ),
            &["result"],
        ).unwrap();
        REGISTRY.register(Box::new(git_push_authorization.clone())).unwrap();

        // Nostr event metrics
        let events_received_total = CounterVec::new(
            Opts::new(
                "ngit_events_received_total",
                "Nostr events received by kind",
            ),
            &["kind"],
        ).unwrap();
        REGISTRY.register(Box::new(events_received_total.clone())).unwrap();

        let events_stored_total = CounterVec::new(
            Opts::new(
                "ngit_events_stored_total",
                "Nostr events successfully stored by kind",
            ),
            &["kind"],
        ).unwrap();
        REGISTRY.register(Box::new(events_stored_total.clone())).unwrap();

        let events_rejected_total = CounterVec::new(
            Opts::new(
                "ngit_events_rejected_total",
                "Nostr events rejected by kind and reason",
            ),
            &["kind", "reason"],
        ).unwrap();
        REGISTRY.register(Box::new(events_rejected_total.clone())).unwrap();

        // Repository metrics
        let repositories_total = Gauge::with_opts(
            Opts::new(
                "ngit_repositories_total",
                "Total repositories hosted",
            )
        ).unwrap();
        REGISTRY.register(Box::new(repositories_total.clone())).unwrap();

        // Build info
        let build_info = GaugeVec::new(
            Opts::new(
                "ngit_build_info",
                "Build information",
            ),
            &["version", "commit"],
        ).unwrap();
        REGISTRY.register(Box::new(build_info.clone())).unwrap();
        
        // Set build info gauge to 1 (it's just for labels)
        build_info
            .with_label_values(&[env!("CARGO_PKG_VERSION"), option_env!("GIT_HASH").unwrap_or("unknown")])
            .set(1.0);

        Self {
            connection_tracker,
            bandwidth_tracker,
            sync_metrics,
            websocket_connections_total,
            websocket_connection_duration,
            websocket_messages_received,
            websocket_messages_sent,
            git_operations_total,
            git_operation_duration,
            git_bytes_total,
            git_push_authorization,
            events_received_total,
            events_stored_total,
            events_rejected_total,
            repositories_total,
            start_time: Instant::now(),
            build_info,
        }
    }
}

/// Timer for tracking WebSocket connection duration.
/// Records the elapsed time when dropped.
pub struct HistogramTimer {
    histogram: Histogram,
    start: Instant,
}

impl HistogramTimer {
    fn new(histogram: Histogram) -> Self {
        Self {
            histogram,
            start: Instant::now(),
        }
    }
}

impl Drop for HistogramTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_secs_f64();
        self.histogram.observe(elapsed);
    }
}

/// Timer for tracking Git operation duration.
/// Records the elapsed time when dropped.
pub struct GitOperationTimer {
    histogram_vec: HistogramVec,
    operation: String,
    start: Instant,
}

impl GitOperationTimer {
    fn new(histogram_vec: HistogramVec, operation: String) -> Self {
        Self {
            histogram_vec,
            operation,
            start: Instant::now(),
        }
    }
}

impl Drop for GitOperationTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_secs_f64();
        self.histogram_vec
            .with_label_values(&[&self.operation])
            .observe(elapsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        // Note: This test may fail if run with other tests due to global registry
        // In production, consider using a test-specific registry
        let metrics = Metrics::new(10);
        
        // Test that we can record metrics without panicking
        metrics.record_websocket_connection();
        metrics.record_message_received("REQ");
        metrics.record_message_sent("EVENT");
        metrics.record_git_operation("clone", "success");
        metrics.record_git_bytes("in", 1024);
        metrics.record_event_received(1);
        metrics.record_event_stored(1);
        metrics.record_event_rejected(1, "invalid_signature");
        metrics.set_repositories_total(5);
    }
}