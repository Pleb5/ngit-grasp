**ARCHIVED: 2025-11-04**  
**Reason:** Decided to validate grasp-audit against ngit-relay first  
**See:** docs/archive/2025-11-04-test-strategy-decision.md for rationale

---

# TDD Plan for GRASP-01 Git Backend

**Date:** 2025-11-04  
**Status:** ARCHIVED - Superseded by test-first approach  
**Goal:** Implement Git Smart HTTP backend with inline authorization using TDD

---

## Current State

### What We Have
- ✅ NIP-01 compliant Nostr relay (working, tested)
- ✅ NIP-34 event handling (announcements accepted)
- ✅ Storage layer (in-memory + disk paths configured)
- ✅ Test infrastructure (integration tests with auto relay management)
- ✅ grasp-audit compliance testing library

### What We Need
- ❌ Git Smart HTTP protocol handler
- ❌ Git repository management (init, receive-pack, upload-pack)
- ❌ Push authorization (validate against Nostr state events)
- ❌ Integration with existing Nostr relay

---

## Tool Selection Analysis

### Option 1: git2-rs (libgit2 bindings)
**Pros:**
- ✅ Pure Rust bindings to battle-tested libgit2
- ✅ Full Git functionality (init, push, pull, refs, objects)
- ✅ Thread-safe, well-maintained
- ✅ Used by cargo, widely deployed
- ✅ Can intercept and validate operations programmatically

**Cons:**
- ❌ Requires libgit2 system dependency
- ❌ Higher-level API - may be overkill for our needs
- ❌ Harder to intercept low-level protocol for inline auth

**Use Cases:**
- Repository initialization
- Reading/writing refs
- Object storage queries
- Validation of commits/trees

### Option 2: Standard git in subprocess
**Pros:**
- ✅ Uses system git (already available)
- ✅ No additional dependencies
- ✅ Battle-tested Git implementation
- ✅ Easy to spawn for upload-pack/receive-pack

**Cons:**
- ❌ Harder to intercept for inline authorization
- ❌ Must parse git protocol to validate pushes
- ❌ Subprocess overhead
- ❌ Complex error handling

**Use Cases:**
- git-upload-pack (clone, fetch)
- git-receive-pack (push) - but need to intercept

### Option 3: git-http-backend crate
**Pros:**
- ✅ Purpose-built for Git Smart HTTP
- ✅ Handles protocol parsing
- ✅ Works with system git
- ✅ Can intercept receive-pack before spawning git

**Cons:**
- ❌ Less mature (but we're already planning to use it per README)
- ❌ Still need to parse pack protocol for validation

**Use Cases:**
- HTTP endpoint handling
- Protocol negotiation
- Spawning git processes

### Option 4: Hybrid Approach (RECOMMENDED)
**Combination:**
1. **git-http-backend** - HTTP protocol handling
2. **git2-rs** - Repository management, ref validation
3. **System git** - Actual pack operations (upload-pack/receive-pack)

**Why Hybrid:**
- git-http-backend handles HTTP → Git protocol translation
- git2 for safe repository operations (init, refs, validation)
- System git for pack operations (proven, fast)
- We intercept at the HTTP layer before spawning git

**Architecture:**
```
HTTP Request → git-http-backend → Our Auth Layer → git2/system git
                                        ↓
                                  Nostr Relay
                                  (state validation)
```

---

## Recommended Approach: Hybrid

### Dependencies to Add
```toml
[dependencies]
# Git operations
git2 = "0.20"           # Repository management, refs
# git-http-backend - TBD (research if available, or implement minimal)

[dev-dependencies]
tempfile = "3.8"        # Temporary repos for testing
```

### Why This Works

1. **git2 for Repository Management:**
   - Initialize bare repos when announcements arrive
   - Read/write refs safely
   - Query repository state
   - Validate commits exist

2. **System git for Pack Operations:**
   - Spawn `git-upload-pack` for clones/fetches (read-only, safe)
   - Spawn `git-receive-pack` ONLY after auth passes
   - Leverage proven pack protocol implementation

3. **Inline Authorization:**
   - Parse HTTP request to extract ref updates
   - Query Nostr relay for latest state event
   - Validate push matches state
   - Only spawn git-receive-pack if authorized

---

## TDD Implementation Plan

### Phase 1: Repository Management (git2)
**Goal:** Create and manage bare Git repositories

**Tests:**
1. ✅ Create bare repository when announcement received
2. ✅ Initialize with proper config (bare, shared)
3. ✅ Set HEAD from state event
4. ✅ Read refs from repository
5. ✅ Write refs to repository
6. ✅ Query if commit exists in repository

**Implementation:**
```rust
// src/git/repository.rs

pub struct GitRepository {
    path: PathBuf,
    repo: git2::Repository,
}

impl GitRepository {
    pub fn init_bare(path: PathBuf) -> Result<Self>;
    pub fn set_head(ref_name: &str) -> Result<()>;
    pub fn get_ref(&self, name: &str) -> Result<Option<String>>;
    pub fn set_ref(&self, name: &str, oid: &str) -> Result<()>;
    pub fn has_commit(&self, oid: &str) -> Result<bool>;
}
```

**Test Example:**
```rust
#[test]
fn test_init_bare_repository() {
    let temp = TempDir::new().unwrap();
    let repo = GitRepository::init_bare(temp.path().to_path_buf()).unwrap();
    
    assert!(temp.path().join("HEAD").exists());
    assert!(temp.path().join("config").exists());
    assert!(temp.path().join("objects").exists());
    assert!(temp.path().join("refs").exists());
}
```

### Phase 2: Git Protocol Parsing
**Goal:** Parse Git Smart HTTP protocol for authorization

**Tests:**
1. ✅ Parse info/refs request
2. ✅ Parse upload-pack request (clone/fetch)
3. ✅ Parse receive-pack request (push)
4. ✅ Extract ref updates from receive-pack
5. ✅ Extract capabilities from request

**Implementation:**
```rust
// src/git/protocol.rs

pub struct RefUpdate {
    pub old_oid: String,
    pub new_oid: String,
    pub ref_name: String,
}

pub fn parse_receive_pack_request(body: &[u8]) -> Result<Vec<RefUpdate>>;
pub fn parse_capabilities(body: &[u8]) -> Result<Vec<String>>;
```

**Test Example:**
```rust
#[test]
fn test_parse_receive_pack_single_ref() {
    let body = b"0000000000000000000000000000000000000000 \
                 a1b2c3d4e5f6789012345678901234567890abcd \
                 refs/heads/main\0 report-status\n";
    
    let updates = parse_receive_pack_request(body).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].ref_name, "refs/heads/main");
    assert_eq!(updates[0].new_oid, "a1b2c3d4e5f6789012345678901234567890abcd");
}
```

### Phase 3: Authorization Logic
**Goal:** Validate pushes against Nostr state events

**Tests:**
1. ✅ Get maintainers from announcement
2. ✅ Get maintainers recursively
3. ✅ Handle circular maintainer references
4. ✅ Validate ref update matches state
5. ✅ Validate branch push matches state
6. ✅ Validate tag push matches state
7. ✅ Accept push to refs/nostr/*
8. ✅ Reject push not matching state
9. ✅ Reject push from non-maintainer

**Implementation:**
```rust
// src/git/authorization.rs

pub struct PushValidator {
    storage: Storage,
}

impl PushValidator {
    pub async fn validate_push(
        &self,
        npub: &str,
        identifier: &str,
        updates: &[RefUpdate],
    ) -> Result<()>;
    
    async fn get_maintainers(&self, npub: &str, identifier: &str) -> Vec<String>;
    async fn get_latest_state(&self, npub: &str, identifier: &str) -> Option<StateEvent>;
    fn validate_ref_update(&self, state: &StateEvent, update: &RefUpdate) -> Result<()>;
}
```

**Test Example:**
```rust
#[tokio::test]
async fn test_validate_matching_push() {
    let storage = test_storage().await;
    
    // Create announcement and state
    let announcement = create_announcement("alice", "repo1");
    let state = create_state("alice", "repo1")
        .branch("main", "a1b2c3d4...");
    
    storage.store_event(announcement).await.unwrap();
    storage.store_event(state).await.unwrap();
    
    // Validate matching push
    let validator = PushValidator::new(storage);
    let update = RefUpdate {
        old_oid: "0000...".into(),
        new_oid: "a1b2c3d4...".into(),
        ref_name: "refs/heads/main".into(),
    };
    
    let result = validator.validate_push("alice", "repo1", &[update]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_reject_mismatched_push() {
    let storage = test_storage().await;
    
    // State points to commit A
    let state = create_state("alice", "repo1")
        .branch("main", "aaaa1111...");
    storage.store_event(state).await.unwrap();
    
    // Try to push commit B
    let validator = PushValidator::new(storage);
    let update = RefUpdate {
        old_oid: "0000...".into(),
        new_oid: "bbbb2222...".into(),  // Different!
        ref_name: "refs/heads/main".into(),
    };
    
    let result = validator.validate_push("alice", "repo1", &[update]).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("state"));
}
```

### Phase 4: HTTP Handlers
**Goal:** Serve Git Smart HTTP protocol

**Tests:**
1. ✅ GET /npub/repo.git/info/refs?service=git-upload-pack
2. ✅ POST /npub/repo.git/git-upload-pack (clone/fetch)
3. ✅ POST /npub/repo.git/git-receive-pack (push with auth)
4. ✅ Return 403 for unauthorized push
5. ✅ Return 404 for non-existent repository
6. ✅ Set correct content-type headers
7. ✅ Include CORS headers

**Implementation:**
```rust
// src/git/handler.rs

pub async fn handle_info_refs(
    npub: String,
    identifier: String,
    service: String,
) -> Result<Response>;

pub async fn handle_upload_pack(
    npub: String,
    identifier: String,
    body: Bytes,
) -> Result<Response>;

pub async fn handle_receive_pack(
    npub: String,
    identifier: String,
    body: Bytes,
    validator: PushValidator,
) -> Result<Response>;
```

**Test Example:**
```rust
#[tokio::test]
async fn test_info_refs_returns_correct_headers() {
    let app = test_app().await;
    
    // Create repository
    app.create_repo("alice", "test-repo").await;
    
    // Request info/refs
    let response = app.get("/alice-npub/test-repo.git/info/refs?service=git-upload-pack").await;
    
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/x-git-upload-pack-advertisement"
    );
    assert_eq!(
        response.headers().get("access-control-allow-origin").unwrap(),
        "*"
    );
}

#[tokio::test]
async fn test_receive_pack_rejects_unauthorized() {
    let app = test_app().await;
    
    // Create repo with state
    let (announcement, state) = app.create_repo_with_state()
        .branch("main", "aaaa1111...")
        .build()
        .await;
    
    // Try to push different commit
    let body = create_receive_pack_request()
        .ref_update("refs/heads/main", "0000...", "bbbb2222...")
        .build();
    
    let response = app.post(
        &format!("/{}/repo.git/git-receive-pack", announcement.author_npub()),
        body
    ).await;
    
    assert_eq!(response.status(), 403);
}
```

### Phase 5: Integration with Nostr Events
**Goal:** Automatic repository creation and state updates

**Tests:**
1. ✅ Create repository when announcement received
2. ✅ Update HEAD when state event received
3. ✅ Handle announcement updates (maintainers change)
4. ✅ Clean up orphaned refs/nostr/* refs

**Implementation:**
```rust
// src/nostr/events.rs (extend existing)

async fn handle_repository_announcement(event: &Event, storage: &Storage) -> Result<()> {
    // Extract npub and identifier
    // Create bare repository
    // Store event
}

async fn handle_repository_state(event: &Event, storage: &Storage) -> Result<()> {
    // Find repository
    // Update refs to match state
    // Update HEAD
}
```

**Test Example:**
```rust
#[tokio::test]
async fn test_repository_created_on_announcement() {
    let app = test_app().await;
    
    let announcement = create_announcement("alice", "test-repo")
        .with_clone_tag(app.domain())
        .build();
    
    app.send_event(announcement).await.unwrap();
    
    // Wait for async processing
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify repository exists
    let repo_path = app.git_data_path()
        .join("alice-npub")
        .join("test-repo.git");
    
    assert!(repo_path.exists());
    assert!(repo_path.join("HEAD").exists());
}
```

### Phase 6: End-to-End Testing
**Goal:** Test with real Git client

**Tests:**
1. ✅ Clone repository with git client
2. ✅ Fetch from repository
3. ✅ Push to repository (authorized)
4. ✅ Push rejected (unauthorized)
5. ✅ Multiple concurrent operations

**Implementation:**
```rust
// tests/e2e/git_client.rs

#[tokio::test]
async fn test_real_git_clone() {
    let app = test_app().await;
    
    // Setup repository with content
    let (announcement, _) = app.create_repo_with_commits()
        .commit("Initial", "README.md", "# Test")
        .build()
        .await;
    
    // Clone with real git
    let temp = TempDir::new().unwrap();
    let url = format!(
        "http://{}/{}/{}.git",
        app.domain(),
        announcement.author_npub(),
        announcement.identifier()
    );
    
    let output = Command::new("git")
        .args(&["clone", &url])
        .current_dir(&temp)
        .output()
        .await
        .unwrap();
    
    assert!(output.status.success());
    
    let cloned_path = temp.path().join(announcement.identifier());
    assert!(cloned_path.exists());
    assert!(cloned_path.join("README.md").exists());
}
```

---

## Testing Strategy

### Unit Tests (40%)
- Git repository operations (git2)
- Protocol parsing
- Authorization logic
- Pure functions, no I/O

### Integration Tests (30%)
- HTTP handlers with test server
- Repository + Nostr event interaction
- Multi-maintainer flows
- State validation

### Compliance Tests (20%)
- GRASP-01 Git requirements
- Use grasp-audit library
- Spec-driven assertions

### E2E Tests (10%)
- Real git client operations
- End-to-end workflows
- Performance testing

---

## Implementation Order

### Week 1: Foundation
1. Add git2 dependency
2. Implement GitRepository (Phase 1)
3. Write unit tests for repository operations
4. Test repository creation from announcements

### Week 2: Protocol & Authorization
1. Implement protocol parsing (Phase 2)
2. Implement authorization logic (Phase 3)
3. Write unit tests for both
4. Integration tests for validation

### Week 3: HTTP & Integration
1. Implement HTTP handlers (Phase 4)
2. Integrate with Nostr events (Phase 5)
3. Integration tests for full flow
4. CORS and error handling

### Week 4: E2E & Polish
1. E2E tests with real git (Phase 6)
2. Performance testing
3. GRASP-01 compliance testing
4. Documentation and examples

---

## Success Criteria

### Functional
- ✅ Clone repository via HTTP
- ✅ Push authorized commits
- ✅ Reject unauthorized pushes
- ✅ Support multi-maintainer
- ✅ Support refs/nostr/* for PRs

### Quality
- ✅ >80% unit test coverage
- ✅ All integration tests pass
- ✅ GRASP-01 compliance 100%
- ✅ E2E tests with real git

### Performance
- ✅ Clone 1MB repo < 1s
- ✅ Push validation < 100ms
- ✅ 100 concurrent ops without errors

---

## Next Steps

1. **Review this plan** - Does the hybrid approach make sense?
2. **Start Phase 1** - Add git2, implement GitRepository
3. **Write first test** - test_init_bare_repository
4. **Iterate with TDD** - Red → Green → Refactor

---

## Questions for Review

1. **Hybrid approach?** git2 + system git + HTTP layer - good balance?
2. **git-http-backend crate?** Worth using or implement minimal HTTP layer?
3. **Authorization granularity?** Validate per-ref or entire push?
4. **Error messages?** How detailed for push rejections?
5. **Testing scope?** Is 6 phases reasonable for first iteration?

---

**Ready to proceed?** Let me know if this plan looks good, or if you'd like to adjust the approach!
