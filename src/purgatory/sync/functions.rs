//! Core sync functions for identifier-based purgatory synchronization.
//!
//! This module provides the two main functions that both the main sync loop
//! and `DomainThrottle` queue processing use:
//!
//! - [`sync_identifier_next_url`]: Pure URL selection logic - finds next URL to try
//! - [`sync_identifier_from_url`]: Pure fetch logic - fetches from a specific URL
//!
//! The separation enables:
//! - Main sync loop to try non-throttled URLs immediately
//! - DomainThrottle to process queued identifiers when capacity frees
//! - Clean testability with mocked SyncContext

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;

use super::context::SyncContext;
use super::throttle::ThrottleManager;

/// Extract domain from a URL.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(extract_domain("https://github.com/foo/bar.git"), Some("github.com".to_string()));
/// assert_eq!(extract_domain("git@github.com:foo/bar.git"), None); // SSH URLs not supported
/// ```
fn extract_domain(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
}

/// Find the next URL to try for an identifier.
///
/// This is pure URL selection logic with no side effects. It:
/// 1. Checks if there are pending events for the identifier
/// 2. Checks if there are OIDs still needed
/// 3. Gets repository data and extracts clone URLs
/// 4. Filters out our own domain and already-tried URLs
/// 5. Returns the first non-throttled URL (when `domain` is None)
///    or a URL from the specified domain (when `domain` is Some)
///
/// # Arguments
///
/// * `ctx` - The sync context providing repository data and OID information
/// * `identifier` - The repository identifier (d-tag value)
/// * `domain` - If Some, only return URLs from this specific domain.
///              If None, return any non-throttled URL.
/// * `tried_urls` - URLs that have already been tried (will be skipped)
/// * `throttle_manager` - Used to check if domains are throttled (when domain is None)
///
/// # Returns
///
/// * `Some(url)` - The next URL to try
/// * `None` - No suitable URL found (all tried, all throttled, or no URLs available)
pub async fn sync_identifier_next_url<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    domain: Option<&str>,
    tried_urls: &HashSet<String>,
    throttle_manager: &ThrottleManager,
) -> Option<String> {
    // 1. Check if we still have pending events
    if !ctx.has_pending_events(identifier) {
        debug!(
            identifier = %identifier,
            "No pending events - skipping URL selection"
        );
        return None;
    }

    // 2. Collect needed OIDs
    let needed_oids = ctx.collect_needed_oids(identifier);
    if needed_oids.is_empty() {
        debug!(
            identifier = %identifier,
            "No OIDs needed - sync is complete"
        );
        return None;
    }

    // 3. Get repository data
    let repo_data = match ctx.fetch_repository_data(identifier).await {
        Ok(data) => data,
        Err(e) => {
            debug!(
                identifier = %identifier,
                error = %e,
                "Failed to fetch repository data"
            );
            return None;
        }
    };

    // 4. Collect clone URLs, excluding our domain
    let our_domain = ctx.our_domain();
    let all_urls: HashSet<String> = repo_data
        .announcements
        .iter()
        .flat_map(|a| a.clone_urls.iter().cloned())
        .filter(|url| our_domain.map_or(true, |d| !url.contains(d)))
        .collect();

    if all_urls.is_empty() {
        debug!(
            identifier = %identifier,
            "No clone URLs available (after filtering our domain)"
        );
        return None;
    }

    // 5. Group by domain
    let urls_by_domain: HashMap<String, Vec<String>> =
        all_urls.iter().fold(HashMap::new(), |mut acc, url| {
            if let Some(d) = extract_domain(url) {
                acc.entry(d).or_default().push(url.clone());
            }
            acc
        });

    // 6. Find an available URL
    match domain {
        Some(specific_domain) => {
            // Only look at URLs from this specific domain
            urls_by_domain.get(specific_domain).and_then(|urls| {
                urls.iter()
                    .find(|url| !tried_urls.contains(*url))
                    .cloned()
            })
        }
        None => {
            // Try any non-throttled domain
            for (d, domain_urls) in &urls_by_domain {
                if throttle_manager.is_throttled(d) {
                    debug!(
                        identifier = %identifier,
                        domain = %d,
                        "Domain is throttled - skipping"
                    );
                    continue;
                }
                if let Some(url) = domain_urls.iter().find(|url| !tried_urls.contains(*url)) {
                    return Some(url.clone());
                }
            }
            None
        }
    }
}

/// Information about throttled domains with untried URLs.
///
/// Used by the main sync loop to know which `DomainThrottle` queues
/// to add the identifier to when it can't complete immediately.
#[derive(Debug, Clone)]
pub struct ThrottledDomainInfo {
    /// The throttled domain name
    pub domain: String,
    /// URLs from this domain that have already been tried
    pub tried_urls_for_domain: HashSet<String>,
}

/// Get information about throttled domains that have untried URLs.
///
/// Called by main sync loop to know which `DomainThrottle` queues to add
/// the identifier to when non-throttled URLs are exhausted.
///
/// # Arguments
///
/// * `ctx` - The sync context providing repository data
/// * `identifier` - The repository identifier
/// * `tried_urls` - All URLs that have been tried (across all domains)
/// * `throttle_manager` - Used to check which domains are throttled
///
/// # Returns
///
/// A list of throttled domains that still have untried URLs, along with
/// the tried URLs for each domain (for proper queue state).
pub async fn get_throttled_domains_with_untried_urls<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    tried_urls: &HashSet<String>,
    throttle_manager: &ThrottleManager,
) -> Vec<ThrottledDomainInfo> {
    let repo_data = match ctx.fetch_repository_data(identifier).await {
        Ok(data) => data,
        Err(_) => return vec![],
    };

    let our_domain = ctx.our_domain();
    let all_urls: HashSet<String> = repo_data
        .announcements
        .iter()
        .flat_map(|a| a.clone_urls.iter().cloned())
        .filter(|url| our_domain.map_or(true, |d| !url.contains(d)))
        .collect();

    let urls_by_domain: HashMap<String, Vec<String>> =
        all_urls.iter().fold(HashMap::new(), |mut acc, url| {
            if let Some(d) = extract_domain(url) {
                acc.entry(d).or_default().push(url.clone());
            }
            acc
        });

    urls_by_domain
        .into_iter()
        .filter_map(|(domain, domain_urls)| {
            if !throttle_manager.is_throttled(&domain) {
                return None; // Not throttled, skip
            }

            let untried: Vec<_> = domain_urls
                .iter()
                .filter(|url| !tried_urls.contains(*url))
                .collect();

            if untried.is_empty() {
                return None; // All URLs tried for this domain
            }

            // Collect tried URLs that belong to this domain
            let tried_urls_for_domain: HashSet<String> = tried_urls
                .iter()
                .filter(|url| extract_domain(url).as_deref() == Some(domain.as_str()))
                .cloned()
                .collect();

            Some(ThrottledDomainInfo {
                domain,
                tried_urls_for_domain,
            })
        })
        .collect()
}

/// Fetch git data from a specific URL for an identifier.
///
/// This function:
/// 1. Records the request with the throttle manager (for rate limiting)
/// 2. Performs the actual git fetch via the context
/// 3. Processes any events that can now be satisfied
/// 4. Records request completion
///
/// # Arguments
///
/// * `ctx` - The sync context providing fetch and processing capabilities
/// * `identifier` - The repository identifier
/// * `url` - The remote URL to fetch from
/// * `throttle_manager` - Used to track request start/completion for rate limiting
///
/// # Returns
///
/// The number of OIDs successfully fetched (0 on failure)
pub async fn sync_identifier_from_url<C: SyncContext>(
    ctx: &C,
    identifier: &str,
    url: &str,
    throttle_manager: &Arc<ThrottleManager>,
) -> usize {
    let domain = match extract_domain(url) {
        Some(d) => d,
        None => {
            debug!(
                identifier = %identifier,
                url = %url,
                "Could not extract domain from URL"
            );
            return 0;
        }
    };

    // Get repository data for target repo path
    let repo_data = match ctx.fetch_repository_data(identifier).await {
        Ok(data) => data,
        Err(e) => {
            debug!(
                identifier = %identifier,
                error = %e,
                "Failed to fetch repo data"
            );
            return 0;
        }
    };

    let target_repo = match ctx.find_target_repo(&repo_data) {
        Some(path) => path,
        None => {
            debug!(identifier = %identifier, "No target repo found");
            return 0;
        }
    };

    // Collect needed OIDs
    let needed_oids: Vec<String> = ctx.collect_needed_oids(identifier).into_iter().collect();
    if needed_oids.is_empty() {
        debug!(
            identifier = %identifier,
            "No OIDs needed - nothing to fetch"
        );
        return 0;
    }

    // Perform the fetch with throttle tracking
    throttle_manager.start_request(&domain);
    let fetch_result = ctx.fetch_oids(&target_repo, url, &needed_oids).await;
    throttle_manager.complete_request(&domain);

    let oids_fetched = match fetch_result {
        Ok(fetched) => {
            debug!(
                identifier = %identifier,
                url = %url,
                oids_fetched = fetched.len(),
                "Fetch succeeded"
            );
            fetched.len()
        }
        Err(e) => {
            debug!(
                identifier = %identifier,
                url = %url,
                error = %e,
                "Fetch failed"
            );
            0
        }
    };

    // Try to process any events that can now be satisfied
    if oids_fetched > 0 {
        let new_oids: HashSet<String> = needed_oids.into_iter().collect();
        if let Err(e) = ctx
            .process_newly_available_git_data(&target_repo, &new_oids)
            .await
        {
            debug!(
                identifier = %identifier,
                error = %e,
                "Failed to process newly available git data"
            );
        }
    }

    oids_fetched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::purgatory::sync::MockSyncContext;

    #[tokio::test]
    async fn next_url_skips_throttled_domains() {
        // Set up mock with URLs from two domains
        let mock = MockSyncContext::new()
            .with_urls(&[
                "https://github.com/foo/bar.git",
                "https://gitlab.com/foo/bar.git",
            ])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true);

        // Create throttle manager and throttle github.com
        let throttle_manager = ThrottleManager::new(1, 100);

        // Saturate github.com by starting a request
        throttle_manager.start_request("github.com");

        // Should return gitlab.com URL since github.com is throttled
        let tried_urls = HashSet::new();
        let result =
            sync_identifier_next_url(&mock, "test-repo", None, &tried_urls, &throttle_manager)
                .await;

        assert!(result.is_some());
        let url = result.unwrap();
        assert!(
            url.contains("gitlab.com"),
            "Expected gitlab.com URL, got: {}",
            url
        );
    }

    #[tokio::test]
    async fn next_url_skips_tried_urls() {
        // Set up mock with two URLs from same domain
        let mock = MockSyncContext::new()
            .with_urls(&[
                "https://github.com/foo/bar.git",
                "https://github.com/foo/bar2.git",
            ])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true);

        let throttle_manager = ThrottleManager::new(5, 100);

        // Mark first URL as tried
        let mut tried_urls = HashSet::new();
        tried_urls.insert("https://github.com/foo/bar.git".to_string());

        // Should return the second URL
        let result =
            sync_identifier_next_url(&mock, "test-repo", None, &tried_urls, &throttle_manager)
                .await;

        assert!(result.is_some());
        let url = result.unwrap();
        assert_eq!(url, "https://github.com/foo/bar2.git");
    }

    #[tokio::test]
    async fn next_url_returns_none_when_no_pending_events() {
        let mock = MockSyncContext::new()
            .with_urls(&["https://github.com/foo/bar.git"])
            .with_needed_oids(&["abc123"])
            .with_pending_events(false); // No pending events

        let throttle_manager = ThrottleManager::new(5, 100);
        let tried_urls = HashSet::new();

        let result =
            sync_identifier_next_url(&mock, "test-repo", None, &tried_urls, &throttle_manager)
                .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn next_url_returns_none_when_no_oids_needed() {
        let mock = MockSyncContext::new()
            .with_urls(&["https://github.com/foo/bar.git"])
            .with_needed_oids(&[]) // No OIDs needed
            .with_pending_events(true);

        let throttle_manager = ThrottleManager::new(5, 100);
        let tried_urls = HashSet::new();

        let result =
            sync_identifier_next_url(&mock, "test-repo", None, &tried_urls, &throttle_manager)
                .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn next_url_filters_our_domain() {
        let mock = MockSyncContext::new()
            .with_urls(&[
                "https://our-relay.com/foo/bar.git",
                "https://github.com/foo/bar.git",
            ])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true)
            .with_our_domain("our-relay.com");

        let throttle_manager = ThrottleManager::new(5, 100);
        let tried_urls = HashSet::new();

        let result =
            sync_identifier_next_url(&mock, "test-repo", None, &tried_urls, &throttle_manager)
                .await;

        assert!(result.is_some());
        let url = result.unwrap();
        assert!(
            url.contains("github.com"),
            "Expected github.com URL (our domain filtered), got: {}",
            url
        );
    }

    #[tokio::test]
    async fn next_url_with_specific_domain() {
        let mock = MockSyncContext::new()
            .with_urls(&[
                "https://github.com/foo/bar.git",
                "https://gitlab.com/foo/bar.git",
            ])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true);

        let throttle_manager = ThrottleManager::new(5, 100);
        let tried_urls = HashSet::new();

        // Request specific domain
        let result = sync_identifier_next_url(
            &mock,
            "test-repo",
            Some("gitlab.com"),
            &tried_urls,
            &throttle_manager,
        )
        .await;

        assert!(result.is_some());
        let url = result.unwrap();
        assert!(
            url.contains("gitlab.com"),
            "Expected gitlab.com URL, got: {}",
            url
        );
    }

    #[tokio::test]
    async fn from_url_fetches_and_processes_on_success() {
        // Set up mock that can provide the needed OID
        let mock = MockSyncContext::new()
            .with_urls(&["https://github.com/foo/bar.git"])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true)
            .url_provides("https://github.com/foo/bar.git", &["abc123"]);

        let throttle_manager = Arc::new(ThrottleManager::new(5, 100));

        // Fetch from the URL
        let fetched = sync_identifier_from_url(
            &mock,
            "test-repo",
            "https://github.com/foo/bar.git",
            &throttle_manager,
        )
        .await;

        // Should have fetched 1 OID
        assert_eq!(fetched, 1);

        // Should have logged the fetch attempt
        let fetch_log = mock.fetch_log();
        assert_eq!(fetch_log.len(), 1);
        assert_eq!(fetch_log[0], "https://github.com/foo/bar.git");

        // OID should no longer be needed
        assert!(mock.current_needed_oids().is_empty());
    }

    #[tokio::test]
    async fn from_url_returns_zero_on_failure() {
        let mock = MockSyncContext::new()
            .with_urls(&["https://bad-server.com/repo.git"])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true)
            .url_should_fail("https://bad-server.com/repo.git");

        let throttle_manager = Arc::new(ThrottleManager::new(5, 100));

        let fetched = sync_identifier_from_url(
            &mock,
            "test-repo",
            "https://bad-server.com/repo.git",
            &throttle_manager,
        )
        .await;

        // Should return 0 on failure
        assert_eq!(fetched, 0);

        // OID should still be needed
        assert!(mock.current_needed_oids().contains("abc123"));
    }

    #[tokio::test]
    async fn from_url_tracks_throttle_requests() {
        let mock = MockSyncContext::new()
            .with_urls(&["https://github.com/foo/bar.git"])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true)
            .url_provides("https://github.com/foo/bar.git", &["abc123"]);

        let throttle_manager = Arc::new(ThrottleManager::new(1, 100));

        // First request should work
        let fetched = sync_identifier_from_url(
            &mock,
            "test-repo",
            "https://github.com/foo/bar.git",
            &throttle_manager,
        )
        .await;
        assert_eq!(fetched, 1);

        // After completion, domain should not be throttled
        assert!(!throttle_manager.is_throttled("github.com"));
    }

    #[tokio::test]
    async fn get_throttled_domains_returns_only_throttled_with_untried() {
        let mock = MockSyncContext::new()
            .with_urls(&[
                "https://github.com/foo/bar.git",
                "https://gitlab.com/foo/bar.git",
                "https://bitbucket.org/foo/bar.git",
            ])
            .with_needed_oids(&["abc123"])
            .with_pending_events(true);

        let throttle_manager = ThrottleManager::new(1, 100);

        // Throttle github.com and gitlab.com
        throttle_manager.start_request("github.com");
        throttle_manager.start_request("gitlab.com");

        // Mark github.com URL as already tried
        let mut tried_urls = HashSet::new();
        tried_urls.insert("https://github.com/foo/bar.git".to_string());

        let throttled =
            get_throttled_domains_with_untried_urls(&mock, "test-repo", &tried_urls, &throttle_manager)
                .await;

        // Should only include gitlab.com (throttled with untried URLs)
        // github.com is throttled but URL was tried
        // bitbucket.org is not throttled
        assert_eq!(throttled.len(), 1);
        assert_eq!(throttled[0].domain, "gitlab.com");
        assert!(throttled[0].tried_urls_for_domain.is_empty());
    }
}
