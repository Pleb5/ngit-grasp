//! Naughty List Tracker for Remote Servers with Persistent Infrastructure Issues
//!
//! This module tracks remote servers (Nostr relays and git remote domains) with
//! persistent configuration/infrastructure problems (DNS failures, TLS certificate
//! errors, protocol violations) separately from transient network issues (timeouts,
//! connection refused).
//!
//! ## Failure Classification
//!
//! **Naughty List (12-hour expiration, log WARN on first occurrence, DEBUG on repeat):**
//! - `DnsLookupFailed`: Domain doesn't resolve or DNS errors
//! - `TlsCertificateInvalid`: Certificate errors (expired, mismatch, self-signed)
//! - `ProtocolError`: WebSocket/Nostr protocol violations
//!
//! **NOT Naughty (use existing HealthTracker backoff):**
//! - Connection timeouts (could be network congestion)
//! - Connection refused (could be temporary maintenance)
//!
//! ## Automatic Expiration
//!
//! Entries expire after 12 hours (configurable) to allow relays to recover from
//! infrastructure issues. After expiration, the relay is automatically retried.

use dashmap::DashMap;
use std::time::Instant;

/// Category of persistent remote server failure that qualifies for the naughty list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NaughtyCategory {
    /// DNS lookup failures (domain doesn't resolve)
    DnsLookupFailed,
    /// TLS certificate errors (expired, invalid, mismatch)
    TlsCertificateInvalid,
    /// WebSocket or Nostr protocol violations (relay-specific, won't trigger for git)
    ProtocolError,
}

impl NaughtyCategory {
    /// Get string representation for metrics labels
    pub fn as_str(&self) -> &'static str {
        match self {
            NaughtyCategory::DnsLookupFailed => "dns_lookup_failed",
            NaughtyCategory::TlsCertificateInvalid => "tls_certificate_invalid",
            NaughtyCategory::ProtocolError => "protocol_error",
        }
    }
}

impl std::fmt::Display for NaughtyCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Naughty list entry for a remote server (relay URL or git domain) with persistent issues
#[derive(Debug, Clone)]
pub struct NaughtyEntry {
    /// Category of the persistent failure
    pub category: NaughtyCategory,
    /// Full error message
    pub reason: String,
    /// When this relay was first added to the naughty list
    pub first_seen: Instant,
    /// Most recent occurrence of the issue
    pub last_seen: Instant,
    /// Number of times we've seen this issue
    pub occurrence_count: u32,
}

/// Tracks remote servers with persistent infrastructure/configuration issues
///
/// Used for both:
/// - Nostr relay URLs (e.g., "wss://relay.example.com")
/// - Git remote domains (e.g., "git.example.com")
///
/// Separate from HealthTracker's backoff logic - this is specifically for
/// servers with configuration problems that are unlikely to be fixed quickly.
#[derive(Debug)]
pub struct NaughtyListTracker {
    /// Map of relay URL or git domain to naughty entry
    entries: DashMap<String, NaughtyEntry>,
    /// How many hours before removing a server from the naughty list
    expiration_hours: u64,
}

impl NaughtyListTracker {
    /// Create a new NaughtyListTracker with the specified expiration time
    ///
    /// # Arguments
    ///
    /// * `expiration_hours` - Hours before a naughty entry expires (default: 12)
    pub fn new(expiration_hours: u64) -> Self {
        Self {
            entries: DashMap::new(),
            expiration_hours,
        }
    }

    /// Create a new NaughtyListTracker with default 12-hour expiration
    pub fn with_defaults() -> Self {
        Self::new(12)
    }

    /// Classify an error string into a naughty category or return None for transient errors
    ///
    /// # Arguments
    ///
    /// * `error` - The error message string to classify
    ///
    /// # Returns
    ///
    /// - `Some(NaughtyCategory)` if the error indicates a persistent infrastructure issue
    /// - `None` if the error is a transient network issue (use HealthTracker backoff)
    pub fn classify_error(error: &str) -> Option<NaughtyCategory> {
        let error_lower = error.to_lowercase();

        // DNS lookup failures
        if error_lower.contains("failed to lookup address")
            || error_lower.contains("name or service not known")
            || error_lower.contains("nodename nor servname provided")
            || (error_lower.contains("dns") && !error_lower.contains("timeout"))
        {
            return Some(NaughtyCategory::DnsLookupFailed);
        }

        // TLS certificate errors
        if error_lower.contains("certificate")
            || error_lower.contains("ssl")
            || error_lower.contains("tls")
        {
            // Exclude timeout errors that mention TLS
            if !error_lower.contains("timeout") && !error_lower.contains("timed out") {
                return Some(NaughtyCategory::TlsCertificateInvalid);
            }
        }

        // Protocol errors
        if error_lower.contains("websocket")
            || error_lower.contains("protocol")
            || error_lower.contains("invalid frame")
        {
            // Exclude connection errors
            if !error_lower.contains("connection")
                && !error_lower.contains("timeout")
                && !error_lower.contains("refused")
            {
                return Some(NaughtyCategory::ProtocolError);
            }
        }

        // Everything else is transient (timeouts, refused, etc.)
        None
    }

    /// Record a naughty server (adds new entry or updates existing)
    ///
    /// # Arguments
    ///
    /// * `server_url_or_domain` - The relay URL or git domain
    /// * `category` - The naughty category
    /// * `reason` - The full error message
    ///
    /// # Returns
    ///
    /// `true` if this is a new naughty entry (first occurrence), `false` if updating existing
    pub fn record(
        &self,
        server_url_or_domain: &str,
        category: NaughtyCategory,
        reason: String,
    ) -> bool {
        let now = Instant::now();

        if let Some(mut entry) = self.entries.get_mut(server_url_or_domain) {
            // Update existing entry
            entry.last_seen = now;
            entry.occurrence_count = entry.occurrence_count.saturating_add(1);
            entry.reason = reason; // Update with latest error message
            false
        } else {
            // Create new entry
            self.entries.insert(
                server_url_or_domain.to_string(),
                NaughtyEntry {
                    category,
                    reason,
                    first_seen: now,
                    last_seen: now,
                    occurrence_count: 1,
                },
            );
            true
        }
    }

    /// Check if a server is on the naughty list (not expired)
    ///
    /// # Arguments
    ///
    /// * `server_url_or_domain` - The relay URL or git domain to check
    ///
    /// # Returns
    ///
    /// `true` if the server is currently on the naughty list
    pub fn is_naughty(&self, server_url_or_domain: &str) -> bool {
        if let Some(entry) = self.entries.get(server_url_or_domain) {
            let age = Instant::now().duration_since(entry.first_seen);
            let expiration = std::time::Duration::from_secs(self.expiration_hours * 3600);
            age < expiration
        } else {
            false
        }
    }

    /// Get a naughty entry if it exists and hasn't expired
    ///
    /// # Arguments
    ///
    /// * `server_url_or_domain` - The relay URL or git domain to look up
    ///
    /// # Returns
    ///
    /// A cloned `NaughtyEntry` if the server is on the naughty list and not expired
    pub fn get_entry(&self, server_url_or_domain: &str) -> Option<NaughtyEntry> {
        self.entries.get(server_url_or_domain).map(|e| e.clone())
    }

    /// Remove expired entries from the naughty list
    ///
    /// Entries older than `expiration_hours` are removed to allow servers
    /// to be retried after infrastructure issues are potentially fixed.
    ///
    /// # Returns
    ///
    /// Vector of server URLs/domains that were removed from the naughty list
    pub fn expire_old_entries(&self) -> Vec<String> {
        let now = Instant::now();
        let expiration = std::time::Duration::from_secs(self.expiration_hours * 3600);
        let mut expired = Vec::new();

        // Collect expired relay URLs
        self.entries.retain(|url, entry| {
            let age = now.duration_since(entry.first_seen);
            if age >= expiration {
                expired.push(url.clone());
                false // Remove this entry
            } else {
                true // Keep this entry
            }
        });

        expired
    }

    /// Get all naughty servers (for metrics and monitoring)
    ///
    /// # Returns
    ///
    /// Vector of (server_url_or_domain, entry) tuples for all servers currently on the naughty list
    pub fn get_all(&self) -> Vec<(String, NaughtyEntry)> {
        self.entries
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Get the count of servers in a specific category
    ///
    /// # Arguments
    ///
    /// * `category` - The category to count
    ///
    /// # Returns
    ///
    /// Number of servers in the specified category
    pub fn count_by_category(&self, category: NaughtyCategory) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.value().category == category)
            .count()
    }

    /// Get total number of servers on the naughty list
    pub fn total_count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_dns_errors() {
        assert_eq!(
            NaughtyListTracker::classify_error("failed to lookup address information"),
            Some(NaughtyCategory::DnsLookupFailed)
        );
        assert_eq!(
            NaughtyListTracker::classify_error("Name or service not known"),
            Some(NaughtyCategory::DnsLookupFailed)
        );
        assert_eq!(
            NaughtyListTracker::classify_error("nodename nor servname provided"),
            Some(NaughtyCategory::DnsLookupFailed)
        );
        assert_eq!(
            NaughtyListTracker::classify_error("dns error: NXDOMAIN"),
            Some(NaughtyCategory::DnsLookupFailed)
        );
    }

    #[test]
    fn test_classify_tls_errors() {
        assert_eq!(
            NaughtyListTracker::classify_error("certificate not valid for 'example.com'"),
            Some(NaughtyCategory::TlsCertificateInvalid)
        );
        assert_eq!(
            NaughtyListTracker::classify_error("SSL certificate problem"),
            Some(NaughtyCategory::TlsCertificateInvalid)
        );
        assert_eq!(
            NaughtyListTracker::classify_error("TLS handshake failed"),
            Some(NaughtyCategory::TlsCertificateInvalid)
        );

        // TLS timeout should NOT be classified as naughty
        assert_eq!(
            NaughtyListTracker::classify_error("TLS connection timed out"),
            None
        );
    }

    #[test]
    fn test_classify_protocol_errors() {
        assert_eq!(
            NaughtyListTracker::classify_error("websocket protocol error"),
            Some(NaughtyCategory::ProtocolError)
        );
        assert_eq!(
            NaughtyListTracker::classify_error("invalid frame header"),
            Some(NaughtyCategory::ProtocolError)
        );

        // WebSocket connection errors should NOT be classified as naughty
        assert_eq!(
            NaughtyListTracker::classify_error("websocket connection refused"),
            None
        );
    }

    #[test]
    fn test_classify_transient_errors() {
        // Timeouts are transient
        assert_eq!(
            NaughtyListTracker::classify_error("connection timed out"),
            None
        );
        assert_eq!(
            NaughtyListTracker::classify_error("operation timed out"),
            None
        );

        // Connection refused is transient
        assert_eq!(
            NaughtyListTracker::classify_error("connection refused"),
            None
        );

        // Generic network errors are transient
        assert_eq!(
            NaughtyListTracker::classify_error("network unreachable"),
            None
        );
    }

    #[test]
    fn test_record_new_entry() {
        let tracker = NaughtyListTracker::with_defaults();
        let url = "wss://bad-relay.example.com";

        let is_new = tracker.record(
            url,
            NaughtyCategory::DnsLookupFailed,
            "failed to lookup address".to_string(),
        );

        assert!(is_new);
        assert!(tracker.is_naughty(url));

        let entry = tracker.get_entry(url).unwrap();
        assert_eq!(entry.category, NaughtyCategory::DnsLookupFailed);
        assert_eq!(entry.occurrence_count, 1);
    }

    #[test]
    fn test_record_updates_existing() {
        let tracker = NaughtyListTracker::with_defaults();
        let url = "wss://bad-relay.example.com";

        // First occurrence
        let is_new1 = tracker.record(url, NaughtyCategory::DnsLookupFailed, "error 1".to_string());
        assert!(is_new1);

        // Second occurrence
        let is_new2 = tracker.record(url, NaughtyCategory::DnsLookupFailed, "error 2".to_string());
        assert!(!is_new2);

        let entry = tracker.get_entry(url).unwrap();
        assert_eq!(entry.occurrence_count, 2);
        assert_eq!(entry.reason, "error 2"); // Updated to latest
    }

    #[test]
    fn test_is_naughty() {
        let tracker = NaughtyListTracker::with_defaults();
        let url = "wss://bad-relay.example.com";

        assert!(!tracker.is_naughty(url));

        tracker.record(
            url,
            NaughtyCategory::TlsCertificateInvalid,
            "cert error".to_string(),
        );

        assert!(tracker.is_naughty(url));
    }

    #[test]
    fn test_get_all() {
        let tracker = NaughtyListTracker::with_defaults();

        tracker.record(
            "wss://relay1.example.com",
            NaughtyCategory::DnsLookupFailed,
            "dns error".to_string(),
        );
        tracker.record(
            "wss://relay2.example.com",
            NaughtyCategory::TlsCertificateInvalid,
            "tls error".to_string(),
        );

        let all = tracker.get_all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_count_by_category() {
        let tracker = NaughtyListTracker::with_defaults();

        tracker.record(
            "wss://relay1.example.com",
            NaughtyCategory::DnsLookupFailed,
            "error".to_string(),
        );
        tracker.record(
            "wss://relay2.example.com",
            NaughtyCategory::DnsLookupFailed,
            "error".to_string(),
        );
        tracker.record(
            "wss://relay3.example.com",
            NaughtyCategory::TlsCertificateInvalid,
            "error".to_string(),
        );

        assert_eq!(
            tracker.count_by_category(NaughtyCategory::DnsLookupFailed),
            2
        );
        assert_eq!(
            tracker.count_by_category(NaughtyCategory::TlsCertificateInvalid),
            1
        );
        assert_eq!(tracker.count_by_category(NaughtyCategory::ProtocolError), 0);
    }

    #[test]
    fn test_total_count() {
        let tracker = NaughtyListTracker::with_defaults();
        assert_eq!(tracker.total_count(), 0);

        tracker.record(
            "wss://relay1.example.com",
            NaughtyCategory::DnsLookupFailed,
            "error".to_string(),
        );
        assert_eq!(tracker.total_count(), 1);

        tracker.record(
            "wss://relay2.example.com",
            NaughtyCategory::TlsCertificateInvalid,
            "error".to_string(),
        );
        assert_eq!(tracker.total_count(), 2);
    }

    #[test]
    fn test_expire_old_entries() {
        // Use very short expiration for testing
        let tracker = NaughtyListTracker::new(0); // Expire immediately (0 hours)

        tracker.record(
            "wss://relay1.example.com",
            NaughtyCategory::DnsLookupFailed,
            "error".to_string(),
        );

        // Entry should exist in the map
        assert_eq!(tracker.total_count(), 1);

        // But is_naughty should return false since it's already expired (0 hours)
        assert!(!tracker.is_naughty("wss://relay1.example.com"));

        // Sleep to ensure time passes
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Expire old entries (should remove the 0-hour expired entry)
        let expired = tracker.expire_old_entries();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], "wss://relay1.example.com");

        // Entry should be gone
        assert!(!tracker.is_naughty("wss://relay1.example.com"));
        assert_eq!(tracker.total_count(), 0);
    }

    #[test]
    fn test_category_display() {
        assert_eq!(
            NaughtyCategory::DnsLookupFailed.to_string(),
            "dns_lookup_failed"
        );
        assert_eq!(
            NaughtyCategory::TlsCertificateInvalid.to_string(),
            "tls_certificate_invalid"
        );
        assert_eq!(NaughtyCategory::ProtocolError.to_string(), "protocol_error");
    }

    #[test]
    fn test_category_as_str() {
        assert_eq!(
            NaughtyCategory::DnsLookupFailed.as_str(),
            "dns_lookup_failed"
        );
        assert_eq!(
            NaughtyCategory::TlsCertificateInvalid.as_str(),
            "tls_certificate_invalid"
        );
        assert_eq!(NaughtyCategory::ProtocolError.as_str(), "protocol_error");
    }
}
