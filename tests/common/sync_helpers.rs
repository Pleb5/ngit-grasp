//! Proactive Sync Test Helpers
//!
//! Provides utilities for testing ngit-grasp's proactive sync functionality:
//! - `TestClient` - Client wrapper with built-in retry logic
//! - Event builders for Layer 2 (kind 1618) and Layer 3 (kinds 1, 1111) events
//! - Assertion helpers that return bool (non-panicking)
//!
//! # nostr-sdk 0.43 API Notes
//! - Use field access: `event.id`, `event.tags`, `event.tags.iter()`
//! - Use `Tag::custom(TagKind::custom("name"), vec![...])` syntax
//! - Use `EventBuilder::new(kind, content).tags(tags)` syntax

use std::time::Duration;

use nostr_sdk::prelude::*;

/// Kind 1618 - Issue (NIP-34 git-related event)
pub const KIND_ISSUE: u16 = 1618;

/// Kind 1111 - NIP-22 Comment
pub const KIND_COMMENT: u16 = 1111;

/// Kind 30617 - Repository state/announcement (NIP-34)
pub const KIND_REPOSITORY_STATE: u16 = 30617;

/// Test client with built-in retry logic for connect and send operations.
///
/// Wraps nostr-sdk Client with automatic retry handling suitable for
/// integration tests where connections may take time to establish.
pub struct TestClient {
    client: Client,
    relay_url: String,
    keys: Keys,
}

impl TestClient {
    /// Create a new TestClient and connect to the specified relay.
    ///
    /// Uses retry logic: up to 30 attempts with 100ms delay between each.
    ///
    /// # Arguments
    /// * `relay_url` - WebSocket URL of the relay (e.g., "ws://127.0.0.1:8080")
    /// * `keys` - Nostr keys for signing events
    ///
    /// # Returns
    /// * `Ok(TestClient)` on successful connection
    /// * `Err(String)` if connection fails after all retries
    pub async fn new(relay_url: &str, keys: Keys) -> Result<Self, String> {
        let client = Client::new(keys.clone());

        client
            .add_relay(relay_url)
            .await
            .map_err(|e| format!("Failed to add relay: {}", e))?;

        let test_client = Self {
            client,
            relay_url: relay_url.to_string(),
            keys,
        };

        test_client.connect().await?;

        Ok(test_client)
    }

    /// Connect to the relay with retry logic.
    ///
    /// Attempts connection up to 30 times with 100ms delays (3 seconds total).
    pub async fn connect(&self) -> Result<(), String> {
        self.client.connect().await;

        // Wait for connection with retries (matching existing pattern)
        for attempt in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let relays = self.client.relays().await;
            if relays.values().any(|r| r.is_connected()) {
                return Ok(());
            }
            if attempt == 29 {
                return Err(format!(
                    "Failed to connect to relay {} after 3 seconds",
                    self.relay_url
                ));
            }
        }

        Err("Connection loop exited unexpectedly".to_string())
    }

    /// Send an event with retry logic.
    ///
    /// Attempts to send up to 3 times with exponential backoff:
    /// - Attempt 1: immediate
    /// - Attempt 2: after 200ms
    /// - Attempt 3: after 400ms
    ///
    /// # Arguments
    /// * `event` - The signed event to send
    ///
    /// # Returns
    /// * `Ok(EventId)` on successful send
    /// * `Err(String)` if all attempts fail
    pub async fn send_event(&self, event: &Event) -> Result<EventId, String> {
        let delays = [0, 200, 400]; // Exponential backoff in ms

        for (attempt, delay_ms) in delays.iter().enumerate() {
            if *delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }

            match self.client.send_event(event).await {
                Ok(output) => {
                    if !output.success.is_empty() {
                        return Ok(output.val);
                    }
                    // Log failures for debugging
                    if !output.failed.is_empty() {
                        eprintln!(
                            "  Send attempt {} - failures: {:?}",
                            attempt + 1,
                            output.failed
                        );
                        // Try reconnecting if relay disconnected
                        self.client.connect().await;
                    }
                }
                Err(e) => {
                    eprintln!("  Send attempt {} - error: {}", attempt + 1, e);
                }
            }
        }

        Err(format!(
            "Failed to send event {} after 3 attempts",
            event.id
        ))
    }

    /// Get a reference to the keys used by this client.
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Disconnect from the relay.
    pub async fn disconnect(self) {
        self.client.disconnect().await;
    }
}

// ============================================================================
// Event Builders
// ============================================================================

/// Build a Layer 2 issue event (kind 1618) with a/A/q tags referencing a repository.
///
/// Creates an issue event that references the specified repository coordinate.
/// Supports different tag types for comprehensive Layer 2 filter testing.
///
/// # Arguments
/// * `keys` - Keys for signing the event
/// * `repo_coord` - Repository coordinate (format: "30617:pubkey_hex:identifier")
/// * `title` - Issue title (used as content)
///
/// # Tag Types
/// Uses lowercase 'a' tag by default. For other tag variations, see:
/// - `build_layer2_issue_with_uppercase_a_tag`
/// - `build_layer2_issue_with_q_tag`
///
/// # Returns
/// * `Ok(Event)` - Signed event ready to send
/// * `Err(String)` - If signing fails
pub fn build_layer2_issue_event(keys: &Keys, repo_coord: &str, title: &str) -> Result<Event, String> {
    build_layer2_issue_with_tag(keys, repo_coord, title, TagVariant::LowercaseA)
}

/// Build a Layer 2 issue with uppercase 'A' tag.
pub fn build_layer2_issue_with_uppercase_a_tag(
    keys: &Keys,
    repo_coord: &str,
    title: &str,
) -> Result<Event, String> {
    build_layer2_issue_with_tag(keys, repo_coord, title, TagVariant::UppercaseA)
}

/// Build a Layer 2 issue with 'q' (quote) tag.
pub fn build_layer2_issue_with_q_tag(
    keys: &Keys,
    repo_coord: &str,
    title: &str,
) -> Result<Event, String> {
    build_layer2_issue_with_tag(keys, repo_coord, title, TagVariant::QuoteQ)
}

/// Tag variant for Layer 2 events (referencing repo coordinates)
#[derive(Debug, Clone, Copy)]
pub enum TagVariant {
    /// Lowercase 'a' tag - standard addressable reference
    LowercaseA,
    /// Uppercase 'A' tag - some clients use this
    UppercaseA,
    /// Quote 'q' tag - NIP-10 quote reference
    QuoteQ,
}

/// Internal helper to build Layer 2 issue with specified tag variant.
fn build_layer2_issue_with_tag(
    keys: &Keys,
    repo_coord: &str,
    title: &str,
    tag_variant: TagVariant,
) -> Result<Event, String> {
    let tag = match tag_variant {
        TagVariant::LowercaseA => Tag::custom(TagKind::custom("a"), vec![repo_coord.to_string()]),
        TagVariant::UppercaseA => Tag::custom(TagKind::custom("A"), vec![repo_coord.to_string()]),
        TagVariant::QuoteQ => Tag::custom(TagKind::custom("q"), vec![repo_coord.to_string()]),
    };

    let tags = vec![tag];

    EventBuilder::new(Kind::Custom(KIND_ISSUE), title)
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| format!("Failed to sign Layer 2 issue event: {}", e))
}

/// Build a Layer 3 comment event (kinds 1 or 1111) with e/E/q tags referencing an event ID.
///
/// Creates a comment/reply event that references the specified parent event ID.
/// Supports different kinds and tag types for comprehensive Layer 3 filter testing.
///
/// # Arguments
/// * `keys` - Keys for signing the event
/// * `parent_event_id` - Event ID being referenced (e.g., an issue or patch)
/// * `content` - Comment content
/// * `kind` - Event kind (Kind::Custom(1) for reply, Kind::Custom(1111) for NIP-22 comment)
///
/// # Tag Types
/// - For kind 1111: Uses uppercase 'E' tag (NIP-22 style)
/// - For kind 1: Uses lowercase 'e' tag with "root" marker (NIP-10 style)
///
/// # Returns
/// * `Ok(Event)` - Signed event ready to send
/// * `Err(String)` - If signing fails
pub fn build_layer3_comment_event(
    keys: &Keys,
    parent_event_id: &EventId,
    content: &str,
    kind: Kind,
) -> Result<Event, String> {
    let kind_num = kind.as_u16();

    // Choose tag based on kind (NIP-22 uses E, NIP-10 style uses e)
    let tag = if kind_num == KIND_COMMENT {
        // NIP-22 comment: uppercase 'E' tag
        Tag::custom(
            TagKind::custom("E"),
            vec![parent_event_id.to_hex()],
        )
    } else {
        // Kind 1 reply: lowercase 'e' tag with root marker (NIP-10)
        Tag::custom(
            TagKind::custom("e"),
            vec![parent_event_id.to_hex(), "".to_string(), "root".to_string()],
        )
    };

    let tags = vec![tag];

    EventBuilder::new(kind, content)
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| format!("Failed to sign Layer 3 comment event: {}", e))
}

/// Build a Layer 3 reply (kind 1) with lowercase 'e' tag.
pub fn build_layer3_reply_with_e_tag(
    keys: &Keys,
    parent_event_id: &EventId,
    content: &str,
) -> Result<Event, String> {
    let tag = Tag::custom(
        TagKind::custom("e"),
        vec![parent_event_id.to_hex(), "".to_string(), "root".to_string()],
    );

    EventBuilder::new(Kind::Custom(1), content)
        .tags(vec![tag])
        .sign_with_keys(keys)
        .map_err(|e| format!("Failed to sign Layer 3 reply event: {}", e))
}

/// Build a Layer 3 comment (kind 1111) with uppercase 'E' tag (NIP-22).
pub fn build_layer3_comment_with_uppercase_e_tag(
    keys: &Keys,
    parent_event_id: &EventId,
    content: &str,
) -> Result<Event, String> {
    let tag = Tag::custom(
        TagKind::custom("E"),
        vec![parent_event_id.to_hex()],
    );

    EventBuilder::new(Kind::Custom(KIND_COMMENT), content)
        .tags(vec![tag])
        .sign_with_keys(keys)
        .map_err(|e| format!("Failed to sign Layer 3 comment event: {}", e))
}

/// Build a Layer 3 quote (kind 1) with 'q' tag.
pub fn build_layer3_quote_with_q_tag(
    keys: &Keys,
    parent_event_id: &EventId,
    content: &str,
) -> Result<Event, String> {
    let tag = Tag::custom(
        TagKind::custom("q"),
        vec![parent_event_id.to_hex()],
    );

    EventBuilder::new(Kind::Custom(1), content)
        .tags(vec![tag])
        .sign_with_keys(keys)
        .map_err(|e| format!("Failed to sign Layer 3 quote event: {}", e))
}

// ============================================================================
// Repository Announcement Helper
// ============================================================================

/// Create a valid repository announcement event for testing sync.
///
/// This creates a kind 30617 event with required clone and relays tags.
/// The event lists all provided domains so it will be accepted by each
/// relay's write policy.
///
/// # Arguments
/// * `keys` - Keys for signing
/// * `domains` - Slice of domain strings (e.g., "127.0.0.1:8080")
/// * `identifier` - Repository identifier (d-tag)
///
/// # Returns
/// A signed repository announcement event ready to send.
pub fn create_repo_announcement(keys: &Keys, domains: &[&str], identifier: &str) -> Event {
    // Build clone URLs for all domains (with .git suffix)
    let clone_urls: Vec<String> = domains
        .iter()
        .map(|d| format!("http://{}/{}.git", d, identifier))
        .collect();

    // Build relay URLs for all domains
    let relay_urls: Vec<String> = domains.iter().map(|d| format!("ws://{}", d)).collect();

    // Build tags for repository announcement
    let tags = vec![
        Tag::identifier(identifier),
        Tag::custom(TagKind::custom("clone"), clone_urls),
        Tag::custom(TagKind::custom("relays"), relay_urls),
    ];

    EventBuilder::new(Kind::Custom(KIND_REPOSITORY_STATE), "Repository state")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("Failed to sign repo announcement")
}

// ============================================================================
// Sync Connection Helpers
// ============================================================================

/// Wait for a sync connection to be established on the syncing relay.
///
/// Polls the relay's metrics endpoint to check for active sync connections.
/// This is more reliable than using fixed sleeps, as it verifies the actual
/// connection state before proceeding with test assertions.
///
/// # Arguments
/// * `syncing_relay_url` - WebSocket URL of the relay that is syncing (e.g., "ws://127.0.0.1:8080")
/// * `expected_connections` - Expected number of sync connections (typically 1 for single bootstrap)
/// * `timeout` - Maximum time to wait for connections to be established
///
/// # Returns
/// * `Ok(())` - Connections established within timeout
/// * `Err(String)` - Timeout waiting for connections, or other error
///
/// # Example
/// ```ignore
/// // After starting relay_b with sync from relay_a
/// let relay_b = TestRelay::start_with_sync(Some(relay_a.url().into())).await;
///
/// // Wait for sync connection to be established
/// wait_for_sync_connection(relay_b.url(), 1, Duration::from_secs(5)).await
///     .expect("Sync connection should be established");
///
/// // Now proceed with test - sync connection is verified
/// ```
pub async fn wait_for_sync_connection(
    syncing_relay_url: &str,
    expected_connections: usize,
    timeout: Duration,
) -> Result<(), String> {
    // Convert ws:// URL to http:// for metrics endpoint
    let http_url = syncing_relay_url
        .replace("ws://", "http://")
        .replace("/", "")
        + "/metrics";

    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(100);

    while start.elapsed() < timeout {
        // Fetch metrics
        if let Ok(response) = reqwest::get(&http_url).await {
            if let Ok(metrics) = response.text().await {
                // Look for sync connection metrics
                // The metric name pattern: ngit_sync_connections or similar
                // We check for any indication of established connections
                if check_sync_connections_in_metrics(&metrics, expected_connections) {
                    return Ok(());
                }
            }
        }

        tokio::time::sleep(poll_interval).await;
    }

    Err(format!(
        "Timeout waiting for {} sync connection(s) on {} after {:?}",
        expected_connections, syncing_relay_url, timeout
    ))
}

/// Check metrics string for expected number of sync connections.
///
/// Looks for various metric patterns that indicate sync connections:
/// - ngit_sync_connections (gauge)
/// - ngit_sync_relay_connections (gauge)
/// - Any metric containing "sync" and "connection" with count > 0
fn check_sync_connections_in_metrics(metrics: &str, expected: usize) -> bool {
    // Parse metrics line by line looking for connection counts
    for line in metrics.lines() {
        // Skip comments and empty lines
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Look for sync connection metrics
        // Format: metric_name{labels} value
        // or: metric_name value
        if line.contains("sync") && line.contains("connect") {
            // Extract the value (last space-separated token)
            if let Some(value_str) = line.split_whitespace().last() {
                if let Ok(value) = value_str.parse::<f64>() {
                    if value as usize >= expected {
                        return true;
                    }
                }
            }
        }

        // Also check for specific metric names that might indicate connections
        // ngit_sync_health_state with value 1 or 2 (connecting/healthy)
        if line.contains("ngit_sync_health") {
            if let Some(value_str) = line.split_whitespace().last() {
                if let Ok(value) = value_str.parse::<f64>() {
                    // Health state > 0 typically means connection attempt or established
                    if value > 0.0 && expected > 0 {
                        return true;
                    }
                }
            }
        }
    }

    false
}

// ============================================================================
// Assertion Helpers
// ============================================================================

/// Wait for an event to appear on a relay.
///
/// Polls the relay for the specified event using the provided filter.
/// Returns true if found within timeout, false otherwise.
///
/// **Important:** This function does NOT panic - it returns a bool to allow
/// tests to make their own assertions with descriptive error messages.
///
/// # Arguments
/// * `relay_url` - WebSocket URL of the relay to check
/// * `filter` - Nostr filter to use for querying (should match the expected event)
/// * `timeout` - Maximum time to wait for the event
///
/// # Returns
/// * `true` - Event matching filter was found
/// * `false` - Event not found within timeout, or connection failed
///
/// # Example
/// ```ignore
/// let filter = Filter::new()
///     .kind(Kind::Custom(1618))
///     .author(keys.public_key())
///     .id(event.id);
///
/// let found = wait_for_event_on_relay(relay.url(), filter, Duration::from_secs(3)).await;
/// assert!(found, "Expected event {} to sync to relay", event.id);
/// ```
pub async fn wait_for_event_on_relay(relay_url: &str, filter: Filter, timeout: Duration) -> bool {
    // Create a temporary client for querying
    let temp_keys = Keys::generate();
    let client = Client::new(temp_keys);

    // Try to connect
    if client.add_relay(relay_url).await.is_err() {
        return false;
    }

    client.connect().await;

    // Wait for connection (brief timeout)
    let mut connected = false;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let relays = client.relays().await;
        if relays.values().any(|r| r.is_connected()) {
            connected = true;
            break;
        }
    }

    if !connected {
        client.disconnect().await;
        return false;
    }

    // Fetch events with the provided timeout
    let result = client.fetch_events(filter, timeout).await;

    client.disconnect().await;

    match result {
        Ok(events) => !events.is_empty(),
        Err(_) => false,
    }
}

/// Build repo coordinate string for use in 'a' tags.
///
/// Format: `30617:pubkey_hex:identifier`
///
/// # Arguments
/// * `keys` - Keys whose public key will be used
/// * `identifier` - Repository identifier (d-tag value)
pub fn repo_coord(keys: &Keys, identifier: &str) -> String {
    format!(
        "{}:{}:{}",
        KIND_REPOSITORY_STATE,
        keys.public_key().to_hex(),
        identifier
    )
}

// ============================================================================
// Metrics Helpers
// ============================================================================

/// Fetch Prometheus metrics from a relay's `/metrics` endpoint.
///
/// Converts the WebSocket URL to HTTP and fetches the metrics endpoint.
/// Useful for verifying sync-related metrics in tests.
///
/// # Arguments
/// * `relay_url` - WebSocket URL of the relay (e.g., "ws://127.0.0.1:8080")
///
/// # Returns
/// * `Ok(String)` - The metrics text in Prometheus format
/// * `Err(reqwest::Error)` - If the request fails
///
/// # Example
/// ```ignore
/// let metrics = fetch_metrics("ws://127.0.0.1:8080").await?;
/// assert!(metrics.contains("ngit_sync_"));
/// ```
pub async fn fetch_metrics(relay_url: &str) -> Result<String, reqwest::Error> {
    // Convert ws:// URL to http:// for metrics endpoint
    let http_url = relay_url
        .replace("ws://", "http://")
        .replace("/", "")
        + "/metrics";

    reqwest::get(&http_url).await?.text().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_coord_format() {
        let keys = Keys::generate();
        let coord = repo_coord(&keys, "test-repo");

        assert!(coord.starts_with("30617:"));
        assert!(coord.ends_with(":test-repo"));
        assert_eq!(coord.split(':').count(), 3);
    }

    #[test]
    fn test_build_layer2_issue_event() {
        let keys = Keys::generate();
        let coord = repo_coord(&keys, "my-repo");

        let event = build_layer2_issue_event(&keys, &coord, "Test Issue")
            .expect("Should create event");

        // nostr-sdk 0.43: use field access
        assert_eq!(event.kind.as_u16(), KIND_ISSUE);

        // Check the tag exists
        let has_a_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "a")
        });
        assert!(has_a_tag, "Event should have 'a' tag");
    }

    #[test]
    fn test_build_layer2_issue_with_uppercase_a() {
        let keys = Keys::generate();
        let coord = repo_coord(&keys, "my-repo");

        let event = build_layer2_issue_with_uppercase_a_tag(&keys, &coord, "Test Issue")
            .expect("Should create event");

        let has_upper_a_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "A")
        });
        assert!(has_upper_a_tag, "Event should have 'A' tag");
    }

    #[test]
    fn test_build_layer2_issue_with_q_tag() {
        let keys = Keys::generate();
        let coord = repo_coord(&keys, "my-repo");

        let event = build_layer2_issue_with_q_tag(&keys, &coord, "Test Issue")
            .expect("Should create event");

        let has_q_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "q")
        });
        assert!(has_q_tag, "Event should have 'q' tag");
    }

    #[test]
    fn test_build_layer3_comment_kind_1111() {
        let keys = Keys::generate();
        let parent_id = EventId::all_zeros();

        let event = build_layer3_comment_event(&keys, &parent_id, "Test comment", Kind::Custom(KIND_COMMENT))
            .expect("Should create event");

        assert_eq!(event.kind.as_u16(), KIND_COMMENT);

        // NIP-22 comment should have uppercase 'E' tag
        let has_e_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "E")
        });
        assert!(has_e_tag, "Kind 1111 event should have 'E' tag");
    }

    #[test]
    fn test_build_layer3_comment_kind_1() {
        let keys = Keys::generate();
        let parent_id = EventId::all_zeros();

        let event = build_layer3_comment_event(&keys, &parent_id, "Test reply", Kind::Custom(1))
            .expect("Should create event");

        assert_eq!(event.kind.as_u16(), 1);

        // Kind 1 reply should have lowercase 'e' tag with root marker
        let has_e_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "e")
        });
        assert!(has_e_tag, "Kind 1 event should have 'e' tag");
    }

    #[test]
    fn test_build_layer3_reply_with_e_tag() {
        let keys = Keys::generate();
        let parent_id = EventId::all_zeros();

        let event = build_layer3_reply_with_e_tag(&keys, &parent_id, "Reply content")
            .expect("Should create event");

        assert_eq!(event.kind.as_u16(), 1);

        let has_e_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "e") &&
            slice.get(3).is_some_and(|m| m == "root")
        });
        assert!(has_e_tag, "Should have 'e' tag with root marker");
    }

    #[test]
    fn test_build_layer3_comment_with_uppercase_e() {
        let keys = Keys::generate();
        let parent_id = EventId::all_zeros();

        let event = build_layer3_comment_with_uppercase_e_tag(&keys, &parent_id, "Comment content")
            .expect("Should create event");

        assert_eq!(event.kind.as_u16(), KIND_COMMENT);

        let has_upper_e_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "E")
        });
        assert!(has_upper_e_tag, "Should have uppercase 'E' tag");
    }

    #[test]
    fn test_build_layer3_quote_with_q() {
        let keys = Keys::generate();
        let parent_id = EventId::all_zeros();

        let event = build_layer3_quote_with_q_tag(&keys, &parent_id, "Quote content")
            .expect("Should create event");

        assert_eq!(event.kind.as_u16(), 1);

        let has_q_tag = event.tags.iter().any(|tag| {
            let slice = tag.as_slice();
            slice.first().is_some_and(|t| t == "q")
        });
        assert!(has_q_tag, "Should have 'q' tag");
    }
}