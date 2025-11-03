# Architecture Decision Summary

## Question: Pre-receive Hook vs. Inline Authorization?

After investigating the `git-http-backend` Rust crate and the reference implementation, we have determined that **inline authorization is both pragmatic and superior**.

## Investigation Findings

### git-http-backend Crate Analysis

The `git-http-backend` crate (v0.1.3) provides:

1. **Low-level Git protocol handling** via actix-web handlers
2. **Process spawning** of `git-receive-pack` and `git-upload-pack`
3. **Stream-based I/O** between HTTP and Git processes
4. **Flexible path rewriting** through the `GitConfig` trait

**Key Finding**: The crate spawns Git as a subprocess in `git_receive_pack.rs`. We can intercept **before** this spawn happens.

### Reference Implementation (ngit-relay) Analysis

The Go-based reference uses:

1. **nginx** as HTTP frontend
2. **git-http-backend** (C binary) for Git protocol
3. **Pre-receive hook** (Go binary) for authorization
4. **Khatru** (Go) for Nostr relay
5. **supervisord** for process management
6. **Docker** for packaging

The pre-receive hook:
- Reads ref updates from stdin
- Queries local Nostr relay via WebSocket
- Validates each ref against state events
- Exits with 0 (accept) or 1 (reject)
- Errors printed to stderr appear as `remote:` messages in git client

## Decision: Inline Authorization ✅

### Why This Is Pragmatic

1. **The crate supports it**: We can implement a custom `git_receive_pack` handler that validates before spawning Git
2. **Better error handling**: Direct HTTP responses vs. parsing hook stderr
3. **Simpler deployment**: Single binary, no hook management
4. **Easier testing**: Pure Rust unit tests, no shell scripts
5. **Performance**: Avoid spawning Git for invalid pushes
6. **Type safety**: Share types between Git and Nostr modules

### Implementation Approach

```rust
// Instead of using git-http-backend's handler as-is:
pub async fn git_receive_pack(
    req: HttpRequest,
    body: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse> {
    // 1. Parse repository path from URL
    let (npub, identifier) = parse_repo_path(&req)?;
    
    // 2. Buffer enough of the request to parse ref updates
    let ref_updates = parse_ref_updates(&body).await?;
    
    // 3. VALIDATE AGAINST NOSTR STATE
    let validator = PushValidator::new(&state.nostr_client);
    match validator.validate_push(&npub, &identifier, &ref_updates).await {
        Ok(_) => {
            // 4. Valid! Spawn git-receive-pack and stream
            spawn_git_receive_pack(req, body, state).await
        }
        Err(e) => {
            // 5. Invalid! Return HTTP error
            Ok(HttpResponse::Forbidden()
                .body(format!("Push rejected: {}", e)))
        }
    }
}
```

### Advantages Over Hooks

| Aspect | Pre-receive Hook | Inline Authorization |
|--------|------------------|---------------------|
| Error messages | Via stderr, prefixed with `remote:` | Direct HTTP response body |
| Testing | Requires Git repo setup | Pure Rust unit tests |
| Debugging | Hook logs separate from server | Unified logging |
| Deployment | Symlinks, permissions, hook scripts | Single binary |
| Performance | Always spawn Git | Skip Git for invalid pushes |
| State sharing | IPC or network | Direct memory access |
| Type safety | Separate binaries | Shared Rust types |

### Potential Concerns & Mitigations

**Concern**: "What if we need to validate the actual pack data, not just refs?"

**Mitigation**: We can still do this inline! Parse the pack stream before forwarding to Git. The `git-http-backend` crate already buffers the request body.

**Concern**: "Doesn't Git expect hooks for certain operations?"

**Mitigation**: We're not eliminating hooks entirely. Post-receive hooks might still be useful for notifications. We're just moving *authorization* out of hooks.

**Concern**: "What about compatibility with standard Git setups?"

**Mitigation**: The Git Smart HTTP protocol is standardized. Our inline validation is transparent to clients. We're still using real Git repositories and spawning real `git-receive-pack`.

## Comparison with Reference Implementation

### Reference (ngit-relay)
```
Client → nginx → git-http-backend → Git → pre-receive hook → validate → accept/reject
                                              ↓
                                    Query Nostr relay (WebSocket)
```

### Our Approach (ngit-grasp)
```
Client → actix-web → validate → Git → accept
                        ↓
                Query Nostr relay (in-process)
                        ↓
                     reject ← return HTTP error
```

## Implementation Complexity

### Hook-based (if we went that route)
- ✅ Simpler: Follow reference implementation
- ❌ More components: Hook binaries, symlinks
- ❌ More complex testing: Need Git repos, shell scripts
- ❌ More complex deployment: Hook installation, permissions

### Inline (our choice)
- ❌ More complex: Custom Git protocol handling
- ✅ Fewer components: Single binary
- ✅ Simpler testing: Pure Rust
- ✅ Simpler deployment: Just run the binary

**Verdict**: Slightly more complex initially, but much simpler long-term.

## Code Reuse from Reference

We can still reuse the **logic** from the reference implementation:

- Maintainer recursion algorithm
- State validation logic
- Event filtering policies
- Repository provisioning workflow

We're just implementing it in Rust within our HTTP handlers rather than in Git hooks.

## Conclusion

**Inline authorization is both pragmatic and superior for a Rust implementation.**

The `git-http-backend` crate provides sufficient flexibility through its handler architecture. By intercepting at the HTTP layer, we gain:

1. Better error handling and user experience
2. Simpler deployment and operations
3. Easier testing and debugging
4. Better performance characteristics
5. Tighter integration between components

The additional complexity of parsing the Git protocol is minimal compared to the benefits, and we're still using the standard Git binaries for the actual repository operations.

## Next Steps

1. ✅ Document architecture (this file + ARCHITECTURE.md)
2. ⏭️ Set up project structure with Cargo workspace
3. ⏭️ Implement core types (RefUpdate, RepositoryState, etc.)
4. ⏭️ Implement Git protocol parsing
5. ⏭️ Implement Nostr relay with policies
6. ⏭️ Implement push validation logic
7. ⏭️ Integration tests
8. ⏭️ GRASP-01 compliance testing
