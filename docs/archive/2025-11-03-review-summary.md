# ngit-grasp Architecture Review Summary

## Investigation Complete ✅

After thorough investigation of:
1. The GRASP protocol specification
2. The reference implementation (ngit-relay in Go)
3. The `git-http-backend` Rust crate
4. The `nostr-relay-builder` Rust crate

## Key Decision: Inline Authorization (Not Hooks)

**Question**: Should we use Git pre-receive hooks or inject logic directly into the HTTP handler?

**Answer**: **Direct injection is both pragmatic and superior** ✅

### Why This Works

The `git-http-backend` Rust crate:
- Provides actix-web handlers for Git Smart HTTP protocol
- Spawns `git-receive-pack` as a subprocess
- We can intercept **before** spawning Git
- Full access to request body for parsing ref updates

### Advantages

1. **Better Error Handling**: Direct HTTP responses vs. parsing hook stderr
2. **Simpler Deployment**: Single binary, no hook management
3. **Easier Testing**: Pure Rust unit tests, no shell scripts
4. **Better Performance**: Skip Git spawn for invalid pushes
5. **Tighter Integration**: Shared state between Git and Nostr

### Architecture

```
Client Request
      ↓
actix-web Router
      ↓
git_receive_pack handler
      ↓
Parse ref updates from body
      ↓
Query local Nostr relay (in-process)
      ↓
Validate refs against state event
      ↓
   Valid? ──No──→ HTTP 403 Error
      ↓
     Yes
      ↓
Spawn git-receive-pack
      ↓
Stream to/from Git
      ↓
Return response to client
```

## Documentation Created

### 1. README.md
- Project overview and goals
- Quick start guide
- Feature list and GRASP compliance
- Technology stack
- Comparison with reference implementation

### 2. docs/ARCHITECTURE.md
- Detailed architectural design
- Component breakdown with code examples
- Data flow diagrams
- Implementation details for:
  - Git protocol handling
  - Nostr relay configuration
  - Push validation logic
  - Repository management
- Performance considerations
- Testing strategy
- Future extensions (GRASP-02, GRASP-05)
- Deployment options

### 3. docs/DECISION_SUMMARY.md
- Investigation findings
- Hook vs. inline comparison
- Detailed rationale for inline approach
- Concerns and mitigations
- Next steps

### 4. docs/COMPARISON.md
- Side-by-side comparison with ngit-relay
- Component breakdown
- Performance estimates
- Code complexity analysis
- Migration path
- When to choose each implementation

### 5. docs/GIT_PROTOCOL.md
- Git Smart HTTP protocol reference
- Pkt-line format explanation
- Ref update parsing
- Validation logic examples
- Integration with actix-web
- Testing examples

### 6. .env.example
- Configuration template

## Technology Stack

### Core
- **Rust 1.75+**: Language
- **actix-web 4**: HTTP server
- **tokio**: Async runtime

### Git
- **git-http-backend 0.1.3**: Git protocol handling
- **tokio::process**: Git subprocess management

### Nostr
- **nostr-relay-builder 0.43**: Relay infrastructure
- **nostr-sdk 0.43**: Event handling and validation

### Storage
- **LMDB or NDB**: Event storage (via nostr-relay-builder)
- **File system**: Git repositories

## Project Structure

```
ngit-grasp/
├── src/
│   ├── main.rs              # Server setup
│   ├── config.rs            # Configuration
│   ├── git/
│   │   ├── mod.rs
│   │   ├── handler.rs       # Git HTTP handlers
│   │   └── authorization.rs # Push validation
│   ├── nostr/
│   │   ├── mod.rs
│   │   ├── relay.rs         # Relay setup
│   │   └── events.rs        # Event handlers
│   └── storage/
│       ├── mod.rs
│       └── repository.rs    # Repo management
├── docs/
│   ├── ARCHITECTURE.md      # Detailed design
│   ├── DECISION_SUMMARY.md  # Why inline auth
│   ├── COMPARISON.md        # vs ngit-relay
│   └── GIT_PROTOCOL.md      # Protocol reference
├── tests/
│   ├── integration/
│   └── fixtures/
├── README.md                # Overview
├── .env.example             # Config template
└── Cargo.toml               # Dependencies
```

## Implementation Complexity

### What We Need to Build

1. **Git Protocol Parsing** (~500 LOC)
   - Pkt-line parser
   - Ref update extraction
   - Request/response handling

2. **Authorization Logic** (~300 LOC)
   - Maintainer resolution (recursive)
   - State validation
   - PR ref handling

3. **Nostr Relay Setup** (~100 LOC)
   - Policies for announcements
   - Event hooks
   - NIP-11 configuration

4. **Repository Management** (~200 LOC)
   - Create/configure repos
   - Path management
   - Git command execution

5. **Main Server** (~200 LOC)
   - Route configuration
   - State management
   - Error handling

**Total: ~1,300-1,500 LOC** (similar to reference implementation)

### What We Get from Libraries

- Nostr relay infrastructure (WebSocket, event store, etc.)
- Git protocol basics (upload-pack, receive-pack)
- Async runtime and HTTP server
- Nostr event parsing and validation

## GRASP Compliance Roadmap

### Phase 1: GRASP-01 Core (MVP)
- [ ] Basic HTTP server with routing
- [ ] Nostr relay with announcement policies
- [ ] Git upload-pack (clone/fetch)
- [ ] Git receive-pack with inline validation
- [ ] Repository provisioning on announcements
- [ ] Multi-maintainer support
- [ ] refs/nostr/* support for PRs
- [ ] CORS support
- [ ] NIP-11 relay info

### Phase 2: GRASP-02 Proactive Sync
- [ ] Background event sync from listed relays
- [ ] Background Git sync from listed clones
- [ ] PR data fetching

### Phase 3: GRASP-05 Archive
- [ ] Accept non-listed repositories
- [ ] Mirror/backup mode

## Risks and Mitigations

### Risk 1: Git Protocol Complexity
**Impact**: Medium  
**Likelihood**: Low  
**Mitigation**: Well-documented protocol, reference implementation exists, comprehensive testing

### Risk 2: Performance of Inline Validation
**Impact**: Low  
**Likelihood**: Low  
**Mitigation**: State caching, async validation, benchmarking

### Risk 3: nostr-relay-builder API Changes
**Impact**: Medium  
**Likelihood**: Medium (it's in alpha)  
**Mitigation**: Pin versions, monitor upstream, abstract relay interface

### Risk 4: Compatibility with ngit Clients
**Impact**: High  
**Likelihood**: Low  
**Mitigation**: Follow GRASP spec exactly, test with ngit CLI

## Success Criteria

1. **Functional**:
   - ✅ Accept repository announcements
   - ✅ Provision Git repositories
   - ✅ Validate pushes against state events
   - ✅ Serve clones/fetches
   - ✅ Support multi-maintainer repos
   - ✅ Handle PR refs

2. **Performance**:
   - ✅ < 50ms push validation overhead
   - ✅ < 100MB memory usage
   - ✅ Handle 100+ concurrent connections

3. **Quality**:
   - ✅ >80% test coverage
   - ✅ No clippy warnings
   - ✅ Comprehensive error handling
   - ✅ Good logging/observability

4. **Compliance**:
   - ✅ GRASP-01 compliant
   - ✅ NIP-34 compliant
   - ✅ NIP-11 compliant
   - ✅ Works with ngit CLI

## Next Steps

### Immediate (Week 1)
1. Set up Cargo workspace
2. Define core types (RefUpdate, RepositoryState, etc.)
3. Implement pkt-line parser
4. Write parser tests

### Short-term (Week 2-3)
1. Implement Nostr relay with policies
2. Implement Git upload-pack handler
3. Implement Git receive-pack with validation
4. Repository management

### Medium-term (Week 4-6)
1. Integration testing
2. GRASP-01 compliance testing
3. Documentation
4. Performance optimization

### Long-term (Month 2+)
1. GRASP-02 implementation
2. Production hardening
3. Deployment tooling
4. Community feedback

## Questions for Review

1. **Architecture**: Does the inline authorization approach make sense?
2. **Complexity**: Is the estimated LOC reasonable?
3. **Dependencies**: Are the chosen libraries appropriate?
4. **Scope**: Should we start with GRASP-01 only, or include GRASP-02?
5. **Testing**: What level of testing is needed before first release?
6. **Deployment**: Single binary, Docker, or both?

## Recommendation

**Proceed with implementation** using the inline authorization architecture.

The design is:
- ✅ Technically sound
- ✅ Pragmatic and achievable
- ✅ Superior to hook-based approach
- ✅ Well-documented
- ✅ Testable
- ✅ GRASP-compliant

The Rust ecosystem provides excellent libraries for both Git and Nostr, making this implementation both feasible and maintainable.

## References

- [GRASP Protocol](https://gitworkshop.dev/danconwaydev.com/grasp)
- [ngit-relay (Reference)](https://gitworkshop.dev/npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr/ngit-relay)
- [NIP-34: Git Stuff](https://nips.nostr.com/34)
- [git-http-backend crate](https://crates.io/crates/git-http-backend)
- [nostr-relay-builder crate](https://crates.io/crates/nostr-relay-builder)
