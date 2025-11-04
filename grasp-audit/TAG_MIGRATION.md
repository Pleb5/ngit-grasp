# Tag Migration to Standard NIP-01 "t" Tags

**Date:** November 4, 2025  
**Status:** ✅ Complete

## Overview

Migrated audit system tags from custom single-letter tags (`g`, `r`, `c`) to standard NIP-01 "t" tags (hashtags) to avoid conflicts and follow Nostr conventions.

## Motivation

The previous tag scheme used:
- `g` tag for `grasp-audit` marker
- `r` tag for `audit-run-id`
- `c` tag for `audit-cleanup` timestamp

However, this could conflict with other uses of these single-letter tags. The "t" tag is the standard NIP-01 tag type for categorization/topics, making it the appropriate choice for audit event tagging.

## Changes Made

### Tag Structure

**Before:**
```rust
vec![
    Tag::custom(TagKind::SingleLetter(g_tag), vec!["grasp-audit"]),
    Tag::custom(TagKind::SingleLetter(r_tag), vec![run_id]),
    Tag::custom(TagKind::SingleLetter(c_tag), vec![cleanup_timestamp]),
]
```

**After:**
```rust
vec![
    Tag::custom(TagKind::SingleLetter(t_tag), vec!["grasp-audit-test-event"]),
    Tag::custom(TagKind::SingleLetter(t_tag), vec![format!("audit-{}", run_id)]),
    Tag::custom(TagKind::SingleLetter(t_tag), vec![format!("audit-cleanup-after-{}", timestamp)]),
]
```

### Tag Values

| Purpose | Old Tag | Old Value | New Tag | New Value |
|---------|---------|-----------|---------|-----------|
| Marker | `g` | `grasp-audit` | `t` | `grasp-audit-test-event` |
| Run ID | `r` | `ci-{uuid}` | `t` | `audit-ci-{uuid}` |
| Cleanup | `c` | `{timestamp}` | `t` | `audit-cleanup-after-{timestamp}` |

### Example Event Tags

```json
[
  ["t", "grasp-audit-test-event"],
  ["t", "audit-ci-a1b2c3d4-e5f6-7890-abcd-ef1234567890"],
  ["t", "audit-cleanup-after-1730707200"]
]
```

## Files Modified

### `src/audit.rs`
- Updated `audit_tags()` to use "t" tags
- Updated tests to check for "t" tag kind
- All values now prefixed for clarity

### `src/client.rs`
- Updated `query()` to filter by "t" tags
- Changed from `.custom_tag(g_tag, ...)` to `.custom_tag(t_tag, ...)`

## Benefits

1. **Standards Compliance**: Uses standard NIP-01 hashtag mechanism
2. **No Conflicts**: "t" tag is designed for categorization
3. **Better Namespacing**: Values prefixed with `audit-` to avoid collisions
4. **Queryable**: Standard tag filtering works as expected
5. **Self-Documenting**: Tag values clearly indicate their purpose

## Testing

All tests pass with the new tag scheme:

```bash
# Unit tests
✓ 12/12 tests passing

# Integration tests  
✓ 1/1 test passing (NIP-01 smoke tests)

# CLI verification
✓ All 6 smoke tests pass
```

## Backwards Compatibility

⚠️ **Breaking Change**: Events created with old tags will not be found by new queries.

This is acceptable because:
- System is in alpha/development
- Old events are test data only
- Cleanup happens automatically via timestamps
- No production deployments exist yet

## Migration Path

For future tag changes:
1. Consider versioning in tag values (e.g., `grasp-audit-v2-test-event`)
2. Support querying both old and new tags during transition
3. Document breaking changes clearly
4. Provide migration tools if needed

## References

- **NIP-01**: https://github.com/nostr-protocol/nips/blob/master/01.md
- **Tag Standardization**: "t" tags for topics/categories
- **Previous Implementation**: Commit `8190a3a` (custom g/r/c tags)
- **Current Implementation**: Uses standard "t" tags

## Verification

To verify the new tag structure:

```bash
# Run tests
nix develop -c cargo test --lib
nix develop -c cargo test -- --ignored

# Run CLI
nix develop -c cargo run -- audit \
  --relay ws://localhost:7000 \
  --mode ci \
  --spec nip01-smoke

# Check event structure (example)
# Events will have tags like:
# ["t", "grasp-audit-test-event"]
# ["t", "audit-ci-{uuid}"]  
# ["t", "audit-cleanup-after-{timestamp}"]
```

## Next Steps

- [ ] Update documentation to reflect new tag scheme
- [ ] Consider adding tag validation helpers
- [ ] Document tag format in API/spec documentation
- [ ] Add examples showing tag usage

---

**Status:** ✅ Migration complete and verified  
**All tests passing:** 13/13 (12 unit + 1 integration)  
**CLI verified:** ✅ Working correctly
