/// HTTP Server Module
///
/// Provides hyper HTTP server with WebSocket upgrade support for the Nostr relay.
pub mod landing;
pub mod nip11;

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use hyper::body::{Bytes, Incoming};
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use http_body_util::{BodyExt, Full};
use nostr_sdk::hashes::sha1::Hash as Sha1Hash;
use nostr_sdk::hashes::{Hash, HashEngine};
use nostr_relay_builder::LocalRelay;
use tokio::net::TcpListener;
use base64::Engine;

use crate::config::Config;
use crate::git;

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
    type Response = Response<Full<Bytes>>;
    type Error = String;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let base = Response::builder().header("server", "ngit-grasp");
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|s| s.to_string());
        let method = req.method().clone();
        let git_data_path = self.config.git_data_path.clone();

        // Check for Git HTTP requests first
        if let Some((npub, identifier, subpath)) = git::parse_git_url(&path) {
            let npub = npub.to_string();
            let identifier = identifier.to_string();
            let subpath = subpath.to_string();
            
            tracing::debug!("Git request: {} {} (npub={}, id={}, subpath={})",
                method, path, npub, identifier, subpath);

            let repo_path = git::resolve_repo_path(&git_data_path, &npub, &identifier);

            return Box::pin(async move {
                // Collect request body once before the match statement
                let body_bytes = req.collect().await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_else(|_| Bytes::new());
                
                let result = match (method.as_ref(), subpath.as_str()) {
                    // GET /info/refs?service=git-upload-pack or git-receive-pack
                    (m, sp) if m == Method::GET && sp.starts_with("info/refs") => {
                        // Parse query string for service parameter
                        let service = query.as_deref().unwrap_or("")
                            .strip_prefix("service=")
                            .and_then(git::protocol::GitService::from_query_param);

                        match service {
                            Some(svc) => {
                                git::handlers::handle_info_refs(repo_path, svc).await
                            }
                            None => {
                                Err(git::handlers::GitError::RepositoryNotFound)
                            }
                        }
                    }
                    
                    // POST /git-upload-pack (clone/fetch)
                    (m, "git-upload-pack") if m == Method::POST => {
                        git::handlers::handle_upload_pack(repo_path, body_bytes).await
                    }
                    
                    // POST /git-receive-pack (push)
                    (m, "git-receive-pack") if m == Method::POST => {
                        git::handlers::handle_receive_pack(repo_path, body_bytes.clone()).await
                    }
                    
                    _ => {
                        Err(git::handlers::GitError::RepositoryNotFound)
                    }
                };

                match result {
                    Ok(response) => Ok(response),
                    Err(e) => {
                        tracing::error!("Git handler error: {}", e);
                        let error_msg = format!("Git error: {}", e);
                        Ok(Response::builder()
                            .status(e.status_code())
                            .body(Full::new(Bytes::from(error_msg)))
                            .unwrap())
                    }
                }
            });
        }

        // Check for NIP-11 relay information request (Accept: application/nostr+json)
        if let Some(accept) = req.headers().get("accept") {
            if accept
                .to_str()
                .map(|s| s.contains("application/nostr+json"))
                .unwrap_or(false)
            {
                let doc = nip11::RelayInformationDocument::from_config(&self.config);
                let json = doc.to_json().unwrap_or_else(|e| {
                    tracing::error!("Failed to serialize NIP-11 document: {}", e);
                    "{}".to_string()
                });
                
                tracing::debug!("Serving NIP-11 relay information document to {}", self.remote);
                
                return Box::pin(async move {
                    Ok(base
                        .status(200)
                        .header("content-type", "application/nostr+json")
                        .header("access-control-allow-origin", "*")
                        .body(Full::new(Bytes::from(json)))
                        .unwrap())
                });
            }
        }

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
                        .body(Full::new(Bytes::new()))
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
                .body(Full::new(Bytes::from(html)))
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
