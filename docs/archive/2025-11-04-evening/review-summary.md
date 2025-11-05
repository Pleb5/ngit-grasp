# GRASP Protocol Review Summary

**Date:** November 4, 2025  
**Purpose:** Document key findings from reviewing GRASP protocol and ngit-relay

---

## 🎯 Critical Discoveries

### 1. Single Port Architecture (CRITICAL!)

**Finding:** Git server and Nostr relay MUST run on the SAME port.

**Evidence:**
```yaml
# ../ngit-relay/docker-compose.yml
ports:
  - "8081:8081"  # Single port only!
```

```nginx
# ../ngit-relay/src/nginx.conf
server {
    listen 8081;  # One listener for everything
    
    location ~ ^/npub1([a-z0-9]+)/([^/]+\.git)(/.*)?$ {
        # Git HTTP via fcgiwrap
    }
    
    location / {
        # Nostr relay via proxy to localhost:3334
    }
}
```

**Impact:**
- Our current architecture is WRONG
- We assumed separate ports (relay on 8080, git on 8081)
- Must use HTTP router to split traffic by path
- actix-web can handle this

**Action Required:**
- Integrate actix-web for HTTP routing
- Route `/<npub>/<id>.git` to Git handler
- Route `/` to Nostr relay (WebSocket upgrade)
- Apply CORS to ALL routes

---

### 2. GRASP-01 Test Requirements

**Finding:** Tests must closely map to GRASP protocol specification.

**GRASP-01 Requirements (from ../grasp/01.md):**

#### Nostr Relay (Lines 1-14)
- ✅ Serve NIP-01 relay at `/` (WebSocket)
- ⏳ Accept NIP-34 repository announcements (kind 30617)
- ⏳ Accept NIP-34 state announcements (kind 30618)
- ⏳ Reject announcements without service in `clone` and `relays` tags
- ⏳ Accept events that tag accepted announcements
- ✅ Serve NIP-11 relay information
- ⏳ Include `supported_grasps`, `repo_acceptance_criteria`, `curation` in NIP-11

#### Git Smart HTTP (Lines 15-31)
- ❌ Serve repos at `/<npub>/<identifier>.git`
- ❌ Accept pushes matching state announcements
- ❌ Respect recursive maintainer sets
- ❌ Set HEAD per state announcement
- ❌ Accept pushes to `refs/nostr/<event-id>` for PRs
- ❌ Include `allow-reachable-sha1-in-want` and `allow-tip-sha1-in-want`
- ❌ Serve webpage for browsers

#### CORS Support (Lines 32-40)
- ❌ `Access-Control-Allow-Origin: *` on ALL responses
- ❌ `Access-Control-Allow-Methods: GET, POST` on ALL responses
- ❌ `Access-Control-Allow-Headers: Content-Type` on ALL responses
- ❌ Respond to OPTIONS with 204 No Content

**Action Required:**
- Create test for each requirement
- Reference GRASP-01 line numbers in test comments
- Example:
  ```rust
  #[tokio::test]
  async fn test_git_http_basic() {
      // Reference: ../grasp/01.md line 15
      // MUST serve git repository via unauthenticated git smart http
      // ...
  }
  ```

---

### 3. Environment Variables

**Finding:** ngit-relay uses specific environment variables we should match.

**From ../ngit-relay/.env.example:**

```bash
# Service Configuration
NGIT_DOMAIN=example.com              # For announcement validation
NGIT_INTERNAL_RELAY_PORT_FOR_SSL_PROXY=8081  # We don't need this

# Relay Information (NIP-11)
NGIT_RELAY_NAME="..."
NGIT_RELAY_DESCRIPTION="..."
NGIT_OWNER_NPUB="..."

# Features
NGIT_PROACTIVE_SYNC_GIT=true         # GRASP-02 (future)
NGIT_PROACTIVE_SYNC_BLOSSOM=true     # Not in GRASP
NGIT_PROACTIVE_SYNC_NOSTR=true       # GRASP-02 (future)

# Blossom Settings
NGIT_BLOSSOM_MAX_FILE_SIZE_MB=100    # Not in GRASP
NGIT_BLOSSOM_MAX_CAPACITY_GB=50      # Not in GRASP

# Logging
NGIT_LOG_DIR=/var/log/ngit-relay
NGIT_LOG_LEVEL=INFO
NGIT_LOG_MAX_SIZE_MB=20
NGIT_LOG_MAX_BACKUPS=10
NGIT_LOG_MAX_AGE_DAYS=30
```

**Our Environment Variables:**

```bash
# Service Configuration
NGIT_DOMAIN=example.com              # REQUIRED - for announcement validation
NGIT_BIND_ADDRESS=127.0.0.1:8080    # REQUIRED - single port

# Relay Information (NIP-11)
NGIT_RELAY_NAME="ngit-grasp instance"
NGIT_RELAY_DESCRIPTION="Rust GRASP implementation"
NGIT_OWNER_NPUB="npub1..."

# Storage Paths
NGIT_GIT_DATA_PATH=./data/repos      # REQUIRED - where to store Git repos
NGIT_RELAY_DATA_PATH=./data/relay    # REQUIRED - where to store events

# Logging
NGIT_LOG_LEVEL=INFO
RUST_LOG=info                         # Standard Rust logging
```

**Action Required:**
- Update `.env.example` with all required fields
- Add `NGIT_GIT_DATA_PATH` to config
- Document which fields are required vs. optional

---

### 4. Repository Path Structure

**Finding:** Repository storage follows specific pattern.

**Pattern:** `{GIT_DATA_PATH}/{npub}/{identifier}.git`

**Example:**
```
./data/repos/
├── npub1abc.../
│   ├── my-project.git/
│   │   ├── HEAD
│   │   ├── config
│   │   ├── objects/
│   │   └── refs/
│   └── another-repo.git/
└── npub1xyz.../
    └── their-project.git/
```

**Action Required:**
- Create repository directory structure
- Initialize bare repositories (`git init --bare`)
- Set ownership/permissions correctly
- Clean up on repository deletion

---

### 5. NIP-11 GRASP Fields

**Finding:** NIP-11 relay information must include GRASP-specific fields.

**From ../grasp/01.md lines 11-14:**

```json
{
  "name": "ngit-grasp instance",
  "description": "Rust GRASP implementation",
  "pubkey": "...",
  "contact": "...",
  "supported_nips": [1, 11, 34],
  "supported_grasps": ["GRASP-01"],           // NEW - array of strings
  "repo_acceptance_criteria": "...",          // NEW - human readable
  "curation": "WoT-based spam prevention"     // NEW - optional
}
```

**Action Required:**
- Add `supported_grasps` field to NIP-11 response
- Add `repo_acceptance_criteria` field
- Add `curation` field (optional)
- Update NIP-11 tests to verify these fields

---

### 6. Announcement Validation

**Finding:** Relay must validate announcements list this service.

**From ../grasp/01.md lines 3-5:**

> MUST reject [git repository announcements] that do not list the service 
> in both `clone` and `relays` tags unless implementing `GRASP-05`.

**NIP-34 Repository Announcement (kind 30617):**
```json
{
  "kind": 30617,
  "tags": [
    ["d", "my-project"],           // identifier
    ["name", "My Project"],
    ["clone", "https://example.com/npub.../my-project.git"],
    ["clone", "https://github.com/user/my-project"],
    ["relays", "wss://example.com"],
    ["relays", "wss://relay.nostr.band"]
  ]
}
```

**Validation Logic:**
```rust
fn validate_announcement(event: &Event, our_domain: &str) -> Result<()> {
    // Check for clone tag with our domain
    let has_clone = event.tags.iter().any(|tag| {
        tag.kind() == TagKind::Custom("clone".into()) &&
        tag.content().map(|c| c.contains(our_domain)).unwrap_or(false)
    });
    
    // Check for relays tag with our domain
    let has_relay = event.tags.iter().any(|tag| {
        tag.kind() == TagKind::Custom("relays".into()) &&
        tag.content().map(|c| c.contains(our_domain)).unwrap_or(false)
    });
    
    if !has_clone || !has_relay {
        return Err(anyhow!("Announcement must list this service in both clone and relays tags"));
    }
    
    Ok(())
}
```

**Action Required:**
- Implement announcement validation
- Check both `clone` and `relays` tags
- Reject if service not listed
- Add tests for validation

---

### 7. State Announcement Handling

**Finding:** State announcements control repository state.

**NIP-34 Repository State (kind 30618):**
```json
{
  "kind": 30618,
  "tags": [
    ["d", "my-project"],                    // identifier
    ["refs/heads/main", "abc123..."],       // branch → commit
    ["refs/heads/develop", "def456..."],
    ["HEAD", "ref: refs/heads/main"],       // symbolic ref
    ["maintainers", "npub1...", "npub2..."] // maintainer set
  ]
}
```

**State Handling:**
1. When state announcement received:
   - Update repository HEAD if needed
   - Store state for push validation
   - Handle maintainer set

2. When push received:
   - Query latest state announcement
   - Validate pusher is in maintainer set (recursive)
   - Validate ref updates match state
   - Accept or reject push

**Action Required:**
- Parse state announcements
- Update repository HEAD
- Implement push validation
- Handle recursive maintainer sets

---

### 8. PR Ref Handling

**Finding:** Special handling for PR refs.

**From ../grasp/01.md lines 22-23:**

> MUST accept pushes via this service to `refs/nostr/<event-id>` but SHOULD 
> reject if event exists on relay listing a different tip and MAY reject based 
> on criteria such as size, SPAM prevention, etc. SHOULD delete and MAY garbage 
> collect these refs if no corresponding [git PR event] or [git PR update event], 
> with a `c` tag that matches the ref tip, is accepted by relay with 20 minutes.

**PR Ref Lifecycle:**
1. Push to `refs/nostr/<event-id>`
2. Verify PR event exists on relay
3. Verify ref tip matches PR event `c` tag
4. Accept push
5. After 20 minutes, check if PR event still exists
6. If not, delete ref and garbage collect

**Action Required:**
- Accept pushes to `refs/nostr/<event-id>`
- Validate against PR events
- Implement 20-minute timeout
- Implement garbage collection

---

### 9. Git HTTP Protocol Details

**Finding:** Must support specific Git protocol features.

**From ../grasp/01.md lines 25-26:**

> MUST include `allow-reachable-sha1-in-want` and `allow-tip-sha1-in-want` 
> in advertisement and serve available oids.

**Git Capabilities:**
```
# info/refs response must include:
allow-reachable-sha1-in-want
allow-tip-sha1-in-want
```

**Action Required:**
- Configure git-http-backend to advertise these capabilities
- Ensure Git process is configured correctly
- Test with actual Git client

---

### 10. CORS Requirements

**Finding:** CORS must be on ALL responses, not just some.

**From ../grasp/01.md lines 32-40:**

```
1. Set `Access-Control-Allow-Origin: *` on ALL responses
2. Set `Access-Control-Allow-Methods: GET, POST` on ALL responses
3. Set `Access-Control-Allow-Headers: Content-Type` on ALL responses
4. Respond to OPTIONS requests with 204 No Content
```

**Implementation:**
```rust
// In actix-web
App::new()
    .wrap(
        Cors::default()
            .allow_any_origin()
            .allowed_methods(vec!["GET", "POST"])
            .allowed_headers(vec!["Content-Type"])
            .max_age(3600)
    )
    // ... routes
```

**Action Required:**
- Add CORS middleware to actix-web
- Verify headers on all responses
- Handle OPTIONS requests
- Test with browser

---

## 📊 Compliance Status

### NIP-01 (Nostr Relay)
- ✅ WebSocket connection
- ✅ EVENT message handling
- ✅ REQ subscription
- ✅ CLOSE subscription
- ✅ Event validation
- ⏳ NIP-11 with GRASP fields

**Status:** ~80% complete

### NIP-34 (Git Announcements)
- ✅ Store announcements (kind 30617)
- ✅ Store state events (kind 30618)
- ⏳ Validate announcements list this service
- ⏳ Handle maintainer sets
- ⏳ Accept related events

**Status:** ~40% complete

### GRASP-01 (Core Requirements)
- ✅ Nostr relay at `/`
- ❌ Git HTTP at `/<npub>/<id>.git`
- ❌ Push validation
- ❌ Repository provisioning
- ❌ CORS support

**Status:** ~20% complete

---

## 🎯 Immediate Next Steps

### 1. Fix Architecture (CRITICAL)
- [ ] Add actix-web dependencies
- [ ] Create HTTP router module
- [ ] Route Git paths to Git handler
- [ ] Route `/` to Nostr relay (WebSocket)
- [ ] Apply CORS to all routes

**Estimated Time:** 2-4 hours  
**Priority:** CRITICAL  
**Blocker:** Nothing else can proceed without this

### 2. Add Git HTTP Backend
- [ ] Integrate git-http-backend crate
- [ ] Create Git request handler
- [ ] Serve from `{GIT_DATA_PATH}/{npub}/{id}.git`
- [ ] Return 404 for missing repos
- [ ] Test with `git clone`

**Estimated Time:** 2-3 hours  
**Priority:** HIGH  
**Blocker:** Requires architecture fix

### 3. Repository Provisioning
- [ ] Create repos when announcements received
- [ ] Initialize bare repositories
- [ ] Set up directory structure
- [ ] Handle repository deletion

**Estimated Time:** 1-2 hours  
**Priority:** HIGH  
**Blocker:** Requires Git HTTP backend

### 4. Update Tests
- [ ] Add GRASP-01 line number references
- [ ] Create Git HTTP tests
- [ ] Create CORS tests
- [ ] Update NIP-11 tests

**Estimated Time:** 2-3 hours  
**Priority:** MEDIUM  
**Blocker:** None (can start now)

---

## 📚 Key Files to Reference

### GRASP Protocol
- `../grasp/01.md` - **THE SPEC** - Lines 1-40
- `../grasp/README.md` - Overview
- `../grasp/02.md` - GRASP-02 (future)
- `../grasp/05.md` - GRASP-05 (future)

### Reference Implementation
- `../ngit-relay/src/nginx.conf` - **ROUTING PATTERN** - Lines 8-94
- `../ngit-relay/docker-compose.yml` - Port configuration
- `../ngit-relay/.env.example` - Environment variables
- `../ngit-relay/README.md` - Architecture overview

### Our Code
- `tests/nip01_compliance.rs` - Current test approach
- `tests/common/relay.rs` - TestRelay fixture (already correct!)
- `src/nostr/relay.rs` - Current relay implementation
- `src/config.rs` - Configuration (needs Git path)

---

## ✅ Checklist for Next Session

Before starting implementation:
- [x] Read GRASP-01 specification (../grasp/01.md)
- [x] Review ngit-relay nginx.conf routing
- [x] Understand single-port architecture
- [x] Review environment variables
- [x] Understand repository path structure

Ready to implement:
- [ ] Add actix-web dependencies to Cargo.toml
- [ ] Create src/http/mod.rs module
- [ ] Create src/http/git.rs handler
- [ ] Create src/http/nostr.rs handler
- [ ] Update src/main.rs
- [ ] Update src/config.rs
- [ ] Update .env.example
- [ ] Update tests/common/relay.rs
- [ ] Create tests/grasp01_git_http.rs

---

**Last Updated:** November 4, 2025  
**Next Review:** After actix-web integration
