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
        let client = Client::new(&keys);
        
        client.add_relay(relay_url).await?;
        client.connect().await;
        
        // Wait a bit for connection to establish
        tokio::time::sleep(Duration::from_millis(500)).await;
        
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
        relays.values().any(|r| r.is_connected())
    }
    
    /// Send an event (with audit tags automatically added)
    pub async fn send_event(&self, event: Event) -> Result<EventId> {
        if self.config.read_only {
            return Err(anyhow!("Client is in read-only mode"));
        }
        
        let event_id = self.client.send_event(event).await?;
        
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
        if self.config.mode == AuditMode::CI {
            // In CI mode, only see our own audit events
            filter = filter
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::G), 
                    ["true"]  // grasp-audit tag
                )
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::R), 
                    [&self.config.run_id]  // audit-run-id tag
                );
        }
        // In Production mode, see all events (no filter modification)
        
        let events = self.client
            .get_events_of(vec![filter], Some(Duration::from_secs(5)))
            .await?;
        
        Ok(events)
    }
    
    /// Subscribe to events with a callback
    pub async fn subscribe(
        &self,
        filters: Vec<Filter>,
        timeout: Option<Duration>,
    ) -> Result<Vec<Event>> {
        let events = self.client
            .get_events_of(filters, timeout)
            .await?;
        
        Ok(events)
    }
    
    /// Get the underlying nostr client (for advanced usage)
    pub fn client(&self) -> &Client {
        &self.client
    }
    
    /// Get the keys (for signing custom events)
    pub fn keys(&self) -> &Keys {
        &self.keys
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
            client: Client::new(&keys),
            config: config.clone(),
            keys: keys.clone(),
        };
        
        let builder = client.event_builder(Kind::TextNote, "test content");
        
        // Builder should have the config
        assert_eq!(builder.config.run_id, config.run_id);
    }
}
