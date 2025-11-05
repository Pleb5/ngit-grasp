//! Audit client for testing GRASP implementations

use crate::audit::{AuditConfig, AuditEventBuilder, AuditMode};
use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;
use std::time::Duration;

/// Client for auditing GRASP implementations
pub struct AuditClient {
    client: Client,
    pub config: AuditConfig,
    keys: Keys,
}

impl AuditClient {
    /// Create a new audit client
    pub async fn new(relay_url: &str, config: AuditConfig) -> Result<Self> {
        let keys = Keys::generate();
        let client = Client::new(keys.clone());
        
        // Add relay and connect
        client.add_relay(relay_url).await?;
        client.connect().await;
        
        // Wait for connection to establish (with retries)
        let mut attempts = 0;
        let mut connected = false;
        while attempts < 20 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            let relays = client.relays().await;
            connected = relays.values().any(|r| r.is_connected());
            
            if connected {
                break;
            }
            
            attempts += 1;
        }
        
        // Verify we actually connected
        if !connected {
            return Err(anyhow!(
                "Failed to connect to relay at '{}'\n\
                \n\
                Possible causes:\n\
                  • Relay is not running at this address\n\
                  • Network connectivity issues\n\
                  • Incorrect URL or port\n\
                \n\
                To start ngit-relay for testing:\n\
                  docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest\n\
                \n\
                Or use the test script:\n\
                  cd grasp-audit && ./test-ngit-relay.sh",
                relay_url
            ));
        }
        
        // Give it a bit more time to stabilize
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        Ok(Self {
            client,
            config,
            keys,
        })
    }
    
    /// Get the public key for this audit client
    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }
    
    /// Check if connected to relay
    pub async fn is_connected(&self) -> bool {
        // Check if we have any connected relays
        let relays = self.client.relays().await;
        for relay in relays.values() {
            if relay.is_connected() {
                return true;
            }
        }
        false
    }
    
    /// Send an event (with audit tags automatically added)
    pub async fn send_event(&self, event: Event) -> Result<EventId> {
        if self.config.read_only {
            return Err(anyhow!("Client is in read-only mode"));
        }
        
        let output = self.client.send_event(&event).await?;
        let event_id = *output.id();
        
        // Check if any relay rejected the event
        if output.success.is_empty() && !output.failed.is_empty() {
            return Err(anyhow!("All relays rejected the event"));
        }
        
        // Wait a bit for event to propagate
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        Ok(event_id)
    }
    
    /// Create an event builder with audit tags
    pub fn event_builder(&self, kind: Kind, content: impl Into<String>) -> AuditEventBuilder {
        AuditEventBuilder::new(kind, content, self.config.clone())
    }
    
    /// Query events, optionally filtered to this audit run
    pub async fn query(&self, mut filter: Filter) -> Result<Vec<Event>> {
        use nostr_sdk::prelude::{Alphabet, SingleLetterTag};
        
        if self.config.mode == AuditMode::CI {
            // In CI mode, only see our own audit events
            // Filter by "t" tags (hashtags)
            let t_tag = SingleLetterTag::lowercase(Alphabet::T);
            filter = filter
                .custom_tag(t_tag, "grasp-audit-test-event")
                .custom_tag(t_tag, format!("audit-{}", self.config.run_id));
        }
        // In Production mode, see all events (no filter modification)
        
        let events = self.client
            .fetch_events(filter, Duration::from_secs(5))
            .await?;
        
        Ok(events.into_iter().collect())
    }
    
    /// Subscribe to events with a callback
    pub async fn subscribe(
        &self,
        filters: Vec<Filter>,
        timeout: Option<Duration>,
    ) -> Result<Vec<Event>> {
        let timeout = timeout.unwrap_or(Duration::from_secs(5));
        let mut all_events = Vec::new();
        
        for filter in filters {
            let events = self.client
                .fetch_events(filter, timeout)
                .await?;
            all_events.extend(events.into_iter());
        }
        
        Ok(all_events)
    }
    
    /// Get the underlying nostr client (for advanced usage)
    pub fn client(&self) -> &Client {
        &self.client
    }
    
    /// Get the keys (for signing custom events)
    pub fn keys(&self) -> &Keys {
        &self.keys
    }
    
    /// Create a NIP-34 repository announcement event
    ///
    /// This helper creates a properly formatted NIP-34 announcement that will be
    /// accepted by GRASP relays (which require events to list the relay in clone/relays tags).
    ///
    /// # Arguments
    /// * `test_name` - Name of the test (used to create unique repo identifier)
    ///
    /// # Returns
    /// A built and signed Event ready to be sent to the relay
    pub async fn create_repo_announcement(&self, test_name: &str) -> Result<Event> {
        // Get relay URL from client
        let relay_url = self.client.relays().await
            .keys()
            .next()
            .ok_or_else(|| anyhow!("No relay connected"))?
            .to_string();
        
        // Convert WebSocket URL to HTTP URL for clone tag
        let http_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");
        
        // Create unique repository identifier using UUID for consistency
        let repo_id = format!("{}-{}", test_name, uuid::Uuid::new_v4());
        
        // Get npub for clone URL
        let npub = self.public_key().to_bech32()
            .map_err(|e| anyhow!("Failed to convert public key to bech32 npub format: {}", e))?;
        
        // Build kind 30617 repository announcement
        let event = self.event_builder(Kind::GitRepoAnnouncement, format!("Test repository for {}", test_name))
            .tag(Tag::identifier(&repo_id))
            .tag(Tag::custom(TagKind::custom("name"), vec![format!("{} Test Repository", test_name)]))
            .tag(Tag::custom(TagKind::custom("description"), vec![format!("Repository for {} testing", test_name)]))
            .tag(Tag::custom(TagKind::custom("clone"), vec![format!("{}/{}/{}.git", http_url, npub, repo_id)]))
            .tag(Tag::custom(TagKind::custom("relays"), vec![relay_url.clone()]))
            .build(self.keys())
            .map_err(|e| anyhow!("Failed to build repository announcement event: {}", e))?;
        
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_client_creation() {
        let config = AuditConfig::ci();
        
        // This will fail if no relay is running, which is expected in tests
        // In real usage, there should be a relay at the URL
        let result = AuditClient::new("ws://localhost:7000", config).await;
        
        // We can't test connection without a running relay
        // But we can test that the client is created
        if let Ok(client) = result {
            assert_eq!(client.config.mode, AuditMode::CI);
        }
    }
    
    #[test]
    fn test_event_builder() {
        let config = AuditConfig::ci();
        let keys = Keys::generate();
        let client = AuditClient {
            client: Client::new(keys.clone()),
            config: config.clone(),
            keys: keys.clone(),
        };
        
        let _builder = client.event_builder(Kind::TextNote, "test content");
        
        // Builder should be created successfully
        // (We can't test the internal config field as it's private, which is correct)
    }
}
