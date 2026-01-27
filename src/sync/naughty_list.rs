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

    /// Strip URLs from an error message to prevent false positives from URL components.
    ///
    /// URLs can contain path components, repository names, or user identifiers that
    /// accidentally match error patterns (e.g., "my-openssl-project", "ssl-team",
    /// "certificate-manager"). By stripping URLs before classification, we ensure
    /// only the actual error message text is analyzed.
    ///
    /// Handles: http://, https://, git://, ws://, wss://
    fn strip_urls(error: &str) -> String {
        let mut result = String::with_capacity(error.len());
        let mut chars = error.chars().peekable();

        while let Some(c) = chars.next() {
            // Check for URL start patterns
            let potential_url = match c {
                'h' => {
                    // Check for http:// or https://
                    let rest: String = chars.clone().take(7).collect();
                    rest.starts_with("ttp://") || rest.starts_with("ttps://")
                }
                'g' => {
                    // Check for git://
                    let rest: String = chars.clone().take(5).collect();
                    rest.starts_with("it://")
                }
                'w' => {
                    // Check for ws:// or wss://
                    let rest: String = chars.clone().take(5).collect();
                    rest.starts_with("s://") || rest.starts_with("ss://")
                }
                _ => false,
            };

            if potential_url {
                // Found URL start, consume until URL end
                result.push_str("[URL]");

                // Skip until we hit a URL terminator
                loop {
                    match chars.peek() {
                        Some(&ch) if Self::is_url_char(ch) => {
                            chars.next();
                        }
                        _ => break,
                    }
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    /// Check if a character can be part of a URL
    #[inline]
    fn is_url_char(c: char) -> bool {
        // URLs end at whitespace, quotes, or certain brackets
        // This is conservative - real URLs can contain more, but git errors
        // typically have URLs followed by these terminators
        !matches!(c, ' ' | '\t' | '\n' | '\r' | '"' | '\'' | '>' | ']' | ')')
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
        // Filter out remote warnings - these are informational messages from the remote
        // server that don't indicate infrastructure problems with the domain itself.
        // Example: "remote: warning: unable to access '/root/.config/git/attributes': Permission denied"
        // These warnings are about the remote server's internal configuration, not connectivity.
        let filtered_error: String = error
            .lines()
            .filter(|line| {
                let line_lower = line.to_lowercase();
                // Keep lines that are NOT remote warnings
                !(line_lower.starts_with("remote: warning:")
                    || line_lower.starts_with("warning: remote"))
            })
            .collect::<Vec<_>>()
            .join("\n");

        // If after filtering we have no content, this was just warnings - not a real error
        if filtered_error.trim().is_empty() {
            return None;
        }

        // Strip URLs to prevent false positives from URL components
        // (e.g., repository named "openssl-test" or path containing "certificate")
        let url_stripped = Self::strip_urls(&filtered_error);
        let error_lower = url_stripped.to_lowercase();

        // DNS lookup failures
        if error_lower.contains("failed to lookup address")
            || error_lower.contains("name or service not known")
            || error_lower.contains("nodename nor servname provided")
            || error_lower.contains("dns error")
            || error_lower.contains("dns lookup")
            || error_lower.contains("dns resolution")
            || error_lower.contains("getaddrinfo")
        {
            return Some(NaughtyCategory::DnsLookupFailed);
        }

        // TLS certificate errors
        if error_lower.contains("certificate")
            || error_lower.contains("ssl error")
            || error_lower.contains("ssl certificate")
            || error_lower.contains("ssl handshake")
            || error_lower.contains("ssl_error")
            || error_lower.contains("tls error")
            || error_lower.contains("tls handshake")
            || error_lower.contains("tls alert")
            || error_lower.contains("tls_error")
            || error_lower.contains("openssl")
            || error_lower.contains("schannel")
            || error_lower.contains("secure channel")
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

    // =========================================================================
    // URL STRIPPING TESTS
    // =========================================================================

    #[test]
    fn test_strip_urls_basic_protocols() {
        // HTTP/HTTPS
        assert_eq!(
            NaughtyListTracker::strip_urls("error: https://example.com/repo.git failed"),
            "error: [URL] failed"
        );
        assert_eq!(
            NaughtyListTracker::strip_urls("error: http://example.com/path failed"),
            "error: [URL] failed"
        );

        // Git protocol
        assert_eq!(
            NaughtyListTracker::strip_urls("fatal: git://github.com/user/repo.git not found"),
            "fatal: [URL] not found"
        );

        // WebSocket protocols (used for relay URLs)
        assert_eq!(
            NaughtyListTracker::strip_urls("error: wss://relay.example.com failed"),
            "error: [URL] failed"
        );
        assert_eq!(
            NaughtyListTracker::strip_urls("error: ws://localhost:8080 failed"),
            "error: [URL] failed"
        );
    }

    #[test]
    fn test_strip_urls_multiple() {
        let error = "failed to clone https://a.com/repo.git and wss://relay.com";
        let stripped = NaughtyListTracker::strip_urls(error);
        assert_eq!(stripped, "failed to clone [URL] and [URL]");
    }

    #[test]
    fn test_strip_urls_preserves_error_text() {
        let error =
            "fatal: unable to access 'https://example.com/repo.git/': SSL certificate problem";
        let stripped = NaughtyListTracker::strip_urls(error);
        assert!(stripped.contains("SSL certificate problem"));
        assert!(!stripped.contains("example.com"));
    }

    // =========================================================================
    // EDGE CASES: TIMEOUT/CONNECTION EXCEPTIONS
    // These are the "unusual rules" where a pattern matches but should be excluded
    // =========================================================================

    #[test]
    fn test_tls_timeout_not_naughty() {
        // TLS errors with timeout should NOT be classified as naughty
        // (timeout is transient, not a certificate problem)
        assert_eq!(
            NaughtyListTracker::classify_error("TLS connection timed out"),
            None
        );
        assert_eq!(
            NaughtyListTracker::classify_error("SSL handshake timeout"),
            None
        );
    }

    #[test]
    fn test_websocket_connection_errors_not_naughty() {
        // WebSocket connection errors are transient, not protocol violations
        assert_eq!(
            NaughtyListTracker::classify_error("websocket connection refused"),
            None
        );
        assert_eq!(
            NaughtyListTracker::classify_error("websocket connection timeout"),
            None
        );
    }

    #[test]
    fn test_remote_warnings_filtered() {
        // Remote warnings should be filtered out before classification
        let warning_only =
            "remote: warning: unable to access '/root/.config/git/attributes': Permission denied";
        assert_eq!(NaughtyListTracker::classify_error(warning_only), None);

        // But real errors after warnings should still be classified
        let warning_with_error = "remote: warning: something\nfatal: failed to lookup address";
        assert_eq!(
            NaughtyListTracker::classify_error(warning_with_error),
            Some(NaughtyCategory::DnsLookupFailed)
        );
    }

    // =========================================================================
    // INTEGRATION: FULL CLASSIFICATION FLOW
    // Verify URL stripping + classification work together correctly
    // =========================================================================

    #[test]
    fn test_url_with_keywords_not_false_positive() {
        // URLs containing keywords should NOT trigger classification
        let cases = [
            ("https://example.com/my-openssl-project.git", "not found"),
            ("https://example.com/ssl-team/repo.git", "not found"),
            ("https://example.com/certificate-manager.git", "not found"),
            ("https://example.com/dns-tools.git", "not found"),
            ("wss://relay-tls-test.example.com", "connection refused"),
        ];

        for (url, suffix) in cases {
            let error = format!("fatal: repository '{}/' {}", url, suffix);
            assert_eq!(
                NaughtyListTracker::classify_error(&error),
                None,
                "URL '{}' should not trigger false positive",
                url
            );
        }
    }

    #[test]
    fn test_real_errors_still_detected() {
        // Real errors in the message text (not URL) should still be detected
        assert_eq!(
            NaughtyListTracker::classify_error(
                "fatal: 'https://example.com/repo.git': SSL certificate problem"
            ),
            Some(NaughtyCategory::TlsCertificateInvalid)
        );
        assert_eq!(
            NaughtyListTracker::classify_error(
                "fatal: 'https://example.com/repo.git': failed to lookup address"
            ),
            Some(NaughtyCategory::DnsLookupFailed)
        );
        assert_eq!(
            NaughtyListTracker::classify_error("websocket protocol error"),
            Some(NaughtyCategory::ProtocolError)
        );
    }

    #[test]
    fn test_url_with_keyword_and_real_error() {
        // URL contains keyword AND there's a real error - should detect the error
        let error = "fatal: 'https://example.com/ssl-tools/repo.git': SSL certificate problem";
        assert_eq!(
            NaughtyListTracker::classify_error(error),
            Some(NaughtyCategory::TlsCertificateInvalid)
        );
    }

    // =========================================================================
    // TRACKER FUNCTIONALITY
    // =========================================================================

    #[test]
    fn test_tracker_record_and_update() {
        let tracker = NaughtyListTracker::with_defaults();
        let url = "wss://bad-relay.example.com";

        // First occurrence
        let is_new = tracker.record(url, NaughtyCategory::DnsLookupFailed, "error 1".to_string());
        assert!(is_new);
        assert!(tracker.is_naughty(url));

        // Second occurrence updates existing
        let is_new2 = tracker.record(url, NaughtyCategory::DnsLookupFailed, "error 2".to_string());
        assert!(!is_new2);

        let entry = tracker.get_entry(url).unwrap();
        assert_eq!(entry.occurrence_count, 2);
        assert_eq!(entry.reason, "error 2");
    }

    #[test]
    fn test_tracker_expiration() {
        let tracker = NaughtyListTracker::new(0); // Expire immediately

        tracker.record(
            "wss://relay.example.com",
            NaughtyCategory::DnsLookupFailed,
            "error".to_string(),
        );

        // Entry exists but is expired
        assert!(!tracker.is_naughty("wss://relay.example.com"));

        std::thread::sleep(std::time::Duration::from_millis(10));

        let expired = tracker.expire_old_entries();
        assert_eq!(expired.len(), 1);
        assert_eq!(tracker.total_count(), 0);
    }

    #[test]
    fn test_tracker_counts() {
        let tracker = NaughtyListTracker::with_defaults();

        tracker.record("wss://r1.com", NaughtyCategory::DnsLookupFailed, "e".into());
        tracker.record("wss://r2.com", NaughtyCategory::DnsLookupFailed, "e".into());
        tracker.record(
            "wss://r3.com",
            NaughtyCategory::TlsCertificateInvalid,
            "e".into(),
        );

        assert_eq!(tracker.total_count(), 3);
        assert_eq!(
            tracker.count_by_category(NaughtyCategory::DnsLookupFailed),
            2
        );
        assert_eq!(
            tracker.count_by_category(NaughtyCategory::TlsCertificateInvalid),
            1
        );
        assert_eq!(tracker.get_all().len(), 3);
    }

    #[test]
    fn test_category_display() {
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
