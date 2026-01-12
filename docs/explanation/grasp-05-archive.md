# GRASP-05 Archive Mode

**Purpose:** Understand archive/mirror/backup functionality  
**Audience:** Operators and developers

---

## What It Does

GRASP-05 enables ngit-grasp to accept repository announcements that **don't list your relay**, allowing you to run an archive, mirror, or backup service.

**Standard GRASP-01:** Announcement must list your service → You host it (read/write)  
**GRASP-05 Extension:** Announcement matches your whitelist → You archive it (read-only)

## Why It Exists

### Problem
In GRASP-01 strict mode, you can only host repositories whose maintainers explicitly list your relay. This prevents:
- Creating backup archives of critical projects without maintainer cooperation
- Building comprehensive mirrors of the Nostr Git ecosystem
- Providing disaster recovery for projects that might disappear

### Solution
Archive mode relaxes the "must list service" requirement for whitelisted repositories, enabling passive mirroring while maintaining read-only guarantees.

## How It Works

### Three Whitelist Formats

| Format | Example | Archives |
|--------|---------|----------|
| `<npub>` | `npub1alice...` | All repos from Alice |
| `<npub>/<identifier>` | `npub1bob.../linux` | Only Bob's linux repo |
| `<identifier>` | `bitcoin-core` | Any bitcoin-core repo (⚠️ any pubkey) |

**Configuration:**
```bash
# Specific repos (safest) - read-only by default
NGIT_ARCHIVE_WHITELIST=npub1torvalds.../linux,npub1satoshi.../bitcoin
# NGIT_ARCHIVE_READ_ONLY defaults to true

# All repos from trusted maintainers
NGIT_ARCHIVE_WHITELIST=npub1alice...,npub1bob...
# NGIT_ARCHIVE_READ_ONLY defaults to true

# Archive everything (⚠️ storage risk)
NGIT_ARCHIVE_ALL=true
# NGIT_ARCHIVE_READ_ONLY defaults to true
```

### Validation Priority

Announcements are checked in this order:

1. **Lists your service?** → `Accept` (GRASP-01, read/write)
2. **Is author a maintainer?** → `AcceptMaintainer` (multi-maintainer, read/write)
3. **Matches archive config?** → `AcceptArchive` (GRASP-05, read-only)
4. **None of the above** → `Reject`

This ensures GRASP-01 compliant repos are always writable, even if they match the archive whitelist.

### Storage Model

Archived repos use the same directory structure as hosted repos:
```
<git_data_path>/
  npub1alice.../
    hosted-repo.git/     # Lists your service (writable)
    archived-repo.git/   # Whitelisted (read-only by default)
```

**No flags or metadata** - archive status determined dynamically from config + announcement contents.

### Read-Only Mode

By default, archive mode operates in read-only mode (`NGIT_ARCHIVE_READ_ONLY=true`):
- Repository announcements are accepted per whitelist/archive-all configuration
- The service is **not listed** in accepted announcements (passive sync only)
- NIP-11 document advertises `GRASP-05` support
- NIP-11 `curation` field indicates read-only sync scope:
  - `"Read-only sync of all repositories found on network"` (if `NGIT_ARCHIVE_ALL=true`)
  - `"Read-only sync of whitelisted repositories and maintainers"` (if whitelist configured)

### Full Sync

Archived repositories trigger complete GRASP-02 sync:
- ✅ Nostr events (PRs, issues, patches)
- ✅ Git data via purgatory
- ✅ Same validation as hosted repos

Archive mode is a **complete mirror**, not just git-only backup.

## Security Considerations

### 1. Archive-All Mode (Dangerous)

**Don't use `NGIT_ARCHIVE_ALL=true` unless:**
- You have unlimited storage/bandwidth
- You trust the relay network
- You've implemented monitoring

**Attack vector:** Anyone can publish announcements → unlimited storage consumption.

### 2. Identifier-Only Format (Risky)

```bash
NGIT_ARCHIVE_WHITELIST=bitcoin-core  # Matches ANY pubkey!
```

Malicious users can publish fake repos with popular identifiers. Use `<npub>/<identifier>` for high-value archives.

### 3. Npub Validation

Invalid npubs → server fails to start (fail-fast). Identifiers aren't validated (any string allowed).

## Operational Guide

### Start Small

```bash
# Day 1: One critical repo
NGIT_ARCHIVE_WHITELIST=npub1torvalds.../linux

# Week 1: Add trusted maintainers
NGIT_ARCHIVE_WHITELIST=npub1alice...,npub1bob...

# Month 1: Consider popular identifiers (with monitoring)
NGIT_ARCHIVE_WHITELIST=npub1alice...,bitcoin-core
```

### Monitor Growth

Watch for:
- Storage consumption rate
- Purgatory git fetch failures
- Bandwidth usage spikes

### Whitelist Changes

**Current:** Static config - edit `.env`, restart server  
**Future:** REST API for dynamic management (no restart)

## Comparison: Hosted vs Archived

| Aspect | Hosted (GRASP-01) | Archived (GRASP-05 Read-Only) |
|--------|-------------------|-------------------------------|
| Announcement must list you | ✅ Required | ❌ Whitelisted instead |
| Git pushes | ✅ Accepted | ❌ Rejected (read-only) |
| GRASP-02 sync | ✅ Full sync | ✅ Full sync |
| Relay discovery | ✅ Listed in announcements | ❌ Not listed (passive sync) |
| NIP-11 supported_grasps | `["GRASP-01", "GRASP-02"]` | `["GRASP-01", "GRASP-05", "GRASP-02"]` |
| NIP-11 curation field | `null` | Describes archive scope |
| Use case | Hosting workspace | Backup/mirror |

## Related Documentation

- [Configuration Reference](../reference/configuration.md) - `NGIT_ARCHIVE_*` options
- [GRASP-05 Spec](https://gitworkshop.dev/danconwaydev.com/grasp/05.md) - Protocol specification
- [GRASP-02 Sync](./grasp-02-proactive-sync.md) - How sync works

---

_Part of the [ngit-grasp explanation documentation](./)_
