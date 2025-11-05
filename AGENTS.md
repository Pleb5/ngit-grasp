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
RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_grasp01_nostr_relay_against_relay -- --ignored --nocapture
```

Tests marked `#[ignore]` need relay - unit tests don't.

**Note:** Always use a random available port for the relay to avoid conflicts with existing services.

### Running Single Test

```bash
# From grasp-audit/
nix develop -c cargo test --lib specific_test_name -- --nocapture
```

### Quick Test Verification

To verify GRASP-01 compliance tests are working correctly:

```bash
cd grasp-audit && RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_grasp01_nostr_relay_against_relay -- --ignored --nocapture 2>&1 | tail -60
```

**Expected Output:**
- 2-3 tests passing
- 15 tests showing "Not implemented yet"

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

## File Restrictions by Mode

Code mode can only edit files matching specific patterns (enforced by system):

- Example: Architect mode restricted to `\.md$` files only
- Attempting to edit restricted files causes FileRestrictionError
- Check mode configuration if edit attempts fail unexpectedly

## Quick Reference

```bash
# Build grasp-audit
cd grasp-audit && nix develop -c cargo build

# Run all tests (requires relay running, set RELAY_URL to match your port)
cd grasp-audit && RELAY_URL="ws://localhost:18081" nix develop -c cargo test --ignored

# Run single test
cd grasp-audit && nix develop -c cargo test --lib test_name -- --nocapture

# Verify GRASP-01 tests (cleaner output)
cd grasp-audit && RELAY_URL="ws://localhost:18081" nix develop -c cargo test --lib test_grasp01_nostr_relay_against_relay -- --ignored --nocapture 2>&1 | tail -60

# Check session files
ls work/  # Should only have README.md when clean
```
