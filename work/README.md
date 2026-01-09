# Work Directory

**Purpose:** Temporary working documents during development sessions  
**Lifecycle:** Created during session → Archived at session end  
**Status:** `.gitignore`d - not committed to version control

---

## What Goes Here

- Session summaries and notes
- Status reports and visual summaries
- Migration documentation (during migration)
- Planning documents
- Temporary analysis files
- **active-issues/**: Issues discovered during production sync testing (see [docs/how-to/production-sync-testing.md](../docs/how-to/production-sync-testing.md))

**Rule:** Nothing in this directory should be permanent. Archive or delete at session end.

---

## Workflow

### During Session
```bash
# Create working docs here
echo "Session notes..." > work/session-notes.md
echo "Status..." > work/status.md
```

### End of Session
```bash
# Archive important docs
mv work/session-notes.md docs/archive/2025-11-04-session-notes.md

# Delete obsolete docs
rm work/status.md

# Clean up
rm -rf work/*
```

---

## .gitignore

This directory is ignored by git (except this README):

```
work/*
!work/README.md
```

**Why:** Working documents are session-specific and shouldn't clutter the repository.

---

## Best Practices

**DO:**
- ✅ Use for temporary session work
- ✅ Use descriptive names
- ✅ Archive valuable content before deleting
- ✅ Clean up at session end

**DON'T:**
- ❌ Put permanent documentation here
- ❌ Reference work/ docs from permanent docs
- ❌ Commit work/ contents to git
- ❌ Let it accumulate files

---

## Alternative: Session-Specific Directories

For complex sessions, create dated subdirectories:

```bash
work/
├── 2025-11-04-diataxis-migration/
│   ├── notes.md
│   ├── checklist.md
│   └── visual-summary.txt
└── 2025-11-05-feature-x/
    └── plan.md
```

Archive the entire directory when done:
```bash
tar czf docs/archive/2025-11-04-diataxis-migration.tar.gz work/2025-11-04-diataxis-migration/
rm -rf work/2025-11-04-diataxis-migration/
```

---

*This README is the only file in work/ committed to git.*
