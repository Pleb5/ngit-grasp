# nostr-relay-builder Limits

This document describes the rate limiting, throttling, and query limits in `nostr-relay-builder` version 0.44. These are the limits that apply to ngit-grasp and any relay built with this crate.

**Note:** Other relay implementations (strfry, nostream, etc.) have different limits. This document focuses on `nostr-relay-builder` specifically.

## Hard Limits (Cannot Be Changed)

These limits are enforced and cannot be overridden by configuration:

### WebSocket Message Limits (from tungstenite)

| Limit | Default Value | Source |
|-------|--------------|--------|
| `max_message_size` | **64 MB** (67,108,864 bytes) | tungstenite default |
| `max_frame_size` | **16 MB** (16,777,216 bytes) | tungstenite default |

nostr-relay-builder does **not** override these tungstenite defaults.

**Practical impact:** A single REQ message or EVENT with extremely large content could hit these limits. A filter with ~1,000,000 32-byte event IDs (~32MB in JSON) would fit, but 2 million would not.

### Negentropy Frame Limit

| Limit | Value | Source |
|-------|-------|--------|
| `frame_size_limit` | **60,000 bytes** (60KB) | Hardcoded in inner.rs |

```rust
let mut negentropy = Negentropy::owned(storage, 60_000)?;
```

If reconciliation needs more data, negentropy splits across multiple NEG-MSG round-trips automatically.

### No Hard Limits On

nostr-relay-builder does **NOT** enforce hard limits on:

| Item | Hard Limit? | Notes |
|------|-------------|-------|
| Tag values per filter (`#e`, `#p`, etc.) | ❌ None | Only limited by message size |
| Filters per REQ array | ❌ None | Only limited by message size |
| Filter JSON size | ❌ None | Only limited by WebSocket message |
| Authors per filter | ❌ None | Only limited by message size |
| Kinds per filter | ❌ None | Only limited by message size |
| IDs per filter | ❌ None | Only limited by message size |

## Configurable Limits (Server-Side)

These limits have defaults but can be configured via `RelayBuilder`:

### Query/Response Limits

| Setting | Default | Builder Method |
|---------|---------|----------------|
| `default_filter_limit` | **500** | `.default_filter_limit(n)` |
| `max_filter_limit` | `None` (no cap) | `.max_filter_limit(n)` |

**Behavior:**
1. If filter has no `limit` field → server applies `default_filter_limit` (500)
2. If filter has `limit > max_filter_limit` → clamped to `max_filter_limit`  
3. If filter has specific `ids` → uses `ids.len()` as limit

### Rate Limiting

| Setting | Default | Description |
|---------|---------|-------------|
| `max_reqs` | **500** | Max active subscriptions per session |
| `notes_per_minute` | **60** | Token bucket rate for EVENT writes |

Rate limiting uses a token bucket: tokens regenerate proportionally over time, each EVENT consumes 1 token.

### Connection/Session Limits

| Setting | Default | Description |
|---------|---------|-------------|
| `max_connections` | `None` | Max concurrent WebSocket connections |
| `max_subid_length` | **250** | Max characters in subscription ID |

## REQ vs Negentropy Limits

| Aspect | REQ (NIP-01) | Negentropy (NIP-77) |
|--------|--------------|---------------------|
| Events returned | Limited (default: 500) | **Unlimited** (all IDs returned) |
| Filter limit applies? | ✅ Yes | ❌ No |
| Returns full events? | ✅ Yes | ❌ No (only EventId + Timestamp) |
| Message size limit | 64MB (WebSocket) | 60KB per frame |

**Why negentropy returns all:** It only returns ~40 bytes per event (ID + timestamp) for set reconciliation. Full events are fetched separately after identifying what's missing.

## Quick Reference

| Limit | Value | Type |
|-------|-------|------|
| **WebSocket message** | 64 MB | Hard (tungstenite) |
| **WebSocket frame** | 16 MB | Hard (tungstenite) |
| **Negentropy frame** | 60 KB | Hard (hardcoded) |
| Tags per filter | **None** | Soft (message size only) |
| Filters per REQ | **None** | Soft (message size only) |
| Events per REQ | 500 | Configurable default |
| Max subscriptions | 500 | Configurable default |
| Write rate | 60/min | Configurable default |

## ngit-grasp Configuration

ngit-grasp uses defaults (no custom limits configured):

```rust
let builder = RelayBuilder::default()
    .database(database.clone())
    .write_policy(write_policy.clone());
```

Additionally, ngit-grasp's memory database limits to **100,000 events** (LMDB has no such limit).

## Related

- [NIP-01: Basic Protocol](https://github.com/nostr-protocol/nips/blob/master/01.md)
- [NIP-77: Negentropy Sync](https://github.com/nostr-protocol/nips/blob/master/77.md)
- [nostr-relay-builder docs](https://docs.rs/nostr-relay-builder)
- [tungstenite WebSocket limits](https://docs.rs/tungstenite/latest/tungstenite/protocol/struct.WebSocketConfig.html)
