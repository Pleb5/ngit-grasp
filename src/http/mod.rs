/// HTTP Server Module
///
/// Provides hyper HTTP server with WebSocket upgrade support for the Nostr relay.
pub mod landing;

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use hyper::body::Incoming;
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use nostr_sdk::hashes::sha1::Hash as Sha1Hash;
use nostr_sdk::hashes::{Hash, HashEngine};
use nostr_relay_builder::LocalRelay;
use tokio::net::TcpListener;
use base64::Engine;

use crate::config::Config;

/// HTTP Service that serves both WebSocket (relay) and HTML landing page
struct HttpService {
    relay: LocalRelay,
    config: Config,
    remote: SocketAddr,
}

impl HttpService {
    fn new(relay: LocalRelay, config: Config, remote: SocketAddr) -> Self {
        Self {
            relay,
            config,
            remote,
        }
    }
}

impl Service<Request<Incoming>> for HttpService {
    type Response = Response<String>;
    type Error = String;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let base = Response::builder().header("server", "ngit-grasp");

        // Check if this is a WebSocket upgrade request
        if let (Some(c), Some(w)) = (
            req.headers().get("connection"),
            req.headers().get("upgrade"),
        ) {
            if c.to_str()
                .map(|s| s.to_lowercase() == "upgrade")
                .unwrap_or(false)
                && w.to_str()
                    .map(|s| s.to_lowercase() == "websocket")
                    .unwrap_or(false)
            {
                let key = req.headers().get("sec-websocket-key");
                let derived = key.map(|k| derive_accept_key(k.as_bytes()));

                let addr = self.remote;
                let relay = self.relay.clone();
                
                tokio::spawn(async move {
                    match hyper::upgrade::on(req).await {
                        Ok(upgraded) => {
                            tracing::info!("WebSocket connection established from {}", addr);
                            if let Err(e) = relay.take_connection(TokioIo::new(upgraded), addr).await
                            {
                                tracing::error!("Relay error for {}: {}", addr, e);
                            }
                            tracing::info!("WebSocket connection closed for {}", addr);
                        }
                        Err(e) => tracing::error!("Upgrade error: {}", e),
                    }
                });

                return Box::pin(async move {
                    Ok(base
                        .status(101)
                        .header(CONNECTION, "upgrade")
                        .header(UPGRADE, "websocket")
                        .header(SEC_WEBSOCKET_ACCEPT, derived.unwrap())
                        .body("".to_string())
                        .unwrap())
                });
            }
        }

        // Serve landing page for HTTP requests
        let html = landing::get_html(&self.config);
        Box::pin(async move {
            Ok(base
                .status(200)
                .header("content-type", "text/html; charset=utf-8")
                .body(html)
                .unwrap())
        })
    }
}

/// Derive the `Sec-WebSocket-Accept` response header from a `Sec-WebSocket-Key` request header
fn derive_accept_key(request_key: &[u8]) -> String {
    const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut engine = Sha1Hash::engine();
    engine.input(request_key);
    engine.input(WS_GUID);
    let hash: Sha1Hash = Sha1Hash::from_engine(engine);
    base64::prelude::BASE64_STANDARD.encode(hash)
}

/// Start the HTTP server with integrated Nostr relay
pub async fn run_server(config: Config, relay: LocalRelay) -> anyhow::Result<()> {
    let bind_addr: SocketAddr = config.bind_address.parse()?;

    tracing::info!("Starting HTTP server on {}", bind_addr);
    tracing::info!("Relay name: {}", config.relay_name);
    tracing::info!("Domain: {}", config.domain);

    let listener = TcpListener::bind(&bind_addr).await?;
    
    loop {
        let (socket, addr) = listener.accept().await?;
        let io = TokioIo::new(socket);
        let service = HttpService::new(relay.clone(), config.clone(), addr);
        
        tokio::spawn(async move {
            if let Err(e) = http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                tracing::error!("Failed to handle request from {}: {}", addr, e);
            }
        });
    }
}
