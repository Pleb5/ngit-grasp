# How-To: Test Sync Against Production Data

> **Quick Start Prompt:** Check work/active-issues/ for existing issues. If issues exist, pick the most important, fix it, test with cargo test, run clippy and fmt, commit, and report back with a brief 1-2 sentence summary of each issue you identified. If no issues exist, run a 30-second production sync test, analyze logs, create individual issue files in work/active-issues/ (one per issue with minimal description), then report summary listing each issue in 1-2 sentences.

**Problem:** Debug and improve sync behavior using real-world data from production relays  
**Difficulty:** Intermediate  
**Time:** 30 minutes per iteration

## Two-Mode Workflow

This guide operates in two modes:

### Mode 1: Fix Existing Issues
**When:** There are files in `work/active-issues/` (excluding README.md)

1. Check for active issues: `ls work/active-issues/`
2. Pick the most important issue to fix
3. **Review proposed fix and ask for permission before implementing**
4. Implement the fix (after approval)
5. Run `cargo test` to verify tests pass
6. Run `cargo clippy` to check for warnings
7. Run `cargo fmt` to format code
8. Commit changes with descriptive message
9. Report back - **DO NOT** do another issue or run more tests

### Mode 2: Discover New Issues
**When:** No active issues in `work/active-issues/`

1. Run 30-second production sync test (logs saved to `tmp/run-{timestamp}/`)
2. Analyze logs for errors, warnings, unexpected patterns
3. Document each issue as a separate markdown file in `work/active-issues/`
4. Keep issue files minimal - just enough to identify the issue
5. Report brief summary listing each issue in 1-2 sentences
6. **DO NOT** create separate detailed analysis files
7. **DO NOT** do thorough investigation or root cause analysis

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
--sync-bootstrap-relay-url wss://git.shakespeare.diy
```

### 3. Run with Time Limit

Start with short runs (30 seconds) to capture manageable log volumes. Each run creates its own subdirectory in `tmp/` to keep data and logs isolated:

```bash
# Create run directory with timestamp
RUN_DIR="tmp/run-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR"

# Run for 30 seconds with sanitized output
timeout 30s cargo run -- \
    --sync-bootstrap-relay-url wss://git.shakespeare.diy \
    --domain ngit.danconwaydev.com \
    --git-data-path "$RUN_DIR/git-data" \
    --relay-data-path "$RUN_DIR/relay-data" \
    2>&1 | ./scripts/sanitize-logs.sh | tee "$RUN_DIR/sync.log"
```

**Note:** The `timeout` command returns exit code 124, which is expected.

**Directory structure after run:**
```
tmp/
└── run-20260109-143022/
    ├── git-data/       # Git repository data
    ├── relay-data/     # Relay database
    └── sync.log        # Sanitized log output
```

## Log Sanitization

Raw logs include full events and hundreds of event IDs per line, making them unwieldy for analysis. The sanitizer truncates long lines:

```bash
./scripts/sanitize-logs.sh < raw.log > sanitized.log

# Or pipe directly
cargo run -- [args] 2>&1 | ./scripts/sanitize-logs.sh
```

**Options:**
- `--head-chars N` - First N characters to show (default: 200)
- `--tail-chars N` - Last N characters to show (default: 100)

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

## Mode 1: Fix Existing Issues (Detailed)

When `work/active-issues/` contains issue files:

### Step 1: Check for Active Issues

```bash
ls work/active-issues/
```

If any `.md` files exist (excluding README.md), you're in Mode 1.

### Step 2: Pick Most Important Issue

Review issue files and select based on:
- Severity (errors > warnings > log quality)
- Impact (functionality > performance > UX)
- Complexity (quick fixes first to clear backlog)

### Step 3: Review Proposed Fix and Get Permission

**IMPORTANT:** Before implementing any changes:

1. Read relevant code files to understand the issue
2. Analyze the root cause
3. Propose a fix with explanation of what will change and why
4. Summarize the proposed fix in 2-3 sentences
5. **Ask for user permission to proceed**

**Do NOT implement changes without explicit approval.**

### Step 4: Implement the Fix

After receiving permission, make the necessary code changes based on the issue description and approved plan.

### Step 5: Test, Lint, Format

```bash
# Run tests
cargo test

# Check for warnings
cargo clippy

# Format code
cargo fmt
```

### Step 6: Commit

```bash
git add .
git commit -m "fix: [brief description of what was fixed]"
```

### Step 7: Report Back

**STOP HERE.** Report what was fixed. Do NOT:
- Fix another issue
- Run production sync test
- Do additional investigation

The workflow will cycle back through Mode 1 if more issues remain.

## Mode 2: Discover New Issues (Detailed)

When `work/active-issues/` is empty (or only contains README.md):

### Step 1: Run Production Sync Test

```bash
# Create run directory with timestamp
RUN_DIR="tmp/run-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR"

# Run 30-second test
timeout 30s cargo run -- \
    --sync-bootstrap-relay-url wss://git.shakespeare.diy \
    --domain ngit.danconwaydev.com \
    --git-data-path "$RUN_DIR/git-data" \
    --relay-data-path "$RUN_DIR/relay-data" \
    2>&1 | ./scripts/sanitize-logs.sh | tee "$RUN_DIR/sync.log"
```

Each run is isolated in its own timestamped directory under `tmp/`, keeping data and logs organized.

### Step 2: Analyze Logs

Scan for errors and unexpected patterns:
```bash
# Find the most recent run
LATEST_RUN=$(ls -1t tmp/run-*/sync.log | head -n1)

# Analyze for issues
grep -i error "$LATEST_RUN"
grep -i warn "$LATEST_RUN"
grep -i panic "$LATEST_RUN"
```

### Step 3: Document Issues

Create **one markdown file per issue** in `work/active-issues/`:

```bash
# Example: Minimal issue documentation
cat > work/active-issues/bootstrap-disconnect.md <<'EOF'
# Bootstrap relay disconnects when empty

Bootstrap relay wss://git.shakespeare.diy disconnects after sync finds 0 events. Should persist since user-specified.

Log: "Disconnecting empty relay relay=wss://git.shakespeare.diy"
File: src/sync/mod.rs (check_disconnects function)
EOF
```

**Keep each file brief:**
- Descriptive title (one line)
- What happens (1-2 sentences max)
- Relevant log excerpt (one line)
- File/function location if obvious (one line)
- **NO** separate detailed analysis files
- **NO** root cause analysis
- **NO** proposed solutions (unless immediately obvious)

### Step 4: Report Summary

Provide a brief closing message with 1-2 sentence summary of **each issue** identified:
- State what the issue is
- Where it occurs (file/component)
- Keep it concise

**STOP HERE.** Do NOT:
- Fix the issues immediately
- Create separate detailed analysis markdown files
- Do thorough investigations
- Write lengthy explanations

The workflow will cycle back through Mode 1 to fix issues one at a time.

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

## Managing Active Issues

Issues are tracked in `work/active-issues/` as individual markdown files.

**Check for active issues:**
```bash
ls work/active-issues/
```

**After fixing an issue:**
```bash
# Delete the resolved issue file
rm work/active-issues/issue-name.md

# Or archive if important for future reference
mv work/active-issues/issue-name.md docs/archive/2026-01-09-issue-name.md
```

**Issue file format (minimal):**
```markdown
# Brief title

What happens (1-2 sentences).

Log evidence: "relevant log line"
File: src/path/to/file.rs (function_name if known)
```

Keep documentation minimal - just enough to identify and locate the issue.

---

## Workflow Summary

```
Check work/active-issues/
    │
    ├─ Has issues? ──► Mode 1: Pick one issue
    │                          │
    │                          ├─ Review & propose fix
    │                          ├─ Ask permission
    │                          ├─ Fix code (after approval)
    │                          ├─ cargo test
    │                          ├─ cargo clippy
    │                          ├─ cargo fmt
    │                          ├─ git commit
    │                          └─ Report & STOP
    │
    └─ No issues? ──► Mode 2: Run production sync
                               │
                               ├─ timeout 30s cargo run ...
                               ├─ Analyze logs
                               ├─ Document issues (minimal)
                               └─ Report summary & STOP
```

**Key Rules:**
- Only do ONE thing per cycle (fix one issue OR discover issues)
- Always stop after reporting
- Keep issue documentation minimal
- No root cause analysis during discovery

---

## Quick Reference

### Minimal Test Command

```bash
# Create run directory
RUN_DIR="tmp/run-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR"

# Run test
timeout 30s cargo run -- \
    --sync-bootstrap-relay-url wss://git.shakespeare.diy \
    --domain ngit.danconwaydev.com \
    --git-data-path "$RUN_DIR/git-data" \
    --relay-data-path "$RUN_DIR/relay-data" \
    2>&1 | ./scripts/sanitize-logs.sh | tee "$RUN_DIR/sync.log"
```

### With Metrics Endpoint

```bash
# Create run directory
RUN_DIR="tmp/run-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR"

# Run with metrics
timeout 30s cargo run -- \
    --sync-bootstrap-relay-url wss://git.shakespeare.diy \
    --domain ngit.danconwaydev.com \
    --git-data-path "$RUN_DIR/git-data" \
    --relay-data-path "$RUN_DIR/relay-data" \
    --metrics-address 127.0.0.1:9090 \
    2>&1 | ./scripts/sanitize-logs.sh | tee "$RUN_DIR/sync.log"
```

Then in another terminal: `curl http://127.0.0.1:9090/metrics`

### Cleanup Old Runs

```bash
# Remove runs older than 7 days
find tmp/run-* -type d -mtime +7 -exec rm -rf {} +

# Remove all test runs
rm -rf tmp/run-*
```

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
