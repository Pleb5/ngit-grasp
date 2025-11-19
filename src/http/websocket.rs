/// WebSocket Handler
///
/// Handles WebSocket upgrade requests and passes connections to the Nostr relay.
use actix_web::{web, Error, HttpRequest, HttpResponse, Result};
use actix_ws::Message;
use futures_util::StreamExt;
use nostr_relay_builder::LocalRelay;

/// Handle WebSocket upgrade and relay connection
pub async fn handle(
    req: HttpRequest,
    stream: web::Payload,
    relay: web::Data<LocalRelay>,
) -> Result<HttpResponse, Error> {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, stream)?;

    let peer_addr = req
        .peer_addr()
        .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());

    tracing::debug!("WebSocket connection from {}", peer_addr);

    // Spawn task to handle the WebSocket connection
    // TODO: Will use relay.take_connection() for full Nostr relay integration
    let _relay = relay.get_ref().clone();
    actix_web::rt::spawn(async move {
        // Create a channel to communicate between actix-ws and relay
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn task to send messages from relay to client
        let mut session_clone = session.clone();
        actix_web::rt::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if session_clone.text(msg).await.is_err() {
                    break;
                }
            }
        });

        // Handle incoming messages from client
        while let Some(Ok(msg)) = msg_stream.next().await {
            match msg {
                Message::Text(text) => {
                    // For now, just echo back - will integrate with relay in next phase
                    tracing::debug!("Received text message: {}", text);
                    if let Err(e) = tx.send(text.to_string()) {
                        tracing::error!("Failed to send message: {}", e);
                        break;
                    }
                }
                Message::Binary(_) => {
                    tracing::warn!("Received unexpected binary message");
                }
                Message::Close(_) => {
                    tracing::debug!("Client closed connection");
                    break;
                }
                Message::Ping(bytes) => {
                    if session.pong(&bytes).await.is_err() {
                        break;
                    }
                }
                Message::Pong(_) => {}
                Message::Continuation(_) => {}
                Message::Nop => {}
            }
        }

        tracing::debug!("WebSocket connection closed for {}", peer_addr);
    });

    Ok(response)
}
