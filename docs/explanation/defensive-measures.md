# Defensive Measures & Rate Limiting

This document describes the defensive measures implemented in ngit-grasp to protect against abuse, spam, and denial-of-service attacks.

## Overview

ngit-grasp employs multiple layers of defense:

1. **Connection & Subscription Limits** - Per-connection limits on subscriptions and event publishing
2. **Content Filtering** - Blacklist/whitelist system for repositories and event authors
3. **Event Validation** - Strict GRASP-01 protocol validation
4. **Relay Health Management** - Intelligent handling of problematic remote relays

## What's Implemented

### Per-Connection Rate Limits

**Source:** Built-in to rust-nostr relay-builder

- **Subscription limit:** Max 500 concurrent subscriptions per connection
- **Event publishing limit:** Max 60 events per minute per connection
- **Subscription ID length:** Max 250 characters
- **Filter limit:** Max 500 results per query (default)

These limits prevent individual connections from overwhelming the relay.

### Per-IP Connection Monitoring

**Source:** Custom ngit-grasp implementation  
**Location:** `src/metrics/connection.rs`

- **Status:** Monitoring only (does NOT enforce limits)
- Tracks connections per IP address internally
- Flags IPs exceeding threshold (default: 10 connections)
- **Privacy:** IP addresses never exposed in Prometheus metrics, only aggregate counts
- Logs warnings when threshold exceeded

**Future:** Could be extended to enforce per-IP connection limits.

### Content Filtering (Blacklists/Whitelists)

**Source:** Custom ngit-grasp implementation  
**Location:** `src/config.rs`, `src/nostr/builder.rs`

**Event Blacklist:**
- Block ALL events from specific authors (npubs)
- Takes precedence over all other validation
- Events never reach storage or purgatory

**Repository Blacklist:**
- Block specific repositories, developers, or identifiers
- Takes precedence over whitelists
- Three formats: `npub`, `npub/identifier`, `identifier`

**Repository Whitelist:**
- Curate which repositories are accepted (GRASP-01 mode)
- Only accept announcements that both list your service AND match whitelist
- Same three formats as blacklist

**Archive Whitelist (GRASP-05):**
- Mirror specific repositories even if they don't list your service
- Same three formats as blacklist
- Default: read-only mode when enabled

**Privacy:** Blacklists not advertised in NIP-11 metadata.

### Event Validation Plugin System

**Source:** Built-in to rust-nostr relay-builder  
**Implementation:** Custom GRASP-01 validation in `src/nostr/builder.rs`

- **WritePolicy trait:** Controls which events are accepted
- **QueryPolicy trait:** Controls which queries are allowed (not currently used)
- Access to client IP address for future per-IP rate limiting
- Modular sub-policies for different event types (announcements, state events, PRs)

### Relay Health Management (GRASP-02 Sync)

**Source:** Custom ngit-grasp implementation  
**Location:** `src/sync/health.rs`

**Exponential Backoff:**
- Failed connections trigger increasing delays: 5s → 10s → 20s → ... → 1 hour max
- Prevents hammering dead or slow relays

**Naughty List:**
- Tracks relays with persistent infrastructure issues (DNS, TLS, protocol errors)
- Separate from normal connection failures
- 12-hour expiration (configurable)
- Reduces retry frequency for broken relays

**Rate Limit Detection:**
- Detects when remote relay rate limits us
- Automatic 65-second cooldown
- Prevents hammering relays that tell us to slow down

**Domain Throttling (Git Data Fetching):**
- Max 5 concurrent requests per domain
- Max 30 requests per minute per domain
- Respectful rate limiting when fetching missing git data

## What's NOT Implemented

### Per-IP Rate Limiting

- **Per-IP connection limits:** Not enforced (only monitored)
- **Per-IP subscription limits:** Not supported
- **Per-IP event publishing limits:** Not supported

**Why:** rust-nostr relay-builder tracks limits per WebSocket connection, not per IP address.

**To implement:** Would require custom middleware/WritePolicy to aggregate across connections from the same IP.

### Total Connection Limit

**Status:** Supported by relay-builder but not currently configured in ngit-grasp.

**To implement:** Add `max_connections(n)` to relay builder configuration.

### Query Filtering

**Status:** QueryPolicy trait available but not currently used.

**Potential uses:** Rate limit queries per IP, block expensive queries, restrict access to certain event kinds.

## Future Enhancements: Per-IP Rate Limiting (Deferred)

### Decision: Defer Until Abuse Detected

After comprehensive review (2026-01-14), we decided to defer per-IP rate limiting (Phase 2 & 3) until abuse patterns are detected in production.

**Current protection (Phase 1):**
- Per-connection limits: 500 subscriptions, 60 events/min
- Total connection limit: 500 (configurable via `NGIT_MAX_CONNECTIONS`)
- Connection monitoring: Tracks IPs, flags abuse at 10 connections
- Content filtering: Event blacklist, repository blacklist/whitelist

**Deferred features (Phase 2 & 3):**
- Per-IP connection enforcement (reject after 10 connections)
- Per-IP event rate limiting (reject after 100 events/min)

### Rationale for Deferral

1. **Config-only approach sufficient** - Total connection limit addresses primary DoS vector
2. **Git relay context** - Developer users less likely to abuse than general public
3. **Existing protections strong** - Per-connection limits + content filtering already robust
4. **Data-driven approach** - Monitor ConnectionTracker metrics, implement if needed
5. **Minimal maintenance** - Avoid custom rate limiting code until proven necessary

### Implementation Path if Needed

**Preferred approach:** Contribute to rust-nostr/relay-builder as PR
- Propose IP-based rate limiting as optional feature
- Let upstream maintain the code
- Benefits entire Nostr ecosystem

**Fallback:** Implement in ngit-grasp
- Per-IP connection enforcement via actix middleware
- Per-IP event rate limiting via token bucket in WritePolicy
- See issue d6ee for detailed implementation plan

### Monitoring for Abuse

Watch these metrics to determine if Phase 2 is needed:
- `ngit_connections_per_ip` - IPs exceeding 10 connections
- `ngit_flagged_abusers` - IPs flagged by ConnectionTracker
- Event publishing patterns from single IPs

**Trigger for Phase 2:** If abuse detected for 2-4 weeks after Phase 1 deployment

### Related Work

**Git endpoint throttling:** Separate concern, tracked in issue ff38
- Git HTTP endpoints have different threat model (bandwidth/CPU intensive)
- Requires separate IP-based throttling (5 concurrent, 30/min per IP)
- No interaction with relay code

## Summary Table

| Feature | Status | Enforced? | Configurable? |
|---------|--------|-----------|---------------|
| **Per-Connection Limits** |
| Max subscriptions (500) | ✅ Active | Yes | No (relay-builder default) |
| Event rate limit (60/min) | ✅ Active | Yes | No (relay-builder default) |
| **Total Connection Limit** |
| Max connections (500) | ✅ Active | Yes | Yes (`NGIT_MAX_CONNECTIONS`) |
| **Per-IP Monitoring** |
| Connection tracking | ✅ Active | No (monitor only) | Threshold only |
| **Content Filtering** |
| Event blacklist | ✅ Active | Yes | Yes |
| Repository blacklist | ✅ Active | Yes | Yes |
| Repository whitelist | ✅ Active | Yes (if set) | Yes |
| Archive whitelist | ✅ Active | Yes (if set) | Yes |
| **Event Validation** |
| GRASP-01 validation | ✅ Active | Yes | Via WritePolicy |
| **Relay Sync Protection** |
| Exponential backoff | ✅ Active | Yes | Yes |
| Naughty list | ✅ Active | Yes | Yes (12h default) |
| Rate limit detection | ✅ Active | Yes | Automatic |
| Domain throttling | ✅ Active | Yes | Hardcoded (5/30) |
| **Deferred (Phase 2)** |
| Per-IP connection limit | ⚠️ Deferred | No | - |
| Per-IP rate limiting | ⚠️ Deferred | No | - |
| Query filtering | ⚠️ Available | No | Not implemented |

## Related Documentation

- [Configuration Reference](../reference/configuration.md) - All config options for defensive features
- [Monitoring Overview](monitoring.md) - Prometheus metrics for tracking abuse
- [GRASP-05 Archive](grasp-05-archive.md) - Archive whitelist details
- [Architecture](architecture.md) - Overall system design
