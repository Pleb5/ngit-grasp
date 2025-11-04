# Archive - Historical Documentation

**Purpose:** Completed session documents, phase reports, and historical records  
**Status:** Read-only - documents are not modified after archiving

---

## Archive Organization

Documents are organized by date (YYYY-MM-DD) and topic.

### November 3, 2025 - Architecture Investigation & Initial Implementation

**Architecture Investigation:**
- `2025-11-03-architecture-investigation.md` - GRASP protocol investigation complete
- `2025-11-03-review-summary.md` - Executive summary of investigation
- `2025-11-03-documentation-index.md` - Initial docs structure

**grasp-audit Implementation:**
- `2025-11-03-grasp-audit-plan.md` - Audit tool design decisions
- `2025-11-03-grasp-audit-implementation.md` - Implementation summary
- `2025-11-03-implementation-complete.md` - Initial implementation complete
- `2025-11-03-verification-complete.md` - Verification results

**Testing:**
- `2025-11-03-compliance-test-proposal.md` - Test strategy proposal
- `2025-11-03-compliance-testing-report.md` - Compliance testing report
- `2025-11-03-test-breakdown.md` - Detailed test breakdown
- `2025-11-03-smoke-test-report.md` - Smoke test results
- `2025-11-03-final-audit-report.md` - Final audit report
- `2025-11-03-final-summary.md` - Final summary

**Reference:**
- `2025-11-03-files-created.md` - Files created during investigation
- `2025-11-03-quick-reference.md` - Quick reference guide
- `2025-11-03-start-here.md` - Getting started guide

---

### November 4, 2025 - Upgrades & Migrations

**Tag Migration:**
- `2025-11-04-tag-migration.md` - Migration to standard "t" tags (detailed)
- `2025-11-04-tag-migration-summary.md` - Migration summary

**Flake Migration:**
- `2025-11-04-flake-migration.md` - shell.nix → flake.nix migration

**nostr-sdk Upgrade:**
- `2025-11-04-nostr-sdk-upgrade.md` - 0.35 → 0.43 upgrade guide
- `2025-11-04-upgrade-complete.md` - Upgrade completion report

**Fixes & Improvements:**
- `2025-11-04-compilation-fixes.md` - Compilation fixes
- `2025-11-04-audit-system-fixed.md` - Audit system fixes
- `2025-11-04-audit-status-report.md` - Audit status report

**Session Summaries:**
- `2025-11-04-session-summary.md` - Main session summary
- `2025-11-04-session-complete-1.md` - Session completion 1
- `2025-11-04-session-complete-2.md` - Session completion 2
- `2025-11-04-session-continuation.md` - Session continuation

**Planning:**
- `2025-11-04-next-session-quickstart.md` - Next session quickstart
- `2025-11-04-next-prompt.md` - Next prompt planning
- `2025-11-04-ready-for-next-phase.md` - Phase readiness report

---

## Using Archived Documents

### When to Reference

✅ **Good reasons to reference:**
- Understanding historical context
- Learning from past decisions
- Reviewing what was tried before
- Tracking project evolution

❌ **Don't reference for:**
- Current implementation details (use `docs/` instead)
- Active development (use `CURRENT_STATUS.md`)
- Reusable patterns (use `docs/learnings/`)

### Extracting Learnings

If you find useful patterns or gotchas in archived documents:

1. Extract to appropriate `docs/learnings/*.md` file
2. Update with current context
3. Link to archive for historical context

**Example:**
```markdown
<!-- In docs/learnings/nostr-sdk.md -->

## Tag Migration Pattern

When changing tag structure...

**Reference:** See `docs/archive/2025-11-04-tag-migration.md` for detailed migration story.
```

---

## Archive Principles

1. **Immutable**: Documents are not modified after archiving
2. **Dated**: All filenames include YYYY-MM-DD prefix
3. **Organized**: Grouped by date and topic
4. **Referenced**: Can be linked from active docs for context
5. **Searchable**: Full-text search helps find historical info

---

## Document Lifecycle

```
Working Doc (root)
    ↓
Extract Learnings → docs/learnings/
    ↓
Archive → docs/archive/
    ↓
Reference (read-only)
```

---

## Quick Find

### By Topic

- **Architecture**: `2025-11-03-architecture-investigation.md`
- **Testing**: `2025-11-03-*-test-*.md`
- **Migrations**: `2025-11-04-*-migration.md`
- **Upgrades**: `2025-11-04-*-upgrade.md`
- **Sessions**: `2025-11-04-session-*.md`

### By Date

- **Nov 3**: Initial investigation and implementation
- **Nov 4**: Upgrades, migrations, and refinements

---

## Related Documentation

- **Active Status**: `../CURRENT_STATUS.md`
- **Learnings**: `../learnings/`
- **Architecture**: `../ARCHITECTURE.md`
- **Guidelines**: `../../AGENTS.md`

---

*Archive established: November 4, 2025*  
*Total documents: 30*
