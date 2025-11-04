# ngit-grasp Architecture

## Executive Summary

`ngit-grasp` implements the GRASP protocol in Rust with **inline authorization** rather than Git hooks. The key architectural insight is that the `git-http-backend` Rust crate provides sufficient flexibility to intercept and validate Git push operations before they reach the Git repository, eliminating the need for pre-receive hooks.

## Architectural Decision: Inline vs. Hook-Based Authorization

### Investigation Summary

After examining both the reference implementation and the `git-http-backend` Rust crate, we have two options:

#### Option 1: Hook-Based (Reference Implementation Approach)
- Use `git-http-backend` crate as-is
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

1. **The `git-http-backend` crate is sufficiently flexible**: Examining `src/actix/git_receive_pack.rs` shows it spawns `git receive-pack` as a subprocess and streams data. We can intercept this.

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
│  │   (actix-web)    │              │  (nostr-relay-   │   │
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

### 1. Main Server (`src/main.rs`)

**Responsibilities:**
- Initialize configuration from environment
- Set up actix-web HTTP server
- Initialize Nostr relay builder
- Set up shared storage
- Configure routes for both Git and Nostr endpoints
- Handle graceful shutdown

**Key Dependencies:**
```rust
actix-web = "4"
tokio = { version = "1", features = ["full"] }
nostr-relay-builder = "0.43"
nostr-sdk = "0.43"
```

### 2. Git Module (`src/git/`)

#### `handler.rs` - Git HTTP Handlers

Implements actix-web handlers for Git Smart HTTP protocol:

```rust
// GET /<npub>/<identifier>.git/info/refs?service=git-upload-pack
async fn info_refs_upload_pack(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<HttpResponse>

// POST /<npub>/<identifier>.git/git-upload-pack
async fn git_upload_pack(
    req: HttpRequest,
    body: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse>

// GET /<npub>/<identifier>.git/info/refs?service=git-receive-pack
async fn info_refs_receive_pack(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> Result<HttpResponse>

// POST /<npub>/<identifier>.git/git-receive-pack
// THIS IS WHERE THE MAGIC HAPPENS
async fn git_receive_pack(
    req: HttpRequest,
    body: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse>
```

#### `authorization.rs` - Push Validation

**Core Logic:**

```rust
pub struct PushValidator {
    nostr_client: Arc<Client>,
    relay_url: String,
}

impl PushValidator {
    /// Validate a push operation against Nostr state
    pub async fn validate_push(
        &self,
        npub: &str,
        identifier: &str,
        ref_updates: Vec<RefUpdate>,
    ) -> Result<ValidationResult> {
        // 1. Fetch announcement and state events from local relay
        let events = self.fetch_events(identifier).await?;
        
        // 2. Extract pubkey from npub
        let pubkey = decode_npub(npub)?;
        
        // 3. Get recursive maintainer set
        let maintainers = get_maintainers(&events, &pubkey, identifier);
        
        // 4. Get latest state from maintainers
        let state = get_state_from_maintainers(&events, &maintainers)?;
        
        // 5. Validate each ref update
        for ref_update in ref_updates {
            if ref_update.ref_name.starts_with("refs/nostr/") {
                // Allow refs/nostr/<event-id> for PRs
                validate_pr_ref(&ref_update)?;
            } else if ref_update.ref_name.starts_with("refs/heads/pr/") {
                // Reject pr/* branches - should use refs/nostr/
                return Err(Error::InvalidRef("pr/* branches must use refs/nostr/"));
            } else {
                // Validate against state event
                validate_state_ref(&state, &ref_update)?;
            }
        }
        
        Ok(ValidationResult::Accept)
    }
}
```

**Key Functions:**

```rust
/// Parse ref updates from git-receive-pack request body
fn parse_ref_updates(body: &[u8]) -> Result<Vec<RefUpdate>>

/// Recursively find all maintainers
fn get_maintainers(
    events: &[Event],
    pubkey: &str,
    identifier: &str,
) -> Vec<String>

/// Get latest state from maintainer set
fn get_state_from_maintainers(
    events: &[Event],
    maintainers: &[String],
) -> Result<RepositoryState>

/// Validate a ref matches the state event
fn validate_state_ref(
    state: &RepositoryState,
    ref_update: &RefUpdate,
) -> Result<()>
```

### 3. Nostr Module (`src/nostr/`)

#### `relay.rs` - Relay Configuration

```rust
pub async fn build_relay(config: &Config) -> Result<LocalRelay> {
    let builder = RelayBuilder::default()
        .write_policy(RepositoryAnnouncementPolicy::new(config.domain.clone()))
        .write_policy(RelatedEventsPolicy::new())
        .query_policy(StandardQueryPolicy::new())
        .on_event_saved(create_repository_hook(config.git_data_path.clone()));
    
    // Configure storage backend (LMDB or NDB)
    let relay = LocalRelay::run(builder).await?;
    
    Ok(relay)
}
```

#### `events.rs` - Event Handlers

```rust
/// Hook called when events are saved
pub fn create_repository_hook(
    git_data_path: PathBuf,
) -> impl Fn(&Event) -> BoxFuture<'static, ()> {
    move |event: &Event| {
        let git_path = git_data_path.clone();
        Box::pin(async move {
            if event.kind == Kind::RepositoryAnnouncement {
                handle_repository_announcement(event, &git_path).await;
            } else if event.kind == Kind::RepositoryState {
                handle_repository_state(event, &git_path).await;
            }
        })
    }
}

async fn handle_repository_announcement(event: &Event, git_path: &Path) {
    // 1. Parse repository from event
    // 2. Check if listed in clone and relays tags
    // 3. Create empty bare Git repository
    // 4. Configure uploadpack.allowTipSHA1InWant
    // 5. Configure uploadpack.allowUnreachable
    // 6. Configure http.receivepack
}

async fn handle_repository_state(event: &Event, git_path: &Path) {
    // 1. Parse state from event
    // 2. Update repository HEAD if needed
    // 3. Trigger proactive sync (GRASP-02)
}
```

**Write Policies:**

```rust
/// Accept repository announcements that list this instance
pub struct RepositoryAnnouncementPolicy {
    domain: String,
}

impl WritePolicy for RepositoryAnnouncementPolicy {
    fn admit_event(&self, event: &Event, _addr: &SocketAddr) 
        -> BoxFuture<PolicyResult> 
    {
        Box::pin(async move {
            if event.kind != Kind::RepositoryAnnouncement {
                return PolicyResult::Accept; // Not our concern
            }
            
            // Check if this instance is in clone and relays tags
            let has_clone = event.tags.iter()
                .any(|t| t.kind() == "clone" && t.content() == Some(&self.domain));
            let has_relay = event.tags.iter()
                .any(|t| t.kind() == "relays" && t.content() == Some(&self.domain));
            
            if has_clone && has_relay {
                PolicyResult::Accept
            } else {
                PolicyResult::Reject("instance not listed in clone and relays".into())
            }
        })
    }
}

/// Accept events related to stored announcements/issues/patches
pub struct RelatedEventsPolicy;

impl WritePolicy for RelatedEventsPolicy {
    fn admit_event(&self, event: &Event, _addr: &SocketAddr) 
        -> BoxFuture<PolicyResult> 
    {
        // Accept if event tags or is tagged by stored events
        // Implementation requires querying the event store
    }
}
```

### 4. Storage Module (`src/storage/`)

#### `repository.rs` - Repository Management

```rust
pub struct RepositoryManager {
    git_data_path: PathBuf,
}

impl RepositoryManager {
    /// Create a new bare Git repository
    pub async fn create_repository(
        &self,
        npub: &str,
        identifier: &str,
    ) -> Result<PathBuf> {
        let repo_path = self.git_data_path
            .join(npub)
            .join(format!("{}.git", identifier));
        
        // Create directory
        tokio::fs::create_dir_all(&repo_path).await?;
        
        // Initialize bare repo
        Command::new("git")
            .args(&["init", "--bare"])
            .arg(&repo_path)
            .output()
            .await?;
        
        // Configure
        self.configure_repository(&repo_path).await?;
        
        Ok(repo_path)
    }
    
    async fn configure_repository(&self, repo_path: &Path) -> Result<()> {
        // Enable unauthenticated push (we handle auth ourselves)
        git_config(repo_path, "http.receivepack", "true").await?;
        
        // Enable tip SHA1 fetching (required for ngit)
        git_config(repo_path, "uploadpack.allowTipSHA1InWant", "true").await?;
        
        // Enable unreachable object fetching
        git_config(repo_path, "uploadpack.allowUnreachable", "true").await?;
        
        Ok(())
    }
    
    /// Check if repository exists
    pub async fn repository_exists(
        &self,
        npub: &str,
        identifier: &str,
    ) -> bool {
        let repo_path = self.git_data_path
            .join(npub)
            .join(format!("{}.git", identifier));
        
        repo_path.join("HEAD").exists() && 
        repo_path.join("config").exists()
    }
}
```

### 5. Configuration (`src/config.rs`)

```rust
pub struct Config {
    pub domain: String,
    pub owner_npub: String,
    pub relay_name: String,
    pub relay_description: String,
    pub git_data_path: PathBuf,
    pub relay_data_path: PathBuf,
    pub bind_address: SocketAddr,
    pub log_level: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
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
            log_level: env::var("RUST_LOG")
                .unwrap_or_else(|_| "info".to_string()),
        })
    }
}
```

## Data Flow

### Push Operation Flow

```
1. Git Client → POST /<npub>/<id>.git/git-receive-pack
                ↓
2. git_receive_pack handler receives request
                ↓
3. Parse ref updates from request body
                ↓
4. Extract npub and identifier from URL
                ↓
5. PushValidator::validate_push()
   ├─ Fetch events from local Nostr relay
   ├─ Get maintainers recursively
   ├─ Get latest state from maintainers
   └─ Validate each ref update
                ↓
6. If VALID:
   ├─ Spawn git-receive-pack subprocess
   ├─ Stream request body to git stdin
   └─ Stream git stdout back to client
                ↓
7. If INVALID:
   └─ Return HTTP 403 with error message
```

### Repository Announcement Flow

```
1. Nostr Client → EVENT (Kind 30317)
                ↓
2. Nostr relay receives event
                ↓
3. RepositoryAnnouncementPolicy::admit_event()
   ├─ Check if instance in clone tags
   ├─ Check if instance in relays tags
   └─ Accept or reject
                ↓
4. If ACCEPTED:
   ├─ Event saved to store
   └─ on_event_saved hook triggered
                ↓
5. handle_repository_announcement()
   ├─ Parse repository details
   ├─ Create Git repository directory
   ├─ Initialize bare Git repo
   └─ Configure Git settings
```

## Key Implementation Details

### 1. Parsing Git Receive-Pack Protocol

The Git receive-pack protocol uses a pkt-line format. We need to parse:

```
0000-0000-0000-0000 0000-0000-0000-0000 refs/heads/main\0 report-status
0000-0000-0000-0000 0000-0000-0000-0000 refs/heads/dev
```

Each line has:
- Old SHA (40 hex chars)
- Space
- New SHA (40 hex chars)
- Space
- Ref name
- Optional capabilities (first line only, after \0)

```rust
pub struct RefUpdate {
    pub old_sha: String,
    pub new_sha: String,
    pub ref_name: String,
}

pub fn parse_ref_updates(body: &[u8]) -> Result<Vec<RefUpdate>> {
    // Parse pkt-line format
    // Extract ref updates
    // Return structured data
}
```

### 2. Maintainer Recursion

The maintainer resolution must handle cycles and correctly build the set:

```rust
fn get_maintainers_recursive(
    events: &[Event],
    pubkey: &str,
    identifier: &str,
    visited: &mut HashSet<String>,
) -> HashSet<String> {
    if visited.contains(pubkey) {
        return HashSet::new();
    }
    
    visited.insert(pubkey.to_string());
    
    let announcement = find_announcement(events, pubkey, identifier);
    if announcement.is_none() {
        return HashSet::new();
    }
    
    let repo = parse_repository(announcement.unwrap());
    
    for maintainer in repo.maintainers {
        get_maintainers_recursive(events, &maintainer, identifier, visited);
    }
    
    visited.clone()
}
```

### 3. State Event Validation

```rust
fn validate_state_ref(
    state: &RepositoryState,
    ref_update: &RefUpdate,
) -> Result<()> {
    if ref_update.ref_name.starts_with("refs/heads/") {
        let branch_name = &ref_update.ref_name[11..];
        if let Some(commit) = state.branches.get(branch_name) {
            if commit == &ref_update.new_sha {
                return Ok(());
            }
            return Err(Error::StateMismatch {
                ref_name: ref_update.ref_name.clone(),
                expected: commit.clone(),
                got: ref_update.new_sha.clone(),
            });
        }
        return Err(Error::RefNotInState(ref_update.ref_name.clone()));
    }
    
    if ref_update.ref_name.starts_with("refs/tags/") {
        let tag_name = &ref_update.ref_name[10..];
        if let Some(commit) = state.tags.get(tag_name) {
            if commit == &ref_update.new_sha {
                return Ok(());
            }
            return Err(Error::StateMismatch {
                ref_name: ref_update.ref_name.clone(),
                expected: commit.clone(),
                got: ref_update.new_sha.clone(),
            });
        }
        return Err(Error::RefNotInState(ref_update.ref_name.clone()));
    }
    
    Err(Error::InvalidRef(ref_update.ref_name.clone()))
}
```

### 4. CORS Support

As per GRASP-01, we must support CORS:

```rust
use actix_cors::Cors;

fn configure_cors() -> Cors {
    Cors::default()
        .allow_any_origin()
        .allowed_methods(vec!["GET", "POST", "OPTIONS"])
        .allowed_headers(vec!["Content-Type"])
        .max_age(3600)
}

// In main.rs
App::new()
    .wrap(configure_cors())
    .configure(git_routes)
    .configure(nostr_routes)
```

## Testing Strategy

See [TEST_STRATEGY.md](TEST_STRATEGY.md) for comprehensive testing documentation, including:

- **GRASP Compliance Testing Tool**: Reusable test suite that validates any GRASP implementation against the spec
- **Spec-Mirrored Tests**: Test structure matches GRASP protocol documents exactly
- **Clear Failure Messages**: Test failures cite exact spec lines (e.g., "GRASP-01:12-13")
- **Multiple Test Levels**: Unit, integration, compliance, and end-to-end tests

### Quick Overview

```rust
// Unit Tests - Individual functions
#[test]
fn test_parse_ref_updates() {
    let body = b"0000... 0000... refs/heads/main\0report-status\n";
    let updates = parse_ref_updates(body).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].ref_name, "refs/heads/main");
}

// Integration Tests - Component interaction
#[tokio::test]
async fn test_full_push_flow() {
    let app = test_app().await;
    let (announcement, state) = app.create_repo_with_state()
        .branch("main", "commit-123")
        .build()
        .await;
    
    let result = app.git_push("main", "commit-123").await;
    assert!(result.success);
}

// Compliance Tests - GRASP spec validation
#[tokio::test]
async fn test_grasp_01_compliance() {
    use grasp_compliance_tests::{TestContext, Grasp01Spec};
    
    let ctx = TestContext::builder()
        .base_url(&server.url())
        .build();
    
    let results = Grasp01Spec::test_compliance(&ctx).await;
    assert!(results.all_passed(), "{}", results.report());
}
```

The compliance testing tool is designed as a **standalone crate** that can be:
- Used by ngit-grasp for self-validation
- Published for other GRASP implementations to use
- Updated as new GRASP specs are released
- Run in CI/CD for continuous compliance verification

## Performance Considerations

### 1. Async All The Way

- Use `tokio` for all I/O
- Non-blocking Git subprocess spawning
- Stream large pack files without buffering

### 2. Connection Pooling

- Reuse Nostr relay connections
- Connection pool for internal relay queries

### 3. Caching

- Cache parsed state events (with TTL)
- Cache maintainer sets
- Invalidate on new state events

```rust
pub struct StateCache {
    cache: Arc<RwLock<HashMap<String, CachedState>>>,
}

struct CachedState {
    state: RepositoryState,
    maintainers: Vec<String>,
    timestamp: Instant,
}

impl StateCache {
    pub async fn get_or_fetch(
        &self,
        identifier: &str,
        fetcher: impl Future<Output = Result<(RepositoryState, Vec<String>)>>,
    ) -> Result<(RepositoryState, Vec<String>)> {
        // Check cache
        // Return if fresh
        // Otherwise fetch and cache
    }
}
```

## Future Extensions

### GRASP-02: Proactive Sync

Add background tasks:

```rust
pub struct ProactiveSyncTask {
    relay_client: Client,
    git_manager: RepositoryManager,
}

impl ProactiveSyncTask {
    pub async fn run(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            
            // Fetch all announcements from our relay
            let announcements = self.fetch_announcements().await;
            
            for ann in announcements {
                // Sync events from listed relays
                self.sync_events(&ann).await;
                
                // Sync git data from listed clones
                self.sync_git_data(&ann).await;
                
                // Fetch PR data
                self.sync_pr_data(&ann).await;
            }
        }
    }
}
```

### GRASP-05: Archive

Relax the policy:

```rust
pub struct ArchiveAnnouncementPolicy;

impl WritePolicy for ArchiveAnnouncementPolicy {
    fn admit_event(&self, event: &Event, _addr: &SocketAddr) 
        -> BoxFuture<PolicyResult> 
    {
        // Accept all repository announcements
        // Don't check clone/relays tags
        PolicyResult::Accept
    }
}
```

## Deployment

### Single Binary

```bash
cargo build --release
./target/release/ngit-grasp
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
5. **Nostr Event Validation**: Strict signature verification

## Conclusion

The inline authorization approach provides a cleaner, more maintainable architecture than hook-based authorization while maintaining full GRASP-01 compliance. The Rust ecosystem provides excellent libraries for both Git and Nostr protocols, enabling a high-performance, type-safe implementation.

The key insight is that we don't need to rely on Git's hook mechanism when we have full control over the HTTP layer that Git operates through. By intercepting at the HTTP handler level, we gain better error handling, easier testing, and tighter integration between the Git and Nostr components.
