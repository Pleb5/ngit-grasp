# Implementation Checklist

**Date:** November 4, 2025  
**Purpose:** Step-by-step checklist for actix-web integration

---

## ✅ Pre-Implementation (DONE)

- [x] Review GRASP-01 specification
- [x] Review ngit-relay reference implementation
- [x] Understand single-port architecture
- [x] Document architecture in work/architecture-diagram.md
- [x] Create detailed plan in work/NEXT_SESSION_START_HERE.md
- [x] Update work/current_status.md

---

## 📦 Phase 1: Dependencies & Setup

### 1.1 Update Cargo.toml

- [ ] Add `actix-web = "4"`
- [ ] Add `actix-cors = "0.7"`
- [ ] Add `actix-ws = "0.3"` (or use actix-web-actors)
- [ ] Add `git-http-backend = "0.2"` (check latest version)
- [ ] Run `cargo check` to verify dependencies

**Verification:**
```bash
cargo tree | grep actix
cargo tree | grep git-http-backend
```

### 1.2 Update .env.example (if needed)

- [x] Already has all required fields
- [x] NGIT_DOMAIN
- [x] NGIT_BIND_ADDRESS
- [x] NGIT_GIT_DATA_PATH
- [x] NGIT_RELAY_DATA_PATH

**Verification:**
```bash
cat .env.example
```

---

## 🏗️ Phase 2: HTTP Server Module

### 2.1 Create src/http/mod.rs

- [ ] Create module structure
- [ ] Add `pub mod git;`
- [ ] Add `pub mod nostr;`
- [ ] Create `run_server()` function
- [ ] Set up actix-web HttpServer
- [ ] Add CORS middleware
- [ ] Add routing for Git and Nostr

**Verification:**
```bash
cargo check
# Should compile without errors
```

**Test:**
```rust
// In src/http/mod.rs
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_module_exists() {
        // Just verify module structure
        assert!(true);
    }
}
```

### 2.2 Create src/http/git.rs

- [ ] Create `handle_git_request()` function
- [ ] Parse npub and repo from path
- [ ] Construct repository path
- [ ] Check if repository exists (return 404 if not)
- [ ] Use git-http-backend crate
- [ ] Handle GET (clone/fetch)
- [ ] Handle POST (push)
- [ ] Return proper HTTP responses

**Verification:**
```bash
cargo check
# Should compile
```

**Test:**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_repo_path() {
        // Test path parsing logic
        let path = "/npub1abc.../my-repo.git";
        // ... verify parsing
    }
}
```

### 2.3 Create src/http/nostr.rs

- [ ] Create `handle_websocket()` function
- [ ] Handle WebSocket upgrade
- [ ] Reuse existing Nostr message handling
- [ ] Create `handle_http_root()` function
- [ ] Serve HTML for browsers
- [ ] Serve NIP-11 JSON for Accept: application/nostr+json

**Verification:**
```bash
cargo check
```

**Test:**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_nip11_response() {
        // Test NIP-11 JSON generation
        // ...
    }
}
```

---

## 🔧 Phase 3: Update Existing Code

### 3.1 Update src/config.rs

- [x] Already has `git_data_path` field
- [x] Already has `from_env()` implementation
- [ ] Verify all fields are present
- [ ] Add any missing validation

**Verification:**
```bash
cargo check
```

### 3.2 Update src/main.rs

- [ ] Remove direct relay start
- [ ] Import `http` module
- [ ] Call `http::run_server(config, storage).await`
- [ ] Update logging messages

**Verification:**
```bash
cargo build
# Should build successfully
```

**Test:**
```bash
# Run server
NGIT_DOMAIN=localhost:8080 \
NGIT_BIND_ADDRESS=127.0.0.1:8080 \
cargo run

# In another terminal, check it's listening
curl -v http://localhost:8080/
```

### 3.3 Move Relay Logic to Library

- [ ] Extract relay logic from src/nostr/relay.rs
- [ ] Make it reusable by WebSocket handler
- [ ] Keep message handling separate from transport
- [ ] Create `handle_nostr_message()` function

**Structure:**
```rust
// src/nostr/relay.rs
pub async fn handle_nostr_message(
    message: &str,
    storage: &Storage,
) -> Result<Vec<String>> {
    // Parse message
    // Handle EVENT, REQ, CLOSE
    // Return response messages
}
```

**Verification:**
```bash
cargo check
cargo test --lib
```

---

## 🧪 Phase 4: Update Tests

### 4.1 Update tests/common/relay.rs

- [ ] Verify NGIT_DOMAIN is set correctly
- [ ] Add NGIT_GIT_DATA_PATH env var
- [ ] Add NGIT_RELAY_DATA_PATH env var
- [ ] Use test-specific directories
- [ ] Clean up test data after tests

**Current Status:** Already sets NGIT_DOMAIN correctly!

**Add:**
```rust
.env("NGIT_GIT_DATA_PATH", "./test-data/repos")
.env("NGIT_RELAY_DATA_PATH", "./test-data/relay")
```

**Verification:**
```bash
cargo test --test nip01_compliance
# Should still pass
```

### 4.2 Create tests/grasp01_git_http.rs

- [ ] Create new test file
- [ ] Add basic Git clone test
- [ ] Add CORS headers test
- [ ] Add OPTIONS request test
- [ ] Add repository not found test
- [ ] Reference GRASP-01 line numbers in comments

**Template:**
```rust
//! GRASP-01 Git HTTP Integration Tests
//!
//! Reference: ../grasp/01.md lines 15-40

mod common;

use common::TestRelay;

#[tokio::test]
async fn test_git_http_basic() {
    // Reference: ../grasp/01.md line 15
    // MUST serve git repository via unauthenticated git smart http
    
    let relay = TestRelay::start().await;
    
    // TODO: Create test repo
    // TODO: Try to clone it
    
    relay.stop().await;
}
```

**Verification:**
```bash
cargo test --test grasp01_git_http
```

### 4.3 Update tests/nip01_compliance.rs

- [ ] Verify tests still pass with new architecture
- [ ] Update any broken tests
- [ ] Add comments referencing GRASP-01 where relevant

**Verification:**
```bash
cargo test --test nip01_compliance
```

---

## 🔍 Phase 5: Integration & Testing

### 5.1 Manual Testing

**Test 1: Server Starts**
```bash
cargo build
NGIT_DOMAIN=localhost:8080 \
NGIT_BIND_ADDRESS=127.0.0.1:8080 \
cargo run
```

**Expected:** Server starts without errors

---

**Test 2: WebSocket Connection**
```bash
# In grasp-audit directory
cargo run -- --url ws://localhost:8080
```

**Expected:** NIP-01 smoke tests pass

---

**Test 3: HTTP Root**
```bash
curl -v http://localhost:8080/
```

**Expected:**
- Status: 200 OK
- Content-Type: text/html
- CORS headers present
- HTML content

---

**Test 4: NIP-11**
```bash
curl -v http://localhost:8080/ \
  -H "Accept: application/nostr+json"
```

**Expected:**
- Status: 200 OK
- Content-Type: application/json
- CORS headers present
- JSON with `supported_grasps` field

---

**Test 5: Git Repository (404)**
```bash
curl -v http://localhost:8080/npub1test/test-repo.git/info/refs?service=git-upload-pack
```

**Expected:**
- Status: 404 Not Found
- CORS headers present

---

**Test 6: Git Repository (Success)**
```bash
# Create test repo
mkdir -p ./data/repos/npub1test
cd ./data/repos/npub1test
git init --bare test-repo.git

# Try to access it
curl -v http://localhost:8080/npub1test/test-repo.git/info/refs?service=git-upload-pack
```

**Expected:**
- Status: 200 OK
- Content-Type: application/x-git-upload-pack-advertisement
- CORS headers present
- Git protocol data

---

**Test 7: Git Clone**
```bash
git clone http://localhost:8080/npub1test/test-repo.git /tmp/test-clone
```

**Expected:**
- Clone succeeds (even if empty repo)
- No errors

---

**Test 8: CORS Preflight**
```bash
curl -v -X OPTIONS http://localhost:8080/ \
  -H "Origin: https://example.com" \
  -H "Access-Control-Request-Method: POST"
```

**Expected:**
- Status: 204 No Content
- Access-Control-Allow-Origin: *
- Access-Control-Allow-Methods: GET, POST
- Access-Control-Allow-Headers: Content-Type
- Access-Control-Max-Age: 3600

---

### 5.2 Automated Testing

**Run All Tests:**
```bash
# Build first
cargo build

# Run all tests
cargo test

# Run specific test suites
cargo test --test nip01_compliance
cargo test --test grasp01_git_http

# With output
cargo test -- --nocapture
```

**Expected:** All tests pass

---

### 5.3 Performance Testing

**Test Concurrent Connections:**
```bash
# Start server
cargo run &

# Run multiple clients
for i in {1..10}; do
  (cd grasp-audit && cargo run -- --url ws://localhost:8080) &
done

# Wait for all to complete
wait
```

**Expected:** All clients connect and pass tests

---

### 5.4 Error Handling Testing

**Test 1: Invalid Repository Path**
```bash
curl -v http://localhost:8080/invalid/path
```

**Expected:** 404 or appropriate error

---

**Test 2: Invalid WebSocket Message**
```bash
# Use websocat or similar
echo "invalid json" | websocat ws://localhost:8080/
```

**Expected:** NOTICE message with error

---

**Test 3: Large Git Push**
```bash
# Create repo with large files
# Try to push
# Verify it works or fails gracefully
```

---

## 📋 Acceptance Criteria

### Must Have (MVP)

- [ ] Server starts on single port
- [ ] WebSocket connects at `/`
- [ ] NIP-01 smoke tests pass
- [ ] Can access Git repo at `/<npub>/<id>.git`
- [ ] Returns 404 for missing repos
- [ ] CORS headers on all responses
- [ ] OPTIONS requests return 204
- [ ] Can clone existing Git repository
- [ ] All integration tests pass

### Should Have (Before Production)

- [ ] Can push to repository (basic, no auth yet)
- [ ] Repository provisioned from announcement
- [ ] NIP-11 includes GRASP fields
- [ ] Proper error messages
- [ ] Logging works correctly
- [ ] Clean shutdown
- [ ] Test data cleanup

### Could Have (Future)

- [ ] Push authorization
- [ ] Maintainer set validation
- [ ] PR ref support
- [ ] State synchronization
- [ ] Proactive sync (GRASP-02)

---

## 🐛 Known Issues to Watch For

### Issue 1: WebSocket Upgrade Timing

**Symptom:** WebSocket upgrade fails intermittently

**Debug:**
```bash
RUST_LOG=debug cargo run
# Check for upgrade-related logs
```

**Solution:** Ensure actix-ws is configured correctly

---

### Issue 2: Git HTTP Protocol Errors

**Symptom:** Git clone fails with protocol error

**Debug:**
```bash
GIT_TRACE_PACKET=1 git clone http://localhost:8080/...
# Shows Git protocol messages
```

**Solution:** Check git-http-backend configuration

---

### Issue 3: CORS Not Applied

**Symptom:** Browser shows CORS error

**Debug:**
```bash
curl -v http://localhost:8080/ -H "Origin: https://example.com"
# Check response headers
```

**Solution:** Verify CORS middleware is first in chain

---

### Issue 4: Port Already in Use

**Symptom:** "Address already in use" error

**Debug:**
```bash
lsof -i :8080
# Find process using port
```

**Solution:**
```bash
kill -9 <PID>
# Or use different port
```

---

### Issue 5: Test Relay Won't Start

**Symptom:** Integration tests fail to start relay

**Debug:**
```bash
# Run test with output
cargo test --test nip01_compliance -- --nocapture

# Check binary exists
ls -la target/debug/ngit-grasp
```

**Solution:** Run `cargo build` before tests

---

## 📚 Reference Commands

### Development

```bash
# Build
cargo build

# Run
cargo run

# Run with logging
RUST_LOG=debug cargo run

# Check without building
cargo check

# Format code
cargo fmt

# Lint
cargo clippy
```

### Testing

```bash
# All tests
cargo test

# Specific test file
cargo test --test nip01_compliance

# Specific test
cargo test --test nip01_compliance test_nip01_smoke

# With output
cargo test -- --nocapture

# With logging
RUST_LOG=debug cargo test -- --nocapture
```

### Debugging

```bash
# Check dependencies
cargo tree

# Check for unused dependencies
cargo +nightly udeps

# Check for outdated dependencies
cargo outdated

# Audit for security issues
cargo audit
```

### Git Testing

```bash
# Create test repo
mkdir -p ./data/repos/npub1test
cd ./data/repos/npub1test
git init --bare test-repo.git

# Clone it
git clone http://localhost:8080/npub1test/test-repo.git /tmp/test

# Push to it
cd /tmp/test
echo "test" > README.md
git add .
git commit -m "test"
git push origin main
```

---

## ✅ Completion Checklist

When all items are checked, Phase 1 (actix-web integration) is complete:

### Code

- [ ] Dependencies added to Cargo.toml
- [ ] src/http/mod.rs created
- [ ] src/http/git.rs created
- [ ] src/http/nostr.rs created
- [ ] src/main.rs updated
- [ ] src/config.rs verified
- [ ] Relay logic refactored

### Tests

- [ ] tests/common/relay.rs updated
- [ ] tests/grasp01_git_http.rs created
- [ ] tests/nip01_compliance.rs still passes
- [ ] All tests pass

### Manual Testing

- [ ] Server starts successfully
- [ ] WebSocket connects
- [ ] NIP-01 smoke tests pass
- [ ] Can access Git repos
- [ ] 404 for missing repos
- [ ] CORS headers present
- [ ] OPTIONS requests work
- [ ] Can clone repository

### Documentation

- [ ] Update README.md status
- [ ] Update work/current_status.md
- [ ] Document any issues found
- [ ] Update NEXT_SESSION_START_HERE.md for next phase

---

## 🎯 Next Phase Preview

After actix-web integration is complete, next phase will be:

**Phase 2: Repository Provisioning**

- Listen for NIP-34 repository announcements
- Create Git repositories automatically
- Initialize bare repositories
- Set up directory structure
- Handle repository deletion

**Estimated Time:** 2-3 hours  
**Prerequisites:** Phase 1 complete

---

**Last Updated:** November 4, 2025  
**Status:** Ready to begin Phase 1
