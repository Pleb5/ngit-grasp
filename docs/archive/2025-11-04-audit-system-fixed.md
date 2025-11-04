# Audit System Fixed - November 4, 2025

## Summary

Successfully fixed the audit system to work with the relay launched via Docker. All tests now pass (6/6 smoke tests, 12/12 unit tests).

## Issues Fixed

### 1. Tag System Incompatibility ✅

**Problem:** 
- Audit events were using custom multi-letter tags (`grasp-audit`, `audit-run-id`, `audit-cleanup`)
- Nostr Filter API only supports single-letter tags for querying
- This caused filtering to fail - couldn't query our own audit events

**Solution:**
- Changed to single-letter tags:
  - `g` = grasp-audit marker (value: "grasp-audit")
  - `r` = audit run ID (value: unique run ID)
  - `c` = cleanup timestamp (value: Unix timestamp)
- Updated `audit_tags()` in `src/audit.rs` to use `TagKind::SingleLetter`
- Updated `query()` in `src/client.rs` to filter using `SingleLetterTag`

**Files Changed:**
- `grasp-audit/src/audit.rs` - Tag generation and tests
- `grasp-audit/src/client.rs` - Query filtering

### 2. Event Validation Detection ✅

**Problem:**
- `send_event()` wasn't checking if relays rejected events
- Validation tests were failing because we couldn't detect relay rejection
- The `SendEventOutput` has `success` and `failed` fields that weren't being checked

**Solution:**
- Updated `send_event()` to check `output.success` and `output.failed`
- Return error if all relays rejected the event
- This allows validation tests to properly detect when relays reject invalid events

**Files Changed:**
- `grasp-audit/src/client.rs` - Event sending validation

### 3. Connection Stability ✅

**Problem:**
- Previous implementation had a simple 500ms sleep for connection
- Could be unreliable on slow networks

**Solution:**
- Implemented retry loop with 20 attempts (2 seconds total)
- Checks actual connection status via `relays().values().any(|r| r.is_connected())`
- More robust connection establishment

**Files Changed:**
- `grasp-audit/src/client.rs` - Connection retry logic

### 4. Event Query Debugging ✅

**Problem:**
- When events weren't found, no debugging information

**Solution:**
- Added debug output to help diagnose query issues
- Direct client query fallback for troubleshooting
- Event tag inspection

**Files Changed:**
- `grasp-audit/src/specs/nip01_smoke.rs` - Debug output

## Test Results

### Unit Tests: 12/12 ✅
```
test audit::tests::test_ci_config ... ok
test audit::tests::test_production_config ... ok
test audit::tests::test_audit_tags ... ok
test audit::tests::test_audit_event_builder ... ok
test client::tests::test_client_creation ... ok
test client::tests::test_event_builder ... ok
test isolation::tests::test_generate_ci_run_id ... ok
test isolation::tests::test_generate_prod_run_id ... ok
test isolation::tests::test_generate_test_id ... ok
test result::tests::test_audit_result ... ok
test result::tests::test_result_pass ... ok
test result::tests::test_result_fail ... ok
```

### Integration Tests: 6/6 ✅
```
✓ websocket_connection (NIP-01:basic)
  Can establish WebSocket connection to /

✓ send_receive_event (NIP-01:event-message)
  Can send EVENT and receive OK response

✓ create_subscription (NIP-01:req-message)
  Can create subscription with REQ and receive EOSE

✓ close_subscription (NIP-01:close-message)
  Can close subscriptions

✓ reject_invalid_signature (NIP-01:validation)
  Rejects events with invalid signatures

✓ reject_invalid_event_id (NIP-01:validation)
  Rejects events with invalid event IDs
```

### CLI Test: ✅
```bash
cargo run -- audit --relay ws://localhost:7000 --mode ci --spec nip01-smoke
# Result: 6/6 passed (100.0%)
```

## Technical Details

### Tag Format Change

**Before:**
```rust
Tag::custom(
    TagKind::Custom(Cow::Borrowed("grasp-audit")),
    vec!["true"]
)
```

**After:**
```rust
Tag::custom(
    TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::G)),
    vec!["grasp-audit"]
)
```

### Query Filter Change

**Before:**
```rust
filter.custom_tag(
    TagKind::Custom(Cow::Borrowed("grasp-audit")),
    vec!["true"]
)
```

**After:**
```rust
filter.custom_tag(
    SingleLetterTag::lowercase(Alphabet::G),
    "grasp-audit"
)
```

### Event Validation Check

**Before:**
```rust
let output = self.client.send_event(&event).await?;
let event_id = *output.id();
Ok(event_id)
```

**After:**
```rust
let output = self.client.send_event(&event).await?;
let event_id = *output.id();

// Check if any relay rejected the event
if output.success.is_empty() && !output.failed.is_empty() {
    return Err(anyhow!("All relays rejected the event"));
}

Ok(event_id)
```

## Architecture Insights

### Why Single-Letter Tags?

The Nostr protocol's Filter structure uses a `BTreeMap<SingleLetterTag, BTreeSet<String>>` for generic tags. This is defined in nostr-sdk's Filter implementation:

```rust
type GenericTags = BTreeMap<SingleLetterTag, BTreeSet<String>>;
```

Multi-letter tags are supported in events (via `TagKind::Custom`), but they cannot be efficiently queried using the Filter API. The Filter API only provides `custom_tag()` and `custom_tags()` methods that accept `SingleLetterTag`.

This is a deliberate design choice in the Nostr protocol to keep filter queries compact and efficient.

### Why Check success/failed?

The `SendEventOutput` structure provides detailed feedback about which relays accepted or rejected an event:

```rust
pub struct SendEventOutput {
    pub id: EventId,
    pub success: Vec<Url>,  // Relays that accepted
    pub failed: Vec<Url>,   // Relays that rejected
}
```

By checking these fields, we can:
1. Detect when ALL relays reject an event (validation failure)
2. Detect when SOME relays reject an event (partial failure)
3. Provide better error messages to users
4. Make validation tests work correctly

## Next Steps

Now that the audit system is working correctly, we can proceed with:

1. ✅ **Path 1 Complete** - Integration tests verified
2. **Path 2** - Implement GRASP-01 compliance tests
3. **Path 3** - Start building ngit-grasp relay
4. **Path 4** - Parallel development (tests + relay)

## Files Modified

```
grasp-audit/
├── src/
│   ├── audit.rs             # Tag generation, test updates
│   ├── client.rs            # Connection retry, query filtering, validation
│   └── specs/
│       └── nip01_smoke.rs   # Debug output
```

## Commands to Verify

```bash
# Start relay (if not running)
docker run --rm --name nostr-test-relay -p 7000:7000 scsibug/nostr-rs-relay

# Run unit tests
cd grasp-audit
nix develop --command cargo test --lib

# Run integration tests
nix develop --command cargo test -- --ignored

# Run CLI
nix develop --command cargo run -- audit \
  --relay ws://localhost:7000 \
  --mode ci \
  --spec nip01-smoke
```

## Key Learnings

1. **Always check the API constraints** - The Filter API's limitation to single-letter tags was documented but easy to miss
2. **Validate at multiple levels** - Check both client-side (event creation) and server-side (relay response)
3. **Use structured output** - The `SendEventOutput` provides rich information we should use
4. **Test incrementally** - Unit tests → Integration tests → CLI tests
5. **Debug output matters** - Adding debug output helped identify the tag filtering issue

## Status

🟢 **ALL SYSTEMS OPERATIONAL**

- ✅ Build system working
- ✅ Unit tests passing (12/12)
- ✅ Integration tests passing (6/6)
- ✅ CLI functional
- ✅ Tag system fixed
- ✅ Validation detection working
- ✅ Connection stability improved

**Ready for next phase of development!**

---

*Last updated: November 4, 2025*
