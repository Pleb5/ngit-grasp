**ARCHIVED: 2025-11-04**  
**Reason:** Analysis complete, validated hybrid approach  
**Outcome:** Will use git-http-backend (forked) + git2 + system git

---

# Analysis Summary: git-http-backend Validation

**Date:** 2025-11-04  
**Status:** ✅ ARCHIVED - Analysis Complete

---

## TL;DR

✅ **VALIDATED:** The hybrid approach in `current_status.md` is sound  
⚠️ **CAVEAT:** Must fork/vendor `git-http-backend` crate for inline authorization  
✅ **READY:** Can proceed with implementation

---

## Key Findings

### 1. git-http-backend Crate (v0.1.3)

**What it provides:**
- ✅ Actix-web based Git Smart HTTP handlers
- ✅ Upload-pack (clone/fetch) - works as-is
- ✅ Receive-pack (push) - **needs modification**
- ✅ Info/refs advertisement
- ✅ Gzip compression support
- ✅ Streaming responses

**What it lacks:**
- ❌ Authorization hooks (spawns git immediately)
- ❌ CORS headers (needed for web clients)
- ❌ Protocol parsing (can't inspect push data)
- ❌ Proper error handling (uses eprintln!)

### 2. Critical Handler: git_receive_pack

**Current flow:**
```
Request → Validate bare repo → Spawn git → Stream response
```

**What we need:**
```
Request → Validate bare repo → Parse ref updates → Validate against Nostr state → Spawn git (if authorized) → Stream response
                                      ↑
                                   ADD THIS
```

**Can't achieve with unmodified crate!**

### 3. Recommended Solution

**Fork the crate and modify `git_receive_pack.rs`:**

```rust
pub async fn git_receive_pack(
    request: HttpRequest,
    mut payload: Payload,
    service: web::Data<impl GitConfig>,
    validator: web::Data<PushValidator>,  // ← ADD
) -> impl Responder {
    // ... existing path resolution and bare check ...
    
    // Read request body
    let body_data = read_and_decode_body(&mut payload, &request).await?;
    
    // ← ADD: Parse ref updates
    let ref_updates = parse_receive_pack_request(&body_data)?;
    
    // ← ADD: Validate authorization
    let (npub, identifier) = extract_repo_info(&request.uri().path())?;
    if let Err(e) = validator.validate_push(&npub, &identifier, &ref_updates).await {
        return HttpResponse::Forbidden()
            .json(json!({
                "error": "unauthorized",
                "message": e.to_string(),
            }));
    }
    
    // Only spawn git if authorized
    let mut cmd = Command::new("git");
    cmd.arg("receive-pack");
    // ... rest of existing code ...
}
```

---

## Updated Implementation Plan

### Phase 0: Setup (NEW)
1. Fork git-http-backend repository
2. Add as git submodule or vendor code
3. Verify existing functionality works
4. Add to Cargo.toml

### Phase 1: Foundation
1. Add git2 dependency
2. Implement GitRepository (repo management)
3. Add protocol parsing module
4. Unit tests for both

### Phase 2: Authorization
1. Modify git_receive_pack handler
2. Implement PushValidator
3. Integration tests for validation
4. Test unauthorized rejection

### Phase 3: Polish
1. Add CORS headers to all handlers
2. Improve error messages
3. Add tracing instead of eprintln!
4. E2E tests with real git

---

## Dependencies

```toml
[dependencies]
# Forked git-http-backend with authorization support
git-http-backend = { git = "https://github.com/our-org/git-http-backend", branch = "ngit-grasp" }

# Git repository management
git2 = "0.20"

# Already have:
actix-web = "4.9"
tokio = { version = "1", features = ["full"] }
nostr-sdk = "0.43"
```

---

## Validation of current_status.md

### ✅ Hybrid Approach - CONFIRMED
- git-http-backend for HTTP layer ✅ (with fork)
- git2 for repository management ✅
- System git for pack operations ✅

### ✅ Inline Authorization - ACHIEVABLE
- Can intercept before spawning git ✅
- Can parse ref updates ✅
- Can validate against Nostr state ✅
- Can return 403 with error message ✅

### ⚠️ Additional Requirements
- Must fork/vendor git-http-backend
- Must implement protocol parsing
- Must add CORS support
- Must improve error handling

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Fork maintenance | Medium | Keep changes minimal, document well |
| Protocol parsing complexity | Medium | Use git2 or implement minimal parser |
| Performance overhead | Low | Keep validation fast (< 100ms), cache state |
| Missing edge cases | Medium | Extensive testing with real git clients |

---

## Next Steps

1. **Decision:** Fork vs. vendor git-http-backend?
   - Fork: Keep upstream tracking, easier updates
   - Vendor: Full control, no external dependency
   - **Recommendation:** Fork (easier to contribute back)

2. **Start Phase 0:** Set up fork
   - Fork https://github.com/lazhenyi/git-http-backend
   - Create branch `ngit-grasp`
   - Add as git submodule

3. **Start Phase 1:** Add git2, implement GitRepository
   - Write tests first (TDD)
   - Focus on bare repo creation, ref management

4. **Add protocol parsing:** Parse ref updates from pack protocol
   - Research: Can git2 help?
   - Or implement minimal parser
   - Unit tests for various push scenarios

5. **Modify receive-pack:** Add authorization logic
   - Integration tests for validation
   - Test rejection scenarios

---

## Questions for Review

1. **Fork vs. Vendor?** 
   - Recommendation: Fork (can contribute back, easier updates)

2. **Protocol parsing?**
   - Option A: Use git2 if it provides parsing
   - Option B: Implement minimal parser (just ref updates)
   - Recommendation: Research git2 first, then decide

3. **CORS policy?**
   - Allow all origins (`*`) for now?
   - Or restrict to configured domains?
   - Recommendation: Start with `*`, make configurable later

4. **Error detail?**
   - How much info in 403 responses?
   - Show ref updates that failed?
   - Show expected vs. actual commit?
   - Recommendation: Detailed errors for better DX

5. **Performance target?**
   - < 100ms for auth validation?
   - Cache state events?
   - Recommendation: Yes to both

---

## Conclusion

✅ **The hybrid approach is validated and sound**

⚠️ **Must fork git-http-backend for inline authorization**

✅ **Ready to proceed with implementation**

**Confidence Level:** High (95%)

The crate provides exactly what we need as a foundation. The modifications required are straightforward and well-scoped. The main work is:
1. Fork setup
2. Protocol parsing
3. Authorization integration
4. CORS and polish

All achievable within the 4-week timeline.

---

**Next:** Review this analysis, make fork vs. vendor decision, then start Phase 0.
