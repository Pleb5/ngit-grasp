/// Landing Page Handler
/// 
/// Serves the HTML landing page or upgrades to WebSocket for Nostr relay connections.

use actix_web::{web, HttpRequest, HttpResponse, Result};
use nostr_relay_builder::LocalRelay;

use crate::config::Config;

/// Handle landing page or WebSocket upgrade
pub async fn handle(
    req: HttpRequest,
    stream: web::Payload,
    config: web::Data<Config>,
    relay: web::Data<LocalRelay>,
) -> Result<HttpResponse> {
    // Check if this is a WebSocket upgrade request
    if let Some(upgrade) = req.headers().get("upgrade") {
        if upgrade.to_str().unwrap_or("").eq_ignore_ascii_case("websocket") {
            // Delegate to WebSocket handler
            return crate::http::websocket::handle(req, stream, relay).await;
        }
    }
    
    // Otherwise, serve the landing page
    let html = format!(
        include_str!("../../templates/landing.html"),
        relay_name = config.relay_name,
        relay_description = config.relay_description,
        domain = config.domain,
        bind_address = config.bind_address,
    );
    
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}