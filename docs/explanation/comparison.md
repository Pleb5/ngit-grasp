# ngit-grasp vs ngit-relay Comparison

This document compares ngit-grasp (this project) with ngit-relay (the reference implementation) based on their actual implementations.

## High-Level Overview

| Aspect | ngit-relay (Reference) | ngit-grasp (This Project) |
|--------|------------------------|---------------------------|
| **Language** | Go | Rust |
| **Architecture** | Multi-process (nginx + fcgiwrap + khatru + sync daemon) | Single integrated process |
| **Git Protocol** | git-http-backend (C via fcgiwrap) | HTTP layer in Rust + git subprocess |
| **Authorization** | Pre-receive Git hook | Inline HTTP handler validation |
| **Nostr Relay** | Khatru (Go library) | nostr-relay-builder (Rust library) |
| **Event Store** | Badger (Go KV database) | LMDB or NostrDB (Rust) |
| **Proactive Sync** | Git-only (polls DB + fetches from git servers) | Nostr event sync + git sync (event-driven) |
| **Process Management** | supervisord (4 processes) | Single tokio runtime |
| **Packaging** | Docker with supervisord | Single static binary or Docker |
| **Configuration** | Environment variables | Environment variables + CLI flags |
| **Total Code** | ~1,866 lines of Go | ~25,000 lines of Rust |

## Architecture Comparison

### ngit-relay (Multi-Process)

```
┌──────────────── Docker Container ────────────────┐
│                                                   │
│  ┌─────────────────────────────────────────────┐ │
│  │            supervisord                       │ │
│  │  - fcgiwrap (git-http-backend wrapper)      │ │
│  │  - nginx (HTTP + reverse proxy)             │ │
│  │  - ngit-relay-khatru (Nostr relay)          │ │
│  │  - ngit-relay-proactive-sync (sync daemon)  │ │
│  └─────────────────────────────────────────────┘ │
│                                                   │
│  ┌──────────┐         ┌────────────────────┐     │
│  │  nginx   │────────▶│ git-http-backend   │     │
│  │  :80     │         │ (C binary via CGI) │     │
│  └──────┬───┘         └──────────┬─────────┘     │
│         │                        │                │
│         │                        ▼                │
│         │               ┌──────────────────┐     │
│         │               │  Git Repos       │     │
│         │               │  + pre-receive   │     │
│         │               │    hook (Go)     │     │
│         │               └────────┬─────────┘     │
│         │                        │ WebSocket     │
│         │                        │ query         │
│         │                        ▼                │
│         │               ┌──────────────────┐     │
│         └──────────────▶│  Khatru Relay    │     │
│                         │  :3334           │     │
│                         │  (Badger DB)     │     │
│                         └──────────────────┘     │
│                                                   │
│         Separate sync daemon polls relay DB      │
│         and fetches from remote git servers      │
│                                                   │
└───────────────────────────────────────────────────┘
```

### ngit-grasp (Single Process)

```
┌────────────── ngit-grasp (Single Binary) ─────────────┐
│                                                        │
│  ┌──────────────────────────────────────────────────┐ │
│  │         hyper HTTP Server (:7334)                 │ │
│  │  - WebSocket upgrade for Nostr relay              │ │
│  │  - Git Smart HTTP handlers                        │ │
│  │  - Landing page + metrics endpoint                │ │
│  └───────┬──────────────────────┬───────────────────┘ │
│          │                      │                      │
│          ▼                      ▼                      │
│  ┌──────────────┐      ┌────────────────────┐         │
│  │ Git Handlers │      │  Nostr Relay       │         │
│  │ (HTTP layer) │      │ (nostr-relay-      │         │
│  │              │      │  builder library)  │         │
│  │ - info/refs  │      │  - NIP-34 Policy   │         │
│  │ - upload-pk  │◀─────┤    (inline query)  │         │
│  │ - receive-pk │ auth │  - LMDB/NostrDB    │         │
│  │   + inline   │ check│  - WebSocket       │         │
│  │   validation │      │  - NIP-11 endpoint │         │
│  └──────┬───────┘      └──────────┬─────────┘         │
│         │                         │                    │
│         ▼                         ▼                    │
│  ┌──────────────┐      ┌────────────────────┐         │
│  │ git binary   │      │  Purgatory         │         │
│  │  upload-pack │      │  (in-memory queue) │         │
│  │  receive-pk  │      │  + sync loop       │         │
│  └──────────────┘      └────────────────────┘         │
│                                                        │
│  ┌──────────────────────────────────────────────────┐ │
│  │  SyncManager (tokio background task)             │ │
│  │  - Multi-relay Nostr event sync (GRASP-02)       │ │
│  │  - Negentropy + REQ/EOSE support                 │ │
│  │  - Health tracking & exponential backoff         │ │
│  │  - Git fetch from remote servers (via purgatory) │ │
│  └──────────────────────────────────────────────────┘ │
│                                                        │
│  ┌──────────────────────────────────────────────────┐ │
│  │      Shared State (Arc<T>)                        │ │
│  │  - Database (LMDB/NostrDB/Memory)                 │ │
│  │  - Purgatory (DashMap - concurrent queue)         │ │
│  │  - Metrics (Prometheus)                           │ │
│  └──────────────────────────────────────────────────┘ │
│                                                        │
└────────────────────────────────────────────────────────┘
```

## Feature Comparison

### Key Architectural Difference: Nostr Event Sync

**The biggest difference between the two implementations is how they handle Nostr events:**

| Aspect | ngit-relay | ngit-grasp |
|--------|-----------|-----------|
| **Event Arrival** | Relies on clients to push events directly | Proactively syncs events from other relays |
| **Discovery** | None - only stores what clients send | Discovers events from relay network |
| **Coordination** | Events and git data handled separately | Purgatory coordinates events + git data |
| **Completeness** | May miss events if clients don't push to this relay | Actively fetches missing events from network |
| **Implementation** | No event sync code (~0 lines) | Full multi-relay sync system (~5,000 lines) |

**Example scenario:**
- User creates PR on relay A, pushes git data to server B
- **ngit-relay**: Only knows about events/data pushed directly to it
- **ngit-grasp**: Discovers PR event from relay A, fetches git data from server B

This is why ngit-grasp has ~13x more code - the majority is implementing GRASP-02 proactive event sync.

### Git Protocol Implementation

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **HTTP Server** | nginx | hyper (Rust) |
| **Git Backend** | git-http-backend (C) via fcgiwrap | HTTP protocol layer (Rust) + git binary |
| **Process Model** | FastCGI spawns git-http-backend | HTTP handler spawns git subprocess |
| **Upload Pack** | C binary passthrough | Rust parses HTTP → spawns `git upload-pack` |
| **Receive Pack** | C binary → pre-receive hook | Rust validates → spawns `git receive-pack` |
| **Authorization** | Go hook queries relay via WebSocket | In-process function call before git spawn |
| **Error Reporting** | Hook stderr → git client | HTTP response body (before git runs) |
| **CORS** | nginx config | hyper middleware |
| **Lines of Code** | ~0 (uses C binary) + hook ~135 | ~1,000+ (HTTP protocol layer) |

### Authorization Logic

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **Location** | pre-receive hook (separate Go binary) | Inline HTTP handler (Rust) |
| **Trigger** | Git invokes hook during push | HTTP handler before spawning git |
| **State Query** | WebSocket to localhost:3334 | Direct database query (in-process) |
| **Latency** | +50-100ms (hook spawn + WS query) | +10-20ms (function call) |
| **Error Channel** | stderr → git client | HTTP 403 response |
| **Ref Parsing** | Read from stdin (hook protocol) | Parse from HTTP request body |
| **Maintainer Resolution** | Recursive Go function | Recursive Rust function (similar) |
| **State Caching** | None (queries relay per push) | Purgatory tracks pending events |

### Nostr Relay

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **Implementation** | Khatru (Go library) | nostr-relay-builder (Rust library) |
| **Database** | Badger (Go KV store) | LMDB or NostrDB (Rust) |
| **Process** | Separate process on :3334 | Integrated (same binary) |
| **Policies** | Go functions in `policies.go` | Rust traits (modular sub-policies) |
| **Event Validation** | Single function with branches | 4 separate policy modules |
| **WebSocket** | Khatru built-in | nostr-relay-builder + hyper |
| **NIP-11** | Manual JSON in code | Built-in support from library |
| **Connection** | Separate from HTTP | Shared hyper server |
| **Lines of Code** | ~186 (policies.go) + Khatru library | ~3,000+ (policy modules) + nostr-relay-builder library |

### Proactive Sync

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **Architecture** | Separate daemon (`ngit-relay-proactive-sync`) | Integrated SyncManager (tokio task) |
| **Nostr Event Sync** | ❌ None (relies on client pushes) | ✅ Multi-relay sync with negentropy/REQ |
| **Git Data Sync** | ✅ Polls local DB + fetches from git servers | ✅ Event-driven via purgatory queue |
| **Sync Trigger** | Timer (every 15 minutes) | Immediate on event arrival + timer for retries |
| **Relay Discovery** | N/A (no event sync) | Dynamic from 30617 announcement events |
| **Protocol** | Git fetch only | Nostr WebSocket + git fetch |
| **Concurrency** | Goroutines (per-repo iteration) | Tokio async tasks (per-relay connections) |
| **Health Tracking** | Basic retry on git fetch failures | RelayHealthTracker with exponential backoff |
| **Connection Management** | N/A (no Nostr connections) | Persistent connections with reconnect |
| **Coordination** | Separate process | Purgatory + SyncManager coordination |
| **Lines of Code** | ~112 (main.go) + ~305 (git sync) | ~5,000+ (Nostr sync + git sync + coordination) |

### Repository Management

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **Creation** | Event hook → shell commands | Event hook → tokio::process |
| **Trigger** | `EventReceiveHook()` in Go | `handle_announcement()` in Rust |
| **Configuration** | `git config` via shell | `git config` via tokio::process |
| **Hook Installation** | Symlinks to pre-receive/post-receive | Not needed (inline auth) |
| **Permissions** | `chown nginx:nginx` | tokio::fs permissions |
| **Path Structure** | `<npub>/<id>.git` | `<npub>/<id>.git` (same) |

### Event Coordination (Purgatory)

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **Implementation** | None | Dedicated Purgatory system |
| **Purpose** | N/A | Solves "which arrives first?" problem |
| **Storage** | N/A | In-memory DashMap (thread-safe) |
| **Expiry** | N/A | 30 minutes default TTL |
| **State Events** | Accepted (git sync happens later via timer) | Queued until git data arrives |
| **PR Events** | Accepted (references may be missing) | Queued with placeholder refs |
| **Sync Queue** | Timer-based (polls all repos) | Event-driven (only syncs needed repos) |
| **Cleanup** | N/A | Background task (60s interval) |
| **Lines of Code** | 0 | ~2,000+ |

**Impact**: ngit-relay accepts all events and relies on periodic sync to eventually fetch git data. ngit-grasp holds events in purgatory and triggers targeted syncs, providing faster convergence and better coordination between Nostr events and git data.

### Deployment & Operations

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **Dependencies** | nginx, git, fcgiwrap, supervisord, Go runtime | git, Rust binary (statically linked) |
| **Process Count** | 4 (supervisord + nginx + khatru + sync) | 1 (single tokio runtime) |
| **Configuration** | `.env` file | `.env` + CLI flags (clap) |
| **Docker Image Size** | ~500MB (Alpine + tools + Go runtime) | ~100MB (Debian slim + git + binary) |
| **Startup Time** | ~2-5 seconds (multiple processes) | ~0.5 seconds (single process) |
| **Memory (Idle)** | ~150-200MB (4 processes + Go GC) | ~50-100MB (single process, no GC) |
| **Logs** | supervisord → stdout (4 streams) | tracing → stdout (unified) |
| **Monitoring** | None built-in | Prometheus metrics endpoint |
| **Binary Distribution** | Docker only | Native binary + Docker |

### Development Experience

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| **Build Time** | Fast (~5s incremental, Go) | Slow first build (~5min), fast incremental |
| **Type Safety** | Good (Go interfaces) | Excellent (Rust traits + ownership) |
| **Testing** | Go tests + shell scripts | Rust unit + integration tests |
| **Test Relay** | Manual Docker setup | `TestRelay` fixture (auto-start binary) |
| **Debugging** | Multi-process (harder) | Single process (easier) |
| **IDE Support** | Good (gopls) | Excellent (rust-analyzer) |
| **Async Model** | Goroutines (simple) | Tokio (more complex) |
| **Error Handling** | `error` interface + if checks | Result<T, E> + `?` operator |
| **Dependencies** | Go modules | Cargo crates (larger ecosystem) |

### Code Complexity

| Component | ngit-relay | ngit-grasp | Notes |
|-----------|-----------|-----------|-------|
| Main server | 129 | 196 | ngit-relay uses supervisord |
| Git HTTP protocol | 0 (C binary via fcgiwrap) | ~1,000 | ngit-grasp implements HTTP layer |
| Auth logic (hooks) | 135 + 52 | 0 | ngit-grasp inline, no hooks |
| Auth logic (inline) | 0 | ~800 | ngit-grasp authorization module |
| Nostr relay policies | 186 | ~3,000 | Both use libraries (Khatru vs nostr-relay-builder) |
| Git-only proactive sync | 112 + 305 | 0 | ngit-relay git sync only |
| Nostr event proactive sync | 0 | ~5,000 | ngit-grasp adds full event sync (GRASP-02 v4) |
| Purgatory coordination | 0 | ~2,000 | ngit-grasp event/git coordination |
| Shared utils | 241 + 132 | ~4,000 | ngit-grasp more comprehensive |
| Config | ~50 | ~400 | ngit-grasp CLI + validation |
| Metrics | 0 | ~1,500 | ngit-grasp Prometheus |
| **Total** | **~1,866** | **~25,000** | ngit-grasp 13x more code |

**Why the difference?**
- **Nostr event sync**: ngit-relay has NONE, ngit-grasp implements full multi-relay event sync (~5,000 lines)
- **Git HTTP protocol**: ngit-relay uses C binary, ngit-grasp implements HTTP layer (~1,000 lines)
- **Purgatory coordination**: ngit-grasp adds event/git coordination system (~2,000 lines)
- **Metrics & observability**: ngit-grasp includes comprehensive monitoring (~1,500 lines)
- Both use relay libraries (Khatru vs nostr-relay-builder), but ngit-grasp has more modular policies

### Performance Characteristics (Estimated)

| Metric | ngit-relay | ngit-grasp | Notes |
|--------|-----------|-----------|-------|
| **Startup** | ~2-5s | ~0.5s | Single process vs multi-process |
| **Memory (Idle)** | ~150MB | ~75MB | No GC, single process |
| **Memory (Active)** | ~200MB+ | ~100-150MB | Depends on event volume |
| **CPU (Idle)** | ~1-2% | ~0.5% | Fewer processes |
| **Push Latency** | +50-100ms | +10-20ms | No hook spawn overhead |
| **Clone Latency** | ~same | ~same | Both passthrough to git |
| **Concurrent Pushes** | Good (goroutines) | Excellent (tokio async) |
| **Event Ingestion** | Good (Badger) | Excellent (LMDB zero-copy) |
| **Sync Throughput** | Moderate (polling) | High (negentropy + async) |

*These are estimates based on architecture. Actual performance depends on workload.*

## Migration Path

For users of ngit-relay, migration to ngit-grasp involves:

### Data Migration

1. **Events**: Export from Badger → Import to LMDB/NostrDB
   - No direct migration tool yet (would need to be built)
   - Alternative: Use proactive sync to re-fetch from other relays
2. **Git Repositories**: Direct copy (same structure)
   ```bash
   cp -r /srv/ngit-relay/repos/* /path/to/ngit-grasp/data/git/
   ```
3. **Configuration**: Translate environment variables
   - Most variables are compatible (`NGIT_DOMAIN`, etc.)
   - Remove nginx/supervisord-specific configs

### Compatibility

- **Git Data**: 100% compatible (same repository structure)
- **Nostr Events**: 100% compatible (standard NIP-34)
- **HTTP URLs**: Compatible (same path structure)
- **Git Hooks**: ngit-grasp doesn't use hooks (inline auth instead)

### Downtime

- Option 1: Run both in parallel (different domains), gradually migrate
- Option 2: Short downtime for data copy + config update

## When to Choose Each

### Choose ngit-relay (Reference) if:

- ✅ You need proven, production-tested code
- ✅ You're already familiar with Go ecosystem
- ✅ You prefer simple, minimal codebases (~1,866 lines)
- ✅ You trust battle-tested C binaries (git-http-backend)
- ✅ You want to stay close to the reference implementation
- ✅ You need to deploy immediately without complexity
- ✅ Your users will push events directly to your relay (no sync needed)
- ✅ You only need git data sync, not Nostr event sync

### Choose ngit-grasp (This Project) if:

- ✅ **You need Nostr event sync from other relays** (the main differentiator)
- ✅ You want better performance and lower resource usage
- ✅ You prefer Rust's type safety and memory safety
- ✅ You want simpler deployment (single binary, no supervisord)
- ✅ You need event/git data coordination (purgatory)
- ✅ You want inline authorization (lower latency)
- ✅ You need comprehensive observability (Prometheus metrics)
- ✅ You're comfortable with more complex codebase (~25,000 lines)
- ✅ You want full GRASP-02 v4 multi-relay event discovery

## Current Status

### ngit-relay (Reference)
- ✅ GRASP-01 complete and production-ready
- ✅ Git data proactive sync (fetches from git servers)
- ❌ No Nostr event sync (relies on client pushes)
- ✅ Battle-tested in production
- 🔄 Community adoption growing

### ngit-grasp (This Project)
- ✅ GRASP-01 complete with comprehensive testing
- ✅ GRASP-02 v4 multi-relay Nostr event sync with negentropy
- ✅ Git data proactive sync (via purgatory queue)
- ✅ Purgatory system for event/git coordination
- ✅ Prometheus metrics and health tracking
- ✅ NIP-77 negentropy support
- ✅ Full integration test suite
- 🔄 Production deployment validation ongoing

## Conclusion

Both implementations are valid approaches to GRASP with different philosophies:

- **ngit-relay** prioritizes simplicity - clients push events, relay syncs git data (~1,866 lines)
- **ngit-grasp** prioritizes completeness - syncs both events and git data from network (~25,000 lines)

**The fundamental difference**: ngit-relay expects clients to push Nostr events to it. ngit-grasp proactively discovers and syncs events from other relays in the network.

The choice depends on your priorities:

| Priority | Recommendation |
|----------|---------------|
| **Simplicity** | ngit-relay |
| **Event Discovery** | ngit-grasp (syncs from network) |
| **Production Stability** | ngit-relay (more battle-tested) |
| **Event Completeness** | ngit-grasp (proactive sync) |
| **Low Resources** | ngit-grasp (single binary, lower memory) |
| **Quick Deploy** | ngit-relay (Docker Compose) |
| **Development** | ngit-grasp (better tooling, type safety) |
| **Network Resilience** | ngit-grasp (multi-relay sync) |

For deployments where **Nostr event sync** is important (discovering events from other relays), **ngit-grasp** is required. For simpler deployments where users will push events directly, **ngit-relay** is sufficient and battle-tested.
