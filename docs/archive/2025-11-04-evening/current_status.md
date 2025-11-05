# Current Status - ngit-grasp Implementation

**Date:** November 4, 2025  
**Status:** In Development - GRASP-01 Core Requirements

---

## 🎯 Project Goal

Implement a **GRASP-01 compliant** Git relay service in Rust that:
- Serves a NIP-01 Nostr relay at `/` (WebSocket)
- Serves Git repositories via Git Smart HTTP at `/<npub>/<identifier>.git`
- **Both on the SAME PORT** (critical requirement!)
- Validates pushes against Nostr state events
- Passes all compliance tests from grasp-audit

---

## 📋 GRASP-01 Requirements (from ../grasp/01.md)

### 1. Nostr Relay Requirements

**MUST:**
- ✅ Serve NIP-01 compliant relay at `/` (WebSocket)
- ✅ Accept NIP-34 repository announcements (kind 30617)
- ✅ Accept NIP-34 state announcements (kind 30618)
- ⏳ Reject announcements that don't list this service in `clone` and `relays` tags
- ⏳ Accept events that tag accepted announcements
- ✅ Serve NIP-11 relay information document
- ⏳ Include `supported_grasps`, `repo_acceptance_criteria`, `curation` in NIP-11

**Current Implementation:**
- Basic WebSocket relay working
- Event storage and querying functional
- NIP-11 basic implementation exists
- **Missing:** Announcement validation against service URL
- **Missing:** Event acceptance policy based on announcements

### 2. Git Smart HTTP Service Requirements

**MUST:**
- ❌ Serve Git repos at `/<npub>/<identifier>.git` via unauthenticated Git Smart HTTP
- ❌ Accept pushes matching latest state announcement (respecting maintainer set)
- ❌ Set repository HEAD per state announcement
- ❌ Accept pushes to `refs/nostr/<event-id>` for PRs
- ❌ Include `allow-reachable-sha1-in-want` and `allow-tip-sha1-in-want`
- ❌ Serve webpage at repo endpoint for browsers

**Current Implementation:**
- **NOT STARTED** - Git HTTP backend not integrated
- No Git repository management
- No push validation

### 3. CORS Support Requirements

**MUST:**
- ❌ Set `Access-Control-Allow-Origin: *` on ALL responses
- ❌ Set `Access-Control-Allow-Methods: GET, POST` on ALL responses
- ❌ Set `Access-Control-Allow-Headers: Content-Type` on ALL responses
- ❌ Respond to OPTIONS requests with 204 No Content

**Current Implementation:**
- **NOT STARTED** - No CORS headers

---

## 🏗️ Architecture Understanding (from ngit-relay)

### Critical Architecture Insight: SINGLE PORT

From `../ngit-relay/docker-compose.yml`:
```yaml
ports:
  - "8081:8081"  # Single port for EVERYTHING
```

From `../ngit-relay/src/nginx.conf`:
```nginx
server {
    listen 8081;  # Single listener
    
    # Git repos at /<npub>/<identifier>.git
    location ~ ^/npub1([a-z0-9]+)/([^/]+\.git)(/.*)?$ {
        # ... git-http-backend via fcgiwrap
    }
    
    # Nostr relay at /
    location / {
        # ... proxy to khatru on localhost:3334
    }
}
```

**Key Points:**
1. **nginx listens on ONE port (8081)**
2. **nginx routes by URL path:**
   - `/<npub>/<identifier>.git/*` → git-http-backend (fcgiwrap)
   - Everything else → Khatru relay (localhost:3334)
3. **Khatru relay runs on INTERNAL port 3334**
4. **git-http-backend runs via fcgiwrap socket**

### Our Rust Implementation Strategy

We need to replicate nginx's routing in Rust:

```
HTTP/WebSocket Request on port 8080
         ↓
    actix-web router
         ↓
    ┌────┴────┐
    ↓         ↓
Git Path   Other Path
/<npub>/   /
<id>.git   
    ↓         ↓
git-http   Nostr Relay
backend    (WebSocket upgrade)
handler    
```

**Implementation Options:**

**Option A: actix-web (HTTP framework)**
- Handle HTTP/WebSocket on same port
- Route by path pattern
- Use `git-http-backend` crate for Git protocol
- Native WebSocket support for Nostr relay

**Option B: Direct TCP + Manual Routing**
- Accept TCP connections
- Parse HTTP headers to determine route
- More complex but more control

**Recommendation: Option A (actix-web)**
- Well-tested HTTP/WebSocket handling
- Easy routing by path
- Good async performance
- Already in our dependencies

---

## 🧪 Test Strategy

### Current Test Structure

```
tests/
├── common/
│   ├── mod.rs           # Test utilities
│   └── relay.rs         # TestRelay fixture
├── nip01_compliance.rs  # NIP-01 smoke tests
└── nip34_announcements.rs  # NIP-34 tests (TODO)
```

### Test Approach

**Integration Tests (tests/*):**
- Use `TestRelay` fixture to start/stop relay
- Use `grasp-audit` library to run compliance tests
- Tests reference GRASP protocol line numbers
- Automatic relay lifecycle management

**Example Test Structure:**
```rust
#[tokio::test]
async fn test_grasp01_git_http_basic() {
    // Reference: ../grasp/01.md lines 15-17
    // Requirement: MUST serve git repository via unauthenticated git smart http
    
    let relay = TestRelay::start().await;
    let config = AuditConfig::ci();
    let client = AuditClient::new(relay.url(), config).await.unwrap();
    
    // Run GRASP-01 git HTTP tests
    let results = specs::Grasp01GitHttp::run_all(&client).await;
    
    relay.stop().await;
    assert!(results.all_passed());
}
```

### Test Coverage Needed

**NIP-01 (Nostr Relay):**
- ✅ WebSocket connection
- ✅ Send/receive events
- ✅ Subscriptions (REQ/CLOSE)
- ✅ Event validation (signatures, IDs)
- ⏳ NIP-11 relay info document

**NIP-34 (Git Announcements):**
- ⏳ Accept valid repository announcements (kind 30617)
- ⏳ Accept valid state announcements (kind 30618)
- ⏳ Reject announcements without service in clone/relays
- ⏳ Validate maintainer sets
- ⏳ Handle related events (issues, patches)

**GRASP-01 (Git HTTP):**
- ❌ Serve Git repo at `/<npub>/<id>.git`
- ❌ Clone repository via HTTP
- ❌ Push matching state announcement
- ❌ Reject push not matching state
- ❌ Handle `refs/nostr/<event-id>` for PRs
- ❌ CORS headers on all responses
- ❌ OPTIONS request handling

---

## 📝 Implementation Plan

### Phase 1: Fix Current Relay (In Progress)

**Goal:** Make NIP-01 relay fully compliant

**Tasks:**
- [x] Basic WebSocket relay working
- [x] Event storage and querying
- [ ] NIP-11 relay info with GRASP fields
  - [ ] Add `supported_grasps: ["GRASP-01"]`
  - [ ] Add `repo_acceptance_criteria`
  - [ ] Add `curation` policy
- [ ] Announcement validation
  - [ ] Check `clone` tag includes our domain
  - [ ] Check `relays` tag includes our domain
  - [ ] Reject if not listed (unless GRASP-05)
- [ ] Event acceptance policy
  - [ ] Accept events tagging accepted announcements
  - [ ] Accept events tagged by accepted announcements

**Test Coverage:**
- [x] NIP-01 smoke tests passing
- [ ] NIP-11 compliance tests
- [ ] NIP-34 announcement tests

### Phase 2: Add Git HTTP Backend (Next)

**Goal:** Serve Git repositories via HTTP on same port as relay

**Tasks:**
1. **Integrate actix-web**
   - [ ] Replace raw WebSocket with actix-web
   - [ ] Add HTTP routing
   - [ ] Preserve WebSocket upgrade for `/`
   - [ ] Add Git HTTP route for `/<npub>/<id>.git`

2. **Integrate git-http-backend crate**
   - [ ] Add dependency on `git-http-backend`
   - [ ] Create Git handler for `/<npub>/<id>.git`
   - [ ] Serve `git-upload-pack` (clone/fetch)
   - [ ] Serve `git-receive-pack` (push)

3. **Repository Management**
   - [ ] Auto-provision repos from announcements
   - [ ] Store repos at `{GIT_DATA_PATH}/<npub>/<id>.git`
   - [ ] Initialize bare repositories
   - [ ] Set HEAD from state announcements

4. **CORS Support**
   - [ ] Add CORS middleware to actix-web
   - [ ] Set required headers on all responses
   - [ ] Handle OPTIONS requests

**Test Coverage:**
- [ ] Can clone repository via HTTP
- [ ] Can fetch from repository
- [ ] Repository provisioned from announcement
- [ ] HEAD set correctly from state
- [ ] CORS headers present
- [ ] OPTIONS requests handled

### Phase 3: Push Authorization (Final)

**Goal:** Validate pushes against Nostr state announcements

**Tasks:**
1. **Inline Authorization**
   - [ ] Intercept `git-receive-pack` before Git process
   - [ ] Parse ref updates from request
   - [ ] Query latest state announcement from relay
   - [ ] Validate push matches state
   - [ ] Handle maintainer sets (recursive)
   - [ ] Return HTTP error if validation fails

2. **PR Support**
   - [ ] Accept pushes to `refs/nostr/<event-id>`
   - [ ] Validate PR event exists on relay
   - [ ] Validate ref tip matches PR event `c` tag
   - [ ] Implement 20-minute timeout for PR refs
   - [ ] Garbage collect orphaned PR refs

3. **State Synchronization**
   - [ ] Update HEAD when state announcement received
   - [ ] Handle state updates for existing repos
   - [ ] Handle multi-maintainer scenarios

**Test Coverage:**
- [ ] Push matching state succeeds
- [ ] Push not matching state fails
- [ ] Multi-maintainer push validation
- [ ] PR ref push/validation
- [ ] PR ref garbage collection
- [ ] State update triggers HEAD change

---

## 🐛 Known Issues

### 1. Architecture Mismatch
**Issue:** Tests assume relay on one port, Git on another  
**Fix:** Both must be on same port (like ngit-relay)  
**Impact:** Need to refactor server architecture

### 2. Missing Git Implementation
**Issue:** No Git HTTP backend integrated  
**Fix:** Add actix-web + git-http-backend  
**Impact:** Core GRASP-01 requirement not met

### 3. No Announcement Validation
**Issue:** Relay accepts all announcements  
**Fix:** Validate `clone` and `relays` tags  
**Impact:** Not GRASP-01 compliant

### 4. No CORS Support
**Issue:** No CORS headers on responses  
**Fix:** Add CORS middleware  
**Impact:** Web clients can't access relay

---

## 🔧 Environment Configuration

From `../ngit-relay/.env.example`, we need:

```bash
# Service Configuration
NGIT_DOMAIN=example.com              # Used for announcement validation
NGIT_BIND_ADDRESS=127.0.0.1:8080    # Single port for HTTP/WS/Git

# Relay Information (NIP-11)
NGIT_RELAY_NAME="ngit-grasp instance"
NGIT_RELAY_DESCRIPTION="Rust GRASP implementation"
NGIT_OWNER_NPUB="npub1..."           # Relay owner

# Storage Paths
NGIT_GIT_DATA_PATH=/srv/ngit-grasp/repos      # Git repositories
NGIT_RELAY_DATA_PATH=/srv/ngit-grasp/relay-db # Nostr events

# Features (Future)
NGIT_PROACTIVE_SYNC_GIT=false        # GRASP-02
NGIT_PROACTIVE_SYNC_NOSTR=false      # GRASP-02

# Logging
NGIT_LOG_LEVEL=INFO
```

**Current .env.example status:**
- ⏳ Needs update with all required fields
- ⏳ Add GRASP-specific configuration
- ⏳ Document which fields are used where

---

## 📊 Progress Summary

### Completed ✅
- Basic Nostr relay (WebSocket)
- Event storage and querying
- NIP-01 smoke tests
- Test infrastructure (TestRelay fixture)
- Integration with grasp-audit library

### In Progress ⏳
- NIP-11 relay information
- NIP-34 announcement handling
- Event acceptance policies

### Not Started ❌
- Git HTTP backend
- Repository provisioning
- Push authorization
- CORS support
- actix-web integration

### Compliance Status
- **NIP-01:** ~60% (basic relay works, missing some features)
- **NIP-34:** ~20% (can store events, no validation)
- **GRASP-01:** ~30% (relay works, Git HTTP not started)

---

## 🎯 Next Session Priorities

1. **Fix Architecture** (CRITICAL)
   - Integrate actix-web for HTTP/WebSocket routing
   - Single port for all services
   - Preserve existing relay functionality

2. **Add Git HTTP** (HIGH)
   - Integrate `git-http-backend` crate
   - Basic clone/fetch support
   - Repository provisioning from announcements

3. **Update Tests** (HIGH)
   - Add GRASP-01 Git HTTP tests
   - Reference protocol line numbers
   - Verify single-port architecture

4. **Fix NIP-11** (MEDIUM)
   - Add GRASP-specific fields
   - Document compliance level
   - Include in tests

---

## 📚 Key References

**GRASP Protocol:**
- `../grasp/README.md` - Overview
- `../grasp/01.md` - GRASP-01 Core Requirements (THE SPEC)
- `../grasp/02.md` - GRASP-02 Proactive Sync
- `../grasp/05.md` - GRASP-05 Archive

**Reference Implementation:**
- `../ngit-relay/README.md` - Architecture overview
- `../ngit-relay/src/nginx.conf` - **CRITICAL: Shows single-port routing**
- `../ngit-relay/docker-compose.yml` - **CRITICAL: Shows port config**
- `../ngit-relay/.env.example` - Configuration template

**Nostr Specs:**
- [NIP-01](https://nips.nostr.com/1) - Basic protocol
- [NIP-11](https://nips.nostr.com/11) - Relay information
- [NIP-34](https://nips.nostr.com/34) - Git stuff

**Our Code:**
- `tests/nip01_compliance.rs` - Current test approach
- `tests/common/relay.rs` - TestRelay fixture
- `grasp-audit/src/specs/nip01_smoke.rs` - Test specs

---

**Last Updated:** November 4, 2025  
**Next Review:** After actix-web integration
