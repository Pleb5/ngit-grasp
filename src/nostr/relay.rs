use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::{Event, EventId, Filter};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::storage::Storage;

type Subscriptions = Arc<RwLock<HashMap<String, Vec<Filter>>>>;

pub struct RelayServer {
    config: Config,
    storage: Storage,
}

impl RelayServer {
    pub fn new(config: Config, storage: Storage) -> Result<Self> {
        Ok(RelayServer { config, storage })
    }

    pub async fn run(self) -> Result<()> {
        let addr: SocketAddr = self.config.bind_address.parse()?;
        let listener = TcpListener::bind(&addr).await?;
        
        info!("✅ Nostr relay listening on ws://{}", addr);
        info!("📡 Ready to accept connections...");

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    debug!("New connection from: {}", peer_addr);
                    let storage = self.storage.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, storage).await {
                            error!("Error handling connection from {}: {}", peer_addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                }
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, storage: Storage) -> Result<()> {
    let ws_stream = accept_async(stream).await?;
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    
    let subscriptions: Subscriptions = Arc::new(RwLock::new(HashMap::new()));

    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                debug!("Received message: {}", text);
                
                match handle_message(&text, &storage, &subscriptions).await {
                    Ok(responses) => {
                        for response in responses {
                            let response_text = serde_json::to_string(&response)?;
                            debug!("Sending response: {}", response_text);
                            ws_sender.send(Message::Text(response_text)).await?;
                        }
                    }
                    Err(e) => {
                        warn!("Error handling message: {}", e);
                        let notice = json!(["NOTICE", format!("Error: {}", e)]);
                        ws_sender.send(Message::Text(notice.to_string())).await?;
                    }
                }
            }
            Ok(Message::Close(_)) => {
                debug!("Client closed connection");
                break;
            }
            Ok(Message::Ping(data)) => {
                ws_sender.send(Message::Pong(data)).await?;
            }
            Ok(_) => {
                // Ignore other message types
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

async fn handle_message(
    text: &str,
    storage: &Storage,
    subscriptions: &Subscriptions,
) -> Result<Vec<Value>> {
    let msg: Value = serde_json::from_str(text)?;
    
    if let Some(arr) = msg.as_array() {
        if arr.is_empty() {
            return Ok(vec![json!(["NOTICE", "Empty message"])]);
        }

        let msg_type = arr[0].as_str().unwrap_or("");
        
        match msg_type {
            "EVENT" => handle_event(arr, storage).await,
            "REQ" => handle_req(arr, storage, subscriptions).await,
            "CLOSE" => handle_close(arr, subscriptions).await,
            _ => Ok(vec![json!(["NOTICE", format!("Unknown message type: {}", msg_type)])]),
        }
    } else {
        Ok(vec![json!(["NOTICE", "Invalid message format"])])
    }
}

async fn handle_event(arr: &[Value], storage: &Storage) -> Result<Vec<Value>> {
    if arr.len() < 2 {
        return Ok(vec![json!(["NOTICE", "EVENT message requires event object"])]);
    }

    let event: Event = serde_json::from_value(arr[1].clone())?;
    let event_id = event.id;

    // Verify event (signature and ID)
    if event.verify().is_err() {
        return Ok(vec![json!(["OK", event_id.to_hex(), false, "invalid: signature or ID verification failed"])]);
    }

    // Check if event already exists
    if storage.get_event(&event_id.to_hex()).await.is_some() {
        return Ok(vec![json!(["OK", event_id.to_hex(), true, "duplicate: event already exists"])]);
    }

    // Store the event
    storage.store_event(event.clone()).await?;
    
    info!("✅ Stored event: {} (kind: {})", event_id, event.kind);
    
    Ok(vec![json!(["OK", event_id.to_hex(), true, ""])])
}

async fn handle_req(
    arr: &[Value],
    storage: &Storage,
    subscriptions: &Subscriptions,
) -> Result<Vec<Value>> {
    if arr.len() < 2 {
        return Ok(vec![json!(["NOTICE", "REQ message requires subscription ID"])]);
    }

    let sub_id = arr[1].as_str().ok_or_else(|| anyhow::anyhow!("Invalid subscription ID"))?;
    
    // Parse filters
    let mut filters = Vec::new();
    for filter_value in &arr[2..] {
        let filter: Filter = serde_json::from_value(filter_value.clone())?;
        filters.push(filter.clone());
    }

    // Store subscription
    {
        let mut subs = subscriptions.write().await;
        subs.insert(sub_id.to_string(), filters.clone());
    }

    debug!("Created subscription: {} with {} filters", sub_id, filters.len());

    // Query and send matching events
    let mut responses = Vec::new();
    
    for filter in filters {
        let events = storage.query_events(|event| {
            matches_filter(event, &filter)
        }).await;

        for event in events {
            responses.push(json!(["EVENT", sub_id, event]));
        }
    }

    // Send EOSE (End of Stored Events)
    responses.push(json!(["EOSE", sub_id]));
    
    debug!("Subscription {} returned {} events", sub_id, responses.len() - 1);

    Ok(responses)
}

async fn handle_close(arr: &[Value], subscriptions: &Subscriptions) -> Result<Vec<Value>> {
    if arr.len() < 2 {
        return Ok(vec![json!(["NOTICE", "CLOSE message requires subscription ID"])]);
    }

    let sub_id = arr[1].as_str().ok_or_else(|| anyhow::anyhow!("Invalid subscription ID"))?;
    
    {
        let mut subs = subscriptions.write().await;
        subs.remove(sub_id);
    }

    debug!("Closed subscription: {}", sub_id);

    Ok(vec![])
}

fn matches_filter(event: &Event, filter: &Filter) -> bool {
    // Check IDs
    if let Some(ref ids) = filter.ids {
        if !ids.is_empty() && !ids.contains(&event.id) {
            return false;
        }
    }

    // Check authors
    if let Some(ref authors) = filter.authors {
        if !authors.is_empty() && !authors.contains(&event.pubkey) {
            return false;
        }
    }

    // Check kinds
    if let Some(ref kinds) = filter.kinds {
        if !kinds.is_empty() && !kinds.contains(&event.kind) {
            return false;
        }
    }

    // Check since
    if let Some(since) = filter.since {
        if event.created_at < since {
            return false;
        }
    }

    // Check until
    if let Some(until) = filter.until {
        if event.created_at > until {
            return false;
        }
    }

    // TODO: Check tags (#e, #p, etc.)

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Kind};

    #[test]
    fn test_matches_filter_by_id() {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test")
            .sign_with_keys(&keys)
            .unwrap();

        // Filter matching the event ID
        let filter = Filter::new().id(event.id);
        assert!(matches_filter(&event, &filter));

        // Filter not matching
        let other_id = EventId::all_zeros();
        let filter = Filter::new().id(other_id);
        assert!(!matches_filter(&event, &filter));
    }

    #[test]
    fn test_matches_filter_by_author() {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test")
            .sign_with_keys(&keys)
            .unwrap();

        // Filter matching the author
        let filter = Filter::new().author(keys.public_key());
        assert!(matches_filter(&event, &filter));

        // Filter not matching
        let other_keys = Keys::generate();
        let filter = Filter::new().author(other_keys.public_key());
        assert!(!matches_filter(&event, &filter));
    }

    #[test]
    fn test_matches_filter_by_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test")
            .sign_with_keys(&keys)
            .unwrap();

        // Filter matching the kind
        let filter = Filter::new().kind(Kind::TextNote);
        assert!(matches_filter(&event, &filter));

        // Filter not matching
        let filter = Filter::new().kind(Kind::Metadata);
        assert!(!matches_filter(&event, &filter));
    }
}
