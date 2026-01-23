# Migrate ngit-relay to ngit-grasp

This guide walks you through migrating a production ngit-relay instance to ngit-grasp. The process involves analyzing your existing data to identify repositories that need attention before switching over.

## Quick Start

Run the migration analysis with a single command:

```bash
# Basic analysis (fetches events, compares relays)
./docs/how-to/migration-scripts/run-migration-analysis.sh \
  --prod-relay wss://relay.ngit.dev \
  --archive-relay wss://archive.relay.ngit.dev

# Full analysis (includes git sync check - run on VPS)
./docs/how-to/migration-scripts/run-migration-analysis.sh \
  --prod-relay wss://relay.ngit.dev \
  --archive-relay wss://archive.relay.ngit.dev \
  --prod-git /var/lib/ngit-relay/git \
  --archive-git /var/lib/ngit-relay-archive/git \
  --service ngit-grasp.service
```

The script produces three output files:
- `results/no-action-required.txt` - Repos ready for migration
- `results/action-required.txt` - Repos needing intervention
- `results/manual-investigation.txt` - Repos needing human review

See [Running the Analysis](#running-the-analysis) for detailed options.

## Prerequisites

### Required Tools

- **nak** - Nostr Army Knife for fetching events ([install](https://github.com/fiatjaf/nak))
- **jq** - JSON processing (install via package manager)

### For Full Analysis (VPS)

- SSH access to the VPS running ngit-relay
- Read access to git data directories
- Access to systemd journal (for log extraction)

### Verify Installation

```bash
# Check required tools
nak --version
jq --version

# Check optional tools (for VPS phases)
journalctl --version
```

## Migration Overview

The migration process has three stages:

### Stage 1: Deploy Archive Instance

Deploy ngit-grasp alongside your production ngit-relay:

1. Configure ngit-grasp with:
   - `domain` set to `<prod-domain>.internal` (temporary)
   - `archiveService` set to your production domain
   - Running on a different port

2. Let it sync for ~1 hour to gather all events and git data

### Stage 2: Analyze Data

Run the migration analysis to identify:
- Repositories successfully migrated (no action needed)
- Repositories with incomplete data (need investigation)
- Repositories with parse failures (may need re-announcement)

### Stage 3: Switch Over

Once all issues are resolved:
1. Set `domain` to your production URL
2. Disable archive mode
3. Update your reverse proxy to point to ngit-grasp

## Running the Analysis

### Basic Usage

```bash
# Preview what will happen (dry run)
./run-migration-analysis.sh \
  --prod-relay wss://relay.ngit.dev \
  --archive-relay wss://archive.relay.ngit.dev \
  --dry-run

# Run the analysis
./run-migration-analysis.sh \
  --prod-relay wss://relay.ngit.dev \
  --archive-relay wss://archive.relay.ngit.dev
```

### Full Analysis on VPS

```bash
./run-migration-analysis.sh \
  --prod-relay wss://relay.ngit.dev \
  --archive-relay wss://archive.relay.ngit.dev \
  --prod-git /var/lib/ngit-relay/git \
  --archive-git /var/lib/ngit-relay-archive/git \
  --service ngit-grasp.service
```

### Phase Control

Skip or run specific phases:

```bash
# Skip Phase 2 (use cached git sync data)
./run-migration-analysis.sh ... --skip-phase-2

# Run only Phase 1 (fetch events)
./run-migration-analysis.sh ... --only-phase-1

# Resume from Phase 3 (using existing data)
./run-migration-analysis.sh ... --from-phase-3 --output work/migration-analysis-20260122-1430
```

### All Options

| Option | Description |
|--------|-------------|
| `--prod-relay <url>` | Production relay WebSocket URL (required) |
| `--archive-relay <url>` | Archive relay WebSocket URL (required) |
| `--prod-git <path>` | Git base directory for prod (enables Phase 2) |
| `--archive-git <path>` | Git base directory for archive (enables Phase 2) |
| `--service <name>` | Systemd service name (enables Phase 4) |
| `--output <dir>` | Output directory (default: auto-generated) |
| `--skip-phase-N` | Skip phase N (1-5) |
| `--only-phase-N` | Run only phase N |
| `--from-phase-N` | Start from phase N |
| `--dry-run` | Show what would be executed |
| `--continue-on-error` | Continue even if a phase fails |

## Understanding Results

### Summary File

The `results/summary.txt` file provides an overview:

```
## Overview

| Category | Count | Percentage |
|----------|-------|------------|
| No Action Required | 450 | 85.7% |
| Action Required | 52 | 9.9% |
| Manual Investigation | 23 | 4.4% |
```

### No Action Required

Repositories in `no-action-required.txt` are ready for migration:

```
myrepo | npub1abc... | complete in both prod and archive
oldrepo | npub1def... | deleted by user
testrepo | npub1ghi... | empty/blank in both (user never pushed)
```

**Common reasons:**
- `complete in both prod and archive` - Successfully migrated
- `deleted by user` - User requested deletion (kind 5 event)
- `empty/blank in both` - No git data was ever pushed
- `purgatory expired` - System already handled the timeout

### Action Required

Repositories in `action-required.txt` need intervention:

```
myrepo | npub1abc... | complete in prod, missing from archive | trigger re-sync or investigate
otherrepo | npub1def... | incomplete in both (prod=cat3, archive=cat2) | investigate git data source
```

**Common actions:**
- **Re-sync needed**: Trigger the archive to re-fetch from the source
- **Wait for sync**: Archive sync may still be in progress
- **Investigate git source**: Original git data may be incomplete
- **Fix parse failure**: Event format issue, may need re-announcement

### Manual Investigation

Repositories in `manual-investigation.txt` have unusual states:

```
weirdrepo | npub1abc... | in archive (cat1) but not in prod | may be new announcement or deleted from prod
conflictrepo | npub1def... | complete in prod, missing from archive, parse failure logged | investigate parse failure
```

These require human judgment to determine the correct action.

## Troubleshooting

### "nak not found"

Install nak from https://github.com/fiatjaf/nak:

```bash
# Using Go
go install github.com/fiatjaf/nak@latest

# Or download binary from releases
```

### "Permission denied" on git directories

Run with sudo or ensure your user has read access:

```bash
# Check permissions
ls -la /var/lib/ngit-relay/git

# Run with sudo if needed
sudo ./run-migration-analysis.sh ...
```

### Phase 2 takes too long

The git sync check processes each repository individually (~20 minutes total). To speed up iteration:

1. Run Phase 2 once and save the output
2. Use `--skip-phase-2` for subsequent runs
3. Use `--from-phase-3` to re-run classification with existing data

### No parse failures found

This is expected if:
- ngit-grasp logging improvements aren't deployed yet
- No events actually failed to parse

The analysis will continue without log data.

### Event counts are multiples of 250

This suggests pagination may have failed. The scripts use `--paginate` by default, but if you see exactly 250, 500, 750 events, verify the relay is responding correctly.

## Architecture

### Analysis Phases

The analysis is split into 5 modular phases:

| Phase | Name | Time | Location | Description |
|-------|------|------|----------|-------------|
| 1 | Fetch Events | ~30s each | Local | Fetch events from both relays |
| 2 | Git Sync Check | ~20 min each | VPS | Compare state events to git data |
| 3 | Categorize & Compare | <1s | Local | Categorize and compare results |
| 4 | Extract Logs | <30s | VPS | Extract parse failures and purgatory expiry |
| 5 | Final Classification | <5s | Local | Combine all data into actionable results |

### Phase Flow Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 1: Fetch Events (~30s, local)                             │
│ Fetches kind 30618 (state), 30617 (announcements), 5 (deletion) │
│ Run twice: once for prod, once for archive                      │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 2: Git Sync Check (~20 mins, VPS required)                │
│ Compares state event refs to actual git data on disk            │
│ Categorizes into: complete, empty, partial, no-match            │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 3: Categorize & Compare (fast, local)                     │
│ Compares prod vs archive categories                             │
│ Identifies gaps and sync issues                                 │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 4: Log-Based Categories (VPS required)                    │
│ Extracts [PARSE_FAIL] and [PURGATORY_EXPIRED] from logs         │
│ Provides context for why repos failed to sync                   │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 5: Final Classification (fast, local)                     │
│ Combines all data sources                                       │
│ Outputs: no-action, action-required, manual-investigation       │
└─────────────────────────────────────────────────────────────────┘
```

### Git Sync Categories

Phase 2 categorizes repositories into 4 categories:

| Category | Description | Meaning |
|----------|-------------|---------|
| 1 | Complete Match | All refs in state event match git data |
| 2 | Empty/Blank | No git data available |
| 3 | Partial Match | Some refs match, some don't |
| 4 | No Match | Git data exists but refs don't match |

### Output Directory Structure

```
work/migration-analysis-YYYYMMDD-HHMM/
├── prod/
│   ├── raw/
│   │   ├── state-events.json       # Phase 1
│   │   ├── announcements.json      # Phase 1
│   │   └── deletions.json          # Phase 1
│   ├── git-sync-status.tsv         # Phase 2
│   └── category*.txt               # Phase 2/3
├── archive/
│   └── (same structure as prod)
├── comparison/
│   ├── complete-in-both.txt        # Phase 3
│   ├── complete-prod-missing-archive.txt
│   ├── complete-prod-incomplete-archive.txt
│   ├── incomplete-in-both.txt
│   ├── in-archive-not-prod.txt
│   └── summary.txt
├── logs/
│   ├── parse-failures.txt          # Phase 4
│   └── purgatory-expired.txt       # Phase 4
└── results/
    ├── no-action-required.txt      # Phase 5
    ├── action-required.txt         # Phase 5
    ├── manual-investigation.txt    # Phase 5
    └── summary.txt                 # Phase 5
```

## Key Differences: ngit-relay vs ngit-grasp

Understanding these differences helps explain why some repositories need attention:

| Aspect | ngit-relay | ngit-grasp |
|--------|------------|------------|
| Git data validation | Accepts commits/tags referenced in state event | Requires all git data to reproduce state |
| PR refs cleanup | Doesn't clear `refs/nostr/<event-id>` | Properly manages PR refs |
| Parse failures | Silently ignores | Logs structured `[PARSE_FAIL]` entries |
| Sync timeout | No timeout | Purgatory expires after configurable period |

## Next Steps

After running the analysis:

1. **Review the summary** - Check `results/summary.txt` for the overview
2. **Address action items** - Work through `results/action-required.txt`
3. **Investigate edge cases** - Review `results/manual-investigation.txt`
4. **Re-run analysis** - After fixing issues, re-run to verify
5. **Plan cutover** - Schedule the switch when all issues are resolved

### When to Re-run

Re-run the analysis when:
- Archive sync has had time to complete
- You've fixed parse failures or re-announced events
- You want to verify fixes before cutover

```bash
# Re-run with existing Phase 2 data (faster)
./run-migration-analysis.sh ... --skip-phase-2 --output work/migration-analysis-20260122-1430
```

## Individual Scripts

For advanced usage, you can run individual phase scripts:

```bash
# Phase 1: Fetch events
./migration-scripts/01-fetch-events.sh wss://relay.ngit.dev output/prod

# Phase 2: Git sync check
./migration-scripts/10-check-git-sync.sh output/prod/raw/state-events.json /var/lib/ngit-relay/git output/prod --categorize

# Phase 3a: Categorize
./migration-scripts/20-categorize.sh output/prod/git-sync-status.tsv output/prod

# Phase 3b: Compare relays
./migration-scripts/21-compare-relays.sh output/prod output/archive output/comparison

# Phase 4a: Extract parse failures
./migration-scripts/30-extract-parse-failures.sh ngit-grasp.service output/logs

# Phase 4b: Extract purgatory expiry
./migration-scripts/31-extract-purgatory-expiry.sh ngit-grasp.service output/logs

# Phase 5: Final classification
./migration-scripts/40-classify-actions.sh work/migration-analysis-20260122-1430
```

Each script has detailed help available with `--help` or by reading the script header.
