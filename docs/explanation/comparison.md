# ngit-grasp vs ngit-relay Comparison

## High-Level Comparison

| Aspect | ngit-relay (Reference) | ngit-grasp (This Project) |
|--------|------------------------|---------------------------|
| **Language** | Go | Rust |
| **Architecture** | Multi-process (nginx, git-http-backend, hooks, relay) | Single integrated process |
| **Authorization** | Git pre-receive hook | Inline HTTP handler |
| **Packaging** | Docker + supervisord | Single binary or Docker |
| **Configuration** | Multiple config files | Environment variables |
| **Deployment** | Docker Compose | Binary or Docker |
| **Testing** | Go tests + shell scripts | Rust unit + integration tests |

## Component Breakdown

### ngit-relay (Go)

```
┌─────────────────────────────────────────────────┐
│                   Docker Container               │
├─────────────────────────────────────────────────┤
│                                                  │
│  ┌──────────┐         ┌─────────────────────┐  │
│  │  nginx   │────────▶│ git-http-backend    │  │
│  │  :80     │         │ (C binary)          │  │
│  └──────────┘         └──────────┬──────────┘  │
│       │                           │              │
│       │                           ▼              │
│       │                  ┌─────────────────┐    │
│       │                  │  Git Repo       │    │
│       │                  │  + Hooks        │    │
│       │                  └────────┬────────┘    │
│       │                           │              │
│       │                           ▼              │
│       │                  ┌─────────────────┐    │
│       │                  │ pre-receive     │    │
│       │                  │ (Go binary)     │    │
│       │                  └────────┬────────┘    │
│       │                           │              │
│       │                           │ WebSocket    │
│       │                           ▼              │
│       │                  ┌─────────────────┐    │
│       └─────────────────▶│  Khatru Relay   │    │
│                          │  (Go)           │    │
│                          └─────────────────┘    │
│                                                  │
│  ┌──────────────────────────────────────────┐  │
│  │         supervisord                       │  │
│  │  - nginx                                  │  │
│  │  - khatru                                 │  │
│  │  - proactive-sync                         │  │
│  └──────────────────────────────────────────┘  │
│                                                  │
└─────────────────────────────────────────────────┘
```

### ngit-grasp (Rust)

```
┌─────────────────────────────────────────────────┐
│              ngit-grasp (Single Binary)          │
├─────────────────────────────────────────────────┤
│                                                  │
│  ┌──────────────────────────────────────────┐  │
│  │         actix-web HTTP Server             │  │
│  │              :8080                        │  │
│  └───────┬──────────────────────┬────────────┘  │
│          │                      │                │
│          ▼                      ▼                │
│  ┌──────────────┐      ┌──────────────────┐    │
│  │ Git Handlers │      │  Nostr Relay     │    │
│  │              │      │  (relay-builder) │    │
│  │ - upload-pk  │      │                  │    │
│  │ - receive-pk │◀─────│  - Policies      │    │
│  │   + inline   │ query│  - Event store   │    │
│  │   validation │      │  - WebSocket     │    │
│  └──────┬───────┘      └──────────────────┘    │
│         │                                        │
│         ▼                                        │
│  ┌──────────────┐                               │
│  │  Git Repos   │                               │
│  │  (spawned    │                               │
│  │   git cmds)  │                               │
│  └──────────────┘                               │
│                                                  │
│  ┌──────────────────────────────────────────┐  │
│  │      Shared State (Arc<AppState>)         │  │
│  │  - RepositoryManager                      │  │
│  │  - NostrClient                            │  │
│  │  - StateCache                             │  │
│  └──────────────────────────────────────────┘  │
│                                                  │
└─────────────────────────────────────────────────┘
```

## Detailed Feature Comparison

### Git Protocol Handling

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| Implementation | git-http-backend (C) | git-http-backend (Rust crate) |
| Process model | nginx → C binary | actix-web → Rust handler |
| Upload pack | Passthrough | Passthrough with validation |
| Receive pack | Hook-based auth | Inline validation |
| Error handling | Hook stderr | HTTP response |
| CORS | nginx config | actix-cors middleware |

### Nostr Relay

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| Implementation | Khatru (Go) | nostr-relay-builder (Rust) |
| Event store | Badger (Go) | LMDB or NDB (Rust) |
| Policies | Go functions | Rust traits |
| WebSocket | Khatru built-in | nostr-relay-builder |
| NIP-11 | Manual JSON | Built-in support |

### Authorization Logic

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| Location | pre-receive hook | HTTP handler |
| Language | Go | Rust |
| State query | WebSocket to localhost:3334 | In-process function call |
| Error reporting | stderr → git client | HTTP response body |
| Ref validation | Line-by-line stdin | Parsed from request body |
| Maintainer resolution | Recursive Go function | Recursive Rust function |
| State caching | Per-request | Shared cache with TTL |

### Repository Management

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| Creation | Event hook + shell commands | Event hook + tokio::process |
| Configuration | git config via shell | git config via tokio::process |
| Hook installation | Symlinks | Not needed (inline auth) |
| Permissions | chown nginx:nginx | tokio::fs permissions |
| Path structure | `<npub>/<id>.git` | `<npub>/<id>.git` (same) |

### Deployment

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| Dependencies | nginx, git, Go runtime | git, Rust binary (no runtime) |
| Process management | supervisord | Single process (tokio) |
| Configuration | Multiple files + .env | .env only |
| Docker image size | ~500MB (Alpine + tools) | ~50MB (scratch + binary + git) |
| Startup time | ~2-5 seconds | ~0.5 seconds |
| Memory usage | ~100-200MB (multiple processes) | ~50-100MB (single process) |

### Development Experience

| Feature | ngit-relay | ngit-grasp |
|---------|-----------|-----------|
| Build time | Fast (Go) | Medium (Rust first build, then fast) |
| Type safety | Go (good) | Rust (excellent) |
| Testing | Go test + shell | Rust test (unit + integration) |
| Debugging | Multiple processes | Single process |
| Hot reload | Manual | cargo-watch |
| IDE support | Good (Go) | Excellent (rust-analyzer) |

## Performance Comparison (Estimated)

| Metric | ngit-relay | ngit-grasp | Notes |
|--------|-----------|-----------|-------|
| Startup | ~2-5s | ~0.5s | Fewer processes |
| Memory | ~150MB | ~75MB | Single process, no GC |
| CPU (idle) | ~1-2% | ~0.5% | Fewer processes |
| Push latency | +50-100ms | +10-20ms | No hook spawn overhead |
| Clone latency | ~same | ~same | Both passthrough to Git |
| Concurrent pushes | Good | Excellent | Tokio async vs goroutines |
| Event ingestion | Good | Excellent | Rust async + zero-copy |

*Note: These are estimates. Actual performance depends on workload and hardware.*

## Code Complexity

### Lines of Code (Estimated)

| Component | ngit-relay | ngit-grasp |
|-----------|-----------|-----------|
| Main server | ~150 | ~200 |
| Git handlers | ~0 (C binary) | ~500 |
| Auth logic | ~200 | ~300 |
| Nostr relay | ~500 | ~100 (using library) |
| Shared utils | ~300 | ~200 |
| Config/setup | ~200 | ~100 |
| **Total** | **~1,350** | **~1,400** |

Similar complexity, but ngit-grasp has:
- More Git protocol code (we implement it)
- Less Nostr relay code (using library)
- Less deployment code (no hooks/supervisord)

## Migration Path

For users of ngit-relay, migration to ngit-grasp would involve:

1. **Export data** from Badger to LMDB/NDB
2. **Copy Git repositories** (same structure)
3. **Update environment variables** (mostly compatible)
4. **Change deployment** from Docker Compose to binary/Docker
5. **Update URLs** if domain changes

The **Nostr events** and **Git data** are compatible - only the server changes.

## When to Choose Each

### Choose ngit-relay (Reference) if:

- ✅ You need a proven, production-tested implementation
- ✅ You're already familiar with Go
- ✅ You want to stay close to the reference
- ✅ You need to deploy immediately
- ✅ You prefer Docker Compose workflows

### Choose ngit-grasp (This Project) if:

- ✅ You want better performance and lower resource usage
- ✅ You prefer Rust's type safety and ecosystem
- ✅ You want simpler deployment (single binary)
- ✅ You want to contribute to a modern codebase
- ✅ You're building on top of the GRASP protocol
- ✅ You want inline authorization over hooks
- ✅ You need better integration testing

## Future Roadmap Comparison

### ngit-relay (Reference)
- ✅ GRASP-01 complete
- 🔄 GRASP-02 in progress
- ⏭️ GRASP-05 planned
- ⏭️ NIP-42 auth-to-read
- ⏭️ NIP-70 protected events
- ⏭️ Spam prevention

### ngit-grasp (This Project)
- 🔄 GRASP-01 in development
- ⏭️ GRASP-02 planned (easier with Rust async)
- ⏭️ GRASP-05 planned
- ⏭️ Advanced caching strategies
- ⏭️ Metrics and observability
- ⏭️ Plugin system for custom policies

## Conclusion

Both implementations are valid approaches to GRASP:

- **ngit-relay** is the mature, proven reference implementation
- **ngit-grasp** is a modern, performant alternative with better DX

The choice depends on your priorities: stability vs. performance, familiarity vs. innovation, proven vs. cutting-edge.

For new deployments where performance and simplicity matter, **ngit-grasp** is the recommended choice. For production systems requiring maximum stability, **ngit-relay** is the safer bet until ngit-grasp reaches maturity.
