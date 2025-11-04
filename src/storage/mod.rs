use anyhow::Result;
use nostr_sdk::Event;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;

/// Simple in-memory storage for events
/// TODO: Persist to disk for production use
#[derive(Clone)]
pub struct Storage {
    events: Arc<RwLock<HashMap<String, Event>>>,
    data_path: String,
}

impl Storage {
    pub fn new(config: &Config) -> Result<Self> {
        // Create data directory if it doesn't exist
        std::fs::create_dir_all(&config.relay_data_path)?;

        Ok(Storage {
            events: Arc::new(RwLock::new(HashMap::new())),
            data_path: config.relay_data_path.clone(),
        })
    }

    pub async fn store_event(&self, event: Event) -> Result<()> {
        let mut events = self.events.write().await;
        events.insert(event.id.to_hex(), event);
        Ok(())
    }

    pub async fn get_event(&self, event_id: &str) -> Option<Event> {
        let events = self.events.read().await;
        events.get(event_id).cloned()
    }

    pub async fn query_events<F>(&self, filter: F) -> Vec<Event>
    where
        F: Fn(&Event) -> bool,
    {
        let events = self.events.read().await;
        events.values().filter(|e| filter(e)).cloned().collect()
    }

    pub async fn count_events(&self) -> usize {
        let events = self.events.read().await;
        events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Kind};

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let config = Config {
            domain: "test".to_string(),
            owner_npub: "npub1test".to_string(),
            relay_name: "test".to_string(),
            relay_description: "test".to_string(),
            git_data_path: "./test_data/git".to_string(),
            relay_data_path: "./test_data/relay".to_string(),
            bind_address: "127.0.0.1:8080".to_string(),
        };

        let storage = Storage::new(&config).unwrap();

        // Create a test event
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test content")
            .sign_with_keys(&keys)
            .unwrap();

        // Store it
        storage.store_event(event.clone()).await.unwrap();

        // Retrieve it
        let retrieved = storage.get_event(&event.id.to_hex()).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, event.id);

        // Count events
        assert_eq!(storage.count_events().await, 1);
    }

    #[tokio::test]
    async fn test_query_events() {
        let config = Config {
            domain: "test".to_string(),
            owner_npub: "npub1test".to_string(),
            relay_name: "test".to_string(),
            relay_description: "test".to_string(),
            git_data_path: "./test_data/git".to_string(),
            relay_data_path: "./test_data/relay".to_string(),
            bind_address: "127.0.0.1:8080".to_string(),
        };

        let storage = Storage::new(&config).unwrap();

        // Create multiple events
        let keys = Keys::generate();
        let event1 = EventBuilder::text_note("message 1")
            .sign_with_keys(&keys)
            .unwrap();
        let event2 = EventBuilder::text_note("message 2")
            .sign_with_keys(&keys)
            .unwrap();

        storage.store_event(event1.clone()).await.unwrap();
        storage.store_event(event2.clone()).await.unwrap();

        // Query all events
        let all_events = storage.query_events(|_| true).await;
        assert_eq!(all_events.len(), 2);

        // Query by kind
        let text_notes = storage
            .query_events(|e| e.kind == Kind::TextNote)
            .await;
        assert_eq!(text_notes.len(), 2);
    }
}
