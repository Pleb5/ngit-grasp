# AGENTS.md

This file provides guidance to agents when working with code in this repository.

## Project Structure

**Workspace with Two Rust Projects:**

- Root: `ngit-grasp` (main GRASP relay implementation)
- `grasp-audit/`: Separate subproject with own `Cargo.toml` and `flake.nix`

Cannot build grasp-audit from root - must `cd grasp-audit` first.

## Build & Test

### Nix Flakes (Non-Standard)

**CRITICAL:** Use `nix develop`, NOT `nix-shell` (we use flake.nix, not shell.nix)

```bash
# ✅ Correct
cd grasp-audit
nix develop -c cargo build
nix develop -c cargo test

# ❌ Wrong
nix-shell
nix-shell --run "cargo build"
```

### Running Tests

**Integration tests require relay running:**

```bash
# Start ngit-relay first (use any available port to avoid conflicts)
docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest

# From grasp-audit directory, set RELAY_URL to match your port
# Run all ignored tests (includes GRASP-01 and other relay-dependent tests)
RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib -- --ignored --nocapture

# Or run a specific test
RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_grasp01_nostr_relay_against_relay -- --ignored --nocapture
```

Tests marked `#[ignore]` need relay - unit tests don't.

**Note:** Always use a random available port for the relay to avoid conflicts with existing services.

### Standard Testing Process (Recommended)

**Use test-ngit-relay.sh for automated relay management:**

This script handles all relay lifecycle management automatically:
- Starts ngit-relay in isolated Docker container
- Uses random port to avoid conflicts
- Creates isolated temporary directories
- Ensures cleanup on exit (success or failure)
- Supports both audit and test modes

**Basic Usage:**

```bash
# Run cargo test suite (recommended for GRASP-01 development)
cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test

# Run audit CLI tool (for quick validation)
cd grasp-audit && nix develop -c bash test-ngit-relay.sh

# Get help
cd grasp-audit && ./test-ngit-relay.sh --help
```

**Benefits:**
- No manual relay startup required
- Automatic cleanup prevents leftover containers
- Random port selection avoids conflicts
- Consistent environment across all runs
- Proper test isolation

**Note:** Manual relay setup is still available but test-ngit-relay.sh is recommended for development workflows.

### Running Single Test

```bash
# From grasp-audit/
nix develop -c cargo test --lib specific_test_name -- --nocapture
```

### Quick Test Verification

To verify GRASP-01 compliance tests are working correctly:

```bash
# Run all ignored library tests (includes GRASP-01)
cd grasp-audit && RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib -- --ignored --nocapture 2>&1 | tail -60

# Or run specific GRASP-01 test
cd grasp-audit && RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_grasp01_nostr_relay_against_relay -- --ignored --nocapture 2>&1 | tail -60
```

**Expected Output:**
- 2-3 tests passing
- 15+ tests showing "Not implemented yet"

### Troubleshooting

**Buffer Size Errors:**
If you see mpsc channel buffer size panics on first test run, this is usually transient. Simply run the tests again.

**Verify Relay is Running:**
Check if relay is accessible before running tests:
```bash
nak req -l 1 ws://localhost:18081  # Replace port with your chosen port
```

**Port Conflicts:**
Always use a random available port to avoid conflicts with existing services. If a port is busy, choose a different one for docker.

## Code Patterns

### nostr-sdk 0.43 Breaking Changes (vs 0.35)

**Field access, not method calls:**

```rust
// ❌ WRONG (0.35 API)
event.id()
event.tags()
for tag in &event.tags { }

// ✅ CORRECT (0.43 API)
event.id          // Direct field access
event.tags        // Direct field access
event.tags.iter() // Iterator method
```

**Tag API changed:**

```rust
// ❌ WRONG (0.35)
Tag::Generic(TagKind::Custom("clone".into()), vec![...])

// ✅ CORRECT (0.43)
Tag::custom(TagKind::custom("clone"), vec![...])
```

**EventBuilder signature changed:**

```rust
// ❌ WRONG (0.35)
EventBuilder::new(kind, content, &[tags])

// ✅ CORRECT (0.43)
EventBuilder::new(kind, content).tags(tags)
```

See `docs/archive/2025-11-04-nostr-sdk-upgrade.md` for full migration.

### Audit Event Tagging (grasp-audit)

**All audit events automatically include cleanup tags:**

The grasp-audit system automatically adds three tags to every event for production cleanup and test isolation. These tags are added transparently via [`AuditEventBuilder::build()`](grasp-audit/src/audit.rs:120-129) with 100% coverage through [`AuditClient::event_builder()`](grasp-audit/src/client.rs:107-138).

**Automatic Tags (no manual intervention needed):**

```rust
// These tags are automatically added to EVERY audit event:
["t", "grasp-audit-test-event"]              // Identifies all audit test events
["t", "audit-{run_id}"]                      // Unique ID for this audit run (correlates events)
["t", "audit-cleanup-after-{unix_timestamp}"] // Unix timestamp for cleanup scheduling
```

**Tag Format Details:**

- Uses standard NIP-01 `"t"` (hashtag) tags for maximum compatibility
- Unix timestamps (not ISO 8601) for easier database queries
- All tags added automatically when calling `client.event_builder().build()`
- No manual tag management required

**Verifying Tags in Tests:**

```rust
// Test that verifies automatic tag addition:
// See: grasp-audit/src/client.rs:273-302
#[test]
fn test_audit_tags_automatically_added() {
    // Creates event and verifies all three tags are present
}
```

**Testing Implications:**

- All audit events are tagged for easy cleanup
- Use `run_id` tag to correlate events from same audit run
- Tags enable production relay cleanup scripts
- No special handling needed in test code - tags are automatic

## Documentation

**Diátaxis Framework Used:**

- `docs/tutorials/` - Learning-oriented
- `docs/how-to/` - Task-oriented
- `docs/reference/` - Information-oriented
- `docs/explanation/` - Understanding-oriented

**Session files go in `work/` (gitignored except README.md)**

- Archive valuable content to `docs/archive/YYYY-MM-DD-*.md` at session end
- Delete temporary files
- Keep root clean (only README.md, AGENTS.md)

## Critical Gotchas

1. **Workspace compilation:** Can't `cargo build` from root for grasp-audit
2. **Nix environment:** Must use `nix develop`, not `nix-shell`
3. **nostr-sdk API:** Fields not methods in 0.43
4. **Test isolation:** Integration tests need relay, marked with `#[ignore]`
5. **Work directory:** All session docs go in `work/`, NOT root
6. **Archive naming:** Use `YYYY-MM-DD-description.md` format
7. **Use test-ngit-relay.sh**: Always use the test script for GRASP-01 tests - it handles cleanup and port management automatically

## File Restrictions by Mode

Code mode can only edit files matching specific patterns (enforced by system):

- Example: Architect mode restricted to `\.md$` files only
- Attempting to edit restricted files causes FileRestrictionError
- Check mode configuration if edit attempts fail unexpectedly

## Quick Reference

```bash
# Recommended: Use test-ngit-relay.sh for all testing
cd grasp-audit && nix develop -c bash test-ngit-relay.sh --mode test

# Build grasp-audit
cd grasp-audit && nix develop -c cargo build

# Manual relay testing (if needed)
# 1. Start relay: docker run --rm -p 18081:8081 ghcr.io/danconwaydev/ngit-relay:latest
# 2. Run all ignored tests: RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib -- --ignored --nocapture
# 3. Or specific test: RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_grasp01_nostr_relay_against_relay -- --ignored --nocapture

# Run single test
cd grasp-audit && nix develop -c cargo test --lib test_name -- --nocapture

# Check session files
ls work/  # Should only have README.md when clean
```
