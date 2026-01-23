# Migrate to ngit-grasp from another GRASP implementation

This guide walks you through migrating a production GRASP relay to ngit-grasp. The process involves analyzing your existing data to identify repositories that need attention before switching over.

## Compatibility

This migration process works with any GRASP implementation that:

- Stores git data in the `<npub>/<identifier>.git` directory structure
- Uses standard GRASP events (kind 30617 announcements, kind 30618 state, kind 5 deletions)
- Exposes a Nostr relay WebSocket endpoint

**Known compatible implementations:**
- ngit-relay (reference implementation)
- ngit-grasp (when migrating between instances or from archive mode)
- Other GRASP-compliant relays following the specification

The migration scripts analyze Nostr events and git data directly, making them implementation-agnostic.

## Quick Start

Run the migration analysis with a single command:

```bash
# Basic analysis (fetches events, compares relays)
./docs/how-to/migration-scripts/run-migration-analysis.sh \
  --prod-relay wss://source-relay.example.com \
  --archive-relay wss://target-relay.example.com

# Full analysis (includes git sync check - run on VPS)
./docs/how-to/migration-scripts/run-migration-analysis.sh \
  --prod-relay wss://source-relay.example.com \
  --archive-relay wss://target-relay.example.com \
  --prod-git /var/lib/grasp-relay/git \
  --archive-git /var/lib/ngit-grasp/git \
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

- SSH access to the VPS running your source relay
- Read access to git data directories
- Access to systemd journal (for log extraction)

### Verify Installation

```bash
# Check required tools
nak --version
jq --version
git --version

# Check optional tools (for VPS phases)
journalctl --version
```

## Gotchas and Common Issues

Before running the analysis, be aware of these common issues discovered during real migrations:

### Git Must Be Installed

The analysis scripts require `git` to be installed and in PATH. This may not be present on minimal VPS installations.

```bash
# Check if git is available
which git || echo "Git not found - install it first"

# Install on Debian/Ubuntu
apt install git

# Install on NixOS (add to configuration.nix)
environment.systemPackages = [ pkgs.git ];
```

### Archive Relay May Only Be Accessible Locally

If your archive relay is configured to listen only on localhost (e.g., `ws://localhost:7443`), you must run the analysis **on the VPS itself**, not from a remote machine.

```bash
# Check if archive relay is accessible
# This will fail if run remotely against a localhost-only relay
nak req -k 30618 --limit 1 ws://localhost:7443

# Solution: SSH into the VPS and run analysis there
ssh user@your-vps
cd /path/to/scripts
./run-migration-analysis.sh --archive-relay ws://localhost:7443 ...
```

### Git Data Paths May Differ from Defaults

Different deployments store git data in different locations. **Always verify paths before running the analysis.**

```bash
# Find actual git data paths from service configuration
systemctl cat ngit-relay.service | grep -E 'ExecStart|WorkingDirectory|Environment'
systemctl cat ngit-grasp-*.service | grep -E 'ExecStart|WorkingDirectory|Environment'

# Common locations:
# - /var/lib/ngit-relay/git (default)
# - /var/lib/ngit-grasp/git (default)
# - /persistent/*/data/repos (custom deployments)

# Verify the path exists and contains expected structure
ls /path/to/git/npub1*/  # Should show *.git directories
```

### Phase 4 Needs the Correct Service Name

> **CRITICAL:** Phase 4 extracts structured logs (`[PARSE_FAIL]`, `[PURGATORY_EXPIRED]`) from journald. These logs **ONLY exist in ngit-grasp services**, NOT in ngit-relay services.

If you specify an ngit-relay service (like `ngit-relay.service`), Phase 4 will find **zero logs** and produce empty results. This is a common mistake that wastes time and produces misleading analysis.

**Correct service names (ngit-grasp):**
- `ngit-grasp.service`
- `ngit-grasp-relay-ngit-dev.service` (NixOS multi-instance)
- `ngit-grasp-archive.service`

**Incorrect service names (ngit-relay - NO structured logging):**
- `ngit-relay.service`
- `relay-ngit-dev.service`

```bash
# Find all ngit-related services
systemctl list-units 'ngit-*' --all

# Check which service has structured logging (should be ngit-grasp)
journalctl -u ngit-grasp-*.service | grep -E '\[PARSE_FAIL\]|\[PURGATORY_EXPIRED\]' | head -5

# Verify ngit-relay does NOT have structured logging
journalctl -u ngit-relay.service | grep -E '\[PARSE_FAIL\]|\[PURGATORY_EXPIRED\]' | head -5
# ^ This should return nothing

# Use the archive service name for Phase 4
./run-migration-analysis.sh ... --service ngit-grasp-relay-ngit-dev.service
```

The migration scripts now validate the service name and will **error** if you specify an ngit-relay service, preventing this common mistake.

### Permission Issues with Service-Owned Directories

Git data directories are typically owned by the service user and may require elevated permissions to read.

```bash
# Check directory permissions
ls -la /var/lib/ngit-grasp/git

# Options:
# 1. Run as root/sudo
sudo ./run-migration-analysis.sh ...

# 2. Run as the service user
sudo -u ngit-grasp ./run-migration-analysis.sh ...

# 3. Add your user to the service group
sudo usermod -aG ngit-grasp $USER
# (logout/login required)
```

### Service Names Vary by Deployment

NixOS multi-instance deployments use service names like `ngit-grasp-<instance>.service`. Always check actual service names.

```bash
# List all ngit services
systemctl list-units 'ngit-*' --all --no-pager

# Example output:
# ngit-relay.service                loaded active running  ngit-relay
# ngit-grasp-relay-ngit-dev.service loaded active running  ngit-grasp (relay-ngit-dev)
```

## Migration Overview

The migration process has three stages:

### Stage 1: Deploy Archive Instance

Deploy ngit-grasp alongside your production relay:

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

### Before You Start

**Verify paths and service names** before running the analysis. Incorrect paths are the most common source of errors.

```bash
# 1. Find actual git data paths
systemctl cat ngit-relay.service | grep -E 'ExecStart|data|git'
systemctl cat ngit-grasp-*.service | grep -E 'ExecStart|data|git'

# 2. Find service names
systemctl list-units 'ngit-*' --all --no-pager

# 3. Verify git data exists at the paths
ls /path/to/prod/git/npub1*/ | head -5
ls /path/to/archive/git/npub1*/ | head -5

# 4. Check if archive relay is accessible
nak req -k 30618 --limit 1 ws://localhost:7443  # or your archive URL
```

### Basic Usage

```bash
# Preview what will happen (dry run)
./run-migration-analysis.sh \
  --prod-relay wss://source-relay.example.com \
  --archive-relay wss://target-relay.example.com \
  --dry-run

# Run the analysis
./run-migration-analysis.sh \
  --prod-relay wss://source-relay.example.com \
  --archive-relay wss://target-relay.example.com
```

### Full Analysis on VPS

**Important:** If your archive relay is localhost-only, you must run this on the VPS.

```bash
# First, discover your actual paths (see "Before You Start" above)
# Then run with the correct values:

./run-migration-analysis.sh \
  --prod-relay wss://source-relay.example.com \
  --archive-relay ws://localhost:7443 \
  --prod-git /path/to/prod/git \
  --archive-git /path/to/archive/git \
  --service ngit-grasp-your-instance.service
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
| `--prod-relay <url>` | Source relay WebSocket URL (required) |
| `--archive-relay <url>` | Target relay WebSocket URL (required) |
| `--prod-git <path>` | Git base directory for prod (enables Phase 2) |
| `--archive-git <path>` | Git base directory for archive (enables Phase 2) |
| `--service <name>` | Systemd service name for Phase 4 log extraction. **MUST be an ngit-grasp service** (not ngit-relay). Structured logging only exists in ngit-grasp. |
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

### "git not found"

Git must be installed and in PATH:

```bash
# Check if git is available
which git

# Install on Debian/Ubuntu
sudo apt install git

# Install on NixOS (add to configuration.nix)
environment.systemPackages = [ pkgs.git ];
```

### "Permission denied" on git directories

Run with sudo or ensure your user has read access:

```bash
# Check permissions
ls -la /var/lib/grasp-relay/git

# Option 1: Run with sudo
sudo ./run-migration-analysis.sh ...

# Option 2: Run as service user
sudo -u ngit-grasp ./run-migration-analysis.sh ...
```

### Archive relay connection failed

If you get connection errors to the archive relay:

```bash
# Check if relay is running
systemctl status ngit-grasp-*.service

# Check if it's localhost-only
# If archive is ws://localhost:7443, you MUST run on the VPS
ssh user@your-vps
./run-migration-analysis.sh --archive-relay ws://localhost:7443 ...
```

### Wrong git paths / "No such file or directory"

Git data paths vary by deployment. Discover the actual paths:

```bash
# Find paths from service configuration
systemctl cat ngit-relay.service | grep -E 'ExecStart|WorkingDirectory|Environment'
systemctl cat ngit-grasp-*.service | grep -E 'ExecStart|WorkingDirectory|Environment'

# Verify the path contains git repos
ls /discovered/path/npub1*/
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

### Phase 4 finds no structured logs

**Symptom:** Phase 4 completes but `parse-failures.txt` and `purgatory-expired.txt` are empty or contain only header comments.

**Most common cause:** You're querying the wrong service (ngit-relay instead of ngit-grasp).

Structured logging (`[PARSE_FAIL]`, `[PURGATORY_EXPIRED]`) **only exists in ngit-grasp services**. If you specify an ngit-relay service, Phase 4 will find zero logs.

**How to diagnose:**

```bash
# 1. Check what service you configured
cat /path/to/output/config.txt | grep SERVICE_NAME

# 2. If it contains "ngit-relay", that's the problem!
# ngit-relay does NOT have structured logging

# 3. Find the correct ngit-grasp service
systemctl list-units 'ngit-grasp*' --all

# 4. Verify the ngit-grasp service has structured logs
journalctl -u ngit-grasp-relay-ngit-dev.service --since "7 days ago" | \
  grep -E '\[PARSE_FAIL\]|\[PURGATORY_EXPIRED\]' | head -5
```

**How to fix:**

```bash
# Update SERVICE_NAME to the ngit-grasp archive service and re-run
./run-migration-analysis.sh \
  --prod-relay wss://relay.ngit.dev \
  --archive-relay ws://localhost:7443 \
  --service ngit-grasp-relay-ngit-dev.service \
  --from-phase-4  # Skip phases 1-3, just re-run phase 4
```

**Other possible causes:**

1. **Structured logging not deployed:** If the ngit-grasp instance doesn't have the logging improvements deployed, no structured logs will exist. Check the ngit-grasp version.

2. **No events in time window:** If there genuinely were no parse failures or purgatory expiry events, the files will be empty. This is valid - it means everything parsed successfully.

3. **Wrong time range:** The default is 30 days. If your archive has been running longer, you may need `--since` to extend the range.

**Prevention:** The migration scripts now validate the service name and will error if you specify an ngit-relay service.

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

## Why Migration May Require Attention

Different GRASP implementations may handle edge cases differently. ngit-grasp has stricter validation and better observability, which can surface issues that were previously hidden:

| Aspect | Typical Source Relay | ngit-grasp |
|--------|---------------------|------------|
| Git data validation | May accept partial data | Requires all git data to reproduce state |
| PR refs cleanup | May not clear `refs/nostr/<event-id>` | Properly manages PR refs |
| Parse failures | May silently ignore | Logs structured `[PARSE_FAIL]` entries |
| Sync timeout | May have no timeout | Purgatory expires after configurable period |

These differences explain why some repositories may need attention during migration - ngit-grasp's stricter validation catches issues that other implementations may have silently accepted.

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
./migration-scripts/01-fetch-events.sh wss://source-relay.example.com output/prod

# Phase 2: Git sync check
./migration-scripts/10-check-git-sync.sh output/prod/raw/state-events.json /var/lib/grasp-relay/git output/prod --categorize

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

## relay.ngit.dev Migration Notes

This section documents the specific configuration and lessons learned from migrating relay.ngit.dev from ngit-relay to ngit-grasp. Use this as a reference for similar deployments.

### Deployment Configuration

| Component | Value |
|-----------|-------|
| **Production relay** | `wss://relay.ngit.dev` |
| **Production service** | `ngit-relay.service` |
| **Production git path** | `/persistent/relay-ngit-dev-ngit-relay/data/repos` |
| **Archive relay** | `ws://localhost:7443` (localhost only) |
| **Archive service** | `ngit-grasp-relay-ngit-dev.service` |
| **Archive git path** | `/persistent/grasp/relay-ngit-dev/git` |

### Key Differences from Defaults

1. **Git paths are non-standard**: The production relay uses `/persistent/relay-ngit-dev-ngit-relay/data/repos` instead of `/var/lib/ngit-relay/git`

2. **Archive is localhost-only**: The archive relay listens on `ws://localhost:7443`, not a public URL. All analysis must run on the VPS.

3. **Service names include instance**: NixOS multi-instance deployment uses `ngit-grasp-relay-ngit-dev.service`, not `ngit-grasp.service`

### Analysis Command

```bash
# Run on VPS (archive is localhost-only)
./docs/how-to/migration-scripts/run-migration-analysis.sh \
  --prod-relay wss://relay.ngit.dev \
  --archive-relay ws://localhost:7443 \
  --prod-git /persistent/relay-ngit-dev-ngit-relay/data/repos \
  --archive-git /persistent/grasp/relay-ngit-dev/git \
  --service ngit-grasp-relay-ngit-dev.service
```

### Analysis Results (January 2026)

| Category | Count | Notes |
|----------|-------|-------|
| Complete in both | ~400 | Ready for migration |
| Complete in prod, missing from archive | 315 | Need re-sync |
| Empty in both | 100 | Users never pushed git data |
| Manual investigation | 5 | Unusual states |
| Purgatory expired | 382 | Structured logging working |

### Lessons Learned

1. **Always verify paths first**: The default paths in examples didn't match the actual deployment. Use `systemctl cat <service>` to find real paths.

2. **Check archive accessibility**: We initially tried to run analysis remotely, but the archive relay was localhost-only. Had to SSH to VPS.

3. **Use archive service for Phase 4 (CRITICAL)**: Structured logging (`[PARSE_FAIL]`, `[PURGATORY_EXPIRED]`) is **ONLY** in the ngit-grasp archive service, NOT the ngit-relay production service. Running Phase 4 against `ngit-relay.service` produces zero results because ngit-relay doesn't emit structured logs. The scripts now validate this and error if you specify an ngit-relay service.

4. **Install git on VPS**: Git wasn't installed on the minimal VPS. The scripts now check for this in prerequisites.

5. **Permissions matter**: Some directories required `sudo` to access. Running as root or the service user resolved this.

### Next Steps for relay.ngit.dev

1. **Re-sync 315 repos**: Trigger archive to re-fetch from production
2. **Investigate 5 edge cases**: Manual review of unusual states
3. **Monitor purgatory**: 382 expired entries indicate sync issues to investigate
4. **Plan cutover**: Once re-sync complete, switch DNS/proxy to ngit-grasp
