# ngit-grasp Architecture Diagram

**Date:** November 4, 2025  
**Purpose:** Visual reference for single-port architecture

---

## Current Architecture (WRONG ❌)

```
┌─────────────────────────────────────────┐
│  Port 8080                              │
│  ┌───────────────────────────────────┐  │
│  │   Nostr Relay (WebSocket)         │  │
│  │   - NIP-01 protocol               │  │
│  │   - Event storage                 │  │
│  │   - Subscriptions                 │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  Port 8081 (WRONG!)                     │
│  ┌───────────────────────────────────┐  │
│  │   Git HTTP Server                 │  │
│  │   - Not implemented               │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘
```

**Problem:** GRASP-01 requires single port!

---

## Target Architecture (CORRECT ✅)

```
┌─────────────────────────────────────────────────────────────┐
│  Single Port (8080)                                         │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐ │
│  │  actix-web HTTP Server                                │ │
│  │                                                         │ │
│  │  ┌──────────────────────────────────────────────────┐ │ │
│  │  │  CORS Middleware (ALL requests)                  │ │ │
│  │  │  - Access-Control-Allow-Origin: *                │ │ │
│  │  │  - Access-Control-Allow-Methods: GET, POST       │ │ │
│  │  │  - Access-Control-Allow-Headers: Content-Type    │ │ │
│  │  └──────────────────────────────────────────────────┘ │ │
│  │                                                         │ │
│  │  ┌──────────────────────────────────────────────────┐ │ │
│  │  │  HTTP Router                                     │ │ │
│  │  │                                                   │ │ │
│  │  │  Path Pattern Matching:                          │ │ │
│  │  │  - /<npub>/<identifier>.git/*  → Git Handler     │ │ │
│  │  │  - /*                          → Nostr Handler   │ │ │
│  │  └──────────────────────────────────────────────────┘ │ │
│  │                                                         │ │
│  │  ┌────────────────────┐  ┌─────────────────────────┐  │ │
│  │  │  Git HTTP Handler  │  │  Nostr Relay Handler    │  │ │
│  │  │                    │  │                         │  │ │
│  │  │  ┌──────────────┐  │  │  ┌──────────────────┐  │  │ │
│  │  │  │ git-http-    │  │  │  │ WebSocket Upgrade│  │  │ │
│  │  │  │ backend      │  │  │  │                  │  │  │ │
│  │  │  │              │  │  │  │ ┌──────────────┐ │  │  │ │
│  │  │  │ - info/refs  │  │  │  │ │ NIP-01       │ │  │  │ │
│  │  │  │ - upload-pack│  │  │  │ │ - EVENT      │ │  │  │ │
│  │  │  │ - receive-pack  │  │  │ │ - REQ        │ │  │  │ │
│  │  │  │              │  │  │  │ │ - CLOSE      │ │  │  │ │
│  │  │  │ Authorization:  │  │  │ └──────────────┘ │  │  │ │
│  │  │  │ - Query state│  │  │  │                  │  │  │ │
│  │  │  │ - Validate   │  │  │  │ ┌──────────────┐ │  │  │ │
│  │  │  │ - Accept/    │  │  │  │ │ NIP-11       │ │  │  │ │
│  │  │  │   Reject     │  │  │  │ │ - GRASP fields│ │  │  │ │
│  │  │  └──────────────┘  │  │  │ └──────────────┘ │  │  │ │
│  │  │                    │  │  │                  │  │  │ │
│  │  │  Repository:       │  │  │ ┌──────────────┐ │  │  │ │
│  │  │  {GIT_DATA_PATH}/  │  │  │ │ NIP-34       │ │  │  │ │
│  │  │  {npub}/           │  │  │ │ - Announce   │ │  │  │ │
│  │  │  {identifier}.git  │  │  │ │ - State      │ │  │  │ │
│  │  │                    │  │  │ │ - Validate   │ │  │  │ │
│  │  └────────────────────┘  │  │ └──────────────┘ │  │  │ │
│  │                           │  │                  │  │  │ │
│  │                           │  │  HTTP Root:      │  │  │ │
│  │                           │  │  - Serve HTML    │  │  │ │
│  │                           │  │  - NIP-11 JSON   │  │  │ │
│  │                           │  └──────────────────┘  │  │ │
│  │                           │                         │  │ │
│  └───────────────────────────┴─────────────────────────┘  │ │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐ │
│  │  Storage Layer                                        │ │
│  │                                                        │ │
│  │  ┌──────────────────────┐  ┌──────────────────────┐  │ │
│  │  │ Git Repositories     │  │ Nostr Events DB      │  │ │
│  │  │                      │  │                      │  │ │
│  │  │ {GIT_DATA_PATH}/     │  │ {RELAY_DATA_PATH}/   │  │ │
│  │  │ ├── npub1.../        │  │ - Announcements      │  │ │
│  │  │ │   ├── repo1.git/   │  │ - State events       │  │ │
│  │  │ │   └── repo2.git/   │  │ - Issues/Patches     │  │ │
│  │  │ └── npub2.../        │  │ - Other events       │  │ │
│  │  │     └── repo3.git/   │  │                      │  │ │
│  │  └──────────────────────┘  └──────────────────────┘  │ │
│  └───────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

---

## Request Flow Examples

### Example 1: Git Clone

```
Client: git clone http://localhost:8080/npub1abc.../my-repo.git
   ↓
actix-web receives HTTP GET request
   ↓
CORS middleware adds headers
   ↓
Router matches path: /npub1abc.../my-repo.git
   ↓
Git Handler receives request
   ↓
git-http-backend processes:
   - GET /npub1abc.../my-repo.git/info/refs?service=git-upload-pack
   ↓
Response includes:
   - CORS headers
   - Git protocol data
   - Capabilities: allow-reachable-sha1-in-want, allow-tip-sha1-in-want
   ↓
Client receives data and clones repository
```

### Example 2: Git Push

```
Client: git push http://localhost:8080/npub1abc.../my-repo.git main
   ↓
actix-web receives HTTP POST request
   ↓
CORS middleware adds headers
   ↓
Router matches path: /npub1abc.../my-repo.git
   ↓
Git Handler receives request
   ↓
BEFORE spawning git-receive-pack:
   1. Parse ref updates from request body
   2. Query latest state announcement from relay
   3. Validate pusher in maintainer set
   4. Validate ref updates match state
   ↓
If validation passes:
   - Spawn git-receive-pack
   - Stream response back to client
   ↓
If validation fails:
   - Return HTTP 403 Forbidden
   - Include error message
   ↓
Client receives success/failure
```

### Example 3: WebSocket Connection (Nostr)

```
Client: new WebSocket('ws://localhost:8080/')
   ↓
actix-web receives HTTP GET with Upgrade: websocket
   ↓
CORS middleware adds headers
   ↓
Router matches path: /
   ↓
Nostr Handler receives request
   ↓
Upgrade to WebSocket
   ↓
Client sends: ["EVENT", {...}]
   ↓
Nostr Handler processes EVENT
   ↓
If kind 30617 (announcement):
   - Validate clone/relays tags
   - Provision Git repository
   - Store event
   ↓
Response: ["OK", event_id, true, ""]
   ↓
Client receives confirmation
```

### Example 4: NIP-11 Request

```
Client: fetch('http://localhost:8080/', {
  headers: { 'Accept': 'application/nostr+json' }
})
   ↓
actix-web receives HTTP GET with Accept header
   ↓
CORS middleware adds headers
   ↓
Router matches path: /
   ↓
Nostr Handler checks Accept header
   ↓
Returns NIP-11 JSON:
{
  "name": "ngit-grasp instance",
  "description": "Rust GRASP implementation",
  "supported_nips": [1, 11, 34],
  "supported_grasps": ["GRASP-01"],
  "repo_acceptance_criteria": "Must list this service in clone and relays tags",
  "curation": "Basic spam prevention"
}
   ↓
Client receives relay information
```

### Example 5: CORS Preflight (OPTIONS)

```
Browser: OPTIONS http://localhost:8080/
Headers:
  - Origin: https://example.com
  - Access-Control-Request-Method: POST
   ↓
actix-web receives OPTIONS request
   ↓
CORS middleware handles preflight
   ↓
Returns 204 No Content with headers:
  - Access-Control-Allow-Origin: *
  - Access-Control-Allow-Methods: GET, POST
  - Access-Control-Allow-Headers: Content-Type
  - Access-Control-Max-Age: 3600
   ↓
Browser caches preflight response
   ↓
Browser proceeds with actual request
```

---

## Component Responsibilities

### actix-web HTTP Server
- Listen on single port
- Route requests by path
- Handle WebSocket upgrades
- Apply CORS to all requests

### CORS Middleware
- Add headers to ALL responses
- Handle OPTIONS preflight
- Allow any origin (GRASP-01 requirement)

### HTTP Router
- Match `/npub.../repo.git` → Git Handler
- Match `/` → Nostr Handler
- Pass through to appropriate handler

### Git Handler
- Serve Git Smart HTTP protocol
- Read from `{GIT_DATA_PATH}/{npub}/{id}.git`
- Validate pushes before accepting
- Return 404 for missing repos

### Nostr Handler
- Upgrade HTTP to WebSocket
- Process NIP-01 messages
- Store/query events
- Serve NIP-11 for HTTP requests
- Provision repos from announcements

### Storage Layer
- Git repositories (bare)
- Nostr events (database)
- Separate paths for each

---

## Configuration Flow

```
Environment Variables
   ↓
.env file (optional)
   ↓
Config::from_env()
   ↓
Config struct:
   - bind_address: "127.0.0.1:8080"
   - domain: "example.com"
   - git_data_path: "./data/repos"
   - relay_data_path: "./data/relay"
   - relay_name: "..."
   - relay_description: "..."
   - owner_npub: "..."
   ↓
Passed to:
   - HTTP server (bind address)
   - Git handler (git_data_path, domain)
   - Nostr handler (relay_data_path, domain, NIP-11 info)
   - Storage layer (both paths)
```

---

## Test Architecture

```
Integration Test
   ↓
TestRelay::start()
   ↓
Spawns ngit-grasp process:
   - NGIT_BIND_ADDRESS=127.0.0.1:{random_port}
   - NGIT_DOMAIN=127.0.0.1:{random_port}
   - NGIT_GIT_DATA_PATH=./test-data/repos
   - NGIT_RELAY_DATA_PATH=./test-data/relay
   ↓
Process starts:
   - actix-web listens on random port
   - Both Git and Nostr available
   ↓
Test runs:
   - Uses grasp-audit library
   - Connects to ws://127.0.0.1:{port}/
   - Runs compliance tests
   ↓
TestRelay::stop()
   ↓
Process killed
   ↓
Test data cleaned up
```

---

## File Structure

```
ngit-grasp/
├── src/
│   ├── main.rs              # Entry point
│   ├── config.rs            # Configuration
│   ├── http/                # NEW - HTTP server
│   │   ├── mod.rs           # Server setup
│   │   ├── git.rs           # Git HTTP handler
│   │   └── nostr.rs         # Nostr WebSocket handler
│   ├── nostr/
│   │   ├── mod.rs
│   │   ├── relay.rs         # Relay logic (reused)
│   │   └── events.rs        # Event handling
│   └── storage/
│       ├── mod.rs
│       └── repository.rs    # Git repo management
├── tests/
│   ├── common/
│   │   ├── mod.rs
│   │   └── relay.rs         # TestRelay fixture
│   ├── nip01_compliance.rs  # NIP-01 tests
│   ├── nip34_announcements.rs  # NIP-34 tests
│   └── grasp01_git_http.rs  # NEW - GRASP-01 Git tests
├── data/                    # Runtime data (gitignored)
│   ├── repos/               # Git repositories
│   └── relay/               # Nostr events
└── test-data/               # Test data (gitignored)
    ├── repos/
    └── relay/
```

---

## Comparison: ngit-relay vs ngit-grasp

### ngit-relay (Go + nginx)

```
nginx (Port 8081)
   ├── Git HTTP → fcgiwrap → git-http-backend
   │                            ↓
   │                     pre-receive hook (Go)
   │                            ↓
   │                     Khatru relay (HTTP API)
   │
   └── Nostr → proxy → Khatru relay (Port 3334)
                            ↓
                     on_event hook (Go)
                            ↓
                     provision repos
```

**Components:**
- nginx (routing)
- fcgiwrap (CGI wrapper)
- git-http-backend (Git protocol)
- pre-receive hook (Go, validates pushes)
- post-receive hook (Go, updates HEAD)
- Khatru relay (Go, Nostr protocol)
- on_event hook (Go, provisions repos)
- supervisord (process management)

### ngit-grasp (Rust)

```
actix-web (Port 8080)
   ├── Git HTTP → git-http-backend crate
   │                   ↓
   │            inline authorization
   │                   ↓
   │            Storage (query state)
   │
   └── Nostr → WebSocket upgrade
                   ↓
              nostr-sdk relay
                   ↓
              on_event (provision repos)
                   ↓
              Storage (store events)
```

**Components:**
- actix-web (routing + HTTP + WebSocket)
- git-http-backend crate (Git protocol)
- nostr-sdk (Nostr protocol)
- Storage (unified storage layer)

**Advantages:**
- Single binary
- No external processes
- Inline authorization (better errors)
- Pure Rust (memory safety)
- Easier testing

---

**Last Updated:** November 4, 2025  
**Purpose:** Reference for implementation
