**ARCHIVED: 2025-11-04**  
**Reason:** Analysis complete, crate validated  
**Outcome:** Confirmed suitable for use (with fork for authorization)

---

# git-http-backend Crate Deep Dive

**Date:** 2025-11-04  
**Status:** ✅ ARCHIVED - Analysis Complete  
**Purpose:** Validate the recommendation in `work/current_status.md` regarding git-http-backend crate

---

## Executive Summary

**Recommendation Status:** ✅ **VALIDATED WITH CAVEATS**

The `git-http-backend` crate (v0.1.3) is a **good foundation** but requires significant customization for our inline authorization needs. The hybrid approach recommended in `current_status.md` is sound, but we'll need to:

1. **Fork or vendor** the crate for customization
2. **Add interception points** for authorization
3. **Enhance error handling** for better push rejection messages
4. **Add CORS support** (missing from current implementation)

---

## Crate Overview

### Basic Info
- **Name:** `git-http-backend`
- **Version:** 0.1.3
- **Author:** lazhenyi
- **License:** MIT
- **Repository:** https://github.com/lazhenyi/git-http-backend
- **Documentation:** https://docs.rs/git-http-backend/0.1.3

### Dependencies
```toml
tokio = { version = "1", features = ["sync","macros","rt", "rt-multi-thread","net"] }
actix-web = { version = "4.9.0", features = ["default"] }
actix-files = { version = "0.6.6", features = ["actix-server"] }
futures-util = { version = "0.3.31", features = ["futures-channel"] }
flate2 = "1.0.35"           # Gzip compression
async-stream = "0.3.6"       # Streaming responses
async-trait = "0.1.83"       # Async trait support
```

**Good news:** Already uses actix-web 4.9.0 (same as we plan to use)

---

## Architecture Analysis

### Core Design

The crate provides:

1. **GitConfig Trait** - Path rewriting abstraction
2. **Actix Router** - Pre-configured routes for Git Smart HTTP
3. **Protocol Handlers** - Upload-pack, receive-pack, info/refs
4. **System Git Integration** - Spawns `git` subprocess

### URL Structure

```
/{namespace}/{repo}/info/refs?service=git-upload-pack
/{namespace}/{repo}/git-upload-pack
/{namespace}/{repo}/git-receive-pack
/{namespace}/{repo}/HEAD
/{namespace}/{repo}/objects/info/packs
/{namespace}/{repo}/objects/pack/{pack}
```

**Perfect match** for our `/{npub}/{identifier}.git/` structure!

### Request Flow

```
HTTP Request
    ↓
Actix Router → Handler Function
    ↓
GitConfig::rewrite() → Path resolution
    ↓
Spawn git subprocess (upload-pack/receive-pack)
    ↓
Stream response back to client
```

---

## Key Handlers Analysis

### 1. info/refs Handler (refs.rs)

**Purpose:** Advertise repository refs (clone/fetch discovery)

**Flow:**
1. Parse `service` query param (upload-pack or receive-pack)
2. Resolve repository path via `GitConfig::rewrite()`
3. Spawn `git upload-pack --advertise-refs --stateless-rpc .`
4. Return with proper content-type header

**Code:**
```rust
pub async fn info_refs(request: HttpRequest, service: web::Data<impl GitConfig>) -> impl Responder {
    let uri = request.uri();
    let path = uri.path().to_string().replace("/info/refs", "");
    let path = service.rewrite(path).await;
    
    // Parse service from query
    let service = query.split('=').map(|x| x.to_string()).collect::<Vec<_>>()[1].clone();
    
    // Spawn git
    let mut cmd = Command::new("git");
    cmd.arg(service_name.clone());
    cmd.arg("--stateless-rpc");
    cmd.arg("--advertise-refs");
    cmd.arg(".");
    cmd.current_dir(path);
    
    // Return response with proper headers
    resp.append_header(("Content-Type", format!("application/x-git-{}-advertisement", service_name)));
    resp.append_header(("Cache-Control", "no-cache, max-age=0, must-revalidate"));
}
```

**Good:**
- ✅ Proper content-type headers
- ✅ Cache control headers
- ✅ Git protocol version support (Git-Protocol header)

**Issues:**
- ❌ No CORS headers
- ❌ No error handling for missing repos
- ❌ Query parsing is fragile (will panic on malformed input)

### 2. git-upload-pack Handler (git_upload_pack.rs)

**Purpose:** Handle clone/fetch operations (read-only)

**Flow:**
1. Resolve repository path
2. Read request body (may be gzipped)
3. Spawn `git upload-pack --stateless-rpc .`
4. Stream response back

**Code:**
```rust
pub async fn git_upload_pack(
    request: HttpRequest,
    mut payload: Payload,
    service: web::Data<impl GitConfig>,
) -> impl Responder {
    // Resolve path
    let path = service.rewrite(path).await;
    
    // Spawn git
    let mut cmd = Command::new("git");
    cmd.arg("upload-pack");
    cmd.arg("--stateless-rpc");
    cmd.arg(".");
    cmd.current_dir(path);
    
    let mut span = cmd.spawn()?;
    let mut stdin = span.stdin.take().unwrap();
    let mut stdout = span.stdout.take().unwrap();
    
    // Read request body
    let mut bytes = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        bytes.extend_from_slice(&data);
    }
    
    // Handle gzip
    let body_data = match encoding {
        Some("gzip") => decode_gzip(bytes),
        _ => bytes.to_vec(),
    };
    
    // Write to git stdin
    stdin.write_all(&body_data)?;
    drop(stdin);
    
    // Stream response
    let body_stream = actix_web::body::BodyStream::new(async_stream::stream! {
        let mut buffer = [0; 8192];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => yield Ok(web::Bytes::copy_from_slice(&buffer[..n])),
                Err(e) => break,
            }
        }
    });
    resp.body(body_stream)
}
```

**Good:**
- ✅ Handles gzip compression
- ✅ Streams response (efficient for large repos)
- ✅ Proper content-type headers

**Issues:**
- ❌ No CORS headers
- ❌ No repository existence check
- ❌ Error handling uses eprintln! (not tracing)

**For our use:** Upload-pack is read-only, so we can use as-is (just add CORS)

### 3. git-receive-pack Handler (git_receive_pack.rs) ⚠️

**Purpose:** Handle push operations (write)

**This is the critical handler for inline authorization!**

**Current Flow:**
1. Resolve repository path
2. **Check if bare repository** (good!)
3. Read request body (may be gzipped)
4. Spawn `git receive-pack --stateless-rpc .`
5. Stream response back

**Code:**
```rust
pub async fn git_receive_pack(
    request: HttpRequest,
    mut payload: Payload,
    service: web::Data<impl GitConfig>,
) -> impl Responder {
    let path = service.rewrite(path).await;
    
    // Check repository exists
    if !path.join("HEAD").exists() || !path.join("config").exists() {
        return HttpResponse::BadRequest().body("Repository not found or invalid.");
    }

    // Check if bare
    let is_bare_repo = match std::fs::read_to_string(path.join("config")) {
        Ok(config) => config.contains("bare = true"),
        Err(_) => false,
    };
    if !is_bare_repo {
        return HttpResponse::BadRequest().body("Push operation requires a bare repository.");
    }
    
    // Spawn git receive-pack
    let mut cmd = Command::new("git");
    cmd.arg("receive-pack");
    cmd.arg("--stateless-rpc");
    cmd.arg(".");
    cmd.current_dir(&path);
    
    let mut git_process = cmd.spawn()?;
    let mut stdin = git_process.stdin.take().unwrap();
    let mut stdout = git_process.stdout.take().unwrap();
    
    // Read request body
    let mut bytes = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        bytes.extend_from_slice(&data);
    }
    
    // Decode if gzipped
    let body_data = match encoding {
        Some(encoding) if encoding.contains("gzip") => decode_gzip(bytes),
        _ => bytes.to_vec(),
    };
    
    // Write to git stdin
    stdin.write_all(&body_data)?;
    drop(stdin);
    
    // Stream response
    let body_stream = /* stream stdout */;
    resp.body(body_stream)
}
```

**Good:**
- ✅ Validates repository exists
- ✅ Validates bare repository
- ✅ Handles gzip compression
- ✅ Streams response

**Critical Issues for Our Use:**
- ❌ **No authorization hook!** Spawns git immediately
- ❌ **No way to inspect push data** before spawning git
- ❌ **No CORS headers**
- ❌ **Can't reject unauthorized pushes** with custom error

**This is where we need customization!**

---

## Customization Requirements

### 1. Authorization Interception Point

**Need to add BEFORE spawning git:**

```rust
pub async fn git_receive_pack(
    request: HttpRequest,
    mut payload: Payload,
    service: web::Data<impl GitConfig>,
    validator: web::Data<PushValidator>,  // ← ADD THIS
) -> impl Responder {
    let path = service.rewrite(path).await;
    
    // Existing checks...
    
    // Read request body
    let body_data = read_and_decode_body(&mut payload, &request).await?;
    
    // ← ADD AUTHORIZATION HERE
    let ref_updates = parse_receive_pack_request(&body_data)?;
    
    // Extract npub and identifier from path
    let (npub, identifier) = extract_repo_info(&request.uri().path())?;
    
    // Validate against Nostr state
    if let Err(e) = validator.validate_push(&npub, &identifier, &ref_updates).await {
        return HttpResponse::Forbidden()
            .json(json!({
                "error": "unauthorized",
                "message": e.to_string(),
                "ref_updates": ref_updates,
            }));
    }
    
    // Only spawn git if authorized
    let mut cmd = Command::new("git");
    // ... rest of existing code
}
```

### 2. Parse Git Protocol

**Need to add protocol parsing:**

```rust
// src/git/protocol.rs

pub struct RefUpdate {
    pub old_oid: String,
    pub new_oid: String,
    pub ref_name: String,
}

pub fn parse_receive_pack_request(body: &[u8]) -> Result<Vec<RefUpdate>> {
    // Parse git pack protocol
    // Format: <old-oid> <new-oid> <ref-name>\0<capabilities>\n
    // Example: 0000000000000000000000000000000000000000 a1b2c3d4... refs/heads/main\0 report-status\n
    
    let mut updates = Vec::new();
    let lines = body.split(|&b| b == b'\n');
    
    for line in lines {
        if line.is_empty() {
            continue;
        }
        
        // Parse pkt-line format
        // First 4 bytes are hex length
        let pkt_len = parse_pkt_len(&line[0..4])?;
        if pkt_len == 0 {
            continue; // flush packet
        }
        
        let data = &line[4..pkt_len];
        let parts: Vec<&[u8]> = data.splitn(3, |&b| b == b' ').collect();
        
        if parts.len() >= 3 {
            let old_oid = String::from_utf8_lossy(parts[0]).to_string();
            let new_oid = String::from_utf8_lossy(parts[1]).to_string();
            
            // Ref name may have capabilities after \0
            let ref_data = parts[2];
            let ref_name = if let Some(null_pos) = ref_data.iter().position(|&b| b == b'\0') {
                String::from_utf8_lossy(&ref_data[..null_pos]).to_string()
            } else {
                String::from_utf8_lossy(ref_data).to_string()
            };
            
            updates.push(RefUpdate {
                old_oid,
                new_oid,
                ref_name,
            });
        }
    }
    
    Ok(updates)
}
```

**Note:** Git pack protocol is complex. We may want to use a library for this:
- `git2` crate has protocol parsing
- Or we can implement minimal parsing for our needs

### 3. Add CORS Support

**Need to add to all handlers:**

```rust
// Add CORS middleware or headers to all responses
resp.append_header(("Access-Control-Allow-Origin", "*"));
resp.append_header(("Access-Control-Allow-Methods", "GET, POST, OPTIONS"));
resp.append_header(("Access-Control-Allow-Headers", "Content-Type, Git-Protocol"));
```

### 4. Better Error Handling

**Replace eprintln! with tracing:**

```rust
use tracing::{error, info, debug};

// Instead of:
eprintln!("Error running command: {}", e);

// Use:
error!(error = ?e, "Failed to spawn git process");
```

---

## Integration Strategy

### Option A: Fork the Crate ✅ RECOMMENDED

**Pros:**
- Full control over authorization logic
- Can add CORS, error handling, protocol parsing
- Can publish as `ngit-grasp-git-http-backend`
- Keep upstream changes visible

**Cons:**
- Need to maintain fork
- Diverges from upstream

**Implementation:**
1. Fork https://github.com/lazhenyi/git-http-backend
2. Add to our workspace as git submodule or copy
3. Modify `git_receive_pack.rs` to add authorization
4. Add protocol parsing module
5. Add CORS support
6. Improve error handling

### Option B: Vendor the Code

**Pros:**
- Complete control
- No external dependency
- Can heavily customize

**Cons:**
- Lose upstream updates
- More code to maintain

**Implementation:**
1. Copy source into `src/git/http_backend/`
2. Modify as needed
3. No external dependency

### Option C: Wrap the Crate

**Pros:**
- Keep upstream crate
- Add authorization via middleware

**Cons:**
- ❌ **Can't intercept before git spawns!**
- Would need to parse response, too late
- Complex to inject validator

**Not recommended** - can't achieve inline authorization

---

## Recommended Approach

### Use Forked git-http-backend + git2 + System Git

**Architecture:**

```
HTTP Request
    ↓
Actix Router (from forked git-http-backend)
    ↓
Custom GitConfig Implementation
    ↓
git_receive_pack Handler (MODIFIED)
    ↓
┌─────────────────────────────────┐
│ 1. Read request body            │
│ 2. Parse ref updates (protocol) │  ← ADD THIS
│ 3. Validate via PushValidator   │  ← ADD THIS
│    ├─ Query Nostr relay         │
│    ├─ Check state event         │
│    └─ Validate maintainers      │
│ 4. If authorized:               │
│    └─ Spawn git receive-pack    │  ← EXISTING
│ 5. If unauthorized:             │
│    └─ Return 403 with error     │  ← ADD THIS
└─────────────────────────────────┘
    ↓
Stream response to client
```

**Dependencies:**

```toml
[dependencies]
# Fork of git-http-backend (or vendored code)
git-http-backend = { git = "https://github.com/our-org/git-http-backend", branch = "ngit-grasp" }

# Or vendor it:
# (no dependency, code in src/git/http_backend/)

# Git operations
git2 = "0.20"  # For repository management, ref queries

# Already have:
actix-web = "4.9"
tokio = { version = "1", features = ["full"] }
nostr-sdk = "0.43"
```

**Implementation Plan:**

1. **Phase 1: Fork & Setup**
   - Fork git-http-backend
   - Add to our project (git submodule or copy)
   - Verify existing functionality works

2. **Phase 2: Protocol Parsing**
   - Add `src/git/protocol.rs`
   - Implement `parse_receive_pack_request()`
   - Unit tests for protocol parsing

3. **Phase 3: Authorization Integration**
   - Modify `git_receive_pack.rs`
   - Add `PushValidator` parameter
   - Call validator before spawning git
   - Return 403 on unauthorized

4. **Phase 4: CORS & Polish**
   - Add CORS headers to all handlers
   - Improve error messages
   - Add tracing instead of eprintln!

5. **Phase 5: Testing**
   - Unit tests for authorization
   - Integration tests with real git
   - GRASP-01 compliance tests

---

## Validation of current_status.md Recommendations

### Hybrid Approach ✅ VALIDATED

**Original recommendation:**
> 1. **git-http-backend** - HTTP protocol handling
> 2. **git2-rs** - Repository management, ref validation
> 3. **System git** - Actual pack operations (upload-pack/receive-pack)

**Analysis:**
- ✅ **git-http-backend** - Good foundation, needs customization
- ✅ **git2** - Perfect for repo management (init, refs, validation)
- ✅ **System git** - Proven pack protocol implementation

**Verdict:** Sound approach, but need to fork/vendor git-http-backend

### Tool Selection ✅ CORRECT

**Original analysis:**
- git2 for repository management ✅
- System git for pack operations ✅
- git-http-backend for HTTP layer ✅ (with modifications)

**Additional findings:**
- Need protocol parsing (can use git2 or implement minimal)
- Need CORS support (add to fork)
- Need better error handling (add to fork)

### Inline Authorization ✅ ACHIEVABLE

**Original goal:**
> We intercept the `git-receive-pack` operation before spawning the Git process

**Analysis:**
- ✅ Possible by modifying `git_receive_pack.rs`
- ✅ Can parse request body before spawning git
- ✅ Can return 403 before git touches repository

**Requirement:**
- Must fork or vendor git-http-backend
- Can't achieve with unmodified crate

---

## Updated Implementation Plan

### Week 1: Foundation (UPDATED)

1. ✅ Add git2 dependency
2. **Fork git-http-backend** (NEW)
3. **Add protocol parsing** (NEW)
4. Implement GitRepository (Phase 1)
5. Write unit tests for repository operations
6. Test repository creation from announcements

### Week 2: Protocol & Authorization

1. Implement protocol parsing (Phase 2)
2. Implement authorization logic (Phase 3)
3. **Modify git_receive_pack handler** (NEW)
4. Write unit tests for both
5. Integration tests for validation

### Week 3: HTTP & Integration

1. **Add CORS support to fork** (NEW)
2. Implement HTTP handlers (Phase 4)
3. Integrate with Nostr events (Phase 5)
4. Integration tests for full flow
5. Error handling improvements

### Week 4: E2E & Polish

1. E2E tests with real git (Phase 6)
2. Performance testing
3. GRASP-01 compliance testing
4. Documentation and examples

---

## Risks & Mitigations

### Risk 1: Fork Maintenance

**Risk:** Fork diverges from upstream, miss updates

**Mitigation:**
- Keep fork minimal (only modify git_receive_pack.rs)
- Document all changes clearly
- Consider upstreaming authorization hooks
- Monitor upstream for security fixes

### Risk 2: Protocol Parsing Complexity

**Risk:** Git pack protocol is complex, may miss edge cases

**Mitigation:**
- Use git2 for protocol parsing if available
- Implement minimal parsing (just ref updates)
- Extensive testing with real git clients
- Refer to Git protocol documentation

### Risk 3: Performance

**Risk:** Authorization adds latency to push operations

**Mitigation:**
- Keep validation logic fast (< 100ms target)
- Cache state events in memory
- Async validation (don't block)
- Profile and optimize

---

## Conclusion

### Summary

The **hybrid approach** recommended in `current_status.md` is **sound and validated**, with these adjustments:

1. **Fork or vendor git-http-backend** - Can't use unmodified crate
2. **Add protocol parsing** - Need to parse ref updates from request
3. **Modify git_receive_pack handler** - Add authorization before spawning git
4. **Add CORS support** - Missing from current implementation
5. **Improve error handling** - Better messages for push rejections

### Next Steps

1. ✅ **Review this analysis** - Confirm approach
2. **Fork git-http-backend** - Set up fork/vendor
3. **Start Phase 1** - Add git2, implement GitRepository
4. **Add protocol parsing** - Parse ref updates from pack protocol
5. **Modify receive-pack handler** - Add authorization logic

### Questions for Review

1. **Fork vs. Vendor?** Fork allows upstream tracking, vendor gives full control
2. **Protocol parsing?** Use git2 or implement minimal parser?
3. **CORS scope?** Support all origins or restrict?
4. **Error detail?** How much info to expose in 403 responses?
5. **Performance target?** Is < 100ms for auth validation reasonable?

---

**Status:** ✅ Analysis complete, ready to proceed with implementation

**Recommendation:** Fork git-http-backend, add authorization to git_receive_pack, use git2 for repo management

---

*Analysis Date: November 4, 2025*
