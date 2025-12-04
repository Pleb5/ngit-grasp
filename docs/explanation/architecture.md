# ngit-grasp Architecture

## Executive Summary

`ngit-grasp` implements the GRASP protocol in Rust with **inline authorization** rather than Git hooks. The key architectural insight is that we can intercept and validate Git push operations at the HTTP handler level before reaching the Git repository, eliminating the need for pre-receive hooks.

## Architectural Decision: Inline vs. Hook-Based Authorization

### Investigation Summary

After examining both the reference implementation and HTTP server options, we have two options:

#### Option 1: Hook-Based (Reference Implementation Approach)
- Use standard Git HTTP backend
- Create pre-receive and post-receive hooks
- Hooks query the Nostr relay and validate pushes
- **Pros**: Follows reference implementation closely
- **Cons**: Requires hook management, harder to test, less Rust-native

#### Option 2: Inline Authorization (Recommended)
- Intercept Git receive-pack requests in the HTTP handler
- Validate against Nostr state before spawning Git process
- Only forward valid pushes to Git
- **Pros**: Better error handling, easier testing, pure Rust, simpler deployment
- **Cons**: Requires custom Git protocol handling

### Decision: Inline Authorization (Option 2)

**Rationale:**

1. **Full control over HTTP layer**: Using Hyper directly gives us complete control over request handling, WebSocket upgrades, and CORS headers.

2. **Better Developer Experience**: 
   - Validation errors can be returned as proper HTTP responses
   - No need to parse hook stderr output
   - Shared state between Git and Nostr components
   - Pure Rust testing without shell scripts

3. **Simpler Deployment**:
   - Single binary
   - No hook symlinks or permissions to manage
   - No multi-process coordination

4. **Performance**:
   - Can parse incoming pack data once
   - Avoid process spawn overhead for invalid pushes
   - Better async integration

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        ngit-grasp                           │
│                     (Single Rust Binary)                    │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌──────────────────┐              ┌──────────────────┐   │
│  │   HTTP Router    │              │  Nostr Relay     │   │
│  │     (Hyper)      │              │  (nostr-relay-   │   │
│  │                  │              │   builder)       │   │
│  └────────┬─────────┘              └────────┬─────────┘   │
│           │                                 │             │
│           │                                 │             │
│  ┌────────▼──────────────────────────────────▼─────────┐  │
│  │           Shared State & Storage                    │  │
│  │  ┌──────────────┐  ┌──────────────┐                │  │
│  │  │  Repository  │  │  Event Store │                │  │
│  │  │  Manager     │  │  (LMDB/NDB)  │                │  │
│  │  └──────────────┘  └──────────────┘                │  │
│  └─────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │            Git Protocol Handler                      │  │
│  │                                                      │  │
│  │  1. Receive git-receive-pack request                │  │
│  │  2. Parse ref updates from request                  │  │
│  │  3. Query Nostr relay for state event               │  │
│  │  4. Validate refs against state                     │  │
│  │  5. If valid: spawn git-receive-pack                │  │
│  │  6. If invalid: return HTTP error                   │  │
│  │                                                      │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
└─────────────────────────────────────────────────────────────┘
         │                                      │
         │ HTTP/Git                             │ WebSocket/Nostr
         ▼                                      ▼
    Git Clients                            Nostr Clients
```

## Component Design

### 1. Main Server ([`src/main.rs`](src/main.rs))

**Responsibilities:**
- Initialize configuration from environment (clap + dotenvy)
- Set up Hyper HTTP server with request routing
- Initialize Nostr relay builder with custom [`Nip34WritePolicy`](src/nostr/builder.rs:51)
- Set up shared storage (LMDB, NostrDB, or Memory)
- Handle WebSocket upgrades for Nostr relay
- Handle graceful shutdown

**Key Dependencies:**
```rust
hyper = "1"
tokio = { version = "1", features = ["full"] }
nostr-relay-builder = "0.43"
nostr-sdk = "0.43"
nostr-lmdb = "0.43"
```

### 2. HTTP Module ([`src/http/mod.rs`](src/http/mod.rs))

**Responsibilities:**
- Route HTTP requests to appropriate handlers
- WebSocket upgrade for Nostr relay at `/`
- Git Smart HTTP endpoints at `/<npub>/<identifier>.git/*`
- Landing pages and NIP-11 document serving
- CORS headers on all responses (GRASP-01 requirement)

**Key Implementation Details:**

```rust
// CORS headers required by GRASP-01 specification
const CORS_ALLOW_ORIGIN: &str = "*";
const CORS_ALLOW_METHODS: &str = "GET, POST";
const CORS_ALLOW_HEADERS: &str = "Content-Type";

/// Add CORS headers to a response builder
fn add_cors_headers(builder: http::response::Builder) -> http::response::Builder {
    builder
        .header("Access-Control-Allow-Origin", CORS_ALLOW_ORIGIN)
        .header("Access-Control-Allow-Methods", CORS_ALLOW_METHODS)
        .header("Access-Control-Allow-Headers", CORS_ALLOW_HEADERS)
}
```

See [`src/http/mod.rs:29-84`](src/http/mod.rs:29-84) for the full CORS implementation.

### 3. Git Module ([`src/git/`](src/git/))

#### [`handlers.rs`](src/git/handlers.rs) - Git HTTP Handlers

Implements handlers for Git Smart HTTP protocol:

```rust
/// Handle GET /info/refs?service=git-{upload,receive}-pack
pub async fn handle_info_refs(
    repo_path: PathBuf,
    service: GitService,
) -> Result<Response<Full<Bytes>>, GitError>

/// Handle POST /git-upload-pack (clone/fetch)
pub async fn handle_upload_pack(
    repo_path: PathBuf,
    body: Bytes,
) -> Result<Response<Full<Bytes>>, GitError>

/// Handle POST /git-receive-pack (push)
/// THIS IS WHERE THE MAGIC HAPPENS - validates against state before accepting
pub async fn handle_receive_pack(
    repo_path: PathBuf,
    body: Bytes,
    database: SharedDatabase,
    npub: &str,
    identifier: &str,
) -> Result<Response<Full<Bytes>>, GitError>
```

See [`src/git/handlers.rs:22-98`](src/git/handlers.rs:22-98) for the info-refs implementation.

#### [`authorization.rs`](src/git/authorization.rs) - Push Validation

**Core Logic:**

```rust
/// Get authorization info for a repository owner
pub async fn get_authorization_for_owner(
    database: &SharedDatabase,
    pubkey: &PublicKey,
    identifier: &str,
) -> Result<AuthorizationResult, AuthorizationError>

/// Validate that pushed refs match the authorized state
pub fn validate_push_refs(
    pushed_refs: &[PushedRef],
    state: &RepositoryState,
) -> Result<(), AuthorizationError>

/// Validate refs/nostr/<event-id> pushes
pub fn validate_nostr_ref_pushes(
    pushed_refs: &[PushedRef],
    database: &SharedDatabase,
) -> Result<(), AuthorizationError>
```

### 4. Nostr Module ([`src/nostr/`](src/nostr/))

#### [`builder.rs`](src/nostr/builder.rs) - Relay Configuration

The [`Nip34WritePolicy`](src/nostr/builder.rs:51) is the core event validation logic:

```rust
/// NIP-34 Write Policy with Full GRASP-01 Event Validation
///
/// Validates all events according to GRASP-01 specification:
/// - Repository announcements must list service in clone and relays tags
///   EXCEPTION: Recursive maintainer announcements are accepted even without
///   listing the service, to enable maintainer chain discovery and GRASP-02 sync
/// - Repository state announcements must have valid structure
/// - Other events must reference accepted repositories or events
/// - Forward references are supported (events referenced by accepted events)
/// - Orphan events with no valid references are rejected
pub struct Nip34WritePolicy {
    domain: String,
    database: SharedDatabase,
    git_data_path: PathBuf,
}
```

See [`src/nostr/builder.rs:38-78`](src/nostr/builder.rs:38-78) for the full policy struct.

#### [`events.rs`](src/nostr/events.rs) - Event Parsing

Provides structures for parsing NIP-34 events:

```rust
/// Parsed repository announcement (Kind 30617)
pub struct RepositoryAnnouncement { ... }

/// Parsed repository state (Kind 30618)  
pub struct RepositoryState { ... }
```

### 5. Configuration ([`src/config.rs`](src/config.rs))

```rust
pub struct Config {
    pub domain: String,
    pub owner_npub: String,
    pub relay_name: String,
    pub relay_description: String,
    pub git_data_path: PathBuf,
    pub relay_data_path: PathBuf,
    pub bind_address: SocketAddr,
    pub database_backend: DatabaseBackend,
}

pub enum DatabaseBackend {
    Lmdb,    // Default, production use
    NostrDb, // Alternative
    Memory,  // Testing
}
```

Configuration is loaded via **clap CLI > environment variables > .env > defaults**.

## Data Flow

### Push Operation Flow

```
1. Git Client → POST /<npub>/<id>.git/git-receive-pack
                ↓
2. HttpService routes to git::handlers::handle_receive_pack()
                ↓
3. Parse ref updates from request body (pkt-line format)
                ↓
4. Extract npub and identifier from URL
                ↓
5. authorization::get_authorization_for_owner()
   ├─ Query database for announcements
   ├─ Build recursive maintainer set
   └─ Get latest authorized state
                ↓
6. authorization::validate_push_refs()
   ├─ Check each ref matches state
   └─ Validate refs/nostr/ pushes
                ↓
7. If VALID:
   ├─ Spawn git-receive-pack subprocess
   ├─ Stream request body to git stdin
   └─ Stream git stdout back to client
                ↓
8. If INVALID:
   └─ Return HTTP 403 with error message
```

### Repository Announcement Flow

```
1. Nostr Client → EVENT (Kind 30617)
                ↓
2. Nostr relay receives event
                ↓
3. Nip34WritePolicy::admit_event()
   ├─ Check if instance in clone tags
   ├─ Check if instance in relays tags
   ├─ OR: Check if recursive maintainer
   └─ Accept or reject
                ↓
4. If ACCEPTED:
   ├─ Event saved to database
   └─ ensure_bare_repository() called
                ↓
5. Bare Git repository created at
   <git_data_path>/<npub>/<identifier>.git
```

### State Event Flow

```
1. Nostr Client → EVENT (Kind 30618)
                ↓
2. Nostr relay receives event
                ↓
3. Nip34WritePolicy::admit_event()
   ├─ Check author is in maintainer set
   ├─ Validate state structure
   └─ Accept or reject
                ↓
4. If ACCEPTED and is latest state:
   ├─ Align repository refs to match state
   ├─ Create/update/delete refs as needed
   └─ Set HEAD if commit available
```

## Testing Strategy

See [test-strategy.md](../reference/test-strategy.md) for comprehensive testing documentation.

### Quick Overview

**Integration Tests** ([`tests/`](tests/)):
- Use [`TestRelay`](tests/common/relay.rs:14) fixture for automatic relay lifecycle
- Each test file in [`tests/`](tests/) covers a GRASP-01 requirement

**Audit Tests** ([`grasp-audit/`](grasp-audit/)):
- Reusable compliance testing for any GRASP implementation
- Spec-mirrored structure in [`grasp-audit/src/specs/grasp01/`](grasp-audit/src/specs/grasp01/)

```rust
// Example: tests/nip01_compliance.rs
#[tokio::test]
async fn test_nip01_websocket_connection() {
    let relay = TestRelay::start().await;
    // Test NIP-01 compliance...
    relay.stop().await;
}
```

## Performance Considerations

### 1. Async All The Way

- Use `tokio` for all I/O
- Non-blocking Git subprocess spawning via [`GitSubprocess`](src/git/subprocess.rs)
- Stream large pack files without buffering

### 2. Shared Database

- Single database instance shared between relay and Git handlers
- Direct queries for push authorization (no WebSocket round-trip)

### 3. Write Policy Caching

- Maintainer sets computed once per event validation
- State lookups use database indexes

## Future Extensions

### GRASP-02: Proactive Sync

See [grasp-02-proactive-sync.md](grasp-02-proactive-sync.md) for detailed design.

### GRASP-05: Archive

Relax the write policy to accept all repository announcements regardless of clone/relays tags.

## Deployment

### Single Binary

```bash
cargo build --release
./target/release/ngit-grasp --domain example.com --owner-npub npub1...
```

### Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y git && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/ngit-grasp /usr/local/bin/
EXPOSE 8080
CMD ["ngit-grasp"]
```

### Systemd

```ini
[Unit]
Description=ngit-grasp GRASP server
After=network.target

[Service]
Type=simple
User=git
WorkingDirectory=/opt/ngit-grasp
EnvironmentFile=/opt/ngit-grasp/.env
ExecStart=/usr/local/bin/ngit-grasp
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

## Security Considerations

1. **Input Validation**: All npub/identifier inputs must be validated
2. **Path Traversal**: Prevent directory traversal in repository paths
3. **DoS Protection**: Rate limiting on both HTTP and WebSocket
4. **Resource Limits**: Limit pack file sizes, event sizes
5. **Nostr Event Validation**: Strict signature verification (handled by nostr-relay-builder)

## Conclusion

The inline authorization approach provides a cleaner, more maintainable architecture than hook-based authorization while maintaining full GRASP-01 compliance. Using Hyper for the HTTP layer gives us complete control over request handling, WebSocket upgrades, and CORS headers.

The key insight is that we don't need to rely on Git's hook mechanism when we have full control over the HTTP layer that Git operates through. By intercepting at the HTTP handler level, we gain better error handling, easier testing, and tighter integration between the Git and Nostr components.

## Related Documentation

- [Inline Authorization Explanation](inline-authorization.md) - Why we chose this approach
- [GRASP-02 Proactive Sync](grasp-02-proactive-sync.md) - Future work design
- [Test Strategy](../reference/test-strategy.md) - Comprehensive testing documentation
- [GRASP-01 Implementation Learnings](../learnings/grasp-01-implementation.md) - Patterns and lessons learned