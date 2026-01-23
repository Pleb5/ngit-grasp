# Migrate ngit-relay to ngit-grasp on NixOS VPS

**Goal:** Replace an ngit-relay instance on a VPS running NixOS with ngit-grasp.

**Specifics:** VPS running NixOS.

## Approach

1. Deploy ngit-grasp with 'domain' of `<prod-domain>.internal` and an `archiveService` of `<prod-domain>` running on a different port. This will gather all the events and git data from the production service and relays/git servers/grasp servers that for repositories that list the service in their announcement event. To sync all git data may take an hour.

2. Analyze the data to see which repositories have not been moved with complete data. Understand why and for each decide if action is needed / not needed to move it.

3. Set the 'domain' to production URL, turn off archive mode, and point your reverse proxy at the new port.

## Challenges

- **ngit-relay accepts any commits/annotated tags** that were at that point of time referenced in the latest state event. **ngit-grasp requires all the git data** to reproduce the latest state. So if the git data is incomplete, it won't accept the repository.

- **ngit-relay doesn't clear out refs/nostr/<event-id>** where it doesn't have a PR event. Fortunately the 'PR' (as opposed to patches) functionality is not widely used so we just need to check a few repositories (shakespeare, ngit and gitworkshop).

## Analysis Categories

### No action required:

| Category | How to Detect | Source |
|----------|---------------|--------|
| **Git Data Complete - Moved** | prod cat1 AND archive cat1 (same repo) | Git sync check |
| **Invalid Announcement** (Won't Parse) | Log: `[PARSE_FAIL] kind=30617` | Archive logs |
| **Deletion Request** | kind 5 event tagging announcement | Event fetch |
| **Announcement Not on Prod But In Archive** | In archive announcements, not in prod | Event comparison |

### Action/decision required:

| Category | How to Detect | Source |
|----------|---------------|--------|
| **Invalid State Event** (Won't Parse) | Log: `[PARSE_FAIL] kind=30618` | Archive logs |
| **Purgatory Expired** (sync should have worked) | Log: `[PURGATORY_EXPIRED]` | Archive logs |
| **Incomplete Git Data** (both relays) | prod cat2/3/4 AND archive cat2/3/4 | Git sync check |
| **No Announcement In Archive** | In prod, not in archive, no deletion | Event comparison |
| **State but incomplete git in Archive** | archive cat3 or cat4 | Git sync check |

### Manual investigation required:

- Repos that don't fit above categories
- Repos with unexpected state (e.g., complete in prod, missing in archive, no log entries)

## Analysis Script Architecture

The analysis is split into modular phases for fast iteration. Phases 1-3 and 5 can run locally; Phase 2 and 4 require VPS access.

```
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 1: Fetch Events (~30s, local)                             │
│ migration-scripts/01-fetch-events.sh <relay> <output-dir>       │
├─────────────────────────────────────────────────────────────────┤
│ Fetches from relay:                                             │
│   - kind 30618 (state events)                                   │
│   - kind 30617 (announcements)                                  │
│   - kind 5 (deletion requests)                                  │
│                                                                 │
│ Run twice: once for prod (relay.ngit.dev), once for archive     │
│ Output: <output-dir>/{state,announcements,deletions}.json       │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 2: Git Sync Check (~20 mins, VPS required)                │
│ migration-scripts/10-check-git-sync.sh <events> <git-base> <out>│
├─────────────────────────────────────────────────────────────────┤
│ For each state event, compares refs to actual git data on disk. │
│                                                                 │
│ Run twice:                                                      │
│   - prod: GIT_BASE=/persistent/relay-ngit-dev-ngit-relay/...    │
│   - archive: GIT_BASE=/persistent/grasp/sync-archive/git        │
│                                                                 │
│ Output: git-sync-status.tsv                                     │
│   repo|npub|state_refs|git_refs|matches|status                  │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 3: Categorize & Compare (fast, local)                     │
│ migration-scripts/20-categorize.sh <sync-status> <output-dir>   │
│ migration-scripts/21-compare-relays.sh <prod> <archive> <out>   │
├─────────────────────────────────────────────────────────────────┤
│ 20-categorize.sh applies 4-category logic:                      │
│   - cat1: complete match (all refs match)                       │
│   - cat2: empty/blank (no git data)                             │
│   - cat3: partial match (some refs match)                       │
│   - cat4: no match (git exists but refs don't match)            │
│                                                                 │
│ 21-compare-relays.sh compares prod vs archive:                  │
│   - complete-in-both.txt (no action needed)                     │
│   - complete-prod-missing-archive.txt (needs investigation)     │
│   - complete-prod-incomplete-archive.txt (sync in progress?)    │
│   - incomplete-in-both.txt (git data incomplete)                │
│   - in-archive-not-prod.txt (deleted or new)                    │
│                                                                 │
│ Output: category-{1,2,3,4}.txt, comparison/*.txt, summary.txt   │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 4: Log-Based Categories (VPS required)                    │
│ migration-scripts/30-extract-parse-failures.sh <service> <out>  │
│ migration-scripts/31-extract-purgatory-expiry.sh <service> <out>│
├─────────────────────────────────────────────────────────────────┤
│ Extracts structured log entries from journalctl:                │
│   - Parse failures: [PARSE_FAIL] kind=X event_id=Y reason=Z     │
│   - Purgatory expiry: [PURGATORY_EXPIRED] repo=X npub=Y         │
│                                                                 │
│ NOTE: Requires logging improvements in ngit-grasp to emit       │
│ these structured log entries. See issue: TBD                    │
│                                                                 │
│ Output: parse-failures.txt, purgatory-expired.txt               │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ PHASE 5: Final Classification (fast, local)                     │
│ migration-scripts/40-classify-actions.sh <all-inputs> <out>     │
├─────────────────────────────────────────────────────────────────┤
│ Combines all data sources to produce final classification:      │
│                                                                 │
│ Inputs:                                                         │
│   - category files (prod and archive)                           │
│   - relay-gaps.txt                                              │
│   - parse-failures.txt                                          │
│   - purgatory-expired.txt                                       │
│   - deletions.json                                              │
│                                                                 │
│ Output:                                                         │
│   - no-action-required.txt (repo|reason)                        │
│   - action-required.txt (repo|reason|suggested_action)          │
│   - manual-investigation.txt (repo|notes)                       │
└─────────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
work/migration-analysis-YYYYMMDD-HHMM/
├── prod/
│   ├── raw/
│   │   ├── state-events.json       # Phase 1 output
│   │   ├── announcements.json      # Phase 1 output
│   │   └── deletions.json          # Phase 1 output
│   ├── git-sync-status.tsv         # Phase 2 output (optional)
│   ├── category1-complete-match.txt    # Phase 2/3 output
│   ├── category2-empty-blank.txt       # Phase 2/3 output
│   ├── category3-partial-match.txt     # Phase 2/3 output
│   └── category4-no-match.txt          # Phase 2/3 output
├── archive/
│   ├── raw/
│   │   ├── state-events.json
│   │   ├── announcements.json
│   │   └── deletions.json
│   ├── git-sync-status.tsv
│   ├── category1-complete-match.txt
│   ├── category2-empty-blank.txt
│   ├── category3-partial-match.txt
│   └── category4-no-match.txt
├── logs/
│   ├── parse-failures.txt          # Phase 4 output
│   └── purgatory-expired.txt       # Phase 4 output
├── comparison/
│   ├── complete-in-both.txt            # Phase 3 output (no action)
│   ├── complete-prod-missing-archive.txt   # Phase 3 output (investigate)
│   ├── complete-prod-incomplete-archive.txt # Phase 3 output (sync in progress?)
│   ├── incomplete-in-both.txt          # Phase 3 output (git incomplete)
│   ├── in-archive-not-prod.txt         # Phase 3 output (deleted/new)
│   └── summary.txt                     # Phase 3 output (human-readable)
└── results/
    ├── no-action-required.txt      # Phase 5 output
    ├── action-required.txt         # Phase 5 output
    └── manual-investigation.txt    # Phase 5 output
```

## Prerequisites

- `nak` - Nostr Army Knife for fetching events
- `jq` - JSON processing
- SSH access to VPS for Phase 2 and 4
- Logging improvements in ngit-grasp for Phase 4 (see Dependencies)

## Dependencies

Phase 4 requires structured logging in ngit-grasp. Create a separate issue to add:

```rust
// On parse failure:
tracing::warn!(
    target: "migration",
    "[PARSE_FAIL] kind={} event_id={} reason=\"{}\"",
    event.kind, event.id, reason
);

// On purgatory expiry:
tracing::warn!(
    target: "migration",
    "[PURGATORY_EXPIRED] repo={} npub={}",
    identifier, npub
);
```

## Gotchas

- Always use `nak req` with `--paginate` flag so we don't miss any events. If we receive increments of 250 (e.g., exactly 500) then it's a red flag that we are not paginating and there are probably more events.
- Phase 1 and 2 should run back-to-back for an accurate snapshot.
- The git sync check (Phase 2) takes ~20 minutes per relay - this is the slow part.
- Existing analysis data from Jan 22 can be used for developing Phase 3/5 logic before re-running Phase 2.
