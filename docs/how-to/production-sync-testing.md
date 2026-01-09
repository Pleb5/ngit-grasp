# How-To: Test Sync Against Production Data

**Problem:** Debug and improve sync behavior using real-world data from production relays  
**Difficulty:** Intermediate  
**Time:** 30 minutes per iteration

## Overview

This guide helps you run ngit-grasp's sync system against production relays to discover unexpected errors, inefficiencies, and edge cases that don't appear in controlled tests.

**Why production testing matters:**
- Real data has inconsistencies, malformed events, and edge cases
- Production relays may behave differently (rate limiting, timeouts, partial NIP-77 support)
- Volume and patterns reveal performance bottlenecks
- Sync discovery leads to cascading subscriptions we can't predict in tests

## Prerequisites

- ngit-grasp compiles successfully (`cargo build`)
- Familiarity with [GRASP-02 Proactive Sync](../explanation/grasp-02-proactive-sync.md)
- Understanding of log levels and tracing

## Test Setup

### 1. Choose a Test Identity

Pick a domain with manageable sync volume. Smaller domains mean fewer repos to sync, making logs tractable.

**Recommended starting point:**
```bash
--domain ngit.danconwaydev.com
```

This domain has few repo announcements listing it, so sync stays manageable.

### 2. Choose a Bootstrap Relay

The bootstrap relay provides the initial set of announcements to discover repos:

```bash
--sync-bootstrap-relay wss://git.shakespeare.diy
```

### 3. Run with Time Limit

Start with short runs (30 seconds) to capture manageable log volumes:

```bash
# Clear any existing data for clean state
rm -rf /tmp/ngit-test-*

# Run for 30 seconds with sanitized output
timeout 30s cargo run -- \
    --sync-bootstrap-relay wss://git.shakespeare.diy \
    --domain ngit.danconwaydev.com \
    --git-path /tmp/ngit-test-git \
    --relay-data-path /tmp/ngit-test-relay \
    2>&1 | ./scripts/sanitize-logs.sh | tee sync-test.log
```

**Note:** The `timeout` command returns exit code 124, which is expected.

## Log Sanitization

Raw logs include full events and hundreds of event IDs per line, making them unwieldy for analysis. The sanitizer truncates long lines:

```bash
./scripts/sanitize-logs.sh < raw.log > sanitized.log

# Or pipe directly
cargo run -- [args] 2>&1 | ./scripts/sanitize-logs.sh
```

**Options:**
- `--head-chars N` - First N characters to show (default: 100)
- `--tail-chars N` - Last N characters to show (default: 20)

Example output:
```
2024-01-09T10:00:00Z DEBUG sync: Processing events ids=[abc123, def456, ghi789, jkl012...<1847 chars>...xyz999, end123]
```

## What to Look For

### Phase 1: Connection & Bootstrap (0-5 seconds)

**Expected behavior:**
- Connection to bootstrap relay succeeds
- Layer 1 (announcement) subscription starts
- First batch of 30617/30618 events received

**Red flags:**
- Connection timeout or failure
- NIP-77 negentropy errors (should fall back gracefully)
- Immediate rate limiting

### Phase 2: Discovery Cascade (5-15 seconds)

**Expected behavior:**
- Self-subscriber batches fire as announcements are processed
- New relays discovered from announcement `relays` tags
- Layer 2 (repo tags) subscriptions created

**Red flags:**
- Excessive relay discovery (>10 relays rapidly)
- Filter consolidation warnings (>70 filters)
- Missing self-subscriber batch logs

### Phase 3: Steady State (15+ seconds)

**Expected behavior:**
- Historic sync batches completing (EOSE received)
- Periodic health checks running
- Events being saved to database

**Red flags:**
- Pending batches never confirming
- Repeated connection/disconnect cycles
- Memory growth (check with `top` in another terminal)

## Debugging Checklist

When analyzing logs, look for these patterns:

### Errors to Investigate

| Pattern | Possible Cause | Action |
|---------|----------------|--------|
| `error` (any) | Unexpected failure | Investigate immediately |
| `connection failed` | Network/relay issue | Check relay URL, try different relay |
| `rate limit` | Too many requests | Check consolidation, increase backoff |
| `negentropy` + `error` | NIP-77 incompatibility | Should fall back - verify it does |
| `timeout` | Slow relay or large sync | Increase timeouts or reduce scope |

### Warnings to Monitor

| Pattern | Meaning | Action |
|---------|---------|--------|
| `consolidating filters` | Filter count high | Expected, but frequent = problem |
| `backing off` | Health tracker retry | Normal, but watch for excessive |
| `batch failed` | Historic sync incomplete | Check which batches, why |

### Debug Patterns to Verify

| Pattern | What it shows |
|---------|---------------|
| `fresh_start` | Full sync initiated |
| `quick_reconnect` | Incremental sync (<15min gap) |
| `historic sync complete` | Sync finished successfully |
| `sync_live` | Live subscriptions active |
| `PendingBatch` | Items awaiting EOSE confirmation |

## Iterative Improvement Process

### Step 1: Run and Capture

```bash
timeout 30s cargo run -- [args] 2>&1 | ./scripts/sanitize-logs.sh > iteration-1.log
```

### Step 2: Identify Issues

Scan logs for errors and unexpected patterns:
```bash
grep -i error iteration-1.log
grep -i warn iteration-1.log
grep -i panic iteration-1.log
```

### Step 3: Document Findings

Add findings to this file's [Known Issues](#known-issues) section or create GitHub issues.

### Step 4: Fix and Re-test

After code changes, run again to verify the fix.

### Step 5: Extend Duration

Once 30-second runs are clean, extend to 2 minutes, then 5 minutes:
```bash
timeout 120s cargo run -- [args] 2>&1 | ./scripts/sanitize-logs.sh > iteration-2.log
```

## Logging Improvements

If the logs aren't helpful enough, improve them. Common needs:

### Add Context to Existing Logs

```rust
// Before
tracing::debug!("Processing events");

// After
tracing::debug!(
    relay = %relay_url,
    event_count = events.len(),
    "Processing events"
);
```

### Add New Log Points

Key places that may need more logging:
- `src/sync/mod.rs` - SyncManager state transitions
- `src/sync/relay_connection.rs` - Connection lifecycle
- `src/sync/self_subscriber.rs` - Batch processing

### Reduce Noise

If a log line appears too frequently:
```rust
// Change from debug! to trace!
tracing::trace!("Per-event detail that's too noisy");
```

## Known Issues

*Document issues discovered during testing here. Delete this section when empty.*

### Template for New Issues

```markdown
### Issue: [Short description]

**Discovered:** [Date]
**Status:** [Open/Fixed in PR#xxx]

**Symptoms:**
- Log pattern observed

**Root cause:**
- [If known]

**Fix:**
- [If known]
```

---

## Quick Reference

### Minimal Test Command

```bash
timeout 30s cargo run -- \
    --sync-bootstrap-relay wss://git.shakespeare.diy \
    --domain ngit.danconwaydev.com \
    --git-path /tmp/ngit-test-git \
    --relay-data-path /tmp/ngit-test-relay \
    2>&1 | ./scripts/sanitize-logs.sh
```

### With Metrics Endpoint

```bash
timeout 30s cargo run -- \
    --sync-bootstrap-relay wss://git.shakespeare.diy \
    --domain ngit.danconwaydev.com \
    --git-path /tmp/ngit-test-git \
    --relay-data-path /tmp/ngit-test-relay \
    --metrics-address 127.0.0.1:9090 \
    2>&1 | ./scripts/sanitize-logs.sh
```

Then in another terminal: `curl http://127.0.0.1:9090/metrics`

### Different Log Level

The default is DEBUG. For more detail:
```bash
RUST_LOG=trace cargo run -- [args]
```

For less noise:
```bash
RUST_LOG=info cargo run -- [args]
```

---

*Part of the [ngit-grasp documentation](../README.md) using the [Diátaxis](https://diataxis.fr/) framework.*
