/// HTTP Server Module
///
/// Provides actix-web HTTP server with WebSocket upgrade support for the Nostr relay.
pub mod landing;
pub mod websocket;

use actix_web::{middleware, web, App, HttpServer};
use nostr_relay_builder::LocalRelay;

use crate::config::Config;

/// Start the HTTP server with integrated Nostr relay
pub async fn run_server(config: Config, relay: LocalRelay) -> anyhow::Result<()> {
    let bind_addr = config.bind_address.clone();

    tracing::info!("Starting HTTP server on {}", bind_addr);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(relay.clone()))
            .wrap(middleware::Logger::default())
            .route("/", web::get().to(landing::handle))
    })
    .bind(&bind_addr)?
    .run()
    .await?;

    Ok(())
}
