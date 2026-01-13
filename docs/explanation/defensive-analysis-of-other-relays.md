# Defensive Analysis of Other Nostr Relays

**Issue:** d6ee - Defensive Relay Features  
**Date:** 2026-01-13  
**Purpose:** Research findings on rate limiting and defensive features in major Nostr relay implementations to inform ngit-grasp's defensive strategy.

## Executive Summary

This analysis examines how three major Nostr relay implementations (strfry, nostr-rs-relay, and khatru) handle rate limiting, connection management, and DoS protection. The goal is to identify industry best practices and concrete defaults to implement in ngit-grasp.

**Key Finding:** Most relays have VERY permissive defaults or no limits at all, relying on operators to configure appropriately or use external reverse proxies. Only khatru provides opinionated secure-by-default settings.

## Current State of ngit-grasp

### Existing Defensive Features ✅

#### 1. Connection Tracking & Abuse Detection
**Location:** `src/metrics/connection.rs`

- Per-IP connection counting via `ConnectionTracker`
- Abuse threshold detection (default: 10 connections per IP)
- Privacy-preserving metrics (IPs never exposed to Prometheus)
- Tracks: total connections, unique IPs, flagged abusers

**Configuration:**
```rust
// src/config.rs:366-372
pub metrics_connection_per_ip_abuse_threshold: u32 = 10
```

**Limitations:**
- ⚠️ **Display-only** - Detection happens but no enforcement
- ⚠️ No connection limit enforcement
- ⚠️ No per-IP subscription limits
- ⚠️ No time-based rate limits

#### 2. Git Remote Throttling (Purgatory Sync)
**Location:** `src/purgatory/sync/throttle.rs`

- Sophisticated domain-based rate limiting for outbound git fetch requests
- Per-domain concurrent request limits (default: 5)
- Per-domain rate limits (default: 30 requests/minute)
- Round-robin queue management for fairness
- Sliding window implementation

**How it works:**
```rust
// Lines 159: Default throttle manager creation
let throttle_manager = Arc::new(ThrottleManager::new(5, 30));

// Lines 96-106: DomainThrottle tracks concurrent and rate limits
pub fn new(domain: String, max_concurrent: u32, max_per_minute: u32)

// Lines 113-129: Checks both limits before allowing requests
```

**Note:** Only applies to **outbound** git fetches, not incoming client connections.

#### 3. Event Blacklisting
**Location:** `src/config.rs` (lines 247-281, 658-668), `src/nostr/builder.rs` (lines 75-86, 495-505)

- Event author blacklist - Block all events from specific npubs
- Repository blacklist - Block announcements for specific repos/identifiers/npubs
- Blacklist checked FIRST in write policy (overrides everything)

**Configuration:**
```bash
NGIT_EVENT_BLACKLIST="npub1...,npub2..."
NGIT_REPOSITORY_BLACKLIST="npub1.../identifier,identifier"
```

#### 4. Naughty List for Problematic Git Remotes
**Location:** `src/sync/naughty_list.rs`

- Tracks git remote domains with persistent infrastructure errors
- Classifies errors (DNS, TLS, protocol, WebSocket)
- Temporary blacklisting with expiration (default: 12 hours)
- Used to skip unreliable relays during sync

#### 5. Metrics & Monitoring
**Location:** `src/metrics/mod.rs`

- WebSocket connection metrics (total, duration, messages by type)
- Git operation tracking (clone, fetch, push by status)
- Nostr event metrics (received, stored, rejected by kind and reason)
- Sync metrics (connections, attempts, failures)
- Repository count tracking

**No enforcement capabilities** - purely observability.

### What's Missing ❌

1. **WebSocket Connection Limits** - No global or per-IP enforcement
2. **Subscription (REQ) Limits** - Clients can open unlimited REQs
3. **Event Rate Limiting** - No per-client/per-IP limits
4. **HTTP Endpoint Protection** - All endpoints unprotected
5. **Message Size Limits** - No WebSocket/event size caps
6. **Rate Limiting Crates** - No dependencies available

### Integration Points for Implementation

#### WebSocket Connection Accept Point
**Location:** `src/http/mod.rs:402-424`

```rust
tokio::spawn(async move {
    match hyper::upgrade::on(req).await {
        Ok(upgraded) => {
            // Track connection
            m.connection_tracker().on_connect(addr.ip());
            // ⬅️ COULD ADD: Check connection limits here
            
            relay.take_connection(TokioIo::new(upgraded), addr).await
            
            m.connection_tracker().on_disconnect(addr.ip());
        }
    }
});
```

This is the ideal location to add connection limit enforcement before accepting the WebSocket upgrade.

## Analysis of Other Relays

### 1. strfry (C++, by hoytech)

**Repository:** https://github.com/hoytech/strfry  
**Stars:** 623 | **Focus:** High performance, custom LMDB schema

#### Configuration File: `strfry.conf`

##### Event Limits
```conf
maxEventSize = 65536              # 64 KB - Maximum normalized JSON size
maxNumTags = 2000                 # Maximum number of tags allowed
maxTagValSize = 1024              # 1 KB - Maximum tag value size
rejectEventsNewerThanSeconds = 900    # 15 minutes - Reject future events
rejectEventsOlderThanSeconds = 94608000  # ~3 years - Reject old events
rejectEphemeralEventsOlderThanSeconds = 60  # 60s - Ephemeral cutoff
ephemeralEventsLifetimeSeconds = 300    # 5 minutes - Ephemeral retention
```

##### Connection & WebSocket Limits
```conf
maxWebsocketPayloadSize = 131072  # 128 KB - Max WebSocket frame size
nofiles = 1000000                 # OS-limit on max open files/sockets
autoPingSeconds = 55              # WebSocket PING frequency
```

##### Query & Subscription Limits
```conf
maxReqFilterSize = 200            # Max filters allowed in a REQ
maxSubsPerConnection = 20         # Max concurrent subscriptions per connection
maxFilterLimit = 500              # Max records returned per filter
queryTimesliceBudgetMicroseconds = 10000  # 10ms - Max CPU per query timeslice
```

##### Thread Pool Configuration
```conf
ingester = 3      # Route incoming requests, validate events/sigs
reqWorker = 3     # Handle initial DB scan for events
reqMonitor = 3    # Handle filtering of new events
negentropy = 2    # Handle negentropy protocol messages
```

##### Compression
```conf
compression:
  enabled = true         # permessage-deflate compression
  slidingWindow = true   # Maintains sliding window (better compression, more memory)
```

#### Implementation Approach

**Architecture Highlights:**
- **No explicit per-IP rate limiting** in config - relies on external reverse proxy or plugin system
- **Query pause/resume**: Long-running queries can be paused (stored as few hundred to few thousand bytes) and resumed when socket buffer drains
- **Query prioritization**: New queries processed before resuming queries that already ran >10ms
- **LMDB-based**: Zero-copy access from page cache, read path requires no locking
- **Batching**: Events written in batches with single fsync for efficiency
- **Plugin system**: External programs (any language) can implement write policies via line-based JSON interface

**Rate Limiting Strategy:**
- Delegates to external plugins for event acceptance policies
- Relies on reverse proxy (nginx, etc.) for connection-level rate limiting
- Focus on efficient query handling rather than built-in rate limits

**Strengths:**
- Extremely high performance
- Sophisticated query engine with pause/resume
- Flexible plugin system

**Weaknesses:**
- No built-in connection or event rate limiting
- Requires external infrastructure for DoS protection
- More complex to deploy securely

---

### 2. nostr-rs-relay (Rust, by scsibug/gheartsfield)

**Repository:** https://git.sr.ht/~gheartsfield/nostr-rs-relay  
**Focus:** Rust implementation with SQLite or PostgreSQL backend

#### Configuration File: `config.toml`

##### Rate Limiting
```toml
# DEFAULT: 0 (unlimited) - Events created per second (server-wide, averaged over 1 minute)
# RECOMMENDED: Set to low value like 5 for public relays
messages_per_sec = 0

# DEFAULT: 0 (unlimited) - Client subscriptions created (averaged over 1 minute)
# RECOMMENDED: Set to low value like 10
subscriptions_per_min = 0

# DEFAULT: 0 (unlimited) - Concurrent DB connections per client
db_conns_per_client = 0
```

##### Event & Message Size Limits
```toml
max_event_bytes = 131072       # 128 KB - Maximum EVENT message size
max_ws_message_bytes = 131072  # 128 KB - Maximum WebSocket message
max_ws_frame_bytes = 131072    # 128 KB - Maximum WebSocket frame
```

##### Buffering & Backpressure
```toml
broadcast_buffer = 16384       # Buffer for subscribers (prevents slow readers consuming memory)
event_persist_buffer = 4096    # Buffer for DB commits (provides backpressure if DB writes slow)
max_blocking_threads = 16      # Limit blocking threads for DB connections
```

##### Time-based Restrictions
```toml
# Reject events with timestamps this far in future
# RECOMMENDED: 30 minutes, but defaults to allowing any date if not set
reject_future_seconds = 1800   # 30 minutes
```

##### Connection Pool
```toml
min_conn = 4        # Minimum reader connections
max_conn = 8        # Maximum reader connections (recommended: approx number of cores)
```

##### WebSocket
```toml
ping_interval = 300  # 5 minutes - WebSocket ping interval
```

##### Event Kind Filtering
```toml
# Optional - Specific event kinds to discard
event_kind_blacklist = []

# Optional - Only accept these event kinds
event_kind_allowlist = []

# Rejects imprecise requests (kind-only, author-only) to improve outbox model adoption
limit_scrapers = false
```

#### Implementation Approach

**Architecture Highlights:**
- **Tokio async runtime**: Non-blocking I/O
- **SQLite or PostgreSQL**: Configurable database backend
- **gRPC plugin support**: External authorization service via `event_admission_server`
- **Rate limiting**: Averaged over time windows (1 minute), applied server-wide
- **No per-IP limits by default**: Relies on configuration or external proxy

**Rate Limiting Strategy:**
- Provides configuration options but defaults to UNLIMITED
- Operators MUST configure limits for production use
- Time-window averaging (1 minute) for rate calculations
- Server-wide limits, not per-IP

**Strengths:**
- Well-documented configuration options
- Flexible database backends
- Buffer-based backpressure mechanism

**Weaknesses:**
- **Dangerously permissive defaults** - unlimited by default
- No per-IP rate limiting built-in
- Requires active operator configuration for security

---

### 3. khatru (Go framework, by fiatjaf)

**Repository:** https://github.com/fiatjaf/khatru  
**Stars:** 133 | **Focus:** Framework for custom relays, not a standalone relay

#### Default Configuration (from `relay.go` and `policies/`)

##### Built-in Defaults (NewRelay)
```go
ReadBufferSize: 1024      // bytes
WriteBufferSize: 1024     // bytes
WriteWait: 10 * time.Second      // Time allowed to write message to peer
PongWait: 60 * time.Second       // Time allowed to read next pong from peer
PingPeriod: 30 * time.Second     // Send pings with this period (must be < PongWait)
MaxMessageSize: 512000    // ~500 KB - Maximum message size from peer
```

##### Sane Defaults Policy (`ApplySaneDefaults`)

**Event Rate Limiting:**
```go
EventIPRateLimiter(
  tokensPerInterval: 2,     // events
  interval: 180,            // 3 minutes (180 seconds)
  maxTokens: 10             // burst capacity
)
// Effective rate: ~0.67 events/minute per IP, burst up to 10
```

**Filter (REQ) Rate Limiting:**
```go
FilterIPRateLimiter(
  tokensPerInterval: 20,    // requests
  interval: 60,             // 1 minute
  maxTokens: 100            // burst capacity
)
// Effective rate: 20 REQs/minute per IP, burst up to 100
```

**Connection Rate Limiting:**
```go
ConnectionRateLimiter(
  tokensPerInterval: 1,     // connection
  interval: 300,            // 5 minutes
  maxTokens: 100            // burst capacity
)
// Effective rate: 1 connection per 5 minutes per IP, burst up to 100
```

**Event Policies:**
- `RejectEventsWithBase64Media` - Rejects events containing `data:image/` or `data:video/`
- `NoComplexFilters` - Rejects filters with >4 total items AND >2 tag filters

#### Available Rate Limiter Functions

1. **`EventIPRateLimiter(tokensPerInterval, interval, maxTokens)`** - Rate limit events by IP
2. **`EventPubKeyRateLimiter(tokensPerInterval, interval, maxTokens)`** - Rate limit by pubkey
3. **`EventAuthedPubKeyRateLimiter(tokensPerInterval, interval, maxTokens)`** - Rate limit authenticated users
4. **`ConnectionRateLimiter(tokensPerInterval, interval, maxTokens)`** - Rate limit new connections
5. **`FilterIPRateLimiter(tokensPerInterval, interval, maxTokens)`** - Rate limit REQ messages

#### Other Available Policies

**Event Rejection:**
- `PreventTooManyIndexableTags(max, ignoreKinds, onlyKinds)` - Limit indexable tags
- `PreventLargeTags(maxTagValueLen)` - Reject large tag values (default: 100 bytes)
- `RestrictToSpecifiedKinds(allowEphemeral, kinds...)` - Whitelist specific kinds
- `PreventTimestampsInThePast(threshold)` - Reject old events
- `PreventTimestampsInTheFuture(threshold)` - Reject future-dated events

**Filter Policies:**
- `NoComplexFilters` - Max 4 items total, max 2 tag filters
- `NoEmptyFilters` - Require at least one filter criterion
- `AntiSyncBots` - Require author for kind:1 queries
- `NoSearchQueries` - Disable search functionality
- `MustAuth` - Require NIP-42 authentication

#### Implementation Approach

**Architecture Highlights:**
- **Token bucket algorithm**: Implemented in `startRateLimitSystem[K]` using atomic counters
- **Per-key tracking**: Uses `xsync.MapOf` for concurrent map access
- **Automatic cleanup**: Goroutine periodically decrements buckets and removes zero/negative entries
- **Framework design**: Relay operators compose policies by adding functions to hook slices
- **No global defaults enforced**: Operators must explicitly apply policies
- **Lightweight**: Pure Go, no external dependencies for rate limiting

**Rate Limiting Strategy:**
- **Most opinionated defaults** of all three relays
- Token bucket with automatic refill
- Per-IP tracking for all limits
- Composable policy system

**Strengths:**
- **Secure by default** when using `ApplySaneDefaults`
- Very clear, composable policy API
- Lightweight token bucket implementation
- Well-suited for custom relay development

**Weaknesses:**
- Framework, not standalone relay (requires custom code)
- Aggressive defaults might be too restrictive for some use cases
- Go-based (not applicable to ngit-grasp, but worth noting)

---

## Comparative Summary

| Feature | strfry | nostr-rs-relay | khatru (sane defaults) |
|---------|--------|----------------|------------------------|
| **Max Event Size** | 64 KB | 128 KB | 500 KB |
| **Max WS Message** | 128 KB | 128 KB | 500 KB |
| **Max Subs/Connection** | 20 | ∞ (unlimited) | ∞ (unlimited) |
| **Max Filters/REQ** | 200 | ∞ (unlimited) | Complexity-based (4 items, 2 tags) |
| **Event Rate Limit** | Plugin-based | 0 (unlimited default) | **2 per 3min per IP** |
| **REQ Rate Limit** | None built-in | 0 (unlimited default) | **20/min per IP** |
| **Connection Rate** | None built-in | None | **1 per 5min per IP** |
| **Future Event Rejection** | 15 minutes | 30 minutes | Policy-based |
| **Rate Limit Technique** | External plugins | Averaged over 1 minute | Token bucket (atomic) |
| **Backpressure** | Query pause/resume | Buffering + blocking | Framework hooks |
| **Default Philosophy** | Permissive + plugins | **Dangerously permissive** | **Conservative** |
| **Per-IP Tracking** | Metrics only | No | Yes (all limits) |
| **Production Ready** | Yes (with config) | Yes (with config) | Framework (DIY) |

## Rust Rate Limiting Ecosystem

### Governor Crate

**Repository:** https://github.com/boinkor-net/governor  
**Documentation:** https://docs.rs/governor/  
**Version:** 0.10.4 (stable)

#### Overview

Governor is the most popular rate limiting library in the Rust ecosystem. It implements the **Generic Cell Rate Algorithm (GCRA)**, which is equivalent to a token bucket but more space-efficient.

#### Features

- **Thread-safe**: Uses atomic operations for lock-free operation
- **Per-key rate limiting**: Built-in support via `DefaultKeyedRateLimiter`
- **Direct rate limiting**: Single-state limiter via `DefaultDirectRateLimiter`
- **Async/await support**: Works with Tokio and other async runtimes
- **Jitter support**: Built-in jitter for avoiding thundering herd
- **Dashmap integration**: Uses `dashmap` for concurrent key-value storage
- **Quota system**: Flexible quota definitions (per second, minute, hour, etc.)

#### Example Usage

```rust
use std::num::NonZeroU32;
use nonzero_ext::*;
use governor::{Quota, RateLimiter};

// Simple direct rate limiter
let mut lim = RateLimiter::direct(Quota::per_second(nonzero!(50u32)));
assert_eq!(Ok(()), lim.check());

// Keyed rate limiter (e.g., per IP)
use governor::state::{InMemoryState, keyed::DefaultKeyedRateLimiter};
use std::net::IpAddr;

let limiter = RateLimiter::keyed(Quota::per_minute(nonzero!(10u32)));
let ip: IpAddr = "192.168.1.1".parse().unwrap();
if limiter.check_key(&ip).is_err() {
    // Rate limit exceeded for this IP
}
```

#### Dependencies

- `cfg-if` - Configuration
- `dashmap` (optional) - Concurrent hashmap for keyed limiters
- `parking_lot` (optional) - More efficient mutexes
- `quanta` (optional) - High-resolution timing
- `portable-atomic` - Atomic operations
- `nonzero_ext` - NonZero integer utilities

#### Pros

- Industry standard, widely used
- Well-maintained and documented
- Efficient implementation (atomic operations)
- Flexible quota system
- Works with async

#### Cons

- Additional dependency (though well-vetted)
- Slightly more complex API than hand-rolled solution
- Uses more memory for keyed limiters with many keys

### Alternative: Extend Existing ThrottleManager

ngit-grasp already has a working rate limiter in `src/purgatory/sync/throttle.rs`:

```rust
pub struct ThrottleManager {
    throttles: DashMap<String, Mutex<DomainThrottle>>,
    max_concurrent_per_domain: u32,
    max_per_minute_per_domain: u32,
}
```

**Sliding window implementation:**
```rust
let recent_count = self.request_times
    .iter()
    .filter(|t| now.duration_since(**t) < window)
    .count();
recent_count < self.max_per_minute as usize
```

#### Pros of Reusing

- No new dependencies
- Already proven to work in production
- Team familiarity with the code
- Consistent patterns across codebase

#### Cons of Reusing

- More maintenance burden
- May not handle all edge cases
- Less efficient than GCRA algorithm
- Would need to be generalized for different use cases

## Recommendations for ngit-grasp

### 1. Rate Limiting Library Choice

**Recommendation: Use `governor` crate**

**Reasoning:**
- Industry standard with proven track record
- More efficient than our sliding window approach
- Handles edge cases we might miss
- Good async support for our Tokio-based architecture
- Active maintenance and community support
- Minimal overhead (atomic operations, lock-free)

### 2. Default Philosophy

**Recommendation: Conservative defaults with clear relaxation path**

**Reasoning:**
- Following khatru's approach: secure by default
- Better to start restrictive and allow operators to relax
- Prevents "configuration debt" where operators forget to harden
- ngit-grasp is infrastructure software - security should be default
- Clear documentation on how to adjust for different use cases

### 3. Proposed Default Values

Based on research and ngit-grasp's specific use case (git-over-nostr relay):

```toml
# Connection Limits
NGIT_MAX_CONNECTIONS_GLOBAL = 1000
NGIT_MAX_CONNECTIONS_PER_IP = 10
NGIT_CONNECTION_RATE_PER_IP = "5/minute"   # 5 connections per minute per IP

# Subscription (REQ) Limits
NGIT_MAX_SUBSCRIPTIONS_PER_CONNECTION = 20
NGIT_MAX_FILTERS_PER_REQ = 100
NGIT_SUBSCRIPTION_RATE_PER_IP = "30/minute"  # 30 REQs per minute per IP

# Event Ingestion Limits
NGIT_EVENT_RATE_PER_IP = "10/minute"         # 10 events per minute per IP
NGIT_EVENT_RATE_BURST = 30                   # Allow burst up to 30
NGIT_MAX_EVENT_SIZE_BYTES = 131072           # 128 KB (matches nostr-rs-relay)
NGIT_MAX_WEBSOCKET_MESSAGE_BYTES = 131072    # 128 KB

# HTTP Endpoint Protection
NGIT_HTTP_RATE_PER_IP = "60/minute"          # 60 HTTP requests per minute per IP

# Time-based Event Restrictions
NGIT_REJECT_EVENTS_NEWER_THAN_SECONDS = 900  # 15 minutes (matches strfry)
NGIT_REJECT_EVENTS_OLDER_THAN_SECONDS = 94608000  # ~3 years (matches strfry)

# Whitelist
NGIT_RATE_LIMIT_WHITELIST_IPS = ""           # Comma-separated IPs exempt from rate limits
```

**Rationale for values:**
- **Connections:** 10/IP is conservative but allows legitimate multi-client use
- **Subscriptions:** 20/connection matches strfry, reasonable for typical clients
- **Events:** 10/min is more permissive than khatru (2 per 3min) but still protective
- **Message size:** 128 KB matches industry standard (nostr-rs-relay, strfry's WS message size)
- **HTTP:** 60/min allows normal browsing without allowing scraping abuse

### 4. Implementation Phases

**Phase 1: Core DoS Prevention (High Priority)**
- Connection limits (global and per-IP)
- Basic event rate limiting (per-IP)
- Message size limits
- WebSocket message limits

**Phase 2: Advanced Subscription Protection (Medium Priority)**
- Subscription limits per connection
- Filter complexity limits
- Subscription rate limiting per IP

**Phase 3: HTTP & Advanced Features (Lower Priority)**
- HTTP endpoint rate limiting
- IP whitelisting
- Fine-grained metrics for rate limit hits
- Configurable rejection messages

### 5. Configuration Management

Following AGENTS.md requirements, ALL configuration changes must update:

1. **`src/config.rs`** - Add fields with proper env var names and defaults
2. **`docs/reference/configuration.md`** - Document each option with examples
3. **`nix/module.nix`** - Add NixOS options in `instanceOptions`
4. **`.env.example`** - Add options with comments

### 6. Metrics & Observability

Add Prometheus metrics for:
- `ngit_rate_limit_hits_total{limit_type, reason}` - Counter of rate limit hits
- `ngit_connections_active` - Current active connections
- `ngit_connections_per_ip` - Histogram of connections per IP
- `ngit_subscriptions_active` - Current active subscriptions
- `ngit_rate_limit_whitelisted_requests_total` - Requests from whitelisted IPs

### 7. Testing Strategy

- **Unit tests**: Test rate limiter logic in isolation
- **Integration tests**: Use `TestRelay` to verify limits enforced
- **Fuzz testing**: Random patterns to ensure no panics
- **Load testing**: Verify performance under rate-limited load
- **Metrics verification**: Ensure metrics accurately reflect limit hits

## Common Attack Patterns

Based on production relay operator experiences:

1. **Connection flooding** - Open thousands of connections to exhaust file descriptors
2. **Subscription spam** - Open many REQs per connection to consume memory
3. **Event spam** - Submit events rapidly to overwhelm storage/processing
4. **Large message attacks** - Send huge WebSocket frames to consume bandwidth
5. **Complex filter DoS** - Submit filters with thousands of authors/kinds to slow queries
6. **Slow read attack** - Connect but never read, filling write buffers
7. **Time-based attacks** - Events with extreme timestamps to bypass caching
8. **Metrics scraping** - Hammer `/metrics` endpoint to consume CPU

All of these are addressed by the proposed implementation.

## Open Questions

1. **Should we implement per-pubkey rate limiting** (like khatru) in addition to per-IP?
   - Useful for authenticated scenarios
   - Requires NIP-42 AUTH support
   - Could be Phase 4

2. **Should ephemeral events have different limits?**
   - strfry has special handling for ephemeral events
   - Consider separate retention and rate limits

3. **Should we support dynamic limit adjustment?**
   - Allow hot-reloading of limits without restart
   - Useful for responding to active attacks

4. **How should we handle IPv6?**
   - Rate limit by /64 or /128?
   - Per-address might be too granular for IPv6

## References

- strfry repository: https://github.com/hoytech/strfry
- strfry config: https://github.com/hoytech/strfry/blob/master/strfry.conf
- nostr-rs-relay repository: https://git.sr.ht/~gheartsfield/nostr-rs-relay
- khatru repository: https://github.com/fiatjaf/khatru
- khatru policies: https://github.com/fiatjaf/khatru/tree/master/policies
- governor crate: https://docs.rs/governor/
- GCRA algorithm: https://en.wikipedia.org/wiki/Generic_cell_rate_algorithm
