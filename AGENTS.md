# AI Agent Guidelines for ngit-grasp

**Purpose:** Ensure AI agents (and humans) maintain consistent documentation practices and avoid common pitfalls.

**Last Updated:** November 4, 2025

---

## 📁 Documentation Structure

### Overview

We maintain a **clean, hierarchical documentation structure** to avoid documentation sprawl. All working documents have a defined lifecycle and location.

```
ngit-grasp/
├── README.md                    # Project overview (keep updated)
├── AGENTS.md                    # This file - agent guidelines
├── CHANGELOG.md                 # User-facing changes (semver)
│
├── docs/                        # Permanent technical documentation
│   ├── README.md               # Docs navigation guide
│   ├── ARCHITECTURE.md         # System architecture
│   ├── TEST_STRATEGY.md        # Testing approach
│   ├── GIT_PROTOCOL.md         # Git protocol reference
│   ├── COMPARISON.md           # vs other implementations
│   ├── GETTING_STARTED.md      # Setup guide
│   └── DECISION_SUMMARY.md     # Key architectural decisions
│
├── docs/archive/               # Completed session/phase docs
│   ├── 2025-11-04-tag-migration.md
│   ├── 2025-11-04-flake-migration.md
│   └── 2025-11-03-architecture-investigation.md
│
├── docs/learnings/             # Extracted knowledge (permanent)
│   ├── nix-flakes.md          # Flake gotchas and patterns
│   ├── nostr-sdk.md           # nostr-sdk patterns and upgrades
│   └── git-http-backend.md    # Git protocol learnings
│
├── grasp-audit/                # Audit tool subproject
│   ├── README.md              # Main audit docs
│   ├── QUICK_START.md         # Getting started
│   └── docs/
│       └── archive/           # Audit-specific archives
│
└── .ai/                        # AI assistant context (ignored in git)
    └── history/               # Conversation history
```

---

## 📋 Document Lifecycle

### 1. Working Documents (Root Level)

**Purpose:** Active development, session notes, status reports  
**Location:** Project root  
**Lifecycle:** Created → Updated → Archived  
**Retention:** Archive after completion, delete if obsolete

**Examples:**
- `TAG_MIGRATION_COMPLETE.md` → Archive when next phase starts
- `SESSION_2025_11_04_SUMMARY.md` → Archive at session end
- `NEXT_STEPS.md` → Update continuously, archive when complete

**Rules:**
- ✅ Use descriptive names with dates: `YYYY-MM-DD-description.md`
- ✅ Mark status clearly: `[WIP]`, `[COMPLETE]`, `[ARCHIVED]`
- ✅ Include date and context at top
- ❌ Don't let root accumulate more than 5-10 working docs
- ❌ Don't create duplicates (merge or link instead)

### 2. Permanent Documentation (docs/)

**Purpose:** Long-term reference, architecture, guides  
**Location:** `docs/`  
**Lifecycle:** Created → Maintained → Updated  
**Retention:** Permanent (version controlled)

**Examples:**
- `docs/ARCHITECTURE.md` - System design
- `docs/TEST_STRATEGY.md` - Testing approach
- `docs/learnings/nix-flakes.md` - Extracted knowledge

**Rules:**
- ✅ Keep updated as project evolves
- ✅ Use clear structure and headings
- ✅ Link between related docs
- ❌ Don't duplicate information (use links)
- ❌ Don't include session-specific details

### 3. Archive (docs/archive/)

**Purpose:** Historical record, completed phases  
**Location:** `docs/archive/`  
**Lifecycle:** Moved from root → Archived  
**Retention:** Permanent (for reference)

**Examples:**
- `docs/archive/2025-11-04-tag-migration.md`
- `docs/archive/2025-11-03-architecture-investigation.md`

**Rules:**
- ✅ Rename with date prefix when archiving
- ✅ Add "ARCHIVED" marker at top
- ✅ Extract learnings to docs/learnings/ first
- ❌ Don't modify after archiving
- ❌ Don't reference in active documentation

### 4. Learnings (docs/learnings/)

**Purpose:** Reusable knowledge, gotchas, patterns  
**Location:** `docs/learnings/`  
**Lifecycle:** Extracted → Maintained → Updated  
**Retention:** Permanent (living documents)

**Examples:**
- `docs/learnings/nix-flakes.md` - Flake patterns and gotchas
- `docs/learnings/nostr-sdk.md` - SDK upgrade notes
- `docs/learnings/git-http-backend.md` - Git protocol tips

**Rules:**
- ✅ Extract from session docs before archiving
- ✅ Organize by topic, not by session
- ✅ Include code examples
- ✅ Update as we learn more
- ❌ Don't duplicate official docs (link instead)

---

## 🔄 Cleanup Process

### When to Clean Up

**Trigger:** Root directory has >10 markdown files  
**Frequency:** End of each major phase or weekly  
**Responsibility:** AI agents should proactively suggest cleanup

### Cleanup Steps

1. **Identify Completed Documents**
   ```bash
   # Find old working docs
   ls -lt *.md | head -20
   ```

2. **Extract Learnings**
   - Review each completed doc
   - Extract gotchas, patterns, solutions
   - Add to appropriate `docs/learnings/*.md`

3. **Archive Completed Work**
   ```bash
   # Move to archive with date prefix
   mv TAG_MIGRATION_COMPLETE.md docs/archive/2025-11-04-tag-migration.md
   ```

4. **Delete Obsolete Documents**
   - Duplicates (keep most recent/complete)
   - Superseded documents
   - Pure status reports (no learnings)

5. **Update References**
   - Update links in active docs
   - Update README.md if needed
   - Commit changes

### Example Cleanup

```bash
# Before cleanup (36 files in root!)
ls *.md | wc -l
# 36

# After cleanup (5-8 files in root)
ls *.md
# README.md
# AGENTS.md
# CHANGELOG.md
# CURRENT_STATUS.md
# NEXT_STEPS.md

# Archived
ls docs/archive/
# 2025-11-04-tag-migration.md
# 2025-11-04-flake-migration.md
# 2025-11-03-architecture-investigation.md
# ...

# Learnings extracted
ls docs/learnings/
# nix-flakes.md
# nostr-sdk.md
# git-http-backend.md
```

---

## 🚨 Common Gotchas

### Nix Flakes

**Always use `nix develop`, not `nix-shell`**

```bash
# ✅ Correct
cd grasp-audit
nix develop
nix develop -c cargo build

# ❌ Wrong
nix-shell
nix-shell --run "cargo build"
```

**Why:** We use `flake.nix`, not `shell.nix`. See `docs/learnings/nix-flakes.md`.

**Flake Commands:**
```bash
# Show flake outputs
nix flake show

# Update flake inputs
nix flake update

# Build package
nix build

# Run package
nix run
```

### Git Subprojects

**grasp-audit is a subproject with its own flake**

```bash
# ✅ Correct - enter grasp-audit environment
cd grasp-audit
nix develop
cargo build

# ❌ Wrong - can't build from root
cd ngit-grasp
cargo build  # This won't find grasp-audit
```

**Why:** `grasp-audit/` has its own `Cargo.toml` and `flake.nix`.

### nostr-sdk Versions

**We use nostr-sdk 0.43.x (latest stable)**

```toml
# ✅ Correct
[dependencies]
nostr-sdk = "0.43"

# ❌ Wrong
nostr-sdk = "0.35"  # Old version, breaking changes
```

**Why:** We upgraded from 0.35 to 0.43. See `docs/learnings/nostr-sdk.md` for migration notes.

**Common Breaking Changes:**
- `EventBuilder::new()` signature changed
- Tag API changed to `Tag::custom()`
- Filter API changed
- See archived upgrade docs for details

### Testing Patterns

**Integration tests require relay**

```bash
# ✅ Correct - start relay first
docker run --rm -p 7000:7000 scsibug/nostr-rs-relay

# Then run tests
cd grasp-audit
nix develop -c cargo test --ignored

# ❌ Wrong - integration tests will fail
cargo test --ignored  # No relay running
```

**Test Organization:**
```rust
// Unit tests (no relay needed)
#[cfg(test)]
mod tests {
    #[test]
    fn test_something() { }
}

// Integration tests (relay required)
#[cfg(test)]
mod tests {
    #[test]
    #[ignore]  // Requires relay
    fn test_against_relay() { }
}
```

### Documentation Updates

**Keep README.md synchronized**

When you:
- Complete a major feature → Update README.md status
- Change architecture → Update docs/ARCHITECTURE.md
- Add dependencies → Update README.md tech stack
- Change workflow → Update docs/GETTING_STARTED.md

**Don't:**
- Create duplicate documentation
- Leave stale status markers
- Forget to update CHANGELOG.md for user-facing changes

---

## 📝 Writing Guidelines

### Markdown Style

```markdown
# Title (H1 - only one per file)

**Date:** YYYY-MM-DD  
**Status:** [WIP|COMPLETE|ARCHIVED]

## Section (H2)

### Subsection (H3)

**Bold** for emphasis, `code` for commands/code.

- Bullet lists for items
- Keep consistent style

1. Numbered lists for sequences
2. Use when order matters

✅ Use emoji for status (sparingly)
❌ Don't overuse emoji

\`\`\`bash
# Code blocks with language
cargo build
\`\`\`
```

### Status Markers

- `[WIP]` - Work in progress
- `[COMPLETE]` - Finished, may be archived
- `[ARCHIVED]` - Moved to archive, historical only
- `✅` - Success/complete
- `❌` - Failure/incorrect
- `⏳` - In progress
- `🔜` - Planned/next

### Document Headers

```markdown
# Document Title

**Purpose:** One-line purpose  
**Date:** YYYY-MM-DD  
**Status:** [WIP|COMPLETE|ARCHIVED]  
**Related:** Links to related docs

---

## Content starts here
```

---

## 🤖 AI Agent Responsibilities

### Before Creating New Documents

1. **Check if document already exists**
   ```bash
   find . -name "*keyword*.md"
   ```

2. **Check if information can be added to existing doc**
   - Prefer updating over creating
   - Use sections/subsections

3. **Determine correct location**
   - Working doc → Root
   - Permanent → docs/
   - Learning → docs/learnings/
   - Historical → docs/archive/

4. **Use descriptive names with dates**
   - `YYYY-MM-DD-description.md` for working docs
   - `topic-name.md` for permanent docs

### During Development

1. **Update status markers**
   - Mark WIP → COMPLETE when done
   - Update README.md status section

2. **Extract learnings as you go**
   - Add gotchas to docs/learnings/
   - Don't wait until cleanup

3. **Keep documentation DRY**
   - Link to existing docs
   - Don't duplicate information

### End of Session

1. **Suggest cleanup if needed**
   - Count root .md files
   - Suggest archiving completed docs

2. **Create session summary**
   - What was accomplished
   - What's next
   - Any blockers

3. **Update permanent docs**
   - Sync README.md with reality
   - Update relevant docs/ files

### Cleanup Time

1. **Review all root .md files**
2. **Extract learnings to docs/learnings/**
3. **Archive completed work to docs/archive/**
4. **Delete obsolete duplicates**
5. **Update links in active docs**
6. **Commit with clear message**

---

## 🎯 Quality Checklist

### For Every Document

- [ ] Clear purpose stated at top
- [ ] Date included
- [ ] Status marker present
- [ ] Proper heading hierarchy (H1 → H2 → H3)
- [ ] Code blocks have language specified
- [ ] Links are valid and relative
- [ ] No duplicate information
- [ ] Spell-checked and readable

### For Working Documents

- [ ] Descriptive filename with date
- [ ] Will be archived or deleted when done
- [ ] Not duplicating permanent docs
- [ ] Learnings extracted to docs/learnings/

### For Permanent Documents

- [ ] In correct docs/ subdirectory
- [ ] Linked from docs/README.md
- [ ] Updated as project evolves
- [ ] No session-specific details
- [ ] Serves long-term purpose

### For Archived Documents

- [ ] Moved to docs/archive/
- [ ] Renamed with date prefix
- [ ] ARCHIVED marker at top
- [ ] Learnings extracted first
- [ ] Not referenced in active docs

---

## 📚 Reference Documents

### Must Read
- **This file (AGENTS.md)** - Guidelines for documentation
- **README.md** - Project overview
- **docs/README.md** - Documentation navigation

### Key Technical Docs
- **docs/ARCHITECTURE.md** - System design
- **docs/TEST_STRATEGY.md** - Testing approach
- **docs/GETTING_STARTED.md** - Setup guide

### Learnings (Gotchas)
- **docs/learnings/nix-flakes.md** - Nix flake patterns
- **docs/learnings/nostr-sdk.md** - nostr-sdk notes
- **docs/learnings/git-http-backend.md** - Git protocol tips

---

## 🔗 Quick Links

- [Project README](README.md)
- [Documentation Index](docs/README.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Learnings](docs/learnings/)
- [Archive](docs/archive/)

---

## 💡 Tips for Success

1. **Less is more** - Prefer updating over creating
2. **Archive often** - Keep root clean
3. **Extract learnings** - Make knowledge reusable
4. **Link, don't duplicate** - DRY applies to docs too
5. **Date everything** - Context is important
6. **Use descriptive names** - Future you will thank you
7. **Check before creating** - Document might already exist
8. **Update as you go** - Don't wait for cleanup time

---

## 🚀 Next Steps

After reading this:

1. **Review current documentation structure**
   ```bash
   ls -la *.md
   ls -la docs/
   ```

2. **Identify cleanup candidates**
   - Completed working docs
   - Obsolete duplicates
   - Session summaries

3. **Extract learnings**
   - Review completed docs
   - Add to docs/learnings/

4. **Archive and clean**
   - Move to docs/archive/
   - Delete obsolete files
   - Update links

5. **Commit changes**
   ```bash
   git add .
   git commit -m "docs: cleanup and reorganization"
   ```

---

**Remember:** Good documentation structure is like good code structure - it makes everything easier.

---

*Last updated: November 4, 2025*  
*Status: ✅ Active guidelines*
