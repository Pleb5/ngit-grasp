# Test Strategy for ngit-grasp

## Overview

This document outlines the comprehensive testing strategy for ngit-grasp, including a **reusable GRASP compliance testing tool** that can validate any GRASP implementation against the protocol specification.

## Testing Philosophy

1. **Specification-Driven**: Tests mirror the GRASP protocol structure exactly
2. **Compliance-First**: Every requirement in the spec has a corresponding test
3. **Reusable**: Compliance tests can validate any GRASP implementation
4. **Clear Failures**: Test failures cite exact spec lines/sections
5. **Comprehensive**: Unit, integration, and compliance testing

## Test Pyramid

```
                    ╱╲
                   ╱  ╲
                  ╱ E2E╲              ~ 10%  End-to-end with real Git
                 ╱──────╲
                ╱        ╲
               ╱Compliance╲           ~ 20%  GRASP spec validation
              ╱────────────╲
             ╱              ╲
            ╱  Integration   ╲        ~ 30%  Component interaction
           ╱──────────────────╲
          ╱                    ╲
         ╱   Unit Tests         ╲     ~ 40%  Individual functions
        ╱────────────────────────╲
```

## GRASP Compliance Testing Tool

### Design Goals

1. **Reusable**: Can test ngit-grasp or any other GRASP implementation
2. **Spec-Mirrored**: Test structure matches GRASP protocol documents
3. **Clear Reporting**: Failures cite exact spec requirements
4. **Automated**: Can run in CI/CD
5. **Extensible**: Easy to add new GRASP versions (GRASP-02, GRASP-05)

### Project Structure

```
grasp-compliance-tests/
├── Cargo.toml                    # Standalone crate
├── README.md                     # Usage instructions
├── src/
│   ├── lib.rs                    # Public API
│   ├── client.rs                 # Test client utilities
│   ├── assertions.rs             # Spec-based assertions
│   └── specs/
│       ├── mod.rs                # Spec registry
│       ├── grasp_01.rs           # GRASP-01 tests
│       ├── grasp_02.rs           # GRASP-02 tests
│       └── grasp_05.rs           # GRASP-05 tests
├── fixtures/
│   ├── repos/                    # Test repositories
│   ├── events/                   # Nostr event fixtures
│   └── keys/                     # Test keypairs
└── examples/
    └── test_implementation.rs    # Example usage
```

### Spec-Mirrored Test Structure

Each GRASP spec document maps to a test module with identical structure:

```rust
// src/specs/grasp_01.rs

use crate::{TestContext, SpecRequirement, ComplianceResult};

/// GRASP-01 - Core Service Requirements
/// Reference: https://gitworkshop.dev/danconwaydev.com/grasp/01.md
pub struct Grasp01Spec;

impl Grasp01Spec {
    /// Run all GRASP-01 compliance tests
    pub async fn test_compliance(ctx: &TestContext) -> ComplianceResult {
        let mut results = ComplianceResult::new("GRASP-01");
        
        // Section: Nostr Relay
        results.add(Self::test_nostr_relay_nip01_compliance(ctx).await);
        results.add(Self::test_accepts_repository_announcements(ctx).await);
        results.add(Self::test_accepts_repository_state_announcements(ctx).await);
        results.add(Self::test_rejects_unlisted_announcements(ctx).await);
        results.add(Self::test_accepts_related_events(ctx).await);
        results.add(Self::test_serves_nip11_document(ctx).await);
        results.add(Self::test_nip11_has_supported_grasps(ctx).await);
        results.add(Self::test_nip11_has_repo_acceptance_criteria(ctx).await);
        results.add(Self::test_nip11_has_curation_policy(ctx).await);
        
        // Section: Git Smart HTTP Service
        results.add(Self::test_serves_git_at_correct_path(ctx).await);
        results.add(Self::test_accepts_matching_pushes(ctx).await);
        results.add(Self::test_rejects_mismatched_pushes(ctx).await);
        results.add(Self::test_respects_recursive_maintainers(ctx).await);
        results.add(Self::test_sets_head_from_state(ctx).await);
        results.add(Self::test_accepts_nostr_refs(ctx).await);
        results.add(Self::test_rejects_pr_branches(ctx).await);
        results.add(Self::test_deletes_orphaned_nostr_refs(ctx).await);
        results.add(Self::test_allows_reachable_sha1_in_want(ctx).await);
        results.add(Self::test_allows_tip_sha1_in_want(ctx).await);
        results.add(Self::test_serves_webpage(ctx).await);
        
        // Section: CORS Support
        results.add(Self::test_cors_allow_origin(ctx).await);
        results.add(Self::test_cors_allow_methods(ctx).await);
        results.add(Self::test_cors_allow_headers(ctx).await);
        results.add(Self::test_cors_options_request(ctx).await);
        
        results
    }
    
    // ================================================================
    // NOSTR RELAY TESTS
    // ================================================================
    
    /// MUST serve a NIP-01 compliant nostr relay at `/`
    /// 
    /// Spec: GRASP-01, Line 9-10
    /// > MUST serve a [NIP-01](https://nips.nostr.com/1) compliant nostr 
    /// > relay at `/` that accepts [git repository announcements]...
    async fn test_nostr_relay_nip01_compliance(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "nostr_relay_nip01_compliance",
            "GRASP-01:9-10",
            "MUST serve a NIP-01 compliant nostr relay at `/`",
        )
        .run(async {
            // Test WebSocket upgrade at /
            let ws = ctx.connect_websocket("/").await?;
            
            // Test NIP-01 REQ/EVENT/CLOSE/NOTICE messages
            ws.send_req("test-sub", vec![]).await?;
            let response = ws.recv().await?;
            assert_nip01_eose(response)?;
            
            Ok(())
        })
        .await
    }
    
    /// MUST reject announcements that do not list the service in both 
    /// `clone` and `relays` tags unless implementing `GRASP-05`
    ///
    /// Spec: GRASP-01, Line 12-13
    /// > MUST reject [git repository announcements] that do not list the 
    /// > service in both `clone` and `relays` tags unless implementing `GRASP-05`.
    async fn test_rejects_unlisted_announcements(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "rejects_unlisted_announcements",
            "GRASP-01:12-13",
            "MUST reject announcements not listing service in clone and relays",
        )
        .run(async {
            let event = ctx.create_announcement()
                .without_clone_tag(ctx.domain())
                .build()
                .await?;
            
            let result = ctx.send_event(event).await?;
            
            assert_eq!(
                result.ok, false,
                "Expected rejection of announcement without clone tag"
            );
            assert!(
                result.message.contains("clone") || result.message.contains("relays"),
                "Expected rejection message to mention clone/relays requirement"
            );
            
            Ok(())
        })
        .await
    }
    
    /// MUST accept other events that tag, or are tagged by, accepted announcements
    ///
    /// Spec: GRASP-01, Line 17-20
    /// > MUST accept other events that tag, or are tagged by, either:
    /// > 1. accepted [git repository announcements]; or
    /// > 2. accepted [issues] or [patches]
    async fn test_accepts_related_events(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "accepts_related_events",
            "GRASP-01:17-20",
            "MUST accept events that tag or are tagged by accepted announcements",
        )
        .run(async {
            // First, create and accept an announcement
            let announcement = ctx.create_announcement()
                .with_clone_tag(ctx.domain())
                .with_relay_tag(ctx.domain())
                .build()
                .await?;
            
            ctx.send_event(announcement.clone()).await?;
            
            // Now send an issue that tags the announcement
            let issue = ctx.create_issue()
                .tag_announcement(&announcement)
                .build()
                .await?;
            
            let result = ctx.send_event(issue).await?;
            
            assert_eq!(
                result.ok, true,
                "Expected acceptance of issue tagging accepted announcement"
            );
            
            Ok(())
        })
        .await
    }
    
    /// MUST serve a NIP-11 document with required fields
    ///
    /// Spec: GRASP-01, Line 24-27
    /// > MUST serve a [NIP-11] document:
    /// > 1. MUST list each supported GRASP under `supported_grasps`
    /// > 2. MUST list repository acceptance criteria under `repo_acceptance_criteria`
    /// > 3. MUST list curation policy under `curation` if events are curated
    async fn test_serves_nip11_document(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "serves_nip11_document",
            "GRASP-01:24-27",
            "MUST serve a NIP-11 document",
        )
        .run(async {
            let nip11 = ctx.fetch_nip11().await?;
            
            assert!(
                nip11.contains_key("supported_nips"),
                "NIP-11 document must have supported_nips"
            );
            
            Ok(())
        })
        .await
    }
    
    /// NIP-11 MUST list supported GRASPs
    ///
    /// Spec: GRASP-01, Line 25
    /// > 1. MUST list each supported GRASP under `supported_grasps` 
    /// >    in format `GRASP-XX` eg `GRASP-01` as a string array
    async fn test_nip11_has_supported_grasps(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "nip11_has_supported_grasps",
            "GRASP-01:25",
            "NIP-11 MUST list supported_grasps as string array",
        )
        .run(async {
            let nip11 = ctx.fetch_nip11().await?;
            
            let grasps = nip11.get("supported_grasps")
                .ok_or("NIP-11 missing supported_grasps field")?
                .as_array()
                .ok_or("supported_grasps must be an array")?;
            
            assert!(
                grasps.iter().any(|g| g.as_str() == Some("GRASP-01")),
                "supported_grasps must include 'GRASP-01'"
            );
            
            // Validate format: GRASP-XX
            for grasp in grasps {
                let s = grasp.as_str().ok_or("GRASP must be a string")?;
                assert!(
                    s.starts_with("GRASP-") && s.len() >= 8,
                    "GRASP format must be 'GRASP-XX', got: {}", s
                );
            }
            
            Ok(())
        })
        .await
    }
    
    // ================================================================
    // GIT SMART HTTP SERVICE TESTS
    // ================================================================
    
    /// MUST serve a git repository via git smart http at /<npub>/<identifier>.git
    ///
    /// Spec: GRASP-01, Line 31-32
    /// > MUST serve a git repository via an unauthenticated [git smart http service]
    /// > at `/<npub>/<identifier>.git` for each accepted announcement
    async fn test_serves_git_at_correct_path(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "serves_git_at_correct_path",
            "GRASP-01:31-32",
            "MUST serve git at /<npub>/<identifier>.git",
        )
        .run(async {
            // Create and send announcement
            let announcement = ctx.create_announcement()
                .with_identifier("test-repo")
                .with_clone_tag(ctx.domain())
                .with_relay_tag(ctx.domain())
                .build()
                .await?;
            
            let npub = announcement.author_npub();
            ctx.send_event(announcement).await?;
            
            // Wait for repo creation
            tokio::time::sleep(Duration::from_secs(2)).await;
            
            // Test git info/refs endpoint
            let path = format!("/{}/test-repo.git/info/refs?service=git-upload-pack", npub);
            let response = ctx.http_get(&path).await?;
            
            assert_eq!(
                response.status(), 200,
                "Git info/refs must return 200 OK"
            );
            
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                "application/x-git-upload-pack-advertisement",
                "Git info/refs must have correct content-type"
            );
            
            Ok(())
        })
        .await
    }
    
    /// MUST accept pushes that match the latest state announcement
    ///
    /// Spec: GRASP-01, Line 34-35
    /// > MUST accept pushes via this service that match the latest 
    /// > [repo state announcement] on the relay, respecting the recursive maintainer set.
    async fn test_accepts_matching_pushes(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "accepts_matching_pushes",
            "GRASP-01:34-35",
            "MUST accept pushes matching latest state announcement",
        )
        .run(async {
            // Setup: Create repo with announcement and state
            let (announcement, state) = ctx.create_repo_with_state()
                .branch("main", "a1b2c3d4...")
                .build()
                .await?;
            
            // Push matching state
            let result = ctx.git_push(&announcement, "main", "a1b2c3d4...").await?;
            
            assert!(
                result.success,
                "Push matching state must succeed, got: {}", result.stderr
            );
            
            Ok(())
        })
        .await
    }
    
    /// MUST reject pushes that don't match the state announcement
    ///
    /// Spec: GRASP-01, Line 34-35 (inverse requirement)
    /// Implied by "MUST accept pushes... that match"
    async fn test_rejects_mismatched_pushes(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "rejects_mismatched_pushes",
            "GRASP-01:34-35",
            "MUST reject pushes not matching state announcement",
        )
        .run(async {
            // Setup: Create repo with state pointing to commit A
            let (announcement, state) = ctx.create_repo_with_state()
                .branch("main", "aaaa1111...")
                .build()
                .await?;
            
            // Try to push different commit B
            let result = ctx.git_push(&announcement, "main", "bbbb2222...").await;
            
            assert!(
                result.is_err() || !result.unwrap().success,
                "Push not matching state must be rejected"
            );
            
            Ok(())
        })
        .await
    }
    
    /// MUST accept pushes to refs/nostr/<event-id>
    ///
    /// Spec: GRASP-01, Line 42-44
    /// > MUST accept pushes via this service to `refs/nostr/<event-id>` but 
    /// > SHOULD reject if event exists on relay listing a different tip
    async fn test_accepts_nostr_refs(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "accepts_nostr_refs",
            "GRASP-01:42-44",
            "MUST accept pushes to refs/nostr/<event-id>",
        )
        .run(async {
            let (announcement, _) = ctx.create_repo_with_state().build().await?;
            
            // Create a PR event
            let pr_event = ctx.create_pr_event()
                .for_repo(&announcement)
                .build()
                .await?;
            
            let event_id = pr_event.id();
            
            // Push to refs/nostr/<event-id>
            let result = ctx.git_push(
                &announcement,
                &format!("refs/nostr/{}", event_id),
                "commit-sha..."
            ).await?;
            
            assert!(
                result.success,
                "Push to refs/nostr/<event-id> must succeed"
            );
            
            Ok(())
        })
        .await
    }
    
    /// MUST reject pr/* branches
    ///
    /// Spec: GRASP-01, Line 42-44 (implied)
    /// PRs should use refs/nostr/, not refs/heads/pr/*
    async fn test_rejects_pr_branches(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "rejects_pr_branches",
            "GRASP-01:42-44",
            "MUST reject refs/heads/pr/* (use refs/nostr/ instead)",
        )
        .run(async {
            let (announcement, _) = ctx.create_repo_with_state().build().await?;
            
            // Try to push to pr/* branch
            let result = ctx.git_push(
                &announcement,
                "refs/heads/pr/123",
                "commit-sha..."
            ).await;
            
            assert!(
                result.is_err() || !result.unwrap().success,
                "Push to refs/heads/pr/* must be rejected"
            );
            
            Ok(())
        })
        .await
    }
    
    /// MUST include allow-reachable-sha1-in-want and allow-tip-sha1-in-want
    ///
    /// Spec: GRASP-01, Line 48-49
    /// > MUST include `allow-reachable-sha1-in-want` and `allow-tip-sha1-in-want` 
    /// > in advertisement and serve available oids.
    async fn test_allows_tip_sha1_in_want(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "allows_tip_sha1_in_want",
            "GRASP-01:48-49",
            "MUST advertise and support allow-tip-sha1-in-want",
        )
        .run(async {
            let (announcement, _) = ctx.create_repo_with_state()
                .branch("main", "a1b2c3d4...")
                .build()
                .await?;
            
            // Fetch git capabilities
            let caps = ctx.git_capabilities(&announcement).await?;
            
            assert!(
                caps.contains("allow-tip-sha1-in-want"),
                "Git advertisement must include allow-tip-sha1-in-want"
            );
            
            assert!(
                caps.contains("allow-reachable-sha1-in-want"),
                "Git advertisement must include allow-reachable-sha1-in-want"
            );
            
            Ok(())
        })
        .await
    }
    
    // ================================================================
    // CORS SUPPORT TESTS
    // ================================================================
    
    /// MUST set Access-Control-Allow-Origin: * on ALL responses
    ///
    /// Spec: GRASP-01, Line 57
    /// > 1. Set `Access-Control-Allow-Origin: *` on ALL responses
    async fn test_cors_allow_origin(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "cors_allow_origin",
            "GRASP-01:57",
            "MUST set Access-Control-Allow-Origin: * on ALL responses",
        )
        .run(async {
            let paths = vec![
                "/",
                "/test-npub/test-repo.git/info/refs?service=git-upload-pack",
            ];
            
            for path in paths {
                let response = ctx.http_get(path).await?;
                
                assert_eq!(
                    response.headers().get("access-control-allow-origin").unwrap(),
                    "*",
                    "Path {} must have Access-Control-Allow-Origin: *", path
                );
            }
            
            Ok(())
        })
        .await
    }
    
    /// MUST respond to OPTIONS requests with 204 No Content
    ///
    /// Spec: GRASP-01, Line 60
    /// > 4. Respond to OPTIONS requests with 204 No Content
    async fn test_cors_options_request(ctx: &TestContext) -> TestResult {
        TestResult::new(
            "cors_options_request",
            "GRASP-01:60",
            "MUST respond to OPTIONS with 204 No Content",
        )
        .run(async {
            let response = ctx.http_options("/test-npub/test-repo.git/info/refs").await?;
            
            assert_eq!(
                response.status(), 204,
                "OPTIONS request must return 204 No Content"
            );
            
            Ok(())
        })
        .await
    }
}
```

### Test Result Reporting

```rust
/// Test result with spec citation
pub struct TestResult {
    pub name: String,
    pub spec_ref: String,        // e.g., "GRASP-01:12-13"
    pub requirement: String,      // Exact text from spec
    pub passed: bool,
    pub error: Option<String>,
    pub duration: Duration,
}

impl TestResult {
    /// Create a new test result
    pub fn new(name: &str, spec_ref: &str, requirement: &str) -> Self {
        TestResult {
            name: name.to_string(),
            spec_ref: spec_ref.to_string(),
            requirement: requirement.to_string(),
            passed: false,
            error: None,
            duration: Duration::default(),
        }
    }
    
    /// Run the test
    pub async fn run<F, Fut>(mut self, test_fn: F) -> Self 
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let start = Instant::now();
        
        match test_fn().await {
            Ok(()) => {
                self.passed = true;
            }
            Err(e) => {
                self.passed = false;
                self.error = Some(e);
            }
        }
        
        self.duration = start.elapsed();
        self
    }
}

/// Collection of test results for a spec
pub struct ComplianceResult {
    pub spec: String,
    pub results: Vec<TestResult>,
}

impl ComplianceResult {
    pub fn report(&self) -> String {
        let mut output = String::new();
        
        output.push_str(&format!("\n{} Compliance Report\n", self.spec));
        output.push_str(&"=".repeat(60));
        output.push_str("\n\n");
        
        let passed = self.results.iter().filter(|r| r.passed).count();
        let total = self.results.len();
        
        output.push_str(&format!("Results: {}/{} passed\n\n", passed, total));
        
        for result in &self.results {
            let status = if result.passed { "✓" } else { "✗" };
            
            output.push_str(&format!(
                "{} {} ({})\n",
                status, result.name, result.spec_ref
            ));
            
            output.push_str(&format!("  Requirement: {}\n", result.requirement));
            
            if let Some(error) = &result.error {
                output.push_str(&format!("  Error: {}\n", error));
            }
            
            output.push_str(&format!("  Duration: {:?}\n\n", result.duration));
        }
        
        output
    }
}
```

### Usage Example

```rust
// examples/test_implementation.rs

use grasp_compliance_tests::{TestContext, Grasp01Spec};

#[tokio::main]
async fn main() {
    // Configure the implementation to test
    let ctx = TestContext::builder()
        .base_url("http://localhost:8080")
        .websocket_url("ws://localhost:8080")
        .domain("localhost:8080")
        .build();
    
    // Run GRASP-01 compliance tests
    let results = Grasp01Spec::test_compliance(&ctx).await;
    
    // Print report
    println!("{}", results.report());
    
    // Exit with error if any tests failed
    if !results.all_passed() {
        std::process::exit(1);
    }
}
```

### Integration with ngit-grasp

In `ngit-grasp/tests/compliance.rs`:

```rust
use grasp_compliance_tests::{TestContext, Grasp01Spec};

#[tokio::test]
async fn test_grasp_01_compliance() {
    // Start test server
    let server = start_test_server().await;
    
    // Configure test context
    let ctx = TestContext::builder()
        .base_url(&server.url())
        .websocket_url(&server.ws_url())
        .domain(&server.domain())
        .build();
    
    // Run compliance tests
    let results = Grasp01Spec::test_compliance(&ctx).await;
    
    // Assert all tests passed
    assert!(
        results.all_passed(),
        "GRASP-01 compliance failed:\n{}", 
        results.report()
    );
}
```

## Unit Testing Strategy

### Git Module Tests

```rust
// src/git/parser.rs tests

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_pkt_line() {
        let data = b"0006a\n";
        let (length, payload) = parse_pkt_line(data).unwrap();
        assert_eq!(length, 6);
        assert_eq!(payload, b"a\n");
    }
    
    #[test]
    fn test_parse_flush_packet() {
        let data = b"0000";
        let result = parse_pkt_line(data).unwrap();
        assert_eq!(result.0, 0);
    }
    
    #[test]
    fn test_parse_ref_updates() {
        let body = b"00820000000000000000000000000000000000000000 \
                     a1b2c3d4e5f6789012345678901234567890abcd \
                     refs/heads/main\0 report-status\n\
                     0000";
        
        let updates = parse_ref_updates(body).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].ref_name, "refs/heads/main");
    }
}
```

### Authorization Module Tests

```rust
// src/git/authorization.rs tests

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_get_maintainers_single() {
        let events = vec![
            create_test_announcement("alice", "repo1", vec![]),
        ];
        
        let maintainers = get_maintainers(&events, "alice", "repo1");
        assert_eq!(maintainers, vec!["alice"]);
    }
    
    #[test]
    fn test_get_maintainers_recursive() {
        let events = vec![
            create_test_announcement("alice", "repo1", vec!["bob"]),
            create_test_announcement("bob", "repo1", vec![]),
        ];
        
        let maintainers = get_maintainers(&events, "alice", "repo1");
        assert!(maintainers.contains(&"alice".to_string()));
        assert!(maintainers.contains(&"bob".to_string()));
    }
    
    #[test]
    fn test_get_maintainers_circular() {
        let events = vec![
            create_test_announcement("alice", "repo1", vec!["bob"]),
            create_test_announcement("bob", "repo1", vec!["alice"]),
        ];
        
        let maintainers = get_maintainers(&events, "alice", "repo1");
        assert_eq!(maintainers.len(), 2);
    }
    
    #[test]
    fn test_validate_state_ref_matching() {
        let state = RepositoryState {
            branches: HashMap::from([
                ("main".into(), "a1b2c3d4...".into()),
            ]),
            tags: HashMap::new(),
        };
        
        let update = RefUpdate {
            old_oid: "0000...".into(),
            new_oid: "a1b2c3d4...".into(),
            ref_name: "refs/heads/main".into(),
        };
        
        assert!(validate_state_ref(&state, &update).is_ok());
    }
    
    #[test]
    fn test_validate_state_ref_mismatch() {
        let state = RepositoryState {
            branches: HashMap::from([
                ("main".into(), "aaaa1111...".into()),
            ]),
            tags: HashMap::new(),
        };
        
        let update = RefUpdate {
            old_oid: "0000...".into(),
            new_oid: "bbbb2222...".into(),
            ref_name: "refs/heads/main".into(),
        };
        
        assert!(validate_state_ref(&state, &update).is_err());
    }
}
```

## Integration Testing Strategy

### Repository Lifecycle Tests

```rust
// tests/integration/repository_lifecycle.rs

#[tokio::test]
async fn test_repository_creation_on_announcement() {
    let app = test_app().await;
    
    // Send repository announcement
    let announcement = create_announcement()
        .with_identifier("test-repo")
        .with_clone_tag(app.domain())
        .with_relay_tag(app.domain())
        .sign()
        .await;
    
    app.send_event(announcement).await.unwrap();
    
    // Wait for async processing
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Verify repository was created
    let repo_path = app.git_data_path()
        .join(announcement.author_npub())
        .join("test-repo.git");
    
    assert!(repo_path.exists());
    assert!(repo_path.join("HEAD").exists());
    assert!(repo_path.join("config").exists());
}

#[tokio::test]
async fn test_push_validation_flow() {
    let app = test_app().await;
    
    // Create repository with state
    let (announcement, state) = app.create_repo_with_state()
        .branch("main", "commit-sha-123")
        .build()
        .await;
    
    // Attempt push matching state
    let result = app.git_push("main", "commit-sha-123").await;
    assert!(result.success);
    
    // Attempt push NOT matching state
    let result = app.git_push("main", "different-sha-456").await;
    assert!(!result.success);
    assert!(result.stderr.contains("state event"));
}
```

### Multi-Maintainer Tests

```rust
#[tokio::test]
async fn test_multi_maintainer_push() {
    let app = test_app().await;
    
    // Alice creates repo, lists Bob as maintainer
    let alice_announcement = create_announcement()
        .author("alice")
        .maintainers(vec!["bob"])
        .build();
    
    app.send_event(alice_announcement).await.unwrap();
    
    // Bob creates state event
    let bob_state = create_state()
        .author("bob")
        .branch("main", "commit-123")
        .build();
    
    app.send_event(bob_state).await.unwrap();
    
    // Bob's push should succeed
    let result = app.git_push_as("bob", "main", "commit-123").await;
    assert!(result.success);
}
```

## End-to-End Testing

### Real Git Client Tests

```rust
// tests/e2e/git_client.rs

#[tokio::test]
async fn test_real_git_clone() {
    let app = test_app().await;
    
    // Setup repository
    let (announcement, _) = app.create_repo_with_commits()
        .commit("Initial commit", "file.txt", "content")
        .build()
        .await;
    
    // Clone with real git client
    let temp_dir = TempDir::new().unwrap();
    let clone_url = format!(
        "http://{}/{}/{}.git",
        app.domain(),
        announcement.author_npub(),
        announcement.identifier()
    );
    
    let output = Command::new("git")
        .args(&["clone", &clone_url])
        .current_dir(&temp_dir)
        .output()
        .await
        .unwrap();
    
    assert!(output.status.success());
    assert!(temp_dir.path().join(announcement.identifier()).exists());
}

#[tokio::test]
async fn test_real_git_push() {
    let app = test_app().await;
    
    // Create repository
    let (announcement, keys) = app.create_repo().await;
    
    // Clone it
    let temp_dir = TempDir::new().unwrap();
    git_clone(&app, &announcement, &temp_dir).await;
    
    // Make changes
    let repo_dir = temp_dir.path().join(announcement.identifier());
    tokio::fs::write(repo_dir.join("new-file.txt"), "content").await.unwrap();
    
    // Commit
    git_commit(&repo_dir, "Add new file").await;
    
    // Send state event for new commit
    let new_commit = git_rev_parse(&repo_dir, "HEAD").await;
    app.send_state(&announcement, "main", &new_commit, &keys).await;
    
    // Push
    let output = Command::new("git")
        .args(&["push", "origin", "main"])
        .current_dir(&repo_dir)
        .output()
        .await
        .unwrap();
    
    assert!(output.status.success());
}
```

## Performance Testing

### Load Tests

```rust
// tests/performance/load.rs

#[tokio::test]
async fn test_concurrent_pushes() {
    let app = test_app().await;
    
    let num_concurrent = 100;
    let mut handles = vec![];
    
    for i in 0..num_concurrent {
        let app = app.clone();
        let handle = tokio::spawn(async move {
            let (announcement, state) = app.create_repo_with_state()
                .branch("main", &format!("commit-{}", i))
                .build()
                .await;
            
            app.git_push("main", &format!("commit-{}", i)).await
        });
        handles.push(handle);
    }
    
    let results = futures::future::join_all(handles).await;
    
    // All should succeed
    for result in results {
        assert!(result.unwrap().success);
    }
}

#[tokio::test]
async fn test_event_ingestion_throughput() {
    let app = test_app().await;
    
    let num_events = 1000;
    let start = Instant::now();
    
    for i in 0..num_events {
        let event = create_announcement()
            .with_identifier(&format!("repo-{}", i))
            .build();
        app.send_event(event).await.unwrap();
    }
    
    let duration = start.elapsed();
    let throughput = num_events as f64 / duration.as_secs_f64();
    
    println!("Event throughput: {:.2} events/sec", throughput);
    assert!(throughput > 100.0, "Throughput too low");
}
```

## Test Utilities

### Test Fixtures

```rust
// tests/common/fixtures.rs

pub struct TestEventBuilder {
    kind: Kind,
    content: String,
    tags: Vec<Tag>,
    keys: Option<Keys>,
}

impl TestEventBuilder {
    pub fn announcement() -> Self {
        TestEventBuilder {
            kind: Kind::RepositoryAnnouncement,
            content: String::new(),
            tags: vec![],
            keys: None,
        }
    }
    
    pub fn with_identifier(mut self, id: &str) -> Self {
        self.tags.push(Tag::Identifier(id.to_string()));
        self
    }
    
    pub fn with_clone_tag(mut self, url: &str) -> Self {
        self.tags.push(Tag::new("clone", vec![url]));
        self
    }
    
    pub async fn build(self) -> Event {
        let keys = self.keys.unwrap_or_else(|| Keys::generate());
        EventBuilder::new(self.kind, self.content, self.tags)
            .to_event(&keys)
            .await
            .unwrap()
    }
}
```

### Test Server

```rust
// tests/common/server.rs

pub struct TestServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
}

impl TestServer {
    pub async fn start() -> Self {
        let config = Config {
            domain: "localhost:0".to_string(),
            git_data_path: TempDir::new().unwrap().into_path(),
            relay_data_path: TempDir::new().unwrap().into_path(),
            // ... other config
        };
        
        let app = create_app(config).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        
        // Wait for server to be ready
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        TestServer { addr, handle }
    }
    
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }
}
```

## CI/CD Integration

### GitHub Actions Workflow

```yaml
# .github/workflows/test.yml

name: Test

on: [push, pull_request]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run unit tests
        run: cargo test --lib
  
  integration-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Install Git
        run: sudo apt-get install -y git
      - name: Run integration tests
        run: cargo test --test '*'
  
  compliance-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run GRASP-01 compliance tests
        run: cargo test --test compliance
      - name: Generate compliance report
        run: cargo run --example compliance-report > compliance-report.txt
      - name: Upload compliance report
        uses: actions/upload-artifact@v3
        with:
          name: compliance-report
          path: compliance-report.txt
```

## Test Coverage

### Target Coverage

- **Unit Tests**: >80% line coverage
- **Integration Tests**: All critical paths
- **Compliance Tests**: 100% of GRASP-01 requirements
- **E2E Tests**: Key user workflows

### Measuring Coverage

```bash
# Install tarpaulin
cargo install cargo-tarpaulin

# Run with coverage
cargo tarpaulin --out Html --output-dir coverage

# View report
open coverage/index.html
```

## Documentation Testing

### Doc Tests

```rust
/// Parse a pkt-line from Git protocol
///
/// # Examples
///
/// ```
/// use ngit_grasp::git::parse_pkt_line;
///
/// let data = b"0006a\n";
/// let (length, payload) = parse_pkt_line(data).unwrap();
/// assert_eq!(length, 6);
/// assert_eq!(payload, b"a\n");
/// ```
pub fn parse_pkt_line(data: &[u8]) -> Result<(usize, &[u8])> {
    // implementation
}
```

## Summary

This comprehensive test strategy ensures:

1. **Spec Compliance**: Every GRASP requirement has a corresponding test
2. **Reusability**: Compliance tests can validate any GRASP implementation
3. **Clear Failures**: Test failures cite exact spec lines
4. **Comprehensive Coverage**: Unit, integration, compliance, and E2E tests
5. **Maintainability**: Tests mirror spec structure for easy updates

The compliance testing tool is a standalone crate that can be:
- Used by ngit-grasp for self-validation
- Published for other GRASP implementations to use
- Updated as new GRASP specs are released (GRASP-02, GRASP-05)
- Run in CI/CD for continuous compliance verification
