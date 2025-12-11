//! Connection tracking with privacy-preserving abuse detection.
//!
//! This module tracks WebSocket connections per IP address internally for abuse
//! detection, but NEVER exposes IP addresses in Prometheus metrics. Only aggregate
//! counts are exposed.
//!
//! # Privacy Model
//!
//! | Data | Location | Exposed? |
//! |------|----------|----------|
//! | Total connections | Prometheus | ✅ Yes |
//! | Unique IP count | Prometheus | ✅ Yes |
//! | Flagged abuser count | Prometheus | ✅ Yes |
//! | Actual IP addresses | Internal HashMap | ❌ No |
//! | IP + abuse flag | Logs (when flagged) | ⚠️ Logs only |

use std::net::IpAddr;
use std::time::Instant;

use dashmap::DashMap;
use prometheus::{IntGauge, Opts, Registry};
use tracing::warn;

/// Information about connections from a specific IP address.
struct ConnectionInfo {
    /// Number of active connections from this IP
    count: u32,
    /// When the first connection from this IP was established (for future rate limiting)
    #[allow(dead_code)]
    first_seen: Instant,
    /// Whether this IP has been flagged as potentially abusive
    flagged_as_abuse: bool,
}

/// Tracks WebSocket connections per IP with abuse detection.
///
/// # Thread Safety
///
/// Uses `DashMap` for lock-free concurrent access, as connection tracking
/// happens across multiple tokio tasks.
///
/// # Privacy
///
/// IP addresses are stored internally only for abuse detection and are
/// NEVER exposed in Prometheus metrics. Only aggregate counts are exposed:
/// - Total active connections
/// - Number of unique IPs
/// - Number of IPs flagged as potential abusers
pub struct ConnectionTracker {
    /// Active connections per IP (INTERNAL ONLY - never exposed to metrics)
    connections: DashMap<IpAddr, ConnectionInfo>,

    /// Threshold for abuse flagging (connections per IP)
    abuse_threshold: u32,

    /// Prometheus gauge: total active connections
    active_connections: IntGauge,

    /// Prometheus gauge: number of unique IPs connected
    unique_ips: IntGauge,

    /// Prometheus gauge: number of IPs flagged as potential abusers
    flagged_abusers: IntGauge,
}

impl ConnectionTracker {
    /// Creates a new ConnectionTracker and registers metrics with Prometheus.
    ///
    /// # Arguments
    ///
    /// * `abuse_threshold` - Number of connections from a single IP before flagging
    /// * `registry` - Prometheus registry to register metrics with
    pub fn new(abuse_threshold: u32, registry: &Registry) -> Self {
        let active_connections = IntGauge::with_opts(Opts::new(
            "ngit_websocket_connections_active",
            "Current active WebSocket connections",
        ))
        .unwrap();
        registry
            .register(Box::new(active_connections.clone()))
            .unwrap();

        let unique_ips = IntGauge::with_opts(Opts::new(
            "ngit_websocket_unique_ips",
            "Number of unique IP addresses connected (NOT the IPs themselves)",
        ))
        .unwrap();
        registry.register(Box::new(unique_ips.clone())).unwrap();

        let flagged_abusers = IntGauge::with_opts(Opts::new(
            "ngit_websocket_flagged_abusers",
            "Number of IPs exceeding connection threshold",
        ))
        .unwrap();
        registry
            .register(Box::new(flagged_abusers.clone()))
            .unwrap();

        Self {
            connections: DashMap::new(),
            abuse_threshold,
            active_connections,
            unique_ips,
            flagged_abusers,
        }
    }

    /// Called when a new WebSocket connection is established.
    ///
    /// This method:
    /// 1. Increments the connection count for this IP
    /// 2. Checks if the IP has exceeded the abuse threshold
    /// 3. Logs a warning if abuse is detected (IP is logged here only)
    /// 4. Updates Prometheus metrics (aggregate counts only)
    ///
    /// # Privacy
    ///
    /// The IP address is logged only when abuse is detected. It is NEVER
    /// exposed in Prometheus metrics.
    pub fn on_connect(&self, ip: IpAddr) {
        let mut is_new_ip = false;
        let mut newly_flagged = false;

        self.connections
            .entry(ip)
            .and_modify(|info| {
                info.count += 1;
                // Check if this connection pushes us over the threshold
                if !info.flagged_as_abuse && info.count >= self.abuse_threshold {
                    info.flagged_as_abuse = true;
                    newly_flagged = true;
                }
            })
            .or_insert_with(|| {
                is_new_ip = true;
                ConnectionInfo {
                    count: 1,
                    first_seen: Instant::now(),
                    flagged_as_abuse: false,
                }
            });

        // Update Prometheus metrics (aggregate counts only)
        self.active_connections.inc();

        if is_new_ip {
            self.unique_ips.inc();
        }

        if newly_flagged {
            self.flagged_abusers.inc();
            // Log the abuse detection - IP is only exposed in logs, not metrics
            warn!(
                ip = %ip,
                threshold = self.abuse_threshold,
                "Potential abuse detected: IP exceeded connection threshold"
            );
        }
    }

    /// Called when a WebSocket connection is closed.
    ///
    /// This method:
    /// 1. Decrements the connection count for this IP
    /// 2. Removes the IP from tracking if count reaches 0
    /// 3. Updates the abuse flag count if the IP was flagged
    /// 4. Updates Prometheus metrics (aggregate counts only)
    pub fn on_disconnect(&self, ip: IpAddr) {
        let mut remove_entry = false;
        let mut was_flagged = false;
        let mut had_connection = false;

        if let Some(mut entry) = self.connections.get_mut(&ip) {
            had_connection = true;
            entry.count = entry.count.saturating_sub(1);
            if entry.count == 0 {
                remove_entry = true;
                was_flagged = entry.flagged_as_abuse;
            }
        }

        // Remove the entry if count is 0
        if remove_entry {
            self.connections.remove(&ip);
            self.unique_ips.dec();
            if was_flagged {
                self.flagged_abusers.dec();
            }
        }

        // Update total connections only if this IP had a tracked connection
        if had_connection {
            self.active_connections.dec();
        }
    }

    /// Returns the current number of active connections.
    pub fn active_connections(&self) -> u64 {
        self.active_connections.get() as u64
    }

    /// Returns the current number of unique IPs.
    pub fn unique_ip_count(&self) -> u64 {
        self.unique_ips.get() as u64
    }

    /// Returns the current number of flagged abusers.
    pub fn flagged_abuser_count(&self) -> u64 {
        self.flagged_abusers.get() as u64
    }

    /// Returns the connection count for a specific IP (for internal use only).
    ///
    /// # Privacy
    ///
    /// This is an internal method. The returned data should NEVER be exposed
    /// in metrics or logs without privacy consideration.
    #[cfg(test)]
    pub(crate) fn connection_count(&self, ip: &IpAddr) -> Option<u32> {
        self.connections.get(ip).map(|info| info.count)
    }

    /// Returns whether an IP is flagged as abusive (for internal use only).
    #[cfg(test)]
    pub(crate) fn is_flagged(&self, ip: &IpAddr) -> bool {
        self.connections
            .get(ip)
            .map(|info| info.flagged_as_abuse)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn test_registry() -> Registry {
        Registry::new()
    }

    #[test]
    fn test_connection_tracking() {
        let registry = test_registry();
        let tracker = ConnectionTracker::new(5, &registry);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        // Connect
        tracker.on_connect(ip);
        assert_eq!(tracker.active_connections(), 1);
        assert_eq!(tracker.unique_ip_count(), 1);
        assert_eq!(tracker.connection_count(&ip), Some(1));

        // Connect again from same IP
        tracker.on_connect(ip);
        assert_eq!(tracker.active_connections(), 2);
        assert_eq!(tracker.unique_ip_count(), 1); // Still 1 unique IP
        assert_eq!(tracker.connection_count(&ip), Some(2));

        // Disconnect one
        tracker.on_disconnect(ip);
        assert_eq!(tracker.active_connections(), 1);
        assert_eq!(tracker.unique_ip_count(), 1);
        assert_eq!(tracker.connection_count(&ip), Some(1));

        // Disconnect last
        tracker.on_disconnect(ip);
        assert_eq!(tracker.active_connections(), 0);
        assert_eq!(tracker.unique_ip_count(), 0);
        assert_eq!(tracker.connection_count(&ip), None);
    }

    #[test]
    fn test_multiple_ips() {
        let registry = test_registry();
        let tracker = ConnectionTracker::new(5, &registry);
        let ip1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));
        let ip3 = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));

        tracker.on_connect(ip1);
        tracker.on_connect(ip2);
        tracker.on_connect(ip3);

        assert_eq!(tracker.active_connections(), 3);
        assert_eq!(tracker.unique_ip_count(), 3);

        tracker.on_disconnect(ip2);
        assert_eq!(tracker.active_connections(), 2);
        assert_eq!(tracker.unique_ip_count(), 2);
    }

    #[test]
    fn test_abuse_detection() {
        let registry = test_registry();
        let threshold = 3;
        let tracker = ConnectionTracker::new(threshold, &registry);
        let abuser_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let normal_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        // Normal user with 1 connection
        tracker.on_connect(normal_ip);
        assert!(!tracker.is_flagged(&normal_ip));
        assert_eq!(tracker.flagged_abuser_count(), 0);

        // Abuser approaching threshold
        tracker.on_connect(abuser_ip);
        tracker.on_connect(abuser_ip);
        assert!(!tracker.is_flagged(&abuser_ip));
        assert_eq!(tracker.flagged_abuser_count(), 0);

        // Abuser hits threshold
        tracker.on_connect(abuser_ip);
        assert!(tracker.is_flagged(&abuser_ip));
        assert_eq!(tracker.flagged_abuser_count(), 1);

        // Normal user still not flagged
        assert!(!tracker.is_flagged(&normal_ip));

        // Abuser disconnects all - should be removed from flagged count
        tracker.on_disconnect(abuser_ip);
        tracker.on_disconnect(abuser_ip);
        tracker.on_disconnect(abuser_ip);
        assert_eq!(tracker.flagged_abuser_count(), 0);
        assert_eq!(tracker.active_connections(), 1); // Only normal user remains
    }

    #[test]
    fn test_disconnect_without_connect() {
        let registry = test_registry();
        let tracker = ConnectionTracker::new(5, &registry);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        // Disconnect without connect should not panic or go negative
        tracker.on_disconnect(ip);
        assert_eq!(tracker.active_connections(), 0);
        assert_eq!(tracker.unique_ip_count(), 0);
    }
}
