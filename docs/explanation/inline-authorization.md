# Explanation: Inline Authorization

**Purpose:** Understand why ngit-grasp validates Git pushes inline rather than using Git hooks  
**Audience:** Developers and architects wanting to understand design decisions

---

## The Problem

Git hosting with authorization requires validating pushes before accepting them. The question is: **where** should this validation happen?

Two approaches exist:

1. **Git Hooks** (traditional): Use Git's pre-receive hook mechanism
2. **Inline Authorization** (our approach): Validate before spawning Git

This document explains why we chose inline authorization and what benefits it provides.

---

## Background: How Git Hooks Work

Git provides a **pre-receive hook** that runs during `git push`:

```
Client              Server
  |                   |
  |--- git push ----->|
  |                   |--- spawn git-receive-pack
  |                   |
  |                   |--- pre-receive hook runs
  |                   |    (reads stdin: old new ref)
  |                   |    (exit 0 = accept, 1 = reject)
  |                   |
  |<--- success ------| (if hook exits 0)
  |<--- error --------| (if hook exits 1)
```

**Pros:**
- Standard Git mechanism
- Language-agnostic (hook can be any executable)
- Well-documented

**Cons:**
- Hook output goes to stderr (client sees as `remote:` messages)
- Hard to provide structured error messages
- Requires hook installation and management
- Difficult to test (needs Git repository setup)
- Hook runs *after* Git has started processing

---

## Background: How Inline Authorization Works

With inline authorization, we validate **before** spawning Git:

```
Client              Server (ngit-grasp)
  |                   |
  |--- git push ----->|--- HTTP handler receives request
  |                   |
  |                   |--- Parse ref updates from request
  |                   |--- Query Nostr relay for state
  |                   |--- Validate push against state
  |                   |
  |                   |--- If invalid: return HTTP error
  |                   |--- If valid: spawn git-receive-pack
  |                   |
  |<--- success ------| (if valid)
  |<--- HTTP error ---| (if invalid)
```

**Pros:**
- Full control over error messages (HTTP response)
- Can skip spawning Git entirely for invalid pushes
- Easier testing (pure Rust, no Git setup needed)
- Shared state between Git and Nostr components
- Better performance (early rejection)

**Cons:**
- Requires parsing Git protocol ourselves
- Less standard than hooks
- Tighter coupling to Git HTTP protocol

---

## Why Inline Authorization Is Better for GRASP

### 1. Better Error Messages

**With hooks:**
```
$ git push
remote: error: Push rejected - not authorized for ref refs/heads/main
remote: See https://docs.gitnostr.com/errors/unauthorized
To https://gitnostr.com/alice/myrepo.git
 ! [remote rejected] main -> main (pre-receive hook declined)
```

**With inline authorization:**
```
$ git push
error: RPC failed; HTTP 403 Forbidden
error: {
  "error": "unauthorized",
  "ref": "refs/heads/main",
  "required_state": "event_id_abc123",
  "your_pubkey": "npub1alice...",
  "docs": "https://docs.gitnostr.com/errors/unauthorized"
}
```

The inline approach can return **structured JSON** with actionable information.

### 2. Performance Benefits

**With hooks:**
- Git process spawns
- Git starts receiving pack data
- Hook runs (might query Nostr relay)
- If rejected, Git throws away received data

**With inline authorization:**
- Parse ref updates from HTTP request
- Validate against Nostr state (cached)
- If rejected, return HTTP 403 immediately
- Never spawn Git for invalid pushes

**Result:** Faster rejection, less resource usage.

### 3. Easier Testing

**With hooks:**
```bash
# Test setup
mkdir -p /tmp/test-repo
cd /tmp/test-repo
git init --bare
cp pre-receive.sh hooks/pre-receive
chmod +x hooks/pre-receive

# Test execution
git push /tmp/test-repo main

# Cleanup
rm -rf /tmp/test-repo
```

**With inline authorization:**
```rust
#[tokio::test]
async fn test_unauthorized_push() {
    let relay = TestRelay::start().await;
    let result = validate_push(&state, "refs/heads/main", alice_pubkey).await;
    assert!(result.is_err());
    relay.stop().await;
}
```

**Result:** Pure Rust unit tests, no shell scripts, no Git setup.

See [`tests/push_authorization.rs`](tests/push_authorization.rs) for actual test examples.

### 4. Shared State and Types

**With hooks:**
- Hook is separate process
- Must query Nostr relay over WebSocket
- Can't share in-memory cache
- Separate error types

**With inline authorization:**
```rust
// From src/git/handlers.rs
pub async fn handle_receive_pack(
    repo_path: PathBuf,
    body: Bytes,
    database: SharedDatabase,  // Shared with Nostr relay!
    npub: &str,
    identifier: &str,
) -> Result<Response<Full<Bytes>>, GitError> {
    // Direct database access for authorization
    let auth = get_authorization_for_owner(&database, pubkey, identifier).await?;
    // ...
}
```

**Result:** Better performance, type safety, simpler architecture.

### 5. Simpler Deployment

**With hooks (ngit-relay):**
```
Docker container:
  - nginx (HTTP frontend)
  - git-http-backend (C binary)
  - pre-receive hook (Go binary) 
  - Khatru relay (Go binary)
  - supervisord (process manager)
  
Setup steps:
  1. Install all components
  2. Configure nginx
  3. Install hook in each repository
  4. Set up supervisord
  5. Configure inter-process communication
```

**With inline authorization (ngit-grasp):**
```
Single Rust binary:
  - HTTP server (Hyper)
  - Git protocol handler
  - Nostr relay (nostr-relay-builder)
  - Authorization logic
  
Setup steps:
  1. Run binary
  2. Configure environment variables
```

**Result:** Simpler deployment, fewer moving parts.

---

## Technical Implementation

### How We Parse Ref Updates

The Git HTTP protocol sends ref updates in the request body:

```
POST /alice/myrepo.git/git-receive-pack HTTP/1.1
Content-Type: application/x-git-receive-pack-request

0000000000000000000000000000000000000000 abc123... refs/heads/main\0 report-status
```

We parse this **before** spawning Git. See [`src/git/authorization.rs`](src/git/authorization.rs) for the implementation:

```rust
/// Parse ref updates from git-receive-pack request body
pub fn parse_pushed_refs(body: &[u8]) -> Result<Vec<PushedRef>, AuthorizationError> {
    // Parse pkt-line format
    // Extract ref updates
    // Return structured data
}
```

### How We Validate

Validation checks (from [`src/git/authorization.rs`](src/git/authorization.rs)):

1. Does pusher's pubkey have write access?
2. Are they listed as a maintainer in the latest state event?
3. Do the refs match the state event?

```rust
/// Validate that pushed refs match the authorized state
pub fn validate_push_refs(
    pushed_refs: &[PushedRef],
    state: &RepositoryState,
) -> Result<(), AuthorizationError> {
    for pushed_ref in pushed_refs {
        if pushed_ref.ref_name.starts_with("refs/heads/") {
            // Validate branch against state
        } else if pushed_ref.ref_name.starts_with("refs/tags/") {
            // Validate tag against state
        } else if pushed_ref.ref_name.starts_with("refs/nostr/") {
            // Allow refs/nostr/<event-id> for PRs
        }
    }
    Ok(())
}
```

---

## Comparison with Reference Implementation

| Aspect | ngit-relay (hooks) | ngit-grasp (inline) |
|--------|-------------------|---------------------|
| **Components** | nginx + git-http-backend + hook + Khatru | Single Rust binary |
| **Validation** | Pre-receive hook (separate process) | Inline HTTP handler |
| **Error messages** | Hook stderr → `remote:` | HTTP response JSON |
| **Performance** | Spawns Git first | Validates first |
| **Testing** | Shell scripts + Go tests | Pure Rust tests |
| **Deployment** | Docker + supervisord | Single binary |
| **State sharing** | WebSocket query | Direct database access |

Both are GRASP-compliant, but inline authorization is simpler and more efficient.

---

## Trade-offs and Limitations

### What We Gain
- ✅ Better error messages
- ✅ Better performance
- ✅ Easier testing
- ✅ Simpler deployment
- ✅ Tighter integration

### What We Lose
- ❌ Non-standard approach (not using Git's hook system)
- ❌ Tighter coupling to Git HTTP protocol
- ❌ Must parse protocol ourselves

### Is It Worth It?

**Yes**, because:
1. We handle protocol parsing in [`src/git/protocol.rs`](src/git/protocol.rs)
2. GRASP is already non-standard (Nostr authorization)
3. Benefits far outweigh the coupling cost
4. We can still add hook support later if needed

---

## Implementation References

Key files in the ngit-grasp implementation:

| Component | Location |
|-----------|----------|
| HTTP routing | [`src/http/mod.rs`](src/http/mod.rs) |
| Git handlers | [`src/git/handlers.rs`](src/git/handlers.rs) |
| Push authorization | [`src/git/authorization.rs`](src/git/authorization.rs) |
| Git protocol parsing | [`src/git/protocol.rs`](src/git/protocol.rs) |
| Subprocess management | [`src/git/subprocess.rs`](src/git/subprocess.rs) |
| Event acceptance policy | [`src/nostr/builder.rs:51`](src/nostr/builder.rs:51) - `Nip34WritePolicy` |

---

## Future Considerations

### If We Need Hooks Later

We can add hook support without removing inline validation:

```rust
pub struct GitConfig {
    inline_validation: bool,  // Default: true
    hook_validation: bool,    // Default: false
}
```

This would allow:
- Migration path for hook-based systems
- Extra validation for paranoid deployments
- Compatibility with other Git tools

### If Git Protocol Changes

The protocol parsing is isolated in [`src/git/protocol.rs`](src/git/protocol.rs). If the Git protocol changes:
- Update the protocol module
- Tests will catch any breakage

---

## Conclusion

**Inline authorization is the right choice for ngit-grasp** because:

1. It provides better error messages for users
2. It's more performant (early rejection)
3. It's easier to test (pure Rust)
4. It's simpler to deploy (single binary)
5. It enables better integration (shared database)

The trade-off (coupling to Git HTTP protocol) is acceptable because:
- The protocol is stable and well-specified
- Protocol handling is isolated in one module
- Benefits far outweigh the cost

This decision aligns with our goal of creating a **developer-friendly, production-ready GRASP implementation**.

---

## Related Documentation

- [Architecture Overview](architecture.md) - Full system design
- [Design Decisions](decisions.md) - All architectural choices
- [Comparison with ngit-relay](comparison.md) - Detailed comparison
- [Git Protocol Reference](../reference/git-protocol.md) - Protocol details
- [Test Strategy](../reference/test-strategy.md) - How we test this

---

*Part of the [ngit-grasp explanation docs](./)*