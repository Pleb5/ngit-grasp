# Next Session Start Here

**Date:** November 4, 2025  
**Purpose:** Quick start guide for next development session  
**Status:** Ready for actix-web integration

---

## 🎯 Immediate Goal

**Integrate actix-web to serve both Nostr relay (WebSocket) and Git HTTP on the SAME PORT.**

This is the critical architectural fix needed to match GRASP-01 requirements and ngit-relay's design.

---

## 🚨 Critical Understanding

### Single Port Architecture (from ../ngit-relay)

```
┌─────────────────────────────────────┐
│   Single Port (8080)                │
│                                     │
│   ┌─────────────────────────────┐  │
│   │  HTTP/WebSocket Router      │  │
│   │  (nginx in ngit-relay)      │  │
│   │  (actix-web in ngit-grasp)  │  │
│   └──────────┬──────────────────┘  │
│              │                      │
│       ┌──────┴──────┐              │
│       ↓             ↓               │
│   Git HTTP      Nostr Relay         │
│   /<npub>/      / (WebSocket)       │
│   <id>.git                          │
└─────────────────────────────────────┘
```

**Key Points:**
1. ONE port listens for all traffic
2. Router inspects path and decides where to send request
3. Git paths go to Git handler
4. Everything else goes to Nostr relay (with WebSocket upgrade)
5. CORS headers on ALL responses

---

## 📋 Step-by-Step Implementation Plan

### Step 1: Add actix-web Dependencies

**File:** `Cargo.toml`

```toml
[dependencies]
# Existing...
actix-web = "4"
actix-cors = "0.7"
actix-ws = "0.3"  # For WebSocket support

# Git HTTP backend
git-http-backend = "0.2"  # Check latest version
```

**Why:**
- `actix-web` - HTTP framework with routing
- `actix-cors` - Easy CORS middleware
- `actix-ws` - WebSocket support
- `git-http-backend` - Git Smart HTTP protocol

### Step 2: Create HTTP Router Module

**File:** `src/http/mod.rs` (NEW)

```rust
//! HTTP server with routing for Git and Nostr

use actix_web::{web, App, HttpServer};
use actix_cors::Cors;

pub mod git;
pub mod nostr;

pub async fn run_server(config: Config, storage: Storage) -> Result<()> {
    let bind_addr = config.bind_address.clone();
    
    HttpServer::new(move || {
        App::new()
            // CORS middleware (GRASP-01 requirement)
            .wrap(
                Cors::default()
                    .allow_any_origin()
                    .allowed_methods(vec!["GET", "POST"])
                    .allowed_headers(vec!["Content-Type"])
                    .max_age(3600)
            )
            // Git HTTP routes
            .service(
                web::scope("/{npub}/{repo}")
                    .guard(guard::fn_guard(|ctx| {
                        // Only match *.git paths
                        ctx.head().uri.path().ends_with(".git")
                    }))
                    .route("", web::get().to(git::handle_git_request))
                    .route("/{tail:.*}", web::to(git::handle_git_request))
            )
            // Nostr relay (WebSocket at /)
            .route("/", web::get().to(nostr::handle_websocket))
            // Static files (optional)
            .route("/", web::get().to(nostr::handle_http_root))
    })
    .bind(bind_addr)?
    .run()
    .await?;
    
    Ok(())
}
```

**Why:**
- Single HTTP server listening on one port
- Routes by URL path pattern
- CORS applied to all routes
- Git paths (*.git) go to Git handler
- Root path (/) handles WebSocket upgrade for Nostr

### Step 3: Create Git HTTP Handler

**File:** `src/http/git.rs` (NEW)

```rust
//! Git Smart HTTP handler

use actix_web::{web, HttpRequest, HttpResponse, Result};
use git_http_backend::{GitHttpBackend, Method};

pub async fn handle_git_request(
    req: HttpRequest,
    body: web::Bytes,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse> {
    let (npub, repo) = path.into_inner();
    
    // Construct repository path
    let repo_path = format!("{}/{}/{}", 
        config.git_data_path, npub, repo);
    
    // Check if repository exists
    if !std::path::Path::new(&repo_path).exists() {
        return Ok(HttpResponse::NotFound()
            .body("Repository not found"));
    }
    
    // Parse Git HTTP request
    let method = match *req.method() {
        actix_web::http::Method::GET => Method::Get,
        actix_web::http::Method::POST => Method::Post,
        _ => return Ok(HttpResponse::MethodNotAllowed().finish()),
    };
    
    // Use git-http-backend to handle request
    let backend = GitHttpBackend::new(&repo_path);
    let response = backend.handle(method, req.path(), &body)?;
    
    // Convert to actix HttpResponse
    Ok(HttpResponse::Ok()
        .content_type(response.content_type)
        .body(response.body))
}
```

**Why:**
- Handles Git Smart HTTP protocol
- Serves from `{GIT_DATA_PATH}/{npub}/{repo}.git`
- Uses `git-http-backend` crate for protocol details
- Returns 404 if repo doesn't exist

### Step 4: Create Nostr WebSocket Handler

**File:** `src/http/nostr.rs` (NEW)

```rust
//! Nostr relay WebSocket handler

use actix_web::{web, HttpRequest, HttpResponse, Result};
use actix_ws::Message;

pub async fn handle_websocket(
    req: HttpRequest,
    stream: web::Payload,
    storage: web::Data<Storage>,
) -> Result<HttpResponse> {
    // Upgrade to WebSocket
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, stream)?;
    
    // Spawn task to handle WebSocket messages
    actix_web::rt::spawn(async move {
        while let Some(Ok(msg)) = msg_stream.next().await {
            match msg {
                Message::Text(text) => {
                    // Handle Nostr message (EVENT, REQ, CLOSE)
                    let responses = handle_nostr_message(&text, &storage).await;
                    for response in responses {
                        session.text(response).await.ok();
                    }
                }
                Message::Ping(bytes) => {
                    session.pong(&bytes).await.ok();
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });
    
    Ok(response)
}

pub async fn handle_http_root() -> Result<HttpResponse> {
    // Serve static HTML for browsers
    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body("<html><body><h1>ngit-grasp</h1><p>Nostr relay at ws://</p></body></html>"))
}
```

**Why:**
- Handles WebSocket upgrade at `/`
- Reuses existing Nostr message handling logic
- Returns HTML for browsers (non-WebSocket requests)

### Step 5: Update main.rs

**File:** `src/main.rs`

```rust
use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod config;
mod http;  // NEW
mod nostr;
mod storage;

use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting ngit-grasp...");

    // Load configuration
    let config = Config::from_env()?;
    info!("Configuration: {}", config.bind_address);

    // Initialize storage
    let storage = storage::Storage::new(&config)?;
    info!("Storage initialized at: {}", config.relay_data_path);

    // Start HTTP server (Git + Nostr on same port)
    info!("Starting server on {}", config.bind_address);
    http::run_server(config, storage).await?;

    Ok(())
}
```

**Why:**
- Replaces separate relay with unified HTTP server
- Single entry point for all services
- Simpler architecture

### Step 6: Update Configuration

**File:** `src/config.rs`

Add field for Git data path:

```rust
pub struct Config {
    pub bind_address: String,
    pub domain: String,
    pub relay_data_path: String,
    pub git_data_path: String,  // NEW
    // ... other fields
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Config {
            bind_address: env::var("NGIT_BIND_ADDRESS")
                .unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            domain: env::var("NGIT_DOMAIN")?,
            relay_data_path: env::var("NGIT_RELAY_DATA_PATH")
                .unwrap_or_else(|_| "./data/relay".to_string()),
            git_data_path: env::var("NGIT_GIT_DATA_PATH")  // NEW
                .unwrap_or_else(|_| "./data/repos".to_string()),
            // ...
        })
    }
}
```

**File:** `.env.example`

```bash
# Service Configuration
NGIT_DOMAIN=example.com
NGIT_BIND_ADDRESS=127.0.0.1:8080

# Relay Information
NGIT_RELAY_NAME="ngit-grasp instance"
NGIT_RELAY_DESCRIPTION="Rust GRASP implementation"
NGIT_OWNER_NPUB="npub1..."

# Storage Paths
NGIT_GIT_DATA_PATH=./data/repos
NGIT_RELAY_DATA_PATH=./data/relay

# Logging
NGIT_LOG_LEVEL=INFO
RUST_LOG=info
```

### Step 7: Update Tests

**File:** `tests/common/relay.rs`

Update `start_with_port` to pass domain correctly:

```rust
pub async fn start_with_port(port: u16) -> Self {
    let bind_address = format!("127.0.0.1:{}", port);
    let domain = format!("127.0.0.1:{}", port);  // NEW
    let url = format!("ws://{}", domain);

    let process = Command::new(&binary_path)
        .env("NGIT_BIND_ADDRESS", &bind_address)
        .env("NGIT_DOMAIN", &domain)  // UPDATED
        .env("NGIT_GIT_DATA_PATH", "./test-data/repos")  // NEW
        .env("NGIT_RELAY_DATA_PATH", "./test-data/relay")  // NEW
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to start relay process");

    // ... rest of method
}
```

**Why:**
- Domain must match bind address for announcement validation
- Separate test data directories
- Clean up test data after tests

### Step 8: Add Git HTTP Tests

**File:** `tests/grasp01_git_http.rs` (NEW)

```rust
//! GRASP-01 Git HTTP Integration Tests
//!
//! Reference: ../grasp/01.md lines 15-40
//!
//! These tests verify Git Smart HTTP service requirements:
//! - Serve repos at /<npub>/<identifier>.git
//! - Accept pushes matching state announcements
//! - CORS support

mod common;

use common::TestRelay;
use std::process::Command;

#[tokio::test]
async fn test_git_clone_basic() {
    // Reference: ../grasp/01.md line 15
    // MUST serve git repository via unauthenticated git smart http service
    
    let relay = TestRelay::start().await;
    let domain = relay.domain();
    
    // TODO: Create test repository announcement
    // TODO: Clone via git clone http://{domain}/{npub}/{id}.git
    
    relay.stop().await;
}

#[tokio::test]
async fn test_cors_headers() {
    // Reference: ../grasp/01.md lines 32-40
    // MUST include CORS headers on all responses
    
    let relay = TestRelay::start().await;
    let url = format!("http://{}/", relay.domain());
    
    let response = reqwest::get(&url).await.unwrap();
    
    // Check CORS headers
    assert_eq!(
        response.headers().get("access-control-allow-origin"),
        Some(&"*".parse().unwrap())
    );
    
    relay.stop().await;
}
```

**Why:**
- Tests reference GRASP protocol line numbers
- Verifies Git HTTP functionality
- Checks CORS compliance

---

## 🔍 Verification Steps

After implementing the above:

### 1. Build and Run

```bash
# Build
cargo build

# Run server
NGIT_DOMAIN=localhost:8080 \
NGIT_BIND_ADDRESS=127.0.0.1:8080 \
NGIT_GIT_DATA_PATH=./data/repos \
NGIT_RELAY_DATA_PATH=./data/relay \
cargo run
```

### 2. Test Nostr Relay (WebSocket)

```bash
# In another terminal
cd grasp-audit
cargo run -- --url ws://localhost:8080
```

**Expected:** NIP-01 smoke tests should pass

### 3. Test Git HTTP (Manual)

```bash
# Create test repository
mkdir -p ./data/repos/npub1test/test-repo.git
cd ./data/repos/npub1test/test-repo.git
git init --bare

# Try to clone
git clone http://localhost:8080/npub1test/test-repo.git
```

**Expected:** Should clone successfully (even if empty)

### 4. Test CORS

```bash
curl -v http://localhost:8080/ -H "Origin: https://example.com"
```

**Expected:** Response should include:
```
access-control-allow-origin: *
access-control-allow-methods: GET, POST
access-control-allow-headers: Content-Type
```

### 5. Run Integration Tests

```bash
# All tests
cargo test

# Just NIP-01
cargo test --test nip01_compliance

# Just Git HTTP (when implemented)
cargo test --test grasp01_git_http
```

**Expected:** All tests pass

---

## 🐛 Common Issues & Solutions

### Issue: Port Already in Use

**Symptom:** "Address already in use" error

**Solution:**
```bash
# Find process using port
lsof -i :8080

# Kill it
kill -9 <PID>

# Or use different port
NGIT_BIND_ADDRESS=127.0.0.1:8081 cargo run
```

### Issue: WebSocket Upgrade Fails

**Symptom:** WebSocket connection refused

**Solution:**
- Check actix-web WebSocket handling
- Verify `Upgrade: websocket` header is present
- Check actix-ws is properly configured

### Issue: Git Clone Fails

**Symptom:** "repository not found" or protocol error

**Solution:**
- Verify repository exists at correct path
- Check git-http-backend configuration
- Ensure repository is bare (`git init --bare`)
- Check file permissions

### Issue: CORS Headers Missing

**Symptom:** Browser console shows CORS error

**Solution:**
- Verify CORS middleware is applied
- Check middleware order (CORS should be first)
- Test with curl to see actual headers

---

## 📚 Key Resources

### GRASP Protocol
- `../grasp/01.md` - **THE SPEC** - Read this first!
- Lines 1-14: Nostr relay requirements
- Lines 15-31: Git HTTP service requirements
- Lines 32-40: CORS requirements

### Reference Implementation
- `../ngit-relay/src/nginx.conf` - **ROUTING PATTERN**
  - Lines 8-13: Single port listener
  - Lines 15-48: Git HTTP routing
  - Lines 50-94: Nostr relay routing
- `../ngit-relay/docker-compose.yml` - Port configuration
- `../ngit-relay/.env.example` - Environment variables

### actix-web Documentation
- [Routing](https://actix.rs/docs/url-dispatch/)
- [WebSocket](https://actix.rs/docs/websockets/)
- [CORS](https://docs.rs/actix-cors/)

### git-http-backend Crate
- [Docs](https://docs.rs/git-http-backend/)
- [Examples](https://github.com/w4/git-http-backend/tree/master/examples)

---

## ✅ Success Criteria

You'll know this step is complete when:

1. ✅ Server starts on single port
2. ✅ WebSocket connects at `ws://localhost:8080/`
3. ✅ NIP-01 smoke tests pass
4. ✅ Can clone Git repo at `http://localhost:8080/npub.../repo.git`
5. ✅ CORS headers present on all responses
6. ✅ OPTIONS requests return 204
7. ✅ All integration tests pass

---

## 🎯 After This Step

Once actix-web integration is complete:

1. **Repository Provisioning**
   - Create repos when announcements received
   - Initialize bare repositories
   - Set up directory structure

2. **Push Authorization**
   - Intercept git-receive-pack
   - Validate against state announcements
   - Handle maintainer sets

3. **Full GRASP-01 Compliance**
   - All tests passing
   - Ready for production testing

---

## 💡 Tips

1. **Start Simple**
   - Get basic HTTP routing working first
   - Add WebSocket support second
   - Add Git HTTP last

2. **Test Incrementally**
   - Test each component as you add it
   - Don't wait until everything is done

3. **Use curl for Debugging**
   ```bash
   # Test HTTP
   curl -v http://localhost:8080/
   
   # Test CORS
   curl -v http://localhost:8080/ -H "Origin: https://example.com"
   
   # Test Git info/refs
   curl http://localhost:8080/npub.../repo.git/info/refs?service=git-upload-pack
   ```

4. **Check ngit-relay for Patterns**
   - nginx.conf shows exact routing logic
   - Copy the pattern, not the implementation

5. **Keep Tests Running**
   ```bash
   # In one terminal
   cargo watch -x 'test --test nip01_compliance'
   
   # Make changes, tests auto-run
   ```

---

**Ready to Start?** Begin with Step 1 (Add Dependencies)

**Questions?** Check `work/current_status.md` for context

**Stuck?** Review `../ngit-relay/src/nginx.conf` for routing pattern

---

**Last Updated:** November 4, 2025  
**Next Update:** After actix-web integration complete
