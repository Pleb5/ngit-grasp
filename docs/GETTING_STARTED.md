# Getting Started with Implementation

This guide helps you start implementing ngit-grasp based on the architecture design.

## Prerequisites

- Rust 1.75 or later
- Git 2.x
- Basic understanding of async Rust (tokio)
- Familiarity with actix-web (helpful)
- Understanding of Nostr basics (helpful)

## Step 1: Initialize Cargo Project

```bash
# Create new binary project
cargo init --name ngit-grasp

# Or if already created:
cargo build
```

## Step 2: Add Dependencies

Edit `Cargo.toml`:

```toml
[package]
name = "ngit-grasp"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[dependencies]
# HTTP Server
actix-web = "4"
actix-cors = "0.7"

# Async Runtime
tokio = { version = "1", features = ["full"] }

# Git Protocol
git-http-backend = "0.1.3"

# Nostr
nostr-sdk = { version = "0.43", features = ["all-nips"] }
nostr-relay-builder = "0.43"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Error Handling
anyhow = "1"
thiserror = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Environment
dotenv = "0.15"

# Utilities
async-trait = "0.1"
futures = "0.3"
bytes = "1"

[dev-dependencies]
tokio-test = "0.4"
```

## Step 3: Project Structure

Create the directory structure:

```bash
mkdir -p src/{git,nostr,storage}
mkdir -p tests/{integration,fixtures}
mkdir -p data/{git,relay}
```

## Step 4: Configuration Module

Create `src/config.rs`:

```rust
use anyhow::Result;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub domain: String,
    pub owner_npub: String,
    pub relay_name: String,
    pub relay_description: String,
    pub git_data_path: PathBuf,
    pub relay_data_path: PathBuf,
    pub bind_address: SocketAddr,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();
        
        Ok(Config {
            domain: env::var("NGIT_DOMAIN")?,
            owner_npub: env::var("NGIT_OWNER_NPUB")?,
            relay_name: env::var("NGIT_RELAY_NAME")?,
            relay_description: env::var("NGIT_RELAY_DESCRIPTION")?,
            git_data_path: PathBuf::from(
                env::var("NGIT_GIT_DATA_PATH")
                    .unwrap_or_else(|_| "./data/git".to_string())
            ),
            relay_data_path: PathBuf::from(
                env::var("NGIT_RELAY_DATA_PATH")
                    .unwrap_or_else(|_| "./data/relay".to_string())
            ),
            bind_address: env::var("NGIT_BIND_ADDRESS")
                .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
                .parse()?,
        })
    }
}
```

## Step 5: Core Types

Create `src/git/types.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefUpdate {
    pub old_oid: String,
    pub new_oid: String,
    pub ref_name: String,
}

impl RefUpdate {
    pub fn is_create(&self) -> bool {
        self.old_oid == "0000000000000000000000000000000000000000"
    }
    
    pub fn is_delete(&self) -> bool {
        self.new_oid == "0000000000000000000000000000000000000000"
    }
    
    pub fn is_update(&self) -> bool {
        !self.is_create() && !self.is_delete()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Invalid pkt-line format")]
    InvalidPktLine,
    
    #[error("Invalid ref update format")]
    InvalidRefUpdate,
    
    #[error("Repository not found: {0}")]
    RepositoryNotFound(String),
    
    #[error("Invalid repository path")]
    InvalidPath,
}
```

## Step 6: Main Application State

Create `src/main.rs`:

```rust
use actix_web::{web, App, HttpServer};
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

mod config;
mod git;
mod nostr;
mod storage;

use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    // TODO: Add NostrClient, RepositoryManager, etc.
}

#[actix_web::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
        )
        .init();
    
    // Load configuration
    let config = Config::from_env()?;
    info!("Starting ngit-grasp on {}", config.bind_address);
    
    // Create application state
    let state = AppState {
        config: Arc::new(config.clone()),
    };
    
    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(git::routes::configure)
            .configure(nostr::routes::configure)
    })
    .bind(config.bind_address)?
    .run()
    .await?;
    
    Ok(())
}
```

## Step 7: Git Module Skeleton

Create `src/git/mod.rs`:

```rust
pub mod routes;
pub mod handler;
pub mod parser;
pub mod authorization;
pub mod types;

pub use types::{RefUpdate, GitError};
```

Create `src/git/routes.rs`:

```rust
use actix_web::web;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/{npub}/{identifier}.git")
            .route("/info/refs", web::get().to(super::handler::info_refs))
            .route("/git-upload-pack", web::post().to(super::handler::git_upload_pack))
            .route("/git-receive-pack", web::post().to(super::handler::git_receive_pack))
    );
}
```

## Step 8: First Test

Create `tests/integration/basic_test.rs`:

```rust
use actix_web::{test, App};

#[actix_web::test]
async fn test_server_starts() {
    // TODO: Initialize test app
    // TODO: Make test request
    assert!(true);
}
```

Run tests:

```bash
cargo test
```

## Step 9: Implementation Order

Follow this order for implementation:

### Phase 1: Basic Infrastructure (Week 1)
1. ✅ Config module
2. ✅ Main server setup
3. ✅ Core types
4. ⏭️ Git pkt-line parser
5. ⏭️ Ref update parser
6. ⏭️ Parser tests

### Phase 2: Git Protocol (Week 2)
1. ⏭️ Git upload-pack handler (read-only)
2. ⏭️ Repository manager
3. ⏭️ Path validation and security
4. ⏭️ Integration tests for cloning

### Phase 3: Nostr Relay (Week 2-3)
1. ⏭️ Nostr relay setup with nostr-relay-builder
2. ⏭️ Repository announcement policy
3. ⏭️ Event hooks for repo creation
4. ⏭️ NIP-11 configuration

### Phase 4: Authorization (Week 3-4)
1. ⏭️ Maintainer resolution logic
2. ⏭️ State validation logic
3. ⏭️ Git receive-pack with inline validation
4. ⏭️ Integration tests for pushing

### Phase 5: Polish (Week 4-6)
1. ⏭️ Error handling improvements
2. ⏭️ Logging and observability
3. ⏭️ Performance optimization
4. ⏭️ GRASP-01 compliance testing
5. ⏭️ Documentation updates

## Development Workflow

### Running Locally

```bash
# Copy environment template
cp .env.example .env

# Edit configuration
vim .env

# Run in development mode
cargo run

# With debug logging
RUST_LOG=debug cargo run
```

### Testing

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_parse_ref_updates

# Run integration tests only
cargo test --test '*'
```

### Code Quality

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt --check

# Lint
cargo clippy

# Lint with all features
cargo clippy --all-features -- -D warnings
```

## Debugging Tips

### Enable Detailed Logging

```bash
RUST_LOG=trace cargo run
```

### Test with Real Git Client

```bash
# In another terminal, after server is running
mkdir test-repo && cd test-repo
git init
echo "test" > README.md
git add . && git commit -m "test"

# Try to push (will fail without Nostr setup)
git remote add origin http://localhost:8080/npub.../test.git
git push origin main
```

### Use curl for HTTP Testing

```bash
# Test info/refs endpoint
curl -v http://localhost:8080/npub.../test.git/info/refs?service=git-upload-pack
```

## Common Issues

### "Repository not found"
- Check that repository announcement was sent to Nostr relay
- Verify repository was created in git_data_path
- Check logs for repo creation

### "Push rejected"
- Verify state event exists on relay
- Check state event matches push refs
- Verify maintainer list includes pusher

### "Cannot connect to relay"
- Check relay is running
- Verify WebSocket endpoint
- Check firewall/network settings

## Next Steps

After basic setup:

1. Implement pkt-line parser (see [GIT_PROTOCOL.md](GIT_PROTOCOL.md))
2. Add comprehensive tests
3. Implement Nostr relay policies
4. Add authorization logic
5. Test with ngit CLI

## Resources

- [ARCHITECTURE.md](ARCHITECTURE.md) - Detailed design
- [GIT_PROTOCOL.md](GIT_PROTOCOL.md) - Git protocol reference
- [actix-web docs](https://actix.rs/docs/)
- [nostr-sdk docs](https://docs.rs/nostr-sdk/)
- [tokio docs](https://docs.rs/tokio/)

## Getting Help

- Check existing documentation in `docs/`
- Review reference implementation at `../ngit-relay`
- Open an issue for questions
- Read GRASP protocol spec

Good luck! 🚀
