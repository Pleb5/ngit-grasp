# TestContext Pattern Migration Guide

## Overview

The `TestContext` pattern solves the isolation vs. rate-limiting problem for grasp-audit tests by supporting dual-mode operation:

- **CI Mode (Isolated)**: Creates fresh events for each test - full isolation
- **Production Mode (Shared)**: Caches and reuses fixtures - 60-90% fewer events

## Architecture

### Core Components

1. **`FixtureKind`** - Enum defining available fixture types
2. **`ContextMode`** - Enum controlling behavior (Isolated vs Shared)
3. **`TestContext<'a>`** - Mode-aware fixture manager with caching

### Files Modified

- [`grasp-audit/src/fixtures.rs`](../grasp-audit/src/fixtures.rs) - New file with TestContext implementation
- [`grasp-audit/src/lib.rs`](../grasp-audit/src/lib.rs) - Exports new types
- [`grasp-audit/src/specs/grasp01/event_acceptance_policy.rs`](../grasp-audit/src/specs/grasp01/event_acceptance_policy.rs) - Example migrations

## Migration Strategy

### Step 1: Identify Prerequisite Events

Look for tests that create prerequisite events (repos, issues, etc.) before testing the actual functionality.

**Before:**

```rust
async fn test_accept_issue_via_a_tag(client: &AuditClient) -> TestResult {
    // 1. Create and send repo announcement
    let repo = Self::create_test_repo(client, "test-repo-1").await?;
    Self::send_and_verify_accepted(client, repo.clone(), "repository announcement").await?;

    // 2. Create issue that references the repo
    let issue = Self::create_issue_for_repo(client, &repo, "Test Issue 1")?;

    // 3. Test actual functionality
    Self::send_and_verify_accepted(client, issue, "issue via 'a' tag").await?;
    Ok(())
}
```

### Step 2: Replace with TestContext

**After:**

```rust
async fn test_accept_issue_via_a_tag(client: &AuditClient) -> TestResult {
    // 1. Create TestContext
    let ctx = TestContext::new(client);

    // 2. Get repository fixture (mode-aware)
    let repo = ctx.get_fixture(FixtureKind::ValidRepo).await?;

    // 3. Create issue and test actual functionality
    let issue = Self::create_issue_for_repo(client, &repo, "Test Issue 1")?;
    Self::send_and_verify_accepted(client, issue, "issue via 'a' tag").await?;
    Ok(())
}
```

### Step 3: Add Imports

At the top of your test file:

```rust
use crate::{TestContext, FixtureKind};
```

## Available Fixtures

### Current Fixture Types

1. **`FixtureKind::ValidRepo`** - Basic repository announcement (kind 30617)
2. **`FixtureKind::RepoWithIssue`** - Repository with one issue (kind 1621)
3. **`FixtureKind::RepoWithComment`** - Repository with issue and comment (kind 1111)
4. **`FixtureKind::RepoState`** - Repository state announcement (kind 30618)

### Adding New Fixtures

To add a new fixture type:

1. Add variant to `FixtureKind` enum:

```rust
pub enum FixtureKind {
    // ... existing variants
    NewFixtureType,
}
```

2. Add case to `build_fixture` method:

```rust
async fn build_fixture(&self, kind: FixtureKind) -> Result<Event> {
    match kind {
        // ... existing cases
        FixtureKind::NewFixtureType => {
            // Create and return event
        }
    }
}
```

## Event Count Comparison

### Before Migration (All Tests)

All modes send the same number of events:

- 15 tests × ~3 events each = **~45 events total**

### After Migration

**CI Mode (Isolated):**

- Still ~45 events (maintains full isolation)

**Production Mode (Shared):**

- Initial setup: ~5 events (one per fixture type)
- Subsequent tests: Reuse cached fixtures
- Total: **~5-35 events (60-90% reduction)**

## Mode-Specific Behavior

### CI Mode (Default for Tests)

```rust
let config = AuditConfig::ci();
let client = AuditClient::new("ws://localhost:7000", config).await?;
let ctx = TestContext::new(&client);

// Always creates fresh fixture
let repo1 = ctx.get_fixture(FixtureKind::ValidRepo).await?;
let repo2 = ctx.get_fixture(FixtureKind::ValidRepo).await?;
assert_ne!(repo1.id, repo2.id); // Different IDs - fresh events
```

### Production Mode (CLI Default)

```rust
let config = AuditConfig::production();
let client = AuditClient::new("ws://localhost:7000", config).await?;
let ctx = TestContext::new(&client);

// Returns cached fixture on second call
let repo1 = ctx.get_fixture(FixtureKind::ValidRepo).await?;
let repo2 = ctx.get_fixture(FixtureKind::ValidRepo).await?;
assert_eq!(repo1.id, repo2.id); // Same ID - reused event
```

## Testing the Migration

### Run Refactored Tests

```bash
# Using test-ngit-relay.sh (recommended)
cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test

# Manual testing
RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_accept_issue_via_a_tag -- --ignored --nocapture
```

### Verify Event Counts

Monitor event publication in relay logs:

```bash
# Count events sent during test run
RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib -- --ignored --nocapture 2>&1 | grep -c "EVENT"
```

## Best Practices

### 1. Use TestContext for Prerequisites Only

✅ **Good:** Use TestContext for setup events

```rust
let ctx = TestContext::new(client);
let repo = ctx.get_fixture(FixtureKind::ValidRepo).await?;
let test_event = create_custom_event(&repo)?; // Test-specific event
```

❌ **Bad:** Don't use for events you're actually testing

```rust
// Wrong - you want to test THIS event, not reuse it
let issue = ctx.get_fixture(FixtureKind::RepoWithIssue).await?;
```

### 2. Error Handling

Never use use `.map_err(|e| e.to_string())?` to convert anyhow errors accept for final display but instead use the error:

❌ **Bad:** Don't use `.map_err(|e| e.to_string())?` to convert anyhow errors unless displaying.

```rust
let repo = ctx.get_fixture(FixtureKind::ValidRepo).await
    .map_err(|e| e.to_string())?;
```

### 3. Clear Cache When Needed

For tests that modify fixtures:

```rust
let ctx = TestContext::new(client);
// ... test that modifies state ...
ctx.clear_cache(); // Ensure fresh fixtures for next test
```

### 4. Document Mode Behavior

Add comments explaining mode-specific behavior:

```rust
// NEW: Request repository fixture - behavior depends on mode
// CI mode: Creates fresh repo for this test
// Production mode: Returns cached repo if available
let repo = ctx.get_fixture(FixtureKind::ValidRepo).await?;
```

## Migration Checklist

For each test:

- [ ] Identify prerequisite events (repos, issues, etc.)
- [ ] Determine appropriate `FixtureKind`
- [ ] Add `TestContext` imports
- [ ] Replace manual event creation with `ctx.get_fixture()`
- [ ] Add `.map_err(|e| e.to_string())?` for error handling
- [ ] Add mode-behavior comments
- [ ] Verify test still passes in CI mode
- [ ] Test in production mode (optional verification)

## Examples

### Example 1: Simple Repository Prerequisite

See [`test_accept_issue_via_a_tag`](../grasp-audit/src/specs/grasp01/event_acceptance_policy.rs:513-530) for a complete example.

### Example 2: Complex State Setup

See [`test_accept_valid_repo_state_announcement`](../grasp-audit/src/specs/grasp01/event_acceptance_policy.rs:354-397) for state announcement example.

## Troubleshooting

### Tests Failing in Production Mode

If tests fail when reusing fixtures, the test may be:

1. Modifying shared state
2. Depending on unique event IDs
3. Testing fixture creation itself (should use CI mode)

**Solution:** Either fix the test or use `ContextMode::Isolated` explicitly:

```rust
let ctx = TestContext::with_mode(client, ContextMode::Isolated);
```

## Future Work

- [ ] Migrate remaining tests (gradual migration)
- [ ] Add more fixture types as needed
- [ ] Add fixture cleanup strategies
- [ ] Add metrics for event count reduction

## References

- [`fixtures.rs`](../grasp-audit/src/fixtures.rs) - TestContext implementation
- [`event_acceptance_policy.rs`](../grasp-audit/src/specs/grasp01/event_acceptance_policy.rs) - Migration examples
- [Original proposal](./testcontext-pattern-proposal.md) - Design rationale
