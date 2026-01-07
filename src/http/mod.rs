/// HTTP Server Module
///
/// Provides hyper HTTP server with WebSocket upgrade support for the Nostr relay.
pub mod landing;
pub mod nip11;

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use base64::Engine;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use nostr_relay_builder::LocalRelay;
use nostr_sdk::hashes::sha1::Hash as Sha1Hash;
use nostr_sdk::hashes::{Hash, HashEngine};
use nostr_sdk::PublicKey;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::git;
use crate::metrics::Metrics;
use crate::nostr::builder::SharedDatabase;
use crate::purgatory::Purgatory;

/// CORS headers required by GRASP-01 specification (lines 40-47)
const CORS_ALLOW_ORIGIN: &str = "*";
const CORS_ALLOW_METHODS: &str = "GET, POST";
const CORS_ALLOW_HEADERS: &str = "Content-Type";

/// Embedded icon image (Grasp logo)
const ICON_PNG: &[u8] = include_bytes!("../../static/icon.png");

/// Extract npub and identifier from a repository URL path (no git subpath required)
///
/// Parses paths like `/<npub>/<identifier>.git` (for repository webpage/404)
///
/// Returns (npub, identifier) if the path matches a repository URL pattern
fn parse_repo_url(path: &str) -> Option<(&str, &str)> {
    // Remove leading slash
    let path = path.strip_prefix('/').unwrap_or(path);

    // Split into components
    let parts: Vec<&str> = path.split('/').collect();

    // Must be exactly 2 parts: npub and repo.git (no subpath)
    if parts.len() != 2 {
        return None;
    }

    let npub = parts[0];
    let repo_part = parts[1];

    // The repo part must end with .git
    if !repo_part.ends_with(".git") {
        return None;
    }

    // Must have an npub that looks valid (starts with npub1)
    if !npub.starts_with("npub1") {
        return None;
    }

    // Extract identifier (remove .git suffix)
    let identifier = repo_part.strip_suffix(".git").unwrap_or(repo_part);

    // Identifier must not be empty
    if identifier.is_empty() {
        return None;
    }

    Some((npub, identifier))
}

/// Add CORS headers to a response builder
fn add_cors_headers(builder: hyper::http::response::Builder) -> hyper::http::response::Builder {
    builder
        .header("Access-Control-Allow-Origin", CORS_ALLOW_ORIGIN)
        .header("Access-Control-Allow-Methods", CORS_ALLOW_METHODS)
        .header("Access-Control-Allow-Headers", CORS_ALLOW_HEADERS)
}

/// HTTP Service that serves both WebSocket (relay) and HTML landing page
struct HttpService {
    relay: LocalRelay,
    config: Config,
    remote: SocketAddr,
    /// Database reference for direct queries (e.g., push authorization)
    database: SharedDatabase,
    /// Optional metrics for Prometheus endpoint
    metrics: Option<Arc<Metrics>>,
    /// Purgatory for event/git coordination
    purgatory: Arc<Purgatory>,
}

impl HttpService {
    fn new(
        relay: LocalRelay,
        config: Config,
        remote: SocketAddr,
        database: SharedDatabase,
        metrics: Option<Arc<Metrics>>,
        purgatory: Arc<Purgatory>,
    ) -> Self {
        Self {
            relay,
            config,
            remote,
            database,
            metrics,
            purgatory,
        }
    }
}

impl Service<Request<Incoming>> for HttpService {
    type Response = Response<Full<Bytes>>;
    type Error = String;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let base = add_cors_headers(Response::builder().header("server", "ngit-grasp"));
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|s| s.to_string());
        let method = req.method().clone();
        let git_data_path = self.config.effective_git_data_path();
        let database = self.database.clone();
        let purgatory = self.purgatory.clone();

        // Handle OPTIONS preflight requests (CORS)
        // GRASP-01 spec line 47: Respond to OPTIONS with 204 No Content
        if method == Method::OPTIONS {
            return Box::pin(async move {
                Ok(
                    add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                        .status(204)
                        .body(Full::new(Bytes::new()))
                        .unwrap(),
                )
            });
        }

        // Check for Git HTTP requests first
        if let Some((npub, identifier, subpath)) = git::parse_git_url(&path) {
            let npub = npub.to_string();
            let identifier = identifier.to_string();
            let subpath = subpath.to_string();

            // Extract Git-Protocol header for protocol v2 support
            let git_protocol = req
                .headers()
                .get("git-protocol")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            tracing::debug!(
                "Git request: {} {} (npub={}, id={}, subpath={}, protocol={:?})",
                method,
                path,
                npub,
                identifier,
                subpath,
                git_protocol
            );

            let repo_path = git::resolve_repo_path(&git_data_path, &npub, &identifier);
            let metrics_clone = self.metrics.clone();
            let relay = self.relay.clone();

            return Box::pin(async move {
                // Collect request body once before the match statement
                let body_bytes = req
                    .collect()
                    .await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_else(|_| Bytes::new());

                let result = match (method.as_ref(), subpath.as_str()) {
                    // GET /info/refs?service=git-upload-pack or git-receive-pack
                    (m, sp) if m == Method::GET && sp.starts_with("info/refs") => {
                        // Parse query string for service parameter
                        let service = query
                            .as_deref()
                            .unwrap_or("")
                            .strip_prefix("service=")
                            .and_then(git::protocol::GitService::from_query_param);

                        match service {
                            Some(svc) => {
                                let result = git::handlers::handle_info_refs(
                                    repo_path,
                                    svc,
                                    git_protocol.as_deref(),
                                )
                                .await;
                                // Track operation
                                if let Some(ref m) = metrics_clone {
                                    let status = if result.is_ok() { "success" } else { "error" };
                                    let operation = match svc {
                                        git::protocol::GitService::UploadPack => "fetch",
                                        git::protocol::GitService::ReceivePack => "push",
                                    };
                                    m.record_git_operation(operation, status);
                                }
                                result
                            }
                            None => Err(git::handlers::GitError::RepositoryNotFound),
                        }
                    }

                    // POST /git-upload-pack (clone/fetch)
                    (m, "git-upload-pack") if m == Method::POST => {
                        let result = git::handlers::handle_upload_pack(
                            repo_path,
                            body_bytes,
                            git_protocol.as_deref(),
                        )
                        .await;
                        if let Some(ref m) = metrics_clone {
                            let status = if result.is_ok() { "success" } else { "error" };
                            m.record_git_operation("clone", status);
                        }
                        result
                    }

                    // POST /git-receive-pack (push) - with GRASP authorization via database
                    (m, "git-receive-pack") if m == Method::POST => {
                        // Convert npub (bech32) to hex pubkey for authorization
                        let owner_pubkey_hex = match PublicKey::parse(&npub) {
                            Ok(pk) => pk.to_hex(),
                            Err(e) => {
                                tracing::warn!("Invalid npub in URL {}: {}", npub, e);
                                // Track failed push due to invalid npub
                                if let Some(ref m) = metrics_clone {
                                    m.record_git_operation("push", "error");
                                }
                                return Ok(add_cors_headers(Response::builder())
                                    .status(hyper::StatusCode::BAD_REQUEST)
                                    .body(Full::new(Bytes::from(format!("Invalid npub: {}", e))))
                                    .unwrap());
                            }
                        };

                        let result = git::handlers::handle_receive_pack(
                            repo_path,
                            body_bytes.clone(),
                            database.clone(),
                            relay.clone(),
                            &identifier,
                            &owner_pubkey_hex,
                            purgatory.clone(),
                            &git_data_path,
                            git_protocol.as_deref(),
                        )
                        .await;

                        if let Some(ref m) = metrics_clone {
                            let status = if result.is_ok() { "success" } else { "error" };
                            m.record_git_operation("push", status);
                        }

                        result
                    }

                    _ => Err(git::handlers::GitError::RepositoryNotFound),
                };

                match result {
                    Ok(response) => {
                        // Add CORS headers to successful Git responses
                        let (parts, body) = response.into_parts();
                        Ok(add_cors_headers(Response::builder().status(parts.status))
                            .header(
                                "content-type",
                                parts
                                    .headers
                                    .get("content-type")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("application/octet-stream"),
                            )
                            .header(
                                "cache-control",
                                parts
                                    .headers
                                    .get("cache-control")
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("no-cache"),
                            )
                            .body(body)
                            .unwrap())
                    }
                    Err(e) => {
                        tracing::error!("Git handler error: {}", e);
                        let error_msg = format!("Git error: {}", e);
                        Ok(add_cors_headers(Response::builder())
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

                tracing::debug!(
                    "Serving NIP-11 relay information document to {}",
                    self.remote
                );

                return Box::pin(async move {
                    Ok(
                        add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                            .status(200)
                            .header("content-type", "application/nostr+json")
                            .body(Full::new(Bytes::from(json)))
                            .unwrap(),
                    )
                });
            }
        }

        // Check for repository URL pattern (e.g., /npub/repo.git without subpath)
        // GRASP-01: "SHOULD serve a webpage at the same endpoint linking to git nostr client(s)
        // to browse the repository and a 404 page for repositories it doesn't host"
        if let Some((npub, identifier)) = parse_repo_url(&path) {
            let npub = npub.to_string();
            let identifier = identifier.to_string();
            let config = self.config.clone();
            let repo_path = git::resolve_repo_path(&git_data_path, &npub, &identifier);

            tracing::debug!(
                "Repository URL request: {} (npub={}, id={}, path={:?})",
                path,
                npub,
                identifier,
                repo_path
            );

            return Box::pin(async move {
                // Check if repository exists
                if repo_path.exists() {
                    // Serve repository webpage
                    let html = landing::get_repo_html(&config, &npub, &identifier);
                    Ok(
                        add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                            .status(200)
                            .header("content-type", "text/html; charset=utf-8")
                            .body(Full::new(Bytes::from(html)))
                            .unwrap(),
                    )
                } else {
                    // Serve 404 page for non-existent repository
                    let html = landing::get_404_html(&config, &npub, &identifier);
                    Ok(
                        add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                            .status(404)
                            .header("content-type", "text/html; charset=utf-8")
                            .body(Full::new(Bytes::from(html)))
                            .unwrap(),
                    )
                }
            });
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
                let metrics_clone = self.metrics.clone();

                tokio::spawn(async move {
                    match hyper::upgrade::on(req).await {
                        Ok(upgraded) => {
                            tracing::info!("WebSocket connection established from {}", addr);
                            // Track connection
                            if let Some(ref m) = metrics_clone {
                                m.connection_tracker().on_connect(addr.ip());
                                m.record_websocket_connection();
                            }
                            if let Err(e) =
                                relay.take_connection(TokioIo::new(upgraded), addr).await
                            {
                                tracing::error!("Relay error for {}: {}", addr, e);
                            }
                            tracing::info!("WebSocket connection closed for {}", addr);
                            // Untrack connection
                            if let Some(ref m) = metrics_clone {
                                m.connection_tracker().on_disconnect(addr.ip());
                            }
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

        // Serve Prometheus metrics if enabled
        if path == "/metrics" {
            if let Some(ref metrics) = self.metrics {
                let metrics = metrics.clone();
                return Box::pin(async move {
                    let output = metrics.render();
                    Ok(
                        add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                            .status(200)
                            .header("content-type", "text/plain; version=0.0.4; charset=utf-8")
                            .body(Full::new(Bytes::from(output)))
                            .unwrap(),
                    )
                });
            } else {
                // Metrics disabled
                return Box::pin(async move {
                    Ok(
                        add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                            .status(404)
                            .body(Full::new(Bytes::from("Metrics disabled")))
                            .unwrap(),
                    )
                });
            }
        }

        // Serve static icon at /icon.png
        if path == "/icon.png" {
            return Box::pin(async move {
                Ok(
                    add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                        .status(200)
                        .header("content-type", "image/png")
                        .header("cache-control", "public, max-age=86400")
                        .body(Full::new(Bytes::from_static(ICON_PNG)))
                        .unwrap(),
                )
            });
        }

        // Only serve landing page for root path "/", 404 for everything else
        let config = self.config.clone();
        Box::pin(async move {
            if path == "/" {
                // Serve landing page for root
                let html = landing::get_html(&config);
                Ok(
                    add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                        .status(200)
                        .header("content-type", "text/html; charset=utf-8")
                        .body(Full::new(Bytes::from(html)))
                        .unwrap(),
                )
            } else {
                // Serve generic 404 for unknown paths
                let html = landing::get_generic_404_html(&config, &path);
                Ok(
                    add_cors_headers(Response::builder().header("server", "ngit-grasp"))
                        .status(404)
                        .header("content-type", "text/html; charset=utf-8")
                        .body(Full::new(Bytes::from(html)))
                        .unwrap(),
                )
            }
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
///
/// # Arguments
/// * `config` - Server configuration
/// * `relay` - The LocalRelay for WebSocket connections
/// * `database` - The database for direct queries (e.g., push authorization)
/// * `metrics` - Optional metrics for Prometheus endpoint
pub async fn run_server(
    config: Config,
    relay: LocalRelay,
    database: SharedDatabase,
    metrics: Option<Arc<Metrics>>,
    purgatory: Arc<Purgatory>,
) -> anyhow::Result<()> {
    let bind_addr: SocketAddr = config.bind_address.parse()?;

    tracing::info!("Starting HTTP server on {}", bind_addr);
    tracing::info!("Relay name: {}", config.relay_name());
    tracing::info!("Domain: {}", config.domain);

    let listener = TcpListener::bind(&bind_addr).await?;

    loop {
        let (socket, addr) = listener.accept().await?;
        let io = TokioIo::new(socket);
        let service = HttpService::new(
            relay.clone(),
            config.clone(),
            addr,
            database.clone(),
            metrics.clone(),
            purgatory.clone(),
        );

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
